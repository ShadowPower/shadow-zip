use std::{
    fs::File,
    io::{self, Cursor, Read},
    path::Path,
};

use ar::Archive as ArArchive;
use asar::AsarReader;
use cab::Cabinet;
use cfb::CompoundFile;
use flate2::read::GzDecoder;
use shadow_zip_archive_core::{
    ArchiveBackend, EntryReader, OpenArchive, SafeWriter, StreamLimits, extension_confidence,
    quick_test_pipeline, random_access_extract_pipeline, sequential_extract_pipeline,
};
use shadow_zip_domain::*;
use tar::Archive as TarReader;
use xz2::read::XzDecoder;
use zip::read::{ZipArchive as ZipReader, ZipFile};

pub struct ContainersBackend;

impl ArchiveBackend for ContainersBackend {
    fn name(&self) -> &'static str {
        "containers"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        let detected = detect_format(source);
        Ok(ProbeResult {
            format: archive_format(detected),
            confidence: probe_confidence(source, detected),
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        _options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        let format = detect_format(&source);
        if matches!(format, ContainerFormat::Unknown) {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Container backend does not recognize this source",
            )
            .with_backend(self.name()));
        }
        Ok(Box::new(ContainerArchive {
            source,
            format,
            listing_cache: None,
            asar_bytes: None,
        }))
    }

    fn create_plan(
        &self,
        _inputs: &[InputPath],
        output: &Path,
        _options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        Err(ArchiveError::new(
            ArchiveErrorKind::UnsupportedFormat,
            format!(
                "Creating container/package format archives is not supported: {}",
                output.display()
            ),
        )
        .with_backend(self.name()))
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![
                ArchiveFormat::Cab,
                ArchiveFormat::Msi,
                ArchiveFormat::Compound,
                ArchiveFormat::Deb,
                ArchiveFormat::Xpi,
                ArchiveFormat::Asar,
            ],
            capabilities: container_backend_capabilities(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerFormat {
    Cab,
    Compound,
    Msi,
    Deb,
    Xpi,
    Asar,
    Unknown,
}

struct ContainerArchive {
    source: ArchiveSource,
    format: ContainerFormat,
    listing_cache: Option<ArchiveListing>,
    asar_bytes: Option<Vec<u8>>,
}

impl OpenArchive for ContainerArchive {
    fn info(&self) -> ArchiveInfo {
        ArchiveInfo {
            format: archive_format(self.format),
            display_name: self.source.display_name(),
            total_bytes: self
                .source
                .path()
                .and_then(|path| fs_err::metadata(path).ok())
                .map(|metadata| metadata.len()),
            entry_count: self
                .listing_cache
                .as_ref()
                .map(|listing| listing.entries.len() as u64),
            codecs: codecs(self.format),
            filters: Vec::new(),
            is_solid: matches!(self.format, ContainerFormat::Cab | ContainerFormat::Deb),
            is_encrypted: false,
            has_header_encryption: false,
            is_multi_volume: false,
        }
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        container_capabilities(self.format)
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        if let Some(listing) = &self.listing_cache {
            return Ok(listing.clone());
        }
        let listing = match self.format {
            ContainerFormat::Cab => read_cab_listing(self.path()?)?,
            ContainerFormat::Compound | ContainerFormat::Msi => {
                read_compound_listing(self.path()?)?
            }
            ContainerFormat::Deb => read_deb_listing(self.path()?)?,
            ContainerFormat::Xpi => read_zip_listing(self.path()?, "xpi")?,
            ContainerFormat::Asar => {
                let bytes = fs_err::read(self.path()?).map_err(io_error)?;
                let listing = read_asar_listing(&bytes)?;
                self.asar_bytes = Some(bytes);
                listing
            }
            ContainerFormat::Unknown => {
                return Err(unsupported("Unknown container format"));
            }
        };
        self.listing_cache = Some(listing.clone());
        Ok(listing)
    }

    fn extract_all(
        &mut self,
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        self.extract(destination, None, options)
    }

    fn extract_selected(
        &mut self,
        entries: &[EntryId],
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        self.extract(destination, Some(entries), options)
    }

    fn open_entry_stream(
        &mut self,
        entry: EntryId,
        _options: StreamOptions,
    ) -> Result<EntryStream, ArchiveError> {
        Ok(EntryStream {
            entry,
            access_cost: match self.format {
                ContainerFormat::Xpi
                | ContainerFormat::Asar
                | ContainerFormat::Compound
                | ContainerFormat::Msi => AccessCost::Random,
                ContainerFormat::Cab | ContainerFormat::Deb => AccessCost::SequentialFromStart,
                ContainerFormat::Unknown => AccessCost::SequentialFromStart,
            },
        })
    }

    fn open_entry_reader(
        &mut self,
        entry: EntryId,
        _options: StreamOptions,
    ) -> Result<EntryReader, ArchiveError> {
        let listing = self.listing(ListingMode::Full)?;
        let archive_entry = listing
            .entries
            .iter()
            .find(|candidate| candidate.id == entry)
            .ok_or_else(|| ArchiveError::new(ArchiveErrorKind::Internal, "Entry id not found"))?
            .clone();
        if archive_entry.kind == EntryKind::Directory {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Cannot open a directory entry as a byte stream",
            )
            .with_entry_path(archive_entry.raw_path));
        }
        let bytes = match self.format {
            ContainerFormat::Cab => read_cab_entry(self.path()?, &archive_entry.raw_path)?,
            ContainerFormat::Compound | ContainerFormat::Msi => {
                read_compound_entry(self.path()?, &archive_entry.raw_path)?
            }
            ContainerFormat::Deb => read_deb_entry(self.path()?, entry)?,
            ContainerFormat::Xpi => read_zip_entry(self.path()?, entry, "xpi")?,
            ContainerFormat::Asar => {
                let bytes = self.asar_data()?;
                read_asar_entry(bytes, &archive_entry.raw_path)?
            }
            ContainerFormat::Unknown => return Err(unsupported("Unknown container format")),
        };
        Ok(EntryReader {
            entry,
            access_cost: AccessCost::SequentialFromStart,
            size: Some(bytes.len() as u64),
            source: Box::new(Cursor::new(bytes)),
        })
    }

    fn test(&mut self, _options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        let listing = self.listing(ListingMode::Full)?;
        for entry in listing
            .entries
            .iter()
            .filter(|entry| entry.kind != EntryKind::Directory)
        {
            let _reader = self.open_entry_reader(entry.id, StreamOptions::default())?;
        }
        Ok(TaskPlan::new(
            TaskKind::Test,
            format!("Test {} container", format_name(self.format)),
        )
        .estimated_entries(listing.entries.len())
        .native(quick_test_pipeline(vec![
            PipelineStep::ProbeArchive,
            PipelineStep::ValidateEntryPath,
        ])))
    }
}

impl ContainerArchive {
    fn path(&self) -> Result<&Path, ArchiveError> {
        self.source.path().ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Container backend requires a local path",
            )
            .with_backend("containers")
        })
    }

    fn asar_data(&mut self) -> Result<&[u8], ArchiveError> {
        if self.asar_bytes.is_none() {
            self.asar_bytes = Some(fs_err::read(self.path()?).map_err(io_error)?);
        }
        Ok(self.asar_bytes.as_deref().unwrap_or(&[]))
    }

    fn extract(
        &mut self,
        destination: &Path,
        selected: Option<&[EntryId]>,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let listing = self.listing(ListingMode::Full)?;
        let selected_entries = listing
            .entries
            .iter()
            .filter(|entry| selected.map(|ids| ids.contains(&entry.id)).unwrap_or(true))
            .cloned()
            .collect::<Vec<_>>();
        let writer = SafeWriter::new(destination.to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(options.overwrite_policy);

        for entry in &selected_entries {
            match entry.kind {
                EntryKind::Directory => {
                    writer.create_dir(&entry.raw_path)?;
                }
                EntryKind::Symlink
                    if matches!(options.symlink_policy, SymlinkPolicy::Conservative) =>
                {
                    return Err(ArchiveError::new(
                        ArchiveErrorKind::SymlinkPolicyBlocked,
                        "Symlink extraction is blocked by policy",
                    )
                    .with_entry_path(entry.raw_path.clone()));
                }
                EntryKind::File | EntryKind::Other | EntryKind::Symlink => {
                    let reader = self.open_entry_reader(entry.id, StreamOptions::default())?;
                    let mut source = reader.source;
                    writer.write_stream(&entry.raw_path, source.as_mut(), |_| Ok(()))?;
                }
            }
        }

        Ok(TaskPlan::new(
            TaskKind::Extract,
            format!(
                "Extract {} to {}",
                format_name(self.format),
                destination.display()
            ),
        )
        .estimated_entries(selected_entries.len())
        .native(extract_pipeline(self.format)))
    }
}

fn detect_format(source: &ArchiveSource) -> ContainerFormat {
    let Some(path) = source.path() else {
        return ContainerFormat::Unknown;
    };
    if let Ok(mut file) = File::open(path) {
        let mut magic = [0_u8; 16];
        if let Ok(read) = file.read(&mut magic) {
            if read >= 8 && &magic[..8] == b"!<arch>\n" {
                return ContainerFormat::Deb;
            }
            if read >= 4 && &magic[..4] == b"MSCF" {
                return ContainerFormat::Cab;
            }
            if read >= 8 && magic[..8] == [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1] {
                return if extension_is(path, "msi")
                    || extension_is(path, "msp")
                    || extension_is(path, "msm")
                {
                    ContainerFormat::Msi
                } else {
                    ContainerFormat::Compound
                };
            }
            if read >= 4 && &magic[..4] == b"PK\x03\x04" && extension_is(path, "xpi") {
                return ContainerFormat::Xpi;
            }
        }
    }
    let name = path.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".cab") {
        ContainerFormat::Cab
    } else if name.ends_with(".msi") || name.ends_with(".msp") || name.ends_with(".msm") {
        ContainerFormat::Msi
    } else if name.ends_with(".cfb") || name.ends_with(".compound") {
        ContainerFormat::Compound
    } else if name.ends_with(".deb") {
        ContainerFormat::Deb
    } else if name.ends_with(".xpi") {
        ContainerFormat::Xpi
    } else if name.ends_with(".asar") {
        ContainerFormat::Asar
    } else {
        ContainerFormat::Unknown
    }
}

fn probe_confidence(source: &ArchiveSource, format: ContainerFormat) -> ProbeConfidence {
    match format {
        ContainerFormat::Cab
        | ContainerFormat::Compound
        | ContainerFormat::Msi
        | ContainerFormat::Deb => ProbeConfidence::Signature,
        ContainerFormat::Xpi
            if extension_confidence(source, &["xpi"]) != ProbeConfidence::Impossible =>
        {
            ProbeConfidence::Strong
        }
        ContainerFormat::Xpi | ContainerFormat::Asar => ProbeConfidence::Extension,
        ContainerFormat::Unknown => ProbeConfidence::Impossible,
    }
}

fn extension_is(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(expected))
}

fn read_cab_listing(path: &Path) -> Result<ArchiveListing, ArchiveError> {
    let file = File::open(path).map_err(io_error)?;
    let cabinet = Cabinet::new(file).map_err(io_error)?;
    let mut entries = Vec::new();
    for folder in cabinet.folder_entries() {
        for file in folder.file_entries() {
            let raw_path = file.name().replace('\\', "/");
            entries.push(entry(
                entries.len(),
                raw_path,
                EntryKind::File,
                Some(file.uncompressed_size() as u64),
                None,
                Some(format!("{:?}", folder.compression_type())),
            ));
        }
    }
    Ok(listing(entries))
}

fn read_cab_entry(path: &Path, name: &str) -> Result<Vec<u8>, ArchiveError> {
    let file = File::open(path).map_err(io_error)?;
    let mut cabinet = Cabinet::new(file).map_err(io_error)?;
    let mut reader = cabinet.read_file(name).map_err(io_error)?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(io_error)?;
    Ok(bytes)
}

fn read_compound_listing(path: &Path) -> Result<ArchiveListing, ArchiveError> {
    let file = File::open(path).map_err(io_error)?;
    let compound = CompoundFile::open(file).map_err(io_error)?;
    let entries = compound
        .walk()
        .filter(|item| !item.is_root())
        .enumerate()
        .map(|(index, item)| {
            let raw_path = compound_path(item.path());
            entry(
                index,
                raw_path,
                if item.is_storage() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                item.is_stream().then_some(item.len()),
                None,
                Some("CFB".into()),
            )
        })
        .collect();
    Ok(listing(entries))
}

fn read_compound_entry(path: &Path, raw_path: &str) -> Result<Vec<u8>, ArchiveError> {
    let file = File::open(path).map_err(io_error)?;
    let mut compound = CompoundFile::open(file).map_err(io_error)?;
    let mut stream = compound
        .open_stream(Path::new("/").join(raw_path))
        .map_err(io_error)?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).map_err(io_error)?;
    Ok(bytes)
}

fn read_zip_listing(path: &Path, backend: &'static str) -> Result<ArchiveListing, ArchiveError> {
    let file = File::open(path).map_err(io_error)?;
    let mut reader = ZipReader::new(file).map_err(|error| zip_error(error, backend))?;
    let mut entries = Vec::with_capacity(reader.len());
    for index in 0..reader.len() {
        let file = reader
            .by_index(index)
            .map_err(|error| zip_error(error, backend))?;
        let raw_path = file.name().to_string();
        entries.push(entry(
            index,
            raw_path,
            if file.is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            Some(file.size()),
            Some(file.compressed_size()),
            Some(format!("{:?}", file.compression())),
        ));
    }
    Ok(listing(entries))
}

fn read_zip_entry(
    path: &Path,
    id: EntryId,
    backend: &'static str,
) -> Result<Vec<u8>, ArchiveError> {
    let file = File::open(path).map_err(io_error)?;
    let mut reader = ZipReader::new(file).map_err(|error| zip_error(error, backend))?;
    let mut file: ZipFile<'_, File> = reader
        .by_index(id.0 as usize)
        .map_err(|error| zip_error(error, backend))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(io_error)?;
    Ok(bytes)
}

fn read_asar_listing(bytes: &[u8]) -> Result<ArchiveListing, ArchiveError> {
    let reader = AsarReader::new(bytes, None).map_err(asar_error)?;
    let mut entries = Vec::new();
    for path in reader
        .directories()
        .keys()
        .filter(|path| !path.as_os_str().is_empty())
    {
        entries.push(entry(
            entries.len(),
            path_to_archive(path),
            EntryKind::Directory,
            None,
            None,
            Some("asar".into()),
        ));
    }
    for (path, file) in reader.files() {
        entries.push(entry(
            entries.len(),
            path_to_archive(path),
            EntryKind::File,
            Some(file.data().len() as u64),
            Some(file.data().len() as u64),
            Some("asar".into()),
        ));
    }
    for path in reader.symlinks().keys() {
        entries.push(entry(
            entries.len(),
            path_to_archive(path),
            EntryKind::Symlink,
            None,
            None,
            Some("asar-link".into()),
        ));
    }
    Ok(listing(entries))
}

fn read_asar_entry(bytes: &[u8], raw_path: &str) -> Result<Vec<u8>, ArchiveError> {
    let reader = AsarReader::new(bytes, None).map_err(asar_error)?;
    reader
        .read(Path::new(raw_path))
        .map(|file| file.data().to_vec())
        .ok_or_else(|| ArchiveError::new(ArchiveErrorKind::Internal, "ASAR entry was not found"))
}

fn read_deb_listing(path: &Path) -> Result<ArchiveListing, ArchiveError> {
    let data = read_deb_data_member(path)?;
    read_tar_listing(data.reader, data.method)
}

fn read_deb_entry(path: &Path, id: EntryId) -> Result<Vec<u8>, ArchiveError> {
    let data = read_deb_data_member(path)?;
    let mut archive = TarReader::new(data.reader);
    for (index, entry) in archive.entries().map_err(io_error)?.enumerate() {
        let mut entry = entry.map_err(io_error)?;
        if EntryId(index as u64) == id {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(io_error)?;
            return Ok(bytes);
        }
    }
    Err(ArchiveError::new(
        ArchiveErrorKind::Internal,
        "DEB data member entry id was not found",
    ))
}

struct DebData {
    reader: Box<dyn Read>,
    method: String,
}

fn read_deb_data_member(path: &Path) -> Result<DebData, ArchiveError> {
    let file = File::open(path).map_err(io_error)?;
    let mut archive = ArArchive::new(file);
    while let Some(entry) = archive.next_entry() {
        let mut entry = entry.map_err(io_error)?;
        let name = String::from_utf8_lossy(entry.header().identifier()).into_owned();
        if !name.starts_with("data.tar") {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.header().size().min(64 * 1024 * 1024) as usize);
        entry.read_to_end(&mut bytes).map_err(io_error)?;
        let (reader, method): (Box<dyn Read>, String) = if name.ends_with(".tar.gz")
            || name.ends_with(".tgz")
        {
            (Box::new(GzDecoder::new(Cursor::new(bytes))), "gzip".into())
        } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
            (Box::new(XzDecoder::new(Cursor::new(bytes))), "xz".into())
        } else if name.ends_with(".tar.zst") || name.ends_with(".tzst") {
            (
                Box::new(zstd::stream::read::Decoder::new(Cursor::new(bytes)).map_err(io_error)?),
                "zstd".into(),
            )
        } else if name.ends_with(".tar.bz2") {
            (
                Box::new(bzip2::read::BzDecoder::new(Cursor::new(bytes))),
                "bzip2".into(),
            )
        } else if name.ends_with(".tar") {
            (Box::new(Cursor::new(bytes)), "tar".into())
        } else {
            return Err(unsupported(format!("Unsupported DEB data member: {name}")));
        };
        return Ok(DebData { reader, method });
    }
    Err(ArchiveError::new(
        ArchiveErrorKind::CorruptArchive,
        "DEB archive has no data.tar member",
    ))
}

fn read_tar_listing(reader: Box<dyn Read>, method: String) -> Result<ArchiveListing, ArchiveError> {
    let mut archive = TarReader::new(reader);
    let mut entries = Vec::new();
    for (index, item) in archive.entries().map_err(io_error)?.enumerate() {
        let tar_entry = item.map_err(io_error)?;
        let path = tar_entry
            .path()
            .map_err(io_error)?
            .to_string_lossy()
            .into_owned();
        let kind = if tar_entry.header().entry_type().is_dir() {
            EntryKind::Directory
        } else if tar_entry.header().entry_type().is_symlink() {
            EntryKind::Symlink
        } else {
            EntryKind::File
        };
        entries.push(entry(
            index,
            path,
            kind,
            tar_entry.header().size().ok(),
            None,
            Some(method.clone()),
        ));
    }
    Ok(listing(entries))
}

fn entry(
    index: usize,
    raw_path: String,
    kind: EntryKind,
    size: Option<u64>,
    compressed_size: Option<u64>,
    method: Option<String>,
) -> ArchiveEntry {
    let normalized_path = raw_path
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();
    ArchiveEntry {
        id: EntryId(index as u64),
        raw_path: normalized_path.clone(),
        normalized_path: normalized_path.clone(),
        display_path: normalized_path.clone(),
        kind,
        size,
        compressed_size,
        modified_at: None,
        method,
        encrypted: false,
        safety: classify_entry_path(&normalized_path),
    }
}

fn listing(entries: Vec<ArchiveEntry>) -> ArchiveListing {
    ArchiveListing {
        entries,
        directories: Default::default(),
        is_complete: true,
    }
}

fn compound_path(path: &Path) -> String {
    path_to_archive(path).trim_start_matches('/').to_string()
}

fn path_to_archive(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn archive_format(format: ContainerFormat) -> ArchiveFormat {
    match format {
        ContainerFormat::Cab => ArchiveFormat::Cab,
        ContainerFormat::Compound => ArchiveFormat::Compound,
        ContainerFormat::Msi => ArchiveFormat::Msi,
        ContainerFormat::Deb => ArchiveFormat::Deb,
        ContainerFormat::Xpi => ArchiveFormat::Xpi,
        ContainerFormat::Asar => ArchiveFormat::Asar,
        ContainerFormat::Unknown => ArchiveFormat::Unknown,
    }
}

fn format_name(format: ContainerFormat) -> &'static str {
    match format {
        ContainerFormat::Cab => "CAB",
        ContainerFormat::Compound => "Compound",
        ContainerFormat::Msi => "MSI",
        ContainerFormat::Deb => "DEB",
        ContainerFormat::Xpi => "XPI",
        ContainerFormat::Asar => "ASAR",
        ContainerFormat::Unknown => "container",
    }
}

fn codecs(format: ContainerFormat) -> Vec<String> {
    match format {
        ContainerFormat::Cab => vec!["MSZIP".into(), "LZX".into(), "Quantum".into()],
        ContainerFormat::Compound | ContainerFormat::Msi => vec!["CFB".into()],
        ContainerFormat::Deb => vec![
            "ar".into(),
            "tar".into(),
            "gzip".into(),
            "xz".into(),
            "zstd".into(),
            "bzip2".into(),
        ],
        ContainerFormat::Xpi => vec!["zip".into(), "deflate".into()],
        ContainerFormat::Asar => vec!["asar".into()],
        ContainerFormat::Unknown => Vec::new(),
    }
}

fn container_capabilities(format: ContainerFormat) -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Full,
        extract_all: CapabilityLevel::Full,
        extract_selected: match format {
            ContainerFormat::Cab | ContainerFormat::Deb => CapabilityLevel::Limited,
            _ => CapabilityLevel::Full,
        },
        create: CapabilityLevel::Unsupported,
        update: CapabilityLevel::Unsupported,
        random_access: match format {
            ContainerFormat::Xpi
            | ContainerFormat::Asar
            | ContainerFormat::Compound
            | ContainerFormat::Msi => CapabilityLevel::Full,
            ContainerFormat::Cab | ContainerFormat::Deb => CapabilityLevel::Limited,
            ContainerFormat::Unknown => CapabilityLevel::Limited,
        },
        password_read: CapabilityLevel::Unsupported,
        password_write: CapabilityLevel::Unsupported,
        header_encryption: CapabilityLevel::Unsupported,
        multi_volume_read: CapabilityLevel::Unsupported,
        multi_volume_write: CapabilityLevel::Unsupported,
        entry_stream_preview: CapabilityLevel::Full,
    }
}

fn container_backend_capabilities() -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Full,
        extract_all: CapabilityLevel::Full,
        extract_selected: CapabilityLevel::Limited,
        create: CapabilityLevel::Unsupported,
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

fn extract_pipeline(format: ContainerFormat) -> NativePipelinePlan {
    match format {
        ContainerFormat::Xpi
        | ContainerFormat::Asar
        | ContainerFormat::Compound
        | ContainerFormat::Msi => random_access_extract_pipeline(PipelineStep::ProbeArchive),
        ContainerFormat::Cab | ContainerFormat::Deb | ContainerFormat::Unknown => {
            sequential_extract_pipeline(vec![PipelineStep::ProbeArchive])
        }
    }
}

fn unsupported(message: impl Into<String>) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::UnsupportedFormat, message.into())
        .with_backend("containers")
}

fn io_error(error: io::Error) -> ArchiveError {
    ArchiveError::new(
        ArchiveErrorKind::Io,
        "Container archive I/O operation failed",
    )
    .with_backend("containers")
    .with_technical_detail(error.to_string())
}

fn zip_error(error: zip::result::ZipError, backend: &'static str) -> ArchiveError {
    ArchiveError::new(
        ArchiveErrorKind::CorruptArchive,
        "ZIP-compatible container operation failed",
    )
    .with_backend(backend)
    .with_technical_detail(error.to_string())
}

fn asar_error(error: asar::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::CorruptArchive, "ASAR operation failed")
        .with_backend("asar")
        .with_technical_detail(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cab_signature() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        fs_err::write(&path, b"MSCF\x00\x00\x00\x00").unwrap();
        assert_eq!(
            detect_format(&ArchiveSource::LocalPath(path)),
            ContainerFormat::Cab
        );
    }

    #[test]
    fn xpi_is_explicit_zip_container() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("addon.xpi");
        fs_err::write(&path, b"PK\x03\x04anything").unwrap();
        assert_eq!(
            detect_format(&ArchiveSource::LocalPath(path)),
            ContainerFormat::Xpi
        );
    }

    #[test]
    fn create_is_explicitly_unsupported() {
        let backend = ContainersBackend;
        let error = backend
            .create_plan(
                &[],
                Path::new("package.deb"),
                CreateOptions {
                    format: ArchiveFormat::Unknown,
                    compression_method: None,
                    compression_level: None,
                    solid: false,
                    encrypt_file_names: false,
                    password: None,
                    volume_size: None,
                    symlink_policy: SymlinkPolicy::Conservative,
                },
            )
            .unwrap_err();
        assert_eq!(error.kind, ArchiveErrorKind::UnsupportedFormat);
    }
}
