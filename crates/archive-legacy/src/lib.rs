use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

use delharc::LhaDecodeReader;
use shadow_zip_archive_core::{
    ArchiveBackend, EntryReader, OpenArchive, SafeWriter, StreamLimits, quick_test_pipeline,
    sequential_extract_pipeline,
};
use shadow_zip_domain::*;

pub struct LegacyArchiveBackend;

impl ArchiveBackend for LegacyArchiveBackend {
    fn name(&self) -> &'static str {
        "legacy"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        let detected = detect_legacy_format(source)?;
        Ok(ProbeResult {
            format: detected
                .map(|format| format.format)
                .unwrap_or(ArchiveFormat::Unknown),
            confidence: detected
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
        let Some(detected) = detect_legacy_format(&source)? else {
            return Err(unsupported_format(
                ArchiveFormat::Unknown,
                "This source is not a recognized legacy archive format",
            ));
        };

        if matches!(detected.format, ArchiveFormat::Lha | ArchiveFormat::Lzh) {
            Ok(Box::new(LhaArchive {
                source,
                format: detected.format,
                listing_cache: None,
                info: ArchiveInfo {
                    format: detected.format,
                    display_name: detected.format.to_string(),
                    total_bytes: None,
                    entry_count: None,
                    codecs: vec![
                        "-lh0-".into(),
                        "-lh1-".into(),
                        "-lh4-".into(),
                        "-lh5-".into(),
                        "-lh6-".into(),
                        "-lh7-".into(),
                        "-lzs-".into(),
                        "-lz4-".into(),
                        "-lz5-".into(),
                    ],
                    filters: Vec::new(),
                    is_solid: false,
                    is_encrypted: false,
                    has_header_encryption: false,
                    is_multi_volume: false,
                },
            }))
        } else {
            Err(unsupported_format(
                detected.format,
                format!(
                    "{} archives are recognized but no reliable pure Rust decoder is wired in",
                    detected.format
                ),
            ))
        }
    }

    fn create_plan(
        &self,
        _inputs: &[InputPath],
        _output: &Path,
        options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let target = if matches!(
            options.format,
            ArchiveFormat::Lha
                | ArchiveFormat::Lzh
                | ArchiveFormat::Ace
                | ArchiveFormat::Alz
                | ArchiveFormat::Arj
                | ArchiveFormat::Bh
                | ArchiveFormat::Egg
                | ArchiveFormat::Pma
                | ArchiveFormat::Wim
                | ArchiveFormat::Swm
                | ArchiveFormat::Zpaq
                | ArchiveFormat::Pea
        ) {
            options.format
        } else {
            ArchiveFormat::Unknown
        };

        let (kind, message) = if matches!(target, ArchiveFormat::Lha | ArchiveFormat::Lzh) {
            (
                ArchiveErrorKind::UnsupportedCodec,
                "LHA/LZH creation, including lh7, is not implemented because no reliable pure Rust encoder is available",
            )
        } else {
            (
                ArchiveErrorKind::UnsupportedFormat,
                "Legacy archive creation is not supported by this backend",
            )
        };
        Err(ArchiveError::new(kind, message)
            .with_backend(self.name())
            .with_technical_detail(format!("requested_format={target}")))
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: supported_formats(),
            capabilities: legacy_backend_capabilities(),
        }
    }
}

struct LhaArchive {
    source: ArchiveSource,
    format: ArchiveFormat,
    listing_cache: Option<ArchiveListing>,
    info: ArchiveInfo,
}

impl OpenArchive for LhaArchive {
    fn info(&self) -> ArchiveInfo {
        let mut info = self.info.clone();
        info.display_name = self.source.display_name();
        info
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        lha_capabilities()
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        if let Some(listing) = &self.listing_cache {
            return Ok(listing.clone());
        }

        let listing = self.read_listing()?;
        self.info.entry_count = Some(listing.entries.len() as u64);
        self.info.codecs = listing
            .entries
            .iter()
            .filter_map(|entry| entry.method.clone())
            .fold(Vec::<String>::new(), |mut codecs, method| {
                if !codecs.contains(&method) {
                    codecs.push(method);
                }
                codecs
            });
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
            access_cost: AccessCost::SequentialFromStart,
        })
    }

    fn open_entry_reader(
        &mut self,
        entry: EntryId,
        _options: StreamOptions,
    ) -> Result<EntryReader, ArchiveError> {
        let mut reader = self.open_reader()?;
        let mut index = 0_u64;
        loop {
            let header = reader.header().clone();
            let current = EntryId(index);
            let path = lha_path(&header);
            if current == entry {
                if header.is_directory() {
                    return Err(ArchiveError::new(
                        ArchiveErrorKind::Internal,
                        "Cannot open a directory entry as a byte stream",
                    )
                    .with_backend("legacy")
                    .with_entry_path(path));
                }
                ensure_lha_decoder_supported(&reader, &header)?;
                let mut bytes =
                    Vec::with_capacity(header.original_size.min(16 * 1024 * 1024) as usize);
                reader.read_to_end(&mut bytes).map_err(map_lha_io_error)?;
                check_lha_crc(&reader, &path)?;
                return Ok(EntryReader {
                    entry,
                    access_cost: AccessCost::SequentialFromStart,
                    source: Box::new(Cursor::new(bytes)),
                    size: Some(header.original_size),
                });
            }
            index += 1;
            if !reader.next_file().map_err(map_lha_decode_error)? {
                break;
            }
        }

        Err(
            ArchiveError::new(ArchiveErrorKind::Internal, "Entry id not found")
                .with_backend("legacy"),
        )
    }

    fn test(&mut self, _options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        let mut reader = self.open_reader()?;
        loop {
            let header = reader.header().clone();
            let path = lha_path(&header);
            if !header.is_directory() {
                ensure_lha_decoder_supported(&reader, &header)?;
                std::io::copy(&mut reader, &mut std::io::sink()).map_err(map_lha_io_error)?;
                check_lha_crc(&reader, &path)?;
            }
            if !reader.next_file().map_err(map_lha_decode_error)? {
                break;
            }
        }

        Ok(TaskPlan::new(TaskKind::Test, "Test LHA/LZH archive")
            .native(quick_test_pipeline(vec![PipelineStep::ProbeArchive])))
    }
}

impl LhaArchive {
    fn open_reader(&self) -> Result<LhaDecodeReader<File>, ArchiveError> {
        let path = self.source.path().ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "LHA/LZH backend requires a local path",
            )
            .with_backend("legacy")
        })?;
        let file = File::open(path).map_err(io_error)?;
        LhaDecodeReader::new(file).map_err(map_lha_decode_error)
    }

    fn read_listing(&self) -> Result<ArchiveListing, ArchiveError> {
        let mut reader = self.open_reader()?;
        let mut listing = ArchiveListing::default();
        let mut index = 0_u64;
        loop {
            let header = reader.header();
            let path = lha_path(header);
            let method = lha_method(header);
            listing.entries.push(ArchiveEntry {
                id: EntryId(index),
                raw_path: path.clone(),
                normalized_path: path.clone(),
                display_path: path.clone(),
                kind: if header.is_directory() {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: Some(header.original_size),
                compressed_size: Some(header.compressed_size),
                modified_at: header.parse_last_modified().to_utc(),
                method: Some(method),
                encrypted: false,
                safety: classify_entry_path(&path),
            });
            index += 1;
            if !reader.next_file().map_err(map_lha_decode_error)? {
                break;
            }
        }
        listing.is_complete = true;
        Ok(listing)
    }

    fn extract_to(
        &mut self,
        destination: &Path,
        selected: Option<&[EntryId]>,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let listing = self.listing(ListingMode::Full)?;
        let mut reader = self.open_reader()?;
        let writer = SafeWriter::new(destination.to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(options.overwrite_policy);
        let mut processed = 0_usize;
        let mut index = 0_u64;
        loop {
            let header = reader.header().clone();
            let current = EntryId(index);
            let path = lha_path(&header);
            let should_extract = selected.is_none_or(|ids| ids.contains(&current));
            if should_extract {
                if header.is_directory() {
                    writer.create_dir(&path)?;
                } else {
                    ensure_lha_decoder_supported(&reader, &header)?;
                    writer.write_stream(&path, &mut reader, |_| Ok(()))?;
                    check_lha_crc(&reader, &path)?;
                }
                processed += 1;
            }
            index += 1;
            if !reader.next_file().map_err(map_lha_decode_error)? {
                break;
            }
        }

        Ok(TaskPlan::new(
            TaskKind::Extract,
            format!("Extract {} to {}", self.format, destination.display()),
        )
        .estimated_entries(selected.map(|ids| ids.len()).unwrap_or(processed))
        .native(sequential_extract_pipeline(vec![
            PipelineStep::ProbeArchive,
        ]))
        .warn(
            "sequential-archive",
            "LHA/LZH extraction scans entries sequentially from the beginning of the archive",
        )
        .warn(
            "listing-count",
            format!("Archive listing contains {} entries", listing.entries.len()),
        ))
    }
}

#[derive(Debug, Clone, Copy)]
struct DetectedLegacyFormat {
    format: ArchiveFormat,
    confidence: ProbeConfidence,
}

fn detect_legacy_format(
    source: &ArchiveSource,
) -> Result<Option<DetectedLegacyFormat>, ArchiveError> {
    let extension = source
        .path()
        .and_then(|path| path.extension())
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    let signature = match source.path() {
        Some(path) => read_signature(path)?,
        None => None,
    };

    if signature.as_deref().is_some_and(is_lha_signature) {
        return Ok(Some(DetectedLegacyFormat {
            format: extension_format(extension.as_deref()).unwrap_or(ArchiveFormat::Lzh),
            confidence: ProbeConfidence::Signature,
        }));
    }
    if signature
        .as_deref()
        .is_some_and(|bytes| bytes.starts_with(b"\x60\xea"))
    {
        return Ok(Some(signature_detected(ArchiveFormat::Arj)));
    }
    if signature
        .as_deref()
        .is_some_and(|bytes| bytes.starts_with(b"ALZ\x01"))
    {
        return Ok(Some(signature_detected(ArchiveFormat::Alz)));
    }
    if signature
        .as_deref()
        .is_some_and(|bytes| bytes.windows(7).any(|window| window == b"**ACE**"))
    {
        return Ok(Some(signature_detected(ArchiveFormat::Ace)));
    }
    if signature
        .as_deref()
        .is_some_and(|bytes| bytes.starts_with(b"MSWIM\0\0\0"))
    {
        return Ok(Some(signature_detected(ArchiveFormat::Wim)));
    }
    if signature
        .as_deref()
        .is_some_and(|bytes| bytes.starts_with(b"EGGA"))
    {
        return Ok(Some(signature_detected(ArchiveFormat::Egg)));
    }
    if signature
        .as_deref()
        .is_some_and(|bytes| bytes.starts_with(b"zPQ"))
    {
        return Ok(Some(signature_detected(ArchiveFormat::Zpaq)));
    }
    if signature
        .as_deref()
        .is_some_and(|bytes| bytes.starts_with(b"PEA"))
    {
        return Ok(Some(signature_detected(ArchiveFormat::Pea)));
    }
    if signature
        .as_deref()
        .is_some_and(|bytes| bytes.starts_with(b"PMA"))
    {
        return Ok(Some(signature_detected(ArchiveFormat::Pma)));
    }
    if matches!(extension.as_deref(), Some("bh"))
        && signature
            .as_deref()
            .is_some_and(|bytes| bytes.starts_with(b"BH"))
    {
        return Ok(Some(signature_detected(ArchiveFormat::Bh)));
    }

    Ok(
        extension_format(extension.as_deref()).map(|format| DetectedLegacyFormat {
            format,
            confidence: ProbeConfidence::Extension,
        }),
    )
}

fn extension_format(extension: Option<&str>) -> Option<ArchiveFormat> {
    match extension? {
        "lha" => Some(ArchiveFormat::Lha),
        "lzh" => Some(ArchiveFormat::Lzh),
        "ace" => Some(ArchiveFormat::Ace),
        "alz" => Some(ArchiveFormat::Alz),
        "arj" => Some(ArchiveFormat::Arj),
        "bh" => Some(ArchiveFormat::Bh),
        "egg" => Some(ArchiveFormat::Egg),
        "pma" => Some(ArchiveFormat::Pma),
        "wim" => Some(ArchiveFormat::Wim),
        "swm" => Some(ArchiveFormat::Swm),
        "zpaq" => Some(ArchiveFormat::Zpaq),
        "pea" => Some(ArchiveFormat::Pea),
        _ => None,
    }
}

fn signature_detected(format: ArchiveFormat) -> DetectedLegacyFormat {
    DetectedLegacyFormat {
        format,
        confidence: ProbeConfidence::Signature,
    }
}

fn read_signature(path: &Path) -> Result<Option<Vec<u8>>, ArchiveError> {
    if !path.is_file() {
        return Ok(None);
    }
    let mut file = fs_err::File::open(path).map_err(io_error)?;
    let mut bytes = vec![0; 64];
    let read = file.read(&mut bytes).map_err(io_error)?;
    bytes.truncate(read);
    Ok(Some(bytes))
}

fn is_lha_signature(bytes: &[u8]) -> bool {
    bytes
        .get(2..7)
        .is_some_and(|method| matches_lha_method(method))
}

fn matches_lha_method(method: &[u8]) -> bool {
    matches!(
        method,
        b"-lhd-"
            | b"-lzs-"
            | b"-lz4-"
            | b"-lz5-"
            | b"-lh0-"
            | b"-lh1-"
            | b"-lh4-"
            | b"-lh5-"
            | b"-lh6-"
            | b"-lh7-"
            | b"-lhx-"
            | b"-pm0-"
            | b"-pm1-"
            | b"-pm2-"
    )
}

fn lha_path(header: &delharc::LhaHeader) -> String {
    let path = header.parse_pathname_to_str().replace('\\', "/");
    if path.is_empty() {
        format!("entry-{:08x}", header.file_crc)
    } else {
        path
    }
}

fn lha_method(header: &delharc::LhaHeader) -> String {
    String::from_utf8_lossy(&header.compression).into_owned()
}

fn ensure_lha_decoder_supported(
    reader: &LhaDecodeReader<File>,
    header: &delharc::LhaHeader,
) -> Result<(), ArchiveError> {
    if reader.is_decoder_supported() || header.is_directory() {
        return Ok(());
    }
    Err(ArchiveError::new(
        ArchiveErrorKind::UnsupportedCodec,
        "LHA/LZH entry uses a compression method this backend cannot decode",
    )
    .with_backend("legacy")
    .with_technical_detail(format!("method={}", lha_method(header)))
    .with_entry_path(lha_path(header)))
}

fn check_lha_crc(reader: &LhaDecodeReader<File>, path: &str) -> Result<(), ArchiveError> {
    reader.crc_check().map(|_| ()).map_err(|error| {
        ArchiveError::new(ArchiveErrorKind::CorruptArchive, "LHA/LZH CRC check failed")
            .with_backend("legacy")
            .with_entry_path(path)
            .with_technical_detail(error.to_string())
    })
}

fn map_lha_decode_error(error: impl std::fmt::Display) -> ArchiveError {
    let text = error.to_string();
    let lower = text.to_ascii_lowercase();
    let kind = if lower.contains("unsupported") {
        ArchiveErrorKind::UnsupportedCodec
    } else if lower.contains("crc") || lower.contains("checksum") {
        ArchiveErrorKind::CorruptArchive
    } else if lower.contains("header") || lower.contains("too short") || lower.contains("eof") {
        ArchiveErrorKind::CorruptArchive
    } else {
        ArchiveErrorKind::CorruptArchive
    };
    ArchiveError::new(kind, "LHA/LZH operation failed")
        .with_backend("legacy")
        .with_technical_detail(text)
}

fn map_lha_io_error(error: std::io::Error) -> ArchiveError {
    let text = error.to_string();
    let kind = if text.to_ascii_lowercase().contains("unsupported") {
        ArchiveErrorKind::UnsupportedCodec
    } else {
        ArchiveErrorKind::CorruptArchive
    };
    ArchiveError::new(kind, "LHA/LZH decode failed")
        .with_backend("legacy")
        .with_technical_detail(text)
}

fn unsupported_format(format: ArchiveFormat, message: impl Into<String>) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::UnsupportedFormat, message)
        .with_backend("legacy")
        .with_technical_detail(format!("format={format}"))
}

fn io_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "I/O operation failed")
        .with_backend("legacy")
        .with_technical_detail(error.to_string())
}

fn supported_formats() -> Vec<ArchiveFormat> {
    vec![ArchiveFormat::Lha, ArchiveFormat::Lzh]
}

fn legacy_backend_capabilities() -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Limited,
        extract_all: CapabilityLevel::Limited,
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

fn lha_capabilities() -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Full,
        extract_all: CapabilityLevel::High,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_lha_signature() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.bin");
        fs_err::write(&path, b"\x20\x00-lh7-\0").unwrap();

        let backend = LegacyArchiveBackend;
        let probe = backend
            .probe(&ArchiveSource::LocalPath(path))
            .expect("probe should succeed");

        assert_eq!(probe.format, ArchiveFormat::Lzh);
        assert_eq!(probe.confidence, ProbeConfidence::Signature);
    }

    #[test]
    fn probes_arj_signature() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.dat");
        fs_err::write(&path, b"\x60\xea\x00\x00").unwrap();

        let backend = LegacyArchiveBackend;
        let probe = backend
            .probe(&ArchiveSource::LocalPath(path))
            .expect("probe should succeed");

        assert_eq!(probe.format, ArchiveFormat::Arj);
        assert_eq!(probe.confidence, ProbeConfidence::Signature);
    }

    #[test]
    fn reports_unsupported_placeholder_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.ace");
        fs_err::write(&path, b"not enough").unwrap();

        let backend = LegacyArchiveBackend;
        let error = match backend.open(ArchiveSource::LocalPath(path), OpenOptions::default()) {
            Ok(_) => panic!("ACE should be recognized but unsupported"),
            Err(error) => error,
        };

        assert_eq!(error.kind, ArchiveErrorKind::UnsupportedFormat);
        assert_eq!(error.backend.as_deref(), Some("legacy"));
    }
}
