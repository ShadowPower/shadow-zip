use std::{
    fs::File,
    io::{Cursor, Read, Write},
    path::Path,
};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use shadow_zip_archive_core::{
    ArchiveBackend, InputScanner, OpenArchive, SafeWriter, StreamLimits, create_pipeline,
    quick_test_pipeline, sequential_extract_pipeline,
};
use shadow_zip_domain::*;
use tar::{Archive as TarReader, Builder as TarBuilder};
use xz2::{read::XzDecoder, write::XzEncoder};

pub struct TarBackend;

impl ArchiveBackend for TarBackend {
    fn name(&self) -> &'static str {
        "tar-stream"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        let format = detect_tar_format(&source.display_name());
        Ok(ProbeResult {
            format,
            confidence: if format == ArchiveFormat::Unknown {
                ProbeConfidence::Impossible
            } else {
                ProbeConfidence::Extension
            },
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        _options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        let format = self.probe(&source)?.format;
        Ok(Box::new(TarArchive {
            source,
            info: ArchiveInfo {
                format,
                display_name: "tar archive".into(),
                total_bytes: None,
                entry_count: None,
                codecs: tar_codecs(format),
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
        Ok(TaskPlan::new(
            TaskKind::Create,
            format!("Stream create {}", output.display()),
        )
        .estimated_entries(inputs.len())
        .native(create_pipeline()))
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![
                ArchiveFormat::Tar,
                ArchiveFormat::TarGz,
                ArchiveFormat::TarXz,
                ArchiveFormat::TarZst,
            ],
            capabilities: tar_capabilities(),
        }
    }
}

struct TarArchive {
    source: ArchiveSource,
    info: ArchiveInfo,
    listing_cache: Option<ArchiveListing>,
}

impl OpenArchive for TarArchive {
    fn info(&self) -> ArchiveInfo {
        let mut info = self.info.clone();
        info.display_name = self.source.display_name();
        info
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        tar_capabilities()
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
        self.extract(destination, None, options)
    }

    fn extract_selected(
        &mut self,
        entries: &[EntryId],
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        self.extract(destination, Some(entries), options)
            .map(|plan| {
                plan.warn(
                    "sequential-access",
                    "tar streams may need to scan from the beginning to reach selected entries",
                )
            })
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

    fn test(&mut self, _options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        Ok(
            TaskPlan::new(TaskKind::Test, "Stream test tar archive").native(quick_test_pipeline(
                vec![
                    PipelineStep::StreamTarEntries,
                    PipelineStep::ValidateEntryPath,
                ],
            )),
        )
    }
}

impl TarArchive {
    fn source_path(&self) -> Result<&Path, ArchiveError> {
        self.source.path().ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "tar backend requires a local path",
            )
        })
    }

    fn open_reader(&self) -> Result<Box<dyn Read>, ArchiveError> {
        let file = File::open(self.source_path()?).map_err(io_error)?;
        Ok(match self.info.format {
            ArchiveFormat::TarGz => Box::new(GzDecoder::new(file)),
            ArchiveFormat::TarXz => Box::new(XzDecoder::new(file)),
            ArchiveFormat::TarZst => {
                Box::new(zstd::stream::read::Decoder::new(file).map_err(io_error)?)
            }
            _ => Box::new(file),
        })
    }

    fn read_listing(&self) -> Result<ArchiveListing, ArchiveError> {
        let reader = self.open_reader()?;
        let mut archive = TarReader::new(reader);
        let mut entries = Vec::new();

        for (index, entry) in archive.entries().map_err(io_error)?.enumerate() {
            let entry = entry.map_err(io_error)?;
            let path = entry
                .path()
                .map_err(io_error)?
                .to_string_lossy()
                .into_owned();
            let kind = if entry.header().entry_type().is_dir() {
                EntryKind::Directory
            } else if entry.header().entry_type().is_symlink() {
                EntryKind::Symlink
            } else {
                EntryKind::File
            };
            entries.push(ArchiveEntry {
                id: EntryId(index as u64),
                raw_path: path.clone(),
                normalized_path: path.replace('\\', "/"),
                display_path: path.clone(),
                kind,
                size: entry.header().size().ok(),
                compressed_size: None,
                modified_at: None,
                method: self.info.codecs.first().cloned(),
                encrypted: false,
                safety: classify_entry_path(&path),
            });
        }

        Ok(ArchiveListing {
            entries,
            directories: Default::default(),
            is_complete: true,
        })
    }

    fn extract(
        &mut self,
        destination: &Path,
        selected: Option<&[EntryId]>,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let listing = self.listing(ListingMode::Full)?;
        let writer = SafeWriter::new(destination.to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(options.overwrite_policy);
        let reader = self.open_reader()?;
        let mut archive = TarReader::new(reader);

        for (index, entry) in archive.entries().map_err(io_error)?.enumerate() {
            let entry_id = EntryId(index as u64);
            if selected
                .map(|ids| !ids.contains(&entry_id))
                .unwrap_or(false)
            {
                continue;
            }
            let mut entry = entry.map_err(io_error)?;
            let path = entry
                .path()
                .map_err(io_error)?
                .to_string_lossy()
                .into_owned();
            if entry.header().entry_type().is_dir() {
                writer.create_dir(&path)?;
            } else {
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).map_err(io_error)?;
                let mut source = Cursor::new(bytes);
                writer.write_stream(&path, &mut source, |_| Ok(()))?;
            }
        }

        Ok(TaskPlan::new(
            TaskKind::Extract,
            format!("Stream extract to {}", destination.display()),
        )
        .estimated_entries(
            selected
                .map(|ids| ids.len())
                .unwrap_or(listing.entries.len()),
        )
        .native(sequential_extract_pipeline(tar_read_steps(
            self.info.format,
        ))))
    }
}

pub fn create_tar_archive(
    inputs: &[InputPath],
    output: &Path,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    let file = File::create(output).map_err(io_error)?;
    let writer: Box<dyn Write> = match options.format {
        ArchiveFormat::TarGz => Box::new(GzEncoder::new(file, Compression::default())),
        ArchiveFormat::TarXz => Box::new(XzEncoder::new(
            file,
            options.compression_level.unwrap_or(6) as u32,
        )),
        ArchiveFormat::TarZst => Box::new(
            zstd::stream::write::Encoder::new(file, options.compression_level.unwrap_or(3) as i32)
                .map_err(io_error)?,
        ),
        _ => Box::new(file),
    };
    let mut builder = TarBuilder::new(writer);
    for input in InputScanner::scan(inputs)? {
        if input.is_dir {
            builder
                .append_dir(input.archive_path, &input.source_path)
                .map_err(io_error)?;
        } else {
            builder
                .append_path_with_name(&input.source_path, input.archive_path)
                .map_err(io_error)?;
        }
    }
    builder.finish().map_err(io_error)
}

fn detect_tar_format(name: &str) -> ArchiveFormat {
    let name = name.to_ascii_lowercase();
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        ArchiveFormat::TarGz
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        ArchiveFormat::TarXz
    } else if name.ends_with(".tar.zst") || name.ends_with(".tzst") {
        ArchiveFormat::TarZst
    } else if name.ends_with(".tar") {
        ArchiveFormat::Tar
    } else {
        ArchiveFormat::Unknown
    }
}

fn tar_read_steps(format: ArchiveFormat) -> Vec<PipelineStep> {
    let mut steps = Vec::new();
    match format {
        ArchiveFormat::TarGz => steps.push(PipelineStep::StreamDecompress {
            codec: CompressionMethod::Gzip,
        }),
        ArchiveFormat::TarXz => steps.push(PipelineStep::StreamDecompress {
            codec: CompressionMethod::Xz,
        }),
        ArchiveFormat::TarZst => steps.push(PipelineStep::StreamDecompress {
            codec: CompressionMethod::Zstandard,
        }),
        _ => {}
    }
    steps.push(PipelineStep::StreamTarEntries);
    steps
}

fn tar_capabilities() -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Medium,
        extract_all: CapabilityLevel::Full,
        extract_selected: CapabilityLevel::Limited,
        create: CapabilityLevel::Full,
        update: CapabilityLevel::Unsupported,
        random_access: CapabilityLevel::Unsupported,
        password_read: CapabilityLevel::Unsupported,
        password_write: CapabilityLevel::Unsupported,
        header_encryption: CapabilityLevel::Unsupported,
        multi_volume_read: CapabilityLevel::Unsupported,
        multi_volume_write: CapabilityLevel::Unsupported,
        entry_stream_preview: CapabilityLevel::Limited,
    }
}

fn tar_codecs(format: ArchiveFormat) -> Vec<String> {
    match format {
        ArchiveFormat::TarGz => vec!["gzip".into()],
        ArchiveFormat::TarXz => vec!["xz".into()],
        ArchiveFormat::TarZst => vec!["zstd".into()],
        _ => Vec::new(),
    }
}

fn io_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "tar I/O operation failed")
        .with_backend("tar-stream")
        .with_technical_detail(error.to_string())
}
