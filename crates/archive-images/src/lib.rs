use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use shadow_zip_archive_core::{
    ArchiveBackend, EntryReader, InputScanner, OpenArchive, SafeWriter, StreamLimits,
    create_pipeline, quick_test_pipeline, random_access_extract_pipeline,
};
use shadow_zip_domain::*;

const SECTOR_SIZE: u64 = 2048;
const VOLUME_DESCRIPTOR_START: u64 = 16;
const CD001: &[u8; 5] = b"CD001";

pub struct ImageArchiveBackend;

impl ArchiveBackend for ImageArchiveBackend {
    fn name(&self) -> &'static str {
        "images"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        let detected = detect_image_format(source)?;
        Ok(ProbeResult {
            format: detected
                .as_ref()
                .map(|format| format.archive_format)
                .unwrap_or(ArchiveFormat::Unknown),
            confidence: detected
                .as_ref()
                .map(|format| format.confidence)
                .unwrap_or(ProbeConfidence::Impossible),
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        _options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        match detect_image_format(&source)? {
            Some(DetectedImageFormat {
                kind: ImageFormatKind::Iso9660,
                ..
            }) => Ok(Box::new(IsoArchive::open(source)?)),
            Some(DetectedImageFormat {
                kind:
                    ImageFormatKind::RawImage
                    | ImageFormatKind::Udf
                    | ImageFormatKind::Isz
                    | ImageFormatKind::Daa,
                ..
            }) => Ok(Box::new(UnsupportedImageArchive::new(source)?)),
            None => Err(unsupported_format("Unsupported disk image format")),
        }
    }

    fn create_plan(
        &self,
        inputs: &[InputPath],
        output: &Path,
        options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        if options.format != ArchiveFormat::Iso {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Image backend can only create basic ISO9660 images",
            )
            .with_backend("images")
            .with_technical_detail(format!("requested_format={}", options.format)));
        }
        Ok(
            TaskPlan::new(TaskKind::Create, format!("Create ISO image {}", output.display()))
                .estimated_entries(inputs.len())
                .native(create_pipeline())
                .warn(
                    "iso-writer-limited",
                    "Image backend creates a basic ISO9660 image without UDF, boot catalog, permissions, or symlink metadata",
                ),
        )
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![ArchiveFormat::Iso],
            capabilities: image_backend_capabilities(),
        }
    }
}

struct IsoArchive {
    source: ArchiveSource,
    info: ArchiveInfo,
    entries: Vec<IsoEntry>,
}

impl IsoArchive {
    fn open(source: ArchiveSource) -> Result<Self, ArchiveError> {
        let path = source.path().ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Image backend requires a local path",
            )
            .with_backend("images")
        })?;
        let mut file = File::open(path).map_err(io_error)?;
        let metadata = file.metadata().map_err(io_error)?;
        let volume = read_volume(&mut file)?;
        let entries = read_directory_tree(&mut file, &volume.root, volume.joliet)?;

        Ok(Self {
            source,
            info: ArchiveInfo {
                format: ArchiveFormat::Iso,
                display_name: "ISO9660 image".into(),
                total_bytes: Some(metadata.len()),
                entry_count: Some(entries.len() as u64),
                codecs: vec![if volume.joliet {
                    "ISO9660/Joliet".into()
                } else {
                    "ISO9660".into()
                }],
                filters: Vec::new(),
                is_solid: false,
                is_encrypted: false,
                has_header_encryption: false,
                is_multi_volume: false,
            },
            entries,
        })
    }

    fn listing_from_entries(&self) -> ArchiveListing {
        let entries = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| ArchiveEntry {
                id: EntryId(index as u64),
                raw_path: entry.path.clone(),
                normalized_path: entry.path.clone(),
                display_path: entry.path.clone(),
                kind: entry.kind,
                size: Some(entry.size),
                compressed_size: Some(entry.size),
                modified_at: None,
                method: Some("store".into()),
                encrypted: false,
                safety: classify_entry_path(&entry.path),
            })
            .collect::<Vec<_>>();
        ArchiveListing {
            directories: directories_for(&entries),
            entries,
            is_complete: true,
        }
    }

    fn selected_entries(
        &self,
        selected: Option<&[EntryId]>,
    ) -> Result<Vec<(EntryId, IsoEntry)>, ArchiveError> {
        let wanted = selected
            .map(|entries| entries.iter().copied().collect::<BTreeSet<_>>())
            .unwrap_or_default();
        let mut out = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            let id = EntryId(index as u64);
            if selected.is_none() || wanted.contains(&id) {
                out.push((id, entry.clone()));
            }
        }
        Ok(out)
    }
}

impl OpenArchive for IsoArchive {
    fn info(&self) -> ArchiveInfo {
        let mut info = self.info.clone();
        info.display_name = self.source.display_name();
        info
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        iso_capabilities()
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        Ok(self.listing_from_entries())
    }

    fn extract_all(
        &mut self,
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        self.extract_to(destination, None, options)
    }

    fn extract_selected(
        &mut self,
        entries: &[EntryId],
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        self.extract_to(destination, Some(entries), options)
    }

    fn open_entry_stream(
        &mut self,
        entry: EntryId,
        _options: StreamOptions,
    ) -> Result<EntryStream, ArchiveError> {
        self.entries
            .get(entry.0 as usize)
            .filter(|entry| entry.kind == EntryKind::File)
            .ok_or_else(|| {
                ArchiveError::new(
                    ArchiveErrorKind::UnsupportedFormat,
                    "Entry does not expose file bytes",
                )
                .with_backend("images")
            })?;
        Ok(EntryStream {
            entry,
            access_cost: AccessCost::Random,
        })
    }

    fn open_entry_reader(
        &mut self,
        entry: EntryId,
        _options: StreamOptions,
    ) -> Result<EntryReader, ArchiveError> {
        let iso_entry = self
            .entries
            .get(entry.0 as usize)
            .filter(|entry| entry.kind == EntryKind::File)
            .ok_or_else(|| {
                ArchiveError::new(
                    ArchiveErrorKind::UnsupportedFormat,
                    "Entry does not expose file bytes",
                )
                .with_backend("images")
            })?;
        let mut file = open_local_file(&self.source)?;
        let bytes = read_extent(&mut file, iso_entry.offset, iso_entry.size)?;
        Ok(EntryReader {
            entry,
            access_cost: AccessCost::Random,
            source: Box::new(Cursor::new(bytes)),
            size: Some(iso_entry.size),
        })
    }

    fn test(&mut self, _options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        let mut file = open_local_file(&self.source)?;
        for entry in &self.entries {
            if entry.kind == EntryKind::File {
                file.seek(SeekFrom::Start(entry.offset)).map_err(io_error)?;
                io::copy(
                    &mut std::io::Read::by_ref(&mut file).take(entry.size),
                    &mut io::sink(),
                )
                .map_err(io_error)?;
            }
        }
        Ok(
            TaskPlan::new(TaskKind::Test, "Test ISO image").native(quick_test_pipeline(vec![
                PipelineStep::ProbeArchive,
                PipelineStep::ReadCentralDirectory,
            ])),
        )
    }
}

impl IsoArchive {
    fn extract_to(
        &mut self,
        destination: &Path,
        selected: Option<&[EntryId]>,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let entries = self.selected_entries(selected)?;
        let mut file = open_local_file(&self.source)?;
        let writer = SafeWriter::new(destination.to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(options.overwrite_policy);
        for (_, entry) in &entries {
            if entry.kind == EntryKind::Directory {
                writer.create_dir(&entry.path)?;
                continue;
            }
            let mut source = ExtentReader::new(&mut file, entry.offset, entry.size)?;
            writer.write_stream(&entry.path, &mut source, |_| Ok(()))?;
        }
        Ok(TaskPlan::new(
            TaskKind::Extract,
            format!("Extract to {}", destination.display()),
        )
        .estimated_entries(entries.len())
        .native(random_access_extract_pipeline(
            PipelineStep::ReadCentralDirectory,
        )))
    }
}

struct UnsupportedImageArchive {
    source: ArchiveSource,
    kind: ImageFormatKind,
    archive_format: ArchiveFormat,
    total_bytes: Option<u64>,
}

impl UnsupportedImageArchive {
    fn new(source: ArchiveSource) -> Result<Self, ArchiveError> {
        let detected = detect_image_format(&source)?
            .ok_or_else(|| unsupported_format("Unsupported disk image format"))?;
        let total_bytes = source
            .path()
            .and_then(|path| path.metadata().ok())
            .map(|metadata| metadata.len());
        Ok(Self {
            source,
            kind: detected.kind,
            archive_format: detected.archive_format,
            total_bytes,
        })
    }

    fn unsupported(&self) -> ArchiveError {
        let (kind, message) = match self.kind {
            ImageFormatKind::Udf => (
                ArchiveErrorKind::UnsupportedFormat,
                "UDF filesystem images are recognized but not yet readable by the native image backend",
            ),
            ImageFormatKind::Isz => (
                ArchiveErrorKind::UnsupportedCodec,
                "ISZ compressed images are recognized but the compression codec is not implemented",
            ),
            ImageFormatKind::Daa => (
                ArchiveErrorKind::UnsupportedCodec,
                "DAA compressed images are recognized but the container codec is not implemented",
            ),
            ImageFormatKind::RawImage => (
                ArchiveErrorKind::UnsupportedFormat,
                "Raw BIN/IMG images are recognized but no supported filesystem was found",
            ),
            ImageFormatKind::Iso9660 => (
                ArchiveErrorKind::Internal,
                "ISO9660 images should be handled by the ISO reader",
            ),
        };
        ArchiveError::new(kind, message)
            .with_backend("images")
            .with_technical_detail(format!("detected_format={}", self.kind.label()))
    }
}

impl OpenArchive for UnsupportedImageArchive {
    fn info(&self) -> ArchiveInfo {
        ArchiveInfo {
            format: self.archive_format,
            display_name: self.source.display_name(),
            total_bytes: self.total_bytes,
            entry_count: None,
            codecs: vec![self.kind.label().into()],
            filters: Vec::new(),
            is_solid: false,
            is_encrypted: false,
            has_header_encryption: false,
            is_multi_volume: false,
        }
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        raw_or_unsupported_capabilities()
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        Err(self.unsupported())
    }

    fn extract_all(
        &mut self,
        _destination: &Path,
        _options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        Err(self.unsupported())
    }

    fn extract_selected(
        &mut self,
        _entries: &[EntryId],
        _destination: &Path,
        _options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        Err(self.unsupported())
    }

    fn open_entry_stream(
        &mut self,
        _entry: EntryId,
        _options: StreamOptions,
    ) -> Result<EntryStream, ArchiveError> {
        Err(self.unsupported())
    }

    fn test(&mut self, _options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        Err(self.unsupported())
    }
}

#[derive(Debug, Clone)]
struct IsoEntry {
    path: String,
    kind: EntryKind,
    offset: u64,
    size: u64,
}

#[derive(Debug, Clone)]
struct DirectoryRecord {
    extent: u32,
    size: u32,
    flags: u8,
    identifier: Vec<u8>,
}

#[derive(Debug, Clone)]
struct IsoVolume {
    root: DirectoryRecord,
    joliet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageFormatKind {
    Iso9660,
    Udf,
    RawImage,
    Isz,
    Daa,
}

impl ImageFormatKind {
    fn label(self) -> &'static str {
        match self {
            Self::Iso9660 => "ISO9660",
            Self::Udf => "UDF",
            Self::RawImage => "raw-image",
            Self::Isz => "ISZ",
            Self::Daa => "DAA",
        }
    }
}

#[derive(Debug, Clone)]
struct DetectedImageFormat {
    kind: ImageFormatKind,
    archive_format: ArchiveFormat,
    confidence: ProbeConfidence,
}

fn detect_image_format(
    source: &ArchiveSource,
) -> Result<Option<DetectedImageFormat>, ArchiveError> {
    let Some(path) = source.path() else {
        return Ok(None);
    };
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    if has_iso9660_signature(&mut file)? {
        return Ok(Some(DetectedImageFormat {
            kind: ImageFormatKind::Iso9660,
            archive_format: ArchiveFormat::Iso,
            confidence: ProbeConfidence::Strong,
        }));
    }
    if has_udf_signature(&mut file)? {
        return Ok(Some(DetectedImageFormat {
            kind: ImageFormatKind::Udf,
            archive_format: ArchiveFormat::Udf,
            confidence: ProbeConfidence::Signature,
        }));
    }
    if has_prefix(&mut file, b"IsZ!")? {
        return Ok(Some(DetectedImageFormat {
            kind: ImageFormatKind::Isz,
            archive_format: ArchiveFormat::Isz,
            confidence: ProbeConfidence::Signature,
        }));
    }
    if extension_matches(path, &["isz"]) {
        return Ok(Some(DetectedImageFormat {
            kind: ImageFormatKind::Isz,
            archive_format: ArchiveFormat::Isz,
            confidence: ProbeConfidence::Extension,
        }));
    }
    if has_prefix(&mut file, b"DAA")? {
        return Ok(Some(DetectedImageFormat {
            kind: ImageFormatKind::Daa,
            archive_format: ArchiveFormat::Daa,
            confidence: ProbeConfidence::Signature,
        }));
    }
    if extension_matches(path, &["daa"]) {
        return Ok(Some(DetectedImageFormat {
            kind: ImageFormatKind::Daa,
            archive_format: ArchiveFormat::Daa,
            confidence: ProbeConfidence::Extension,
        }));
    }
    if extension_matches(path, &["udf"]) {
        return Ok(Some(DetectedImageFormat {
            kind: ImageFormatKind::Udf,
            archive_format: ArchiveFormat::Udf,
            confidence: ProbeConfidence::Extension,
        }));
    }
    if extension_matches(path, &["img", "bin"]) {
        return Ok(Some(DetectedImageFormat {
            kind: ImageFormatKind::RawImage,
            archive_format: if extension_matches(path, &["bin"]) {
                ArchiveFormat::Bin
            } else {
                ArchiveFormat::Img
            },
            confidence: ProbeConfidence::Extension,
        }));
    }
    if extension_matches(path, &["iso"]) {
        return Ok(Some(DetectedImageFormat {
            kind: ImageFormatKind::Iso9660,
            archive_format: ArchiveFormat::Iso,
            confidence: ProbeConfidence::Extension,
        }));
    }
    Ok(None)
}

fn has_iso9660_signature(file: &mut File) -> Result<bool, ArchiveError> {
    let mut sector = [0_u8; SECTOR_SIZE as usize];
    for index in 0..32 {
        let offset = (VOLUME_DESCRIPTOR_START + index) * SECTOR_SIZE;
        if file.seek(SeekFrom::Start(offset)).is_err() {
            return Ok(false);
        }
        if file.read_exact(&mut sector).is_err() {
            return Ok(false);
        }
        if &sector[1..6] == CD001 {
            return Ok(true);
        }
        if sector[0] == 255 {
            return Ok(false);
        }
    }
    Ok(false)
}

fn has_udf_signature(file: &mut File) -> Result<bool, ArchiveError> {
    let mut sector = [0_u8; SECTOR_SIZE as usize];
    for sector_index in 16..64 {
        file.seek(SeekFrom::Start(sector_index * SECTOR_SIZE))
            .map_err(io_error)?;
        if file.read_exact(&mut sector).is_err() {
            return Ok(false);
        }
        if &sector[1..6] == b"NSR02" || &sector[1..6] == b"NSR03" {
            return Ok(true);
        }
    }
    Ok(false)
}

fn has_prefix(file: &mut File, prefix: &[u8]) -> Result<bool, ArchiveError> {
    file.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut bytes = vec![0_u8; prefix.len()];
    if file.read_exact(&mut bytes).is_err() {
        return Ok(false);
    }
    Ok(bytes == prefix)
}

fn read_volume(file: &mut File) -> Result<IsoVolume, ArchiveError> {
    let mut pvd_root = None;
    let mut joliet_root = None;
    let mut sector = [0_u8; SECTOR_SIZE as usize];

    for index in 0..128 {
        let offset = (VOLUME_DESCRIPTOR_START + index) * SECTOR_SIZE;
        file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        file.read_exact(&mut sector).map_err(io_error)?;
        if &sector[1..6] != CD001 {
            continue;
        }
        match sector[0] {
            1 => pvd_root = Some(parse_directory_record(&sector[156..])?),
            2 if is_joliet_descriptor(&sector) => {
                joliet_root = Some(parse_directory_record(&sector[156..])?)
            }
            255 => break,
            _ => {}
        }
    }

    if let Some(root) = joliet_root {
        return Ok(IsoVolume { root, joliet: true });
    }
    pvd_root
        .map(|root| IsoVolume {
            root,
            joliet: false,
        })
        .ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::CorruptArchive,
                "ISO9660 primary volume descriptor was not found",
            )
            .with_backend("images")
        })
}

fn is_joliet_descriptor(sector: &[u8]) -> bool {
    matches!(&sector[88..91], b"%/@" | b"%/C" | b"%/E")
}

fn read_directory_tree(
    file: &mut File,
    root: &DirectoryRecord,
    joliet: bool,
) -> Result<Vec<IsoEntry>, ArchiveError> {
    let mut entries = Vec::new();
    read_directory(file, root, "", joliet, &mut entries)?;
    Ok(entries)
}

fn read_directory(
    file: &mut File,
    directory: &DirectoryRecord,
    parent: &str,
    joliet: bool,
    entries: &mut Vec<IsoEntry>,
) -> Result<(), ArchiveError> {
    let bytes = read_extent(
        file,
        directory.extent as u64 * SECTOR_SIZE,
        directory.size as u64,
    )?;
    let mut offset = 0;
    while offset < bytes.len() {
        let length = bytes[offset] as usize;
        if length == 0 {
            offset = ((offset / SECTOR_SIZE as usize) + 1) * SECTOR_SIZE as usize;
            continue;
        }
        if offset + length > bytes.len() {
            break;
        }
        let record = parse_directory_record(&bytes[offset..offset + length])?;
        offset += length;
        let Some(name) = decode_identifier(&record.identifier, joliet) else {
            continue;
        };
        let path = join_iso_path(parent, &name);
        let is_directory = record.flags & 0x02 != 0;
        entries.push(IsoEntry {
            path: path.clone(),
            kind: if is_directory {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            offset: record.extent as u64 * SECTOR_SIZE,
            size: record.size as u64,
        });
        if is_directory {
            read_directory(file, &record, &path, joliet, entries)?;
        }
    }
    Ok(())
}

fn parse_directory_record(bytes: &[u8]) -> Result<DirectoryRecord, ArchiveError> {
    if bytes.len() < 34 || bytes[0] == 0 {
        return Err(corrupt_iso("Invalid ISO9660 directory record"));
    }
    let length = bytes[0] as usize;
    if bytes.len() < length || length < 34 {
        return Err(corrupt_iso("Truncated ISO9660 directory record"));
    }
    let identifier_len = bytes[32] as usize;
    let identifier_start = 33;
    let identifier_end = identifier_start + identifier_len;
    if identifier_end > length {
        return Err(corrupt_iso("Invalid ISO9660 identifier length"));
    }
    Ok(DirectoryRecord {
        extent: le_u32(&bytes[2..6]),
        size: le_u32(&bytes[10..14]),
        flags: bytes[25],
        identifier: bytes[identifier_start..identifier_end].to_vec(),
    })
}

fn decode_identifier(identifier: &[u8], joliet: bool) -> Option<String> {
    if identifier == [0] || identifier == [1] {
        return None;
    }
    let name = if joliet {
        let units = identifier
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(identifier).into_owned()
    };
    let trimmed = name
        .trim_end_matches(";1")
        .trim_end_matches('.')
        .replace('\\', "/");
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn join_iso_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.into()
    } else {
        format!("{parent}/{name}")
    }
}

fn read_extent(file: &mut File, offset: u64, size: u64) -> Result<Vec<u8>, ArchiveError> {
    file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
    let mut bytes = vec![0_u8; size as usize];
    file.read_exact(&mut bytes).map_err(io_error)?;
    Ok(bytes)
}

struct ExtentReader<'a> {
    file: &'a mut File,
    remaining: u64,
}

impl<'a> ExtentReader<'a> {
    fn new(file: &'a mut File, offset: u64, size: u64) -> Result<Self, ArchiveError> {
        file.seek(SeekFrom::Start(offset)).map_err(io_error)?;
        Ok(Self {
            file,
            remaining: size,
        })
    }
}

impl Read for ExtentReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let limit = buffer.len().min(self.remaining as usize);
        let read = self.file.read(&mut buffer[..limit])?;
        self.remaining -= read as u64;
        Ok(read)
    }
}

pub fn create_iso_archive(
    inputs: &[InputPath],
    output: PathBuf,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    if options.format != ArchiveFormat::Iso {
        return Err(ArchiveError::new(
            ArchiveErrorKind::UnsupportedFormat,
            "Image backend can only create basic ISO9660 images",
        )
        .with_backend("images")
        .with_technical_detail(format!("requested_format={}", options.format)));
    }
    let scanned = InputScanner::scan(inputs)?;
    let image = IsoImageBuilder::from_scanned(scanned)?;
    let mut file = File::create(output).map_err(io_error)?;
    image.write(&mut file)
}

struct IsoImageBuilder {
    entries: Vec<BuildEntry>,
    root_dir_sector: u32,
    root_dir_size: u32,
}

#[derive(Clone)]
struct BuildEntry {
    archive_path: String,
    source_path: PathBuf,
    is_dir: bool,
    size: u64,
    sector: u32,
}

impl IsoImageBuilder {
    fn from_scanned(
        scanned: Vec<shadow_zip_archive_core::ScannedInput>,
    ) -> Result<Self, ArchiveError> {
        let mut entries = scanned
            .into_iter()
            .map(|input| BuildEntry {
                archive_path: sanitize_iso_path(&input.archive_path),
                source_path: input.source_path,
                is_dir: input.is_dir,
                size: input.size.unwrap_or(0),
                sector: 0,
            })
            .collect::<Vec<_>>();
        if entries.iter().any(|entry| entry.is_dir) {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Basic ISO writer does not yet serialize directory trees",
            )
            .with_backend("images"));
        }
        if entries.iter().any(|entry| entry.archive_path.contains('/')) {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Basic ISO writer currently supports files in the image root only",
            )
            .with_backend("images"));
        }
        entries.sort_by(|left, right| left.archive_path.cmp(&right.archive_path));

        let root_dir_size = directory_table_size(&entries, "");
        let mut sector = 18 + sectors_for(root_dir_size as u64) as u32;
        for entry in &mut entries {
            if !entry.is_dir {
                entry.sector = sector;
                sector += sectors_for(entry.size) as u32;
            }
        }
        Ok(Self {
            entries,
            root_dir_sector: 18,
            root_dir_size,
        })
    }

    fn write(&self, writer: &mut File) -> Result<(), ArchiveError> {
        writer
            .write_all(&vec![0; (16 * SECTOR_SIZE) as usize])
            .map_err(io_error)?;
        writer
            .write_all(&self.primary_volume_descriptor())
            .map_err(io_error)?;
        writer
            .write_all(&terminator_descriptor())
            .map_err(io_error)?;
        writer
            .write_all(&self.root_directory_sector())
            .map_err(io_error)?;
        for entry in &self.entries {
            if entry.is_dir {
                continue;
            }
            let mut source = File::open(&entry.source_path).map_err(io_error)?;
            io::copy(&mut source, writer).map_err(io_error)?;
            pad_to_sector(writer, entry.size)?;
        }
        Ok(())
    }

    fn primary_volume_descriptor(&self) -> Vec<u8> {
        let mut sector = vec![0_u8; SECTOR_SIZE as usize];
        sector[0] = 1;
        sector[1..6].copy_from_slice(CD001);
        sector[6] = 1;
        write_a_chars(&mut sector[8..40], "SHADOW_ZIP");
        write_d_chars(&mut sector[40..72], "SHADOW_ZIP_ISO");
        write_both_u32(&mut sector[80..88], self.volume_space_size());
        sector[120] = 1;
        sector[124] = 1;
        write_both_u16(&mut sector[120..124], 1);
        write_both_u16(&mut sector[124..128], 1);
        write_both_u16(&mut sector[128..132], SECTOR_SIZE as u16);
        let root = directory_record(0, self.root_dir_sector, self.root_dir_size, true, "");
        sector[156..156 + root.len()].copy_from_slice(&root);
        sector
    }

    fn root_directory_sector(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(directory_record(
            0,
            self.root_dir_sector,
            self.root_dir_size,
            true,
            "",
        ));
        bytes.extend(directory_record(
            1,
            self.root_dir_sector,
            self.root_dir_size,
            true,
            "",
        ));
        for entry in self
            .entries
            .iter()
            .filter(|entry| !entry.archive_path.contains('/'))
        {
            bytes.extend(directory_record(
                0,
                if entry.is_dir {
                    self.root_dir_sector
                } else {
                    entry.sector
                },
                if entry.is_dir { 0 } else { entry.size as u32 },
                entry.is_dir,
                &iso_file_identifier(&entry.archive_path, entry.is_dir),
            ));
        }
        pad_vec_to_sector(&mut bytes);
        bytes
    }

    fn volume_space_size(&self) -> u32 {
        let data_sectors = self
            .entries
            .iter()
            .filter(|entry| !entry.is_dir)
            .map(|entry| sectors_for(entry.size) as u32)
            .sum::<u32>();
        18 + sectors_for(self.root_dir_size as u64) as u32 + data_sectors
    }
}

fn directory_table_size(entries: &[BuildEntry], parent: &str) -> u32 {
    let mut size =
        directory_record(0, 0, 0, true, "").len() + directory_record(1, 0, 0, true, "").len();
    for entry in entries.iter().filter(|entry| {
        if parent.is_empty() {
            !entry.archive_path.contains('/')
        } else {
            entry
                .archive_path
                .strip_prefix(parent)
                .is_some_and(|tail| !tail.trim_start_matches('/').contains('/'))
        }
    }) {
        size += directory_record(
            0,
            0,
            0,
            entry.is_dir,
            &iso_file_identifier(&entry.archive_path, entry.is_dir),
        )
        .len();
    }
    size as u32
}

fn directory_record(
    special: u8,
    sector: u32,
    size: u32,
    is_dir: bool,
    identifier: &str,
) -> Vec<u8> {
    let id_bytes = if special <= 1 && identifier.is_empty() {
        vec![special]
    } else {
        identifier.as_bytes().to_vec()
    };
    let len = 33 + id_bytes.len() + usize::from(id_bytes.len() % 2 == 0);
    let mut bytes = vec![0_u8; len];
    bytes[0] = len as u8;
    write_both_u32(&mut bytes[2..10], sector);
    write_both_u32(&mut bytes[10..18], size);
    bytes[18..25].copy_from_slice(&[126, 1, 1, 0, 0, 0, 0]);
    bytes[25] = if is_dir { 0x02 } else { 0 };
    bytes[28] = 1;
    bytes[31] = 1;
    bytes[32] = id_bytes.len() as u8;
    bytes[33..33 + id_bytes.len()].copy_from_slice(&id_bytes);
    bytes
}

fn terminator_descriptor() -> Vec<u8> {
    let mut sector = vec![0_u8; SECTOR_SIZE as usize];
    sector[0] = 255;
    sector[1..6].copy_from_slice(CD001);
    sector[6] = 1;
    sector
}

fn iso_file_identifier(path: &str, is_dir: bool) -> String {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_uppercase();
    if is_dir || name.contains(';') {
        name
    } else {
        format!("{name};1")
    }
}

fn sanitize_iso_path(path: &str) -> String {
    path.replace('\\', "/")
        .split('/')
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(|part| {
            part.chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
                        ch.to_ascii_uppercase()
                    } else {
                        '_'
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn pad_to_sector(writer: &mut File, size: u64) -> Result<(), ArchiveError> {
    let padding = (SECTOR_SIZE - (size % SECTOR_SIZE)) % SECTOR_SIZE;
    if padding > 0 {
        writer
            .write_all(&vec![0; padding as usize])
            .map_err(io_error)?;
    }
    Ok(())
}

fn pad_vec_to_sector(bytes: &mut Vec<u8>) {
    let padding =
        (SECTOR_SIZE as usize - (bytes.len() % SECTOR_SIZE as usize)) % SECTOR_SIZE as usize;
    bytes.extend(std::iter::repeat_n(0, padding));
}

fn sectors_for(size: u64) -> u64 {
    size.div_ceil(SECTOR_SIZE)
}

fn write_a_chars(target: &mut [u8], text: &str) {
    write_space_padded(target, text);
}

fn write_d_chars(target: &mut [u8], text: &str) {
    write_space_padded(target, text);
}

fn write_space_padded(target: &mut [u8], text: &str) {
    target.fill(b' ');
    let bytes = text.as_bytes();
    let len = bytes.len().min(target.len());
    target[..len].copy_from_slice(&bytes[..len]);
}

fn write_both_u16(target: &mut [u8], value: u16) {
    target[..2].copy_from_slice(&value.to_le_bytes());
    target[2..4].copy_from_slice(&value.to_be_bytes());
}

fn write_both_u32(target: &mut [u8], value: u32) {
    target[..4].copy_from_slice(&value.to_le_bytes());
    target[4..8].copy_from_slice(&value.to_be_bytes());
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn directories_for(entries: &[ArchiveEntry]) -> BTreeMap<String, Vec<EntryId>> {
    let mut directories: BTreeMap<String, Vec<EntryId>> = BTreeMap::new();
    directories.entry("/".into()).or_default();
    for entry in entries {
        let parent = entry
            .normalized_path
            .rsplit_once('/')
            .map(|(parent, _)| format!("/{parent}"))
            .unwrap_or_else(|| "/".into());
        directories.entry(parent).or_default().push(entry.id);
    }
    directories
}

fn extension_matches(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            extensions
                .iter()
                .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        })
}

fn open_local_file(source: &ArchiveSource) -> Result<File, ArchiveError> {
    let path = source.path().ok_or_else(|| {
        ArchiveError::new(
            ArchiveErrorKind::UnsupportedFormat,
            "Image backend requires a local path",
        )
        .with_backend("images")
    })?;
    File::open(path).map_err(io_error)
}

fn image_backend_capabilities() -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Limited,
        extract_all: CapabilityLevel::Limited,
        extract_selected: CapabilityLevel::Limited,
        create: CapabilityLevel::Limited,
        update: CapabilityLevel::Unsupported,
        random_access: CapabilityLevel::Limited,
        password_read: CapabilityLevel::Unsupported,
        password_write: CapabilityLevel::Unsupported,
        header_encryption: CapabilityLevel::Unsupported,
        multi_volume_read: CapabilityLevel::Unsupported,
        multi_volume_write: CapabilityLevel::Unsupported,
        entry_stream_preview: CapabilityLevel::Limited,
    }
}

fn iso_capabilities() -> ArchiveCapabilities {
    image_backend_capabilities()
}

fn raw_or_unsupported_capabilities() -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Unsupported,
        extract_all: CapabilityLevel::Unsupported,
        extract_selected: CapabilityLevel::Unsupported,
        create: CapabilityLevel::Unsupported,
        update: CapabilityLevel::Unsupported,
        random_access: CapabilityLevel::Limited,
        password_read: CapabilityLevel::Unsupported,
        password_write: CapabilityLevel::Unsupported,
        header_encryption: CapabilityLevel::Unsupported,
        multi_volume_read: CapabilityLevel::Unsupported,
        multi_volume_write: CapabilityLevel::Unsupported,
        entry_stream_preview: CapabilityLevel::Unsupported,
    }
}

fn unsupported_format(message: impl Into<String>) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::UnsupportedFormat, message).with_backend("images")
}

fn corrupt_iso(message: impl Into<String>) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::CorruptArchive, message).with_backend("images")
}

fn io_error(error: io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "I/O operation failed")
        .with_backend("images")
        .with_technical_detail(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_options() -> CreateOptions {
        CreateOptions {
            format: ArchiveFormat::Iso,
            compression_method: None,
            compression_level: None,
            solid: false,
            encrypt_file_names: false,
            password: None,
            volume_size: None,
            symlink_policy: SymlinkPolicy::Conservative,
        }
    }

    #[test]
    fn writes_and_reads_basic_iso() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("hello.txt");
        std::fs::write(&input, b"hello iso").unwrap();
        let iso = dir.path().join("out.iso");

        create_iso_archive(
            &[InputPath {
                path: input,
                archive_path: Some("hello.txt".into()),
            }],
            iso.clone(),
            create_options(),
        )
        .unwrap();

        let backend = ImageArchiveBackend;
        let source = ArchiveSource::LocalPath(iso);
        assert_eq!(
            backend.probe(&source).unwrap().confidence,
            ProbeConfidence::Strong
        );
        let mut archive = backend.open(source, OpenOptions::default()).unwrap();
        let listing = archive.listing(ListingMode::Full).unwrap();
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].raw_path, "HELLO.TXT");

        let reader = archive
            .open_entry_reader(listing.entries[0].id, StreamOptions::default())
            .unwrap();
        let mut source = reader.source;
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4];
        loop {
            let read = source.read_chunk(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        assert_eq!(bytes, b"hello iso");
    }

    #[test]
    fn raw_img_is_recognized_but_not_listed() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("disk.img");
        std::fs::write(&img, b"not a filesystem").unwrap();
        let source = ArchiveSource::LocalPath(img);

        let backend = ImageArchiveBackend;
        assert_eq!(
            backend.probe(&source).unwrap().confidence,
            ProbeConfidence::Extension
        );
        let mut archive = backend.open(source, OpenOptions::default()).unwrap();
        let error = archive.listing(ListingMode::Full).unwrap_err();
        assert_eq!(error.kind, ArchiveErrorKind::UnsupportedFormat);
    }
}
