use std::{io::Cursor, path::Path};

use shadow_zip_archive_core::{
    ArchiveBackend, EntryReader, OpenArchive, SafeWriter, StreamLimits, create_pipeline,
    extension_confidence, quick_test_pipeline, random_access_extract_pipeline,
};
use shadow_zip_domain::*;

pub struct SevenZipBackend;

impl ArchiveBackend for SevenZipBackend {
    fn name(&self) -> &'static str {
        "7z"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        Ok(ProbeResult {
            format: ArchiveFormat::SevenZip,
            confidence: extension_confidence(source, &["7z"]),
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        let is_multi_volume = source.path().is_some_and(is_split_7z_volume);
        let display_name = source.display_name();
        Ok(Box::new(SevenZipArchive {
            source,
            password: options.password,
            info: ArchiveInfo {
                format: ArchiveFormat::SevenZip,
                display_name,
                total_bytes: None,
                entry_count: None,
                codecs: vec!["LZMA2".into(), "ZSTD".into(), "LZ4".into(), "Brotli".into()],
                filters: Vec::new(),
                is_solid: false,
                is_encrypted: false,
                has_header_encryption: false,
                is_multi_volume,
            },
        }))
    }

    fn create_plan(
        &self,
        inputs: &[InputPath],
        output: &Path,
        options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let plan = TaskPlan::new(TaskKind::Create, format!("Create {}", output.display()))
            .estimated_entries(inputs.len())
            .native(create_pipeline());
        Ok(if options.solid {
            plan.warn(
                "solid-access-cost",
                "Solid archives make single-file preview and extraction slower",
            )
        } else {
            plan
        })
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![ArchiveFormat::SevenZip],
            capabilities: seven_zip_capabilities(false),
        }
    }
}

struct SevenZipArchive {
    source: ArchiveSource,
    password: Option<String>,
    info: ArchiveInfo,
}

impl OpenArchive for SevenZipArchive {
    fn info(&self) -> ArchiveInfo {
        self.info.clone()
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        seven_zip_capabilities(self.info.is_solid)
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        let Some(path) = self.source.path() else {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "7z listing requires a local file",
            ));
        };
        let mut listing = ArchiveListing::default();
        let archive = sevenz_rust2::Archive::open_with_password(
            path,
            &password_from_option(self.password.as_ref()),
        )
        .map_err(map_7z_error)?;
        for (index, entry) in archive.files.iter().enumerate() {
            let path = entry.name.replace('\\', "/");
            listing.entries.push(ArchiveEntry {
                id: EntryId(index as u64),
                raw_path: path.clone(),
                normalized_path: path.clone(),
                display_path: path,
                kind: if entry.is_directory {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
                size: Some(entry.size),
                compressed_size: Some(entry.compressed_size),
                modified_at: None,
                method: None,
                encrypted: archive
                    .folders
                    .iter()
                    .flat_map(|folder| &folder.coders)
                    .any(|coder| coder.decompression_method_id() == [0x06, 0xf1, 0x07, 0x01]),
                safety: classify_entry_path(&entry.name),
            });
        }
        listing.is_complete = true;
        self.info.entry_count = Some(listing.entries.len() as u64);
        self.info.is_solid = archive.is_solid;
        self.info.is_encrypted = listing.entries.iter().any(|entry| entry.encrypted);
        self.info.has_header_encryption = false;
        Ok(listing)
    }

    fn extract_all(
        &mut self,
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        self.extract_to(destination, None, options)?;
        Ok(self.extract_plan(destination, None))
    }

    fn extract_selected(
        &mut self,
        entries: &[EntryId],
        destination: &Path,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        self.extract_to(destination, Some(entries), options)?;
        Ok(self.extract_plan(destination, Some(entries.len())))
    }

    fn open_entry_stream(
        &mut self,
        entry: EntryId,
        _options: StreamOptions,
    ) -> Result<EntryStream, ArchiveError> {
        Ok(EntryStream {
            entry,
            access_cost: if self.info.is_solid {
                AccessCost::SolidBlockScan
            } else {
                AccessCost::Random
            },
        })
    }

    fn open_entry_reader(
        &mut self,
        entry: EntryId,
        options: StreamOptions,
    ) -> Result<EntryReader, ArchiveError> {
        let Some(path) = self.source.path() else {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "7z entry reading requires a local file",
            ));
        };
        let password = options.password.as_ref().or(self.password.as_ref());
        let mut reader = sevenz_rust2::SevenZReader::open(path, password_from_option(password))
            .map_err(map_7z_error)?;
        let mut bytes = Vec::new();
        let mut found = false;
        let mut index = 0_u64;
        reader
            .for_each_entries(|archive_entry, source| {
                let current = EntryId(index);
                index += 1;
                if archive_entry.is_directory() {
                    return Ok(true);
                }
                if found {
                    return Ok(false);
                }
                if current != entry {
                    return Ok(true);
                }
                std::io::copy(source, &mut bytes).map_err(sevenz_rust2::Error::io)?;
                found = true;
                Ok(false)
            })
            .map_err(map_7z_error)?;
        if !found {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Entry id not found",
            ));
        }
        let size = Some(bytes.len() as u64);
        Ok(EntryReader {
            entry,
            access_cost: if self.info.is_solid {
                AccessCost::SolidBlockScan
            } else {
                AccessCost::Random
            },
            source: Box::new(Cursor::new(bytes)),
            size,
        })
    }

    fn test(&mut self, options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        self.test_archive(options)?;
        Ok(
            TaskPlan::new(TaskKind::Test, "Test 7z archive").native(quick_test_pipeline(vec![
                PipelineStep::ReadSevenZipHeader,
                PipelineStep::ProbeArchive,
            ])),
        )
    }
}

impl SevenZipArchive {
    fn extract_to(
        &self,
        destination: &Path,
        selected: Option<&[EntryId]>,
        options: ExtractOptions,
    ) -> Result<(), ArchiveError> {
        let Some(path) = self.source.path() else {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "7z extraction requires a local file",
            ));
        };
        let password = options.password.as_ref().or(self.password.as_ref());
        let mut reader = sevenz_rust2::SevenZReader::open(path, password_from_option(password))
            .map_err(map_7z_error)?;
        let writer = SafeWriter::new(destination.to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(options.overwrite_policy);
        let mut index = 0_u64;
        reader
            .for_each_entries(|entry, source| {
                let current = EntryId(index);
                index += 1;
                if selected.is_some_and(|ids| !ids.contains(&current)) {
                    return Ok(true);
                }
                let entry_path = entry.name().replace('\\', "/");
                if entry.is_directory() {
                    writer
                        .create_dir(&entry_path)
                        .map_err(archive_error_to_7z)?;
                } else {
                    let mut bytes = Vec::with_capacity(entry.size().min(16 * 1024 * 1024) as usize);
                    std::io::copy(source, &mut bytes).map_err(sevenz_rust2::Error::io)?;
                    writer
                        .write_stream(&entry_path, &mut Cursor::new(bytes), |_| Ok(()))
                        .map_err(archive_error_to_7z)?;
                }
                Ok(true)
            })
            .map_err(map_7z_error)
    }

    fn test_archive(&self, options: TestOptions) -> Result<(), ArchiveError> {
        let Some(path) = self.source.path() else {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "7z testing requires a local file",
            ));
        };
        let password = options.password.as_ref().or(self.password.as_ref());
        let mut reader = sevenz_rust2::SevenZReader::open(path, password_from_option(password))
            .map_err(map_7z_error)?;
        reader
            .for_each_entries(|_entry, source| {
                std::io::copy(source, &mut std::io::sink()).map_err(sevenz_rust2::Error::io)?;
                Ok(true)
            })
            .map_err(map_7z_error)
    }

    fn extract_plan(&self, destination: &Path, entries: Option<usize>) -> TaskPlan {
        let plan = TaskPlan::new(
            TaskKind::Extract,
            format!("Extract to {}", destination.display()),
        )
        .native(random_access_extract_pipeline(
            PipelineStep::ReadSevenZipHeader,
        ));
        let has_selected_entries = entries.is_some();
        let plan = if let Some(count) = entries {
            plan.estimated_entries(count)
        } else {
            plan
        };
        if self.info.is_solid && has_selected_entries {
            plan.warn(
                "solid-scan",
                "Selected extraction from a solid 7z archive may need to decode preceding files",
            )
        } else {
            plan
        }
    }
}

fn password_from_option(password: Option<&String>) -> sevenz_rust2::Password {
    password
        .map(|password| sevenz_rust2::Password::from(password.as_str()))
        .unwrap_or_else(sevenz_rust2::Password::empty)
}

fn archive_error_to_7z(error: ArchiveError) -> sevenz_rust2::Error {
    sevenz_rust2::Error::io(std::io::Error::other(error.to_string()))
}

fn is_split_7z_volume(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.ends_with(".7z.001") || name.ends_with(".7z.002") || name.contains(".7z.")
}

fn map_7z_error(error: impl std::fmt::Display) -> ArchiveError {
    let text = error.to_string();
    let kind = if text.to_ascii_lowercase().contains("password") {
        ArchiveErrorKind::InvalidPassword
    } else if text.to_ascii_lowercase().contains("unsupported codec") {
        ArchiveErrorKind::UnsupportedCodec
    } else if text.to_ascii_lowercase().contains("filter") {
        ArchiveErrorKind::UnsupportedFilter
    } else {
        ArchiveErrorKind::CorruptArchive
    };
    ArchiveError::new(kind, "7z operation failed")
        .with_backend("7z")
        .with_technical_detail(text)
}

fn seven_zip_capabilities(solid: bool) -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Full,
        extract_all: CapabilityLevel::Full,
        extract_selected: if solid {
            CapabilityLevel::Limited
        } else {
            CapabilityLevel::High
        },
        create: CapabilityLevel::High,
        update: CapabilityLevel::Medium,
        random_access: if solid {
            CapabilityLevel::Limited
        } else {
            CapabilityLevel::High
        },
        password_read: CapabilityLevel::Full,
        password_write: CapabilityLevel::Full,
        header_encryption: CapabilityLevel::Full,
        multi_volume_read: CapabilityLevel::Medium,
        multi_volume_write: CapabilityLevel::Medium,
        entry_stream_preview: if solid {
            CapabilityLevel::Limited
        } else {
            CapabilityLevel::High
        },
    }
}
