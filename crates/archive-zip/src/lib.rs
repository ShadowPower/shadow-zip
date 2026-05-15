use std::{
    fs::File,
    io::Cursor,
    path::{Path, PathBuf},
};

use shadow_zip_archive_core::{
    ArchiveBackend, EntryReader, InputScanner, OpenArchive, SafeWriter, StreamLimits,
    create_pipeline, extension_confidence, quick_test_pipeline, random_access_extract_pipeline,
};
use shadow_zip_domain::*;
use zip::{ZipWriter, read::ZipArchive as ZipReader, write::SimpleFileOptions};

pub struct ZipBackend;

impl ArchiveBackend for ZipBackend {
    fn name(&self) -> &'static str {
        "zip"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        Ok(ProbeResult {
            format: ArchiveFormat::Zip,
            confidence: extension_confidence(source, &["zip", "zipx"]),
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        _options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        Ok(Box::new(ZipArchive {
            source,
            info: ArchiveInfo {
                format: ArchiveFormat::Zip,
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
            formats: vec![ArchiveFormat::Zip],
            capabilities: zip_capabilities(),
        }
    }
}

struct ZipArchive {
    source: ArchiveSource,
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
        _options: StreamOptions,
    ) -> Result<EntryReader, ArchiveError> {
        let mut reader = self.open_reader()?;
        let mut file = reader.by_index(entry.0 as usize).map_err(zip_error)?;
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

    fn test(&mut self, _options: TestOptions) -> Result<TaskPlan, ArchiveError> {
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
            let file = reader.by_index(index).map_err(zip_error)?;
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
            let mut file = reader.by_index(entry.id.0 as usize).map_err(zip_error)?;
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
    let file = File::create(output).map_err(io_error)?;
    let mut writer = ZipWriter::new(file);
    let zip_options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

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

    let _ = options;
    writer.finish().map_err(zip_error)?;
    Ok(())
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

fn zip_error(error: zip::result::ZipError) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::CorruptArchive, "ZIP operation failed")
        .with_backend("zip")
        .with_technical_detail(error.to_string())
}

fn io_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "I/O operation failed")
        .with_backend("zip")
        .with_technical_detail(error.to_string())
}
