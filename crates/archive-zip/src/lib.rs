use std::{
    fs::File,
    io::{Cursor, Read, Seek, Write},
    path::{Path, PathBuf},
};

use crc32fast::Hasher;
use lzma_rust2::{LZMA2Options, LZMAWriter};
use shadow_zip_archive_core::{
    ArchiveBackend, EntryReader, InputScanner, OpenArchive, SafeWriter, StreamLimits,
    ScannedInput, create_pipeline, extension_confidence, quick_test_pipeline,
    random_access_extract_pipeline,
};
use shadow_zip_domain::*;
use xz2::write::XzEncoder;
use zip::{ZipWriter, read::ZipArchive as ZipReader, read::ZipFile, write::SimpleFileOptions};

const ZIP_LOCAL_FILE_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const ZIP_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0201_4b50;
const ZIP_END_OF_CENTRAL_DIRECTORY_SIGNATURE: u32 = 0x0605_4b50;
const ZIP_DATA_DESCRIPTOR_SIGNATURE: u32 = 0x0807_4b50;
const ZIP_FLAG_UTF8: u16 = 0x0800;
const ZIP_FLAG_DATA_DESCRIPTOR: u16 = 0x0008;
const ZIP_FLAG_LZMA_EOS_MARKER: u16 = 0x0002;
const ZIP_METHOD_STORE: u16 = 0;
const ZIP_METHOD_LZMA: u16 = 14;
const ZIP_VERSION_STORE: u16 = 20;
const ZIP_VERSION_LZMA: u16 = 63;
const ZIP_DOS_DATE_1980_01_01: u16 = 33;
const ZIP_DOS_TIME_00_00_00: u16 = 0;
const ZIP_DIRECTORY_EXTERNAL_ATTRIBUTES: u32 = 0x10;

pub struct ZipBackend;

impl ArchiveBackend for ZipBackend {
    fn name(&self) -> &'static str {
        "zip"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        let display_name = source.display_name();
        let format = if display_name
            .rsplit_once('.')
            .is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("zipx"))
        {
            ArchiveFormat::ZipX
        } else {
            ArchiveFormat::Zip
        };
        Ok(ProbeResult {
            format,
            confidence: extension_confidence(source, &["zip", "zipx"]),
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        let format = self.probe(&source)?.format;
        Ok(Box::new(ZipArchive {
            source,
            password: options.password,
            info: ArchiveInfo {
                format,
                display_name: "ZIP archive".into(),
                total_bytes: None,
                entry_count: None,
                codecs: vec!["Deflate".into(), "AES".into(), "ZIP64".into()],
                filters: Vec::new(),
                is_solid: false,
                is_encrypted: false,
                has_header_encryption: false,
                is_multi_volume: false,
            },
            listing_cache: None,
        }))
    }

    fn create_plan(
        &self,
        inputs: &[InputPath],
        output: &Path,
        _options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        Ok(
            TaskPlan::new(TaskKind::Create, format!("Create {}", output.display()))
                .estimated_entries(inputs.len())
                .native(create_pipeline()),
        )
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![ArchiveFormat::Zip, ArchiveFormat::ZipX],
            capabilities: zip_capabilities(),
        }
    }
}

struct ZipArchive {
    source: ArchiveSource,
    password: Option<String>,
    info: ArchiveInfo,
    listing_cache: Option<ArchiveListing>,
}

impl OpenArchive for ZipArchive {
    fn info(&self) -> ArchiveInfo {
        let mut info = self.info.clone();
        info.display_name = self.source.display_name();
        info
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        zip_capabilities()
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        if let Some(listing) = &self.listing_cache {
            return Ok(listing.clone());
        }
        let listing = self.read_listing()?;
        self.listing_cache = Some(listing.clone());
        Ok(listing)
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
        Ok(EntryStream {
            entry,
            access_cost: AccessCost::Random,
        })
    }

    fn open_entry_reader(
        &mut self,
        entry: EntryId,
        options: StreamOptions,
    ) -> Result<EntryReader, ArchiveError> {
        let mut reader = self.open_reader()?;
        let password = options.password.as_ref().or(self.password.as_ref());
        let mut file = zip_file_by_index(&mut reader, entry.0 as usize, password)?;
        if file.is_dir() {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Cannot open a directory entry as a byte stream",
            )
            .with_backend("zip")
            .with_entry_path(file.name().to_string()));
        }
        let size = Some(file.size());
        let mut bytes = Vec::with_capacity(size.unwrap_or_default().min(16 * 1024 * 1024) as usize);
        std::io::copy(&mut file, &mut bytes).map_err(io_error)?;
        Ok(EntryReader {
            entry,
            access_cost: AccessCost::Random,
            source: Box::new(Cursor::new(bytes)),
            size,
        })
    }

    fn test(&mut self, options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        let mut reader = self.open_reader()?;
        let password = options.password.as_ref().or(self.password.as_ref());
        for index in 0..reader.len() {
            let mut file = zip_file_by_index(&mut reader, index, password)?;
            if !file.is_dir() {
                std::io::copy(&mut file, &mut std::io::sink()).map_err(io_error)?;
            }
        }
        Ok(
            TaskPlan::new(TaskKind::Test, "Test ZIP archive").native(quick_test_pipeline(vec![
                PipelineStep::ReadCentralDirectory,
                PipelineStep::ProbeArchive,
            ])),
        )
    }
}

impl ZipArchive {
    fn open_reader(&self) -> Result<ZipReader<File>, ArchiveError> {
        let path = self.source.path().ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "ZIP backend requires a local path",
            )
        })?;
        let file = File::open(path).map_err(io_error)?;
        ZipReader::new(file).map_err(zip_error)
    }

    fn read_listing(&self) -> Result<ArchiveListing, ArchiveError> {
        let mut reader = self.open_reader()?;
        let mut entries = Vec::with_capacity(reader.len());
        for index in 0..reader.len() {
            let file = zip_file_by_index(&mut reader, index, self.password.as_ref())?;
            let raw_path = file.name().to_string();
            entries.push(ArchiveEntry {
                id: EntryId(index as u64),
                raw_path: raw_path.clone(),
                normalized_path: raw_path.replace('\\', "/"),
                display_path: raw_path.clone(),
                kind: if file.is_dir() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: Some(file.size()),
                compressed_size: Some(file.compressed_size()),
                modified_at: None,
                method: Some(format!("{:?}", file.compression())),
                encrypted: file.encrypted(),
                safety: classify_entry_path(&raw_path),
            });
        }
        Ok(ArchiveListing {
            entries,
            directories: Default::default(),
            is_complete: true,
        })
    }

    fn extract_to(
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

        let mut reader = self.open_reader()?;
        let writer = SafeWriter::new(destination.to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(options.overwrite_policy);
        for entry in selected_entries {
            let mut file =
                zip_file_by_index(&mut reader, entry.id.0 as usize, options.password.as_ref())?;
            if entry.kind == EntryKind::Directory {
                writer.create_dir(&entry.raw_path)?;
            } else {
                writer.write_stream(&entry.raw_path, &mut file, |_| Ok(()))?;
            }
        }

        Ok(TaskPlan::new(
            TaskKind::Extract,
            format!("Extract to {}", destination.display()),
        )
        .estimated_entries(
            selected
                .map(|ids| ids.len())
                .unwrap_or(listing.entries.len()),
        )
        .native(random_access_extract_pipeline(
            PipelineStep::ReadCentralDirectory,
        )))
    }
}

pub fn create_zip_archive(
    inputs: &[InputPath],
    output: PathBuf,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    let compression_method = zip_compression_method(&options)?;
    match compression_method {
        ZipCreateMethod::Lzma => return create_zip_lzma_archive(inputs, output, options),
        ZipCreateMethod::Xz => return create_zip_xz_archive(inputs, output, options),
        ZipCreateMethod::Native(_) => {}
    }
    let file = File::create(output).map_err(io_error)?;
    let mut writer = ZipWriter::new(file);
    let zip_options =
        SimpleFileOptions::default().compression_method(compression_method.native_method());

    for input in InputScanner::scan(inputs)? {
        if input.is_dir {
            writer
                .add_directory(input.archive_path, zip_options)
                .map_err(zip_error)?;
            continue;
        }
        writer
            .start_file(input.archive_path, zip_options)
            .map_err(zip_error)?;
        let mut source = File::open(&input.source_path).map_err(io_error)?;
        std::io::copy(&mut source, &mut writer).map_err(io_error)?;
    }

    writer.finish().map_err(zip_error)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZipCreateMethod {
    Native(zip::CompressionMethod),
    Lzma,
    Xz,
}

impl ZipCreateMethod {
    fn native_method(self) -> zip::CompressionMethod {
        match self {
            Self::Native(method) => method,
            Self::Lzma | Self::Xz => zip::CompressionMethod::Stored,
        }
    }
}

fn zip_compression_method(options: &CreateOptions) -> Result<ZipCreateMethod, ArchiveError> {
    let method = options
        .compression_method
        .as_deref()
        .unwrap_or(if options.format == ArchiveFormat::ZipX {
            "zstd"
        } else {
            "deflate"
        })
        .to_ascii_lowercase();
    match method.as_str() {
        "store" | "stored" | "none" => Ok(ZipCreateMethod::Native(zip::CompressionMethod::Stored)),
        "deflate" | "deflated" => Ok(ZipCreateMethod::Native(zip::CompressionMethod::Deflated)),
        "bzip2" | "bz2" => Ok(ZipCreateMethod::Native(zip::CompressionMethod::Bzip2)),
        "zstd" | "zstandard" => Ok(ZipCreateMethod::Native(zip::CompressionMethod::Zstd)),
        "xz" => Ok(ZipCreateMethod::Xz),
        "lzma" => Ok(ZipCreateMethod::Lzma),
        _ => Err(ArchiveError::new(
            ArchiveErrorKind::UnsupportedCodec,
            "Unsupported ZIP compression method",
        )
        .with_backend("zip")
        .with_technical_detail(format!("method={method}"))),
    }
}

fn create_zip_xz_archive(
    inputs: &[InputPath],
    output: PathBuf,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    let scanned = InputScanner::scan(inputs)?;
    let mut output = File::create(output).map_err(io_error)?;
    let mut central = Vec::with_capacity(scanned.len());
    for input in scanned {
        let entry = write_xz_zip_entry(&mut output, &input, &options)?;
        central.push(entry);
    }
    write_lzma_zip_central_directory(&mut output, &central)
}

fn create_zip_lzma_archive(
    inputs: &[InputPath],
    output: PathBuf,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    let scanned = InputScanner::scan(inputs)?;
    let mut output = File::create(output).map_err(io_error)?;
    let mut central = Vec::with_capacity(scanned.len());
    for input in scanned {
        let entry = write_lzma_zip_entry(&mut output, &input, &options)?;
        central.push(entry);
    }
    write_lzma_zip_central_directory(&mut output, &central)
}

#[derive(Debug)]
struct ZipCentralEntry {
    name: String,
    method: u16,
    flags: u16,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    local_header_offset: u32,
    external_attributes: u32,
}

fn write_lzma_zip_entry(
    output: &mut File,
    input: &ScannedInput,
    options: &CreateOptions,
) -> Result<ZipCentralEntry, ArchiveError> {
    let name = input.archive_path.replace('\\', "/");
    let local_header_offset = checked_u32(output.stream_position().map_err(io_error)?)?;
    if input.is_dir {
        let directory_name = if name.ends_with('/') {
            name
        } else {
            format!("{name}/")
        };
        write_zip_local_header(
            output,
            &directory_name,
            ZIP_VERSION_STORE,
            ZIP_FLAG_UTF8,
            ZIP_METHOD_STORE,
            0,
            0,
            0,
        )?;
        return Ok(ZipCentralEntry {
            name: directory_name,
            method: ZIP_METHOD_STORE,
            flags: ZIP_FLAG_UTF8,
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            local_header_offset,
            external_attributes: ZIP_DIRECTORY_EXTERNAL_ATTRIBUTES,
        });
    }

    let flags = ZIP_FLAG_UTF8 | ZIP_FLAG_DATA_DESCRIPTOR | ZIP_FLAG_LZMA_EOS_MARKER;
    write_zip_local_header(output, &name, ZIP_VERSION_LZMA, flags, ZIP_METHOD_LZMA, 0, 0, 0)?;
    let mut source = File::open(&input.source_path).map_err(io_error)?;
    let level = options.compression_level.unwrap_or(6).min(9) as u32;
    let mut lzma_options = LZMA2Options::with_preset(level);
    lzma_options.dict_size = LZMA2Options::DICT_SIZE_DEFAULT;
    let mut compressed_size = 0_u64;
    let mut uncompressed_size = 0_u64;
    let mut crc = Hasher::new();

    write_lzma_zip_properties(output, &lzma_options, &mut compressed_size)?;
    {
        let mut counter = CountingZipWriter {
            inner: output,
            written: &mut compressed_size,
        };
        let mut encoder =
            LZMAWriter::new_no_header(&mut counter, &lzma_options, true).map_err(io_error)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = source.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            crc.update(&buffer[..read]);
            uncompressed_size += read as u64;
            encoder.write_all(&buffer[..read]).map_err(io_error)?;
        }
        encoder.finish().map_err(io_error)?;
    }

    let crc32 = crc.finalize();
    let compressed_size_u32 = checked_u32(compressed_size)?;
    let uncompressed_size_u32 = checked_u32(uncompressed_size)?;
    write_zip_data_descriptor(output, crc32, compressed_size_u32, uncompressed_size_u32)?;
    Ok(ZipCentralEntry {
        name,
        method: ZIP_METHOD_LZMA,
        flags,
        crc32,
        compressed_size: compressed_size_u32,
        uncompressed_size: uncompressed_size_u32,
        local_header_offset,
        external_attributes: 0,
    })
}

fn write_xz_zip_entry(
    output: &mut File,
    input: &ScannedInput,
    options: &CreateOptions,
) -> Result<ZipCentralEntry, ArchiveError> {
    let name = input.archive_path.replace('\\', "/");
    let local_header_offset = checked_u32(output.stream_position().map_err(io_error)?)?;
    if input.is_dir {
        let directory_name = if name.ends_with('/') {
            name
        } else {
            format!("{name}/")
        };
        write_zip_local_header(
            output,
            &directory_name,
            ZIP_VERSION_STORE,
            ZIP_FLAG_UTF8,
            ZIP_METHOD_STORE,
            0,
            0,
            0,
        )?;
        return Ok(ZipCentralEntry {
            name: directory_name,
            method: ZIP_METHOD_STORE,
            flags: ZIP_FLAG_UTF8,
            crc32: 0,
            compressed_size: 0,
            uncompressed_size: 0,
            local_header_offset,
            external_attributes: ZIP_DIRECTORY_EXTERNAL_ATTRIBUTES,
        });
    }

    let flags = ZIP_FLAG_UTF8 | ZIP_FLAG_DATA_DESCRIPTOR;
    write_zip_local_header(output, &name, ZIP_VERSION_LZMA, flags, 95, 0, 0, 0)?;
    let mut source = File::open(&input.source_path).map_err(io_error)?;
    let level = options.compression_level.unwrap_or(6).min(9) as u32;
    let mut compressed_size = 0_u64;
    let mut uncompressed_size = 0_u64;
    let mut crc = Hasher::new();
    {
        let counter = CountingZipWriter {
            inner: output,
            written: &mut compressed_size,
        };
        let mut encoder = XzEncoder::new(counter, level);
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = source.read(&mut buffer).map_err(io_error)?;
            if read == 0 {
                break;
            }
            crc.update(&buffer[..read]);
            uncompressed_size += read as u64;
            encoder.write_all(&buffer[..read]).map_err(io_error)?;
        }
        encoder.finish().map_err(io_error)?;
    }

    let crc32 = crc.finalize();
    let compressed_size_u32 = checked_u32(compressed_size)?;
    let uncompressed_size_u32 = checked_u32(uncompressed_size)?;
    write_zip_data_descriptor(output, crc32, compressed_size_u32, uncompressed_size_u32)?;
    Ok(ZipCentralEntry {
        name,
        method: 95,
        flags,
        crc32,
        compressed_size: compressed_size_u32,
        uncompressed_size: uncompressed_size_u32,
        local_header_offset,
        external_attributes: 0,
    })
}

struct CountingZipWriter<'a> {
    inner: &'a mut File,
    written: &'a mut u64,
}

impl Write for CountingZipWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        *self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn write_lzma_zip_properties(
    output: &mut File,
    options: &LZMA2Options,
    compressed_size: &mut u64,
) -> Result<(), ArchiveError> {
    let props = [
        9_u8,
        4_u8,
        5_u8,
        0_u8,
        options.get_props(),
        (options.dict_size & 0xff) as u8,
        ((options.dict_size >> 8) & 0xff) as u8,
        ((options.dict_size >> 16) & 0xff) as u8,
        ((options.dict_size >> 24) & 0xff) as u8,
    ];
    output.write_all(&props).map_err(io_error)?;
    *compressed_size += props.len() as u64;
    Ok(())
}

fn write_lzma_zip_central_directory(
    output: &mut File,
    entries: &[ZipCentralEntry],
) -> Result<(), ArchiveError> {
    let central_offset = checked_u32(output.stream_position().map_err(io_error)?)?;
    for entry in entries {
        write_u32(output, ZIP_CENTRAL_DIRECTORY_SIGNATURE)?;
        write_u16(output, ZIP_VERSION_LZMA)?;
        write_u16(
            output,
            if entry.method == ZIP_METHOD_LZMA {
                ZIP_VERSION_LZMA
            } else {
                ZIP_VERSION_STORE
            },
        )?;
        write_u16(output, entry.flags)?;
        write_u16(output, entry.method)?;
        write_u16(output, ZIP_DOS_TIME_00_00_00)?;
        write_u16(output, ZIP_DOS_DATE_1980_01_01)?;
        write_u32(output, entry.crc32)?;
        write_u32(output, entry.compressed_size)?;
        write_u32(output, entry.uncompressed_size)?;
        write_u16(output, checked_u16(entry.name.len())?)?;
        write_u16(output, 0)?;
        write_u16(output, 0)?;
        write_u16(output, 0)?;
        write_u16(output, 0)?;
        write_u32(output, entry.external_attributes)?;
        write_u32(output, entry.local_header_offset)?;
        output.write_all(entry.name.as_bytes()).map_err(io_error)?;
    }
    let central_size =
        checked_u32(output.stream_position().map_err(io_error)? - u64::from(central_offset))?;
    write_u32(output, ZIP_END_OF_CENTRAL_DIRECTORY_SIGNATURE)?;
    write_u16(output, 0)?;
    write_u16(output, 0)?;
    write_u16(output, checked_u16(entries.len())?)?;
    write_u16(output, checked_u16(entries.len())?)?;
    write_u32(output, central_size)?;
    write_u32(output, central_offset)?;
    write_u16(output, 0)?;
    Ok(())
}

fn write_zip_local_header(
    output: &mut File,
    name: &str,
    version_needed: u16,
    flags: u16,
    method: u16,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
) -> Result<(), ArchiveError> {
    write_u32(output, ZIP_LOCAL_FILE_HEADER_SIGNATURE)?;
    write_u16(output, version_needed)?;
    write_u16(output, flags)?;
    write_u16(output, method)?;
    write_u16(output, ZIP_DOS_TIME_00_00_00)?;
    write_u16(output, ZIP_DOS_DATE_1980_01_01)?;
    write_u32(output, crc32)?;
    write_u32(output, compressed_size)?;
    write_u32(output, uncompressed_size)?;
    write_u16(output, checked_u16(name.len())?)?;
    write_u16(output, 0)?;
    output.write_all(name.as_bytes()).map_err(io_error)
}

fn write_zip_data_descriptor(
    output: &mut File,
    crc32: u32,
    compressed_size: u32,
    uncompressed_size: u32,
) -> Result<(), ArchiveError> {
    write_u32(output, ZIP_DATA_DESCRIPTOR_SIGNATURE)?;
    write_u32(output, crc32)?;
    write_u32(output, compressed_size)?;
    write_u32(output, uncompressed_size)
}

fn write_u16(output: &mut File, value: u16) -> Result<(), ArchiveError> {
    output.write_all(&value.to_le_bytes()).map_err(io_error)
}

fn write_u32(output: &mut File, value: u32) -> Result<(), ArchiveError> {
    output.write_all(&value.to_le_bytes()).map_err(io_error)
}

fn checked_u16(value: usize) -> Result<u16, ArchiveError> {
    u16::try_from(value).map_err(|_| {
        ArchiveError::new(ArchiveErrorKind::Internal, "ZIP field exceeds u16 range")
            .with_backend("zip")
    })
}

fn checked_u32(value: u64) -> Result<u32, ArchiveError> {
    u32::try_from(value).map_err(|_| {
        ArchiveError::new(
            ArchiveErrorKind::Internal,
            "ZIP LZMA writer currently requires ZIP32-sized entries",
        )
        .with_backend("zip")
    })
}

fn zip_capabilities() -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Full,
        extract_all: CapabilityLevel::Full,
        extract_selected: CapabilityLevel::Full,
        create: CapabilityLevel::Full,
        update: CapabilityLevel::High,
        random_access: CapabilityLevel::Full,
        password_read: CapabilityLevel::Full,
        password_write: CapabilityLevel::Full,
        header_encryption: CapabilityLevel::Unsupported,
        multi_volume_read: CapabilityLevel::High,
        multi_volume_write: CapabilityLevel::Medium,
        entry_stream_preview: CapabilityLevel::Full,
    }
}

fn zip_file_by_index<'a>(
    reader: &'a mut ZipReader<File>,
    index: usize,
    password: Option<&String>,
) -> Result<ZipFile<'a, File>, ArchiveError> {
    match password {
        Some(password) => reader
            .by_index_decrypt(index, password.as_bytes())
            .map_err(zip_error),
        None => reader.by_index(index).map_err(zip_error),
    }
}

fn zip_error(error: zip::result::ZipError) -> ArchiveError {
    let text = error.to_string();
    let lower = text.to_ascii_lowercase();
    let kind = if lower.contains("password") {
        ArchiveErrorKind::InvalidPassword
    } else {
        ArchiveErrorKind::CorruptArchive
    };
    ArchiveError::new(kind, "ZIP operation failed")
        .with_backend("zip")
        .with_technical_detail(text)
}

fn io_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "I/O operation failed")
        .with_backend("zip")
        .with_technical_detail(error.to_string())
}
