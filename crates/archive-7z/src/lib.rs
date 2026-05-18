use std::{
    fs::{self, File},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use shadow_zip_archive_core::{
    ArchiveBackend, ByteSource, EntryReader, InputScanner, OpenArchive, SafeWriter, ScannedInput,
    StreamLimits, create_pipeline, extension_confidence, quick_test_pipeline,
    random_access_extract_pipeline,
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
        let mut plan = TaskPlan::new(TaskKind::Create, format!("Create 7z {}", output.display()))
            .estimated_entries(inputs.len())
            .native(create_pipeline());
        if options.solid {
            plan = plan.warn(
                "7z-solid",
                "7z creation uses per-entry streams; solid archive creation is not enabled in this path",
            );
        }
        if options.volume_size.is_some() {
            plan = plan.warn(
                "7z-volume",
                "7z multi-volume output is emitted by splitting the completed native 7z stream",
            );
        }
        Ok(plan)
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![ArchiveFormat::SevenZip],
            capabilities: seven_zip_capabilities(false),
        }
    }
}

pub fn create_7z_archive(
    inputs: &[InputPath],
    output: &Path,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    let volume_size = options.volume_size;
    let scanned = InputScanner::scan(inputs)?;
    let write_target = if volume_size.is_some() {
        temporary_7z_path(output)
    } else {
        output.to_path_buf()
    };
    let mut writer = sevenz_rust2::SevenZWriter::create(&write_target).map_err(map_7z_error)?;
    writer.set_encrypt_header(options.encrypt_file_names && options.password.is_some());
    writer.set_content_methods(seven_zip_methods(&options));
    for input in &scanned {
        push_scanned_entry(&mut writer, input)?;
    }
    writer.finish().map_err(map_7z_error)?;
    if let Some(volume_size) = volume_size {
        split_7z_volumes(&write_target, output, volume_size)?;
        fs::remove_file(&write_target).map_err(|error| {
            ArchiveError::new(ArchiveErrorKind::Io, "temporary 7z file cleanup failed")
                .with_backend("7z")
                .with_technical_detail(error.to_string())
        })?;
    }
    Ok(())
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
                    let mut source = SevenZipEntrySource { inner: source };
                    writer
                        .write_stream(&entry_path, &mut source, |_| Ok(()))
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

struct SevenZipEntrySource<'a, R: std::io::Read + ?Sized> {
    inner: &'a mut R,
}

impl<R: std::io::Read + ?Sized> ByteSource for SevenZipEntrySource<'_, R> {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buffer)
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

fn temporary_7z_path(output: &Path) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let file_name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive.7z");
    output.with_file_name(format!("{file_name}.{stamp}.tmp"))
}

fn split_7z_volumes(source: &Path, output: &Path, volume_size: u64) -> Result<(), ArchiveError> {
    let mut input = File::open(source).map_err(|error| {
        ArchiveError::new(ArchiveErrorKind::Io, "temporary 7z file open failed")
            .with_backend("7z")
            .with_technical_detail(error.to_string())
    })?;
    let base = volume_base_path(output);
    let mut index = 1_u32;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let part_path = numbered_volume_path(&base, index);
        let mut part = File::create(&part_path).map_err(|error| {
            ArchiveError::new(ArchiveErrorKind::Io, "7z volume file create failed")
                .with_backend("7z")
                .with_technical_detail(error.to_string())
        })?;
        let mut remaining = volume_size;
        let mut wrote_any = false;
        while remaining > 0 {
            let chunk = buffer.len().min(remaining as usize);
            let read = input.read(&mut buffer[..chunk]).map_err(|error| {
                ArchiveError::new(ArchiveErrorKind::Io, "temporary 7z file read failed")
                    .with_backend("7z")
                    .with_technical_detail(error.to_string())
            })?;
            if read == 0 {
                if !wrote_any {
                    let _ = fs::remove_file(&part_path);
                }
                return Ok(());
            }
            part.write_all(&buffer[..read]).map_err(|error| {
                ArchiveError::new(ArchiveErrorKind::Io, "7z volume file write failed")
                    .with_backend("7z")
                    .with_technical_detail(error.to_string())
            })?;
            remaining -= read as u64;
            wrote_any = true;
        }
        index += 1;
    }
}

fn volume_base_path(output: &Path) -> PathBuf {
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive.7z");
    if let Some(base) = name.strip_suffix(".001") {
        output.with_file_name(base)
    } else {
        output.to_path_buf()
    }
}

fn numbered_volume_path(base: &Path, index: u32) -> PathBuf {
    let name = base
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("archive.7z");
    base.with_file_name(format!("{name}.{index:03}"))
}

fn seven_zip_methods(options: &CreateOptions) -> Vec<sevenz_rust2::SevenZMethodConfiguration> {
    let mut methods = Vec::new();
    if let Some(password) = options.password.as_ref() {
        methods.push(sevenz_rust2::AesEncoderOptions::new(password.as_str().into()).into());
    }
    methods.push(sevenz_rust2::SevenZMethod::LZMA2.into());
    methods
}

fn push_scanned_entry(
    writer: &mut sevenz_rust2::SevenZWriter<File>,
    input: &ScannedInput,
) -> Result<(), ArchiveError> {
    let entry = sevenz_rust2::SevenZArchiveEntry::from_path(
        &input.source_path,
        input.archive_path.clone(),
    );
    if input.is_dir {
        writer
            .push_archive_entry::<&[u8]>(entry, None)
            .map_err(map_7z_error)?;
    } else {
        let source = File::open(&input.source_path).map_err(|error| {
            ArchiveError::new(ArchiveErrorKind::Io, "7z input file open failed")
                .with_backend("7z")
                .with_entry_path(input.archive_path.clone())
                .with_technical_detail(error.to_string())
        })?;
        writer
            .push_archive_entry(entry, Some(source))
            .map_err(map_7z_error)?;
    }
    Ok(())
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
        create: CapabilityLevel::Full,
        update: CapabilityLevel::Unsupported,
        random_access: if solid {
            CapabilityLevel::Limited
        } else {
            CapabilityLevel::High
        },
        password_read: CapabilityLevel::Full,
        password_write: CapabilityLevel::Full,
        header_encryption: CapabilityLevel::Full,
        multi_volume_read: CapabilityLevel::Medium,
        multi_volume_write: CapabilityLevel::Limited,
        entry_stream_preview: if solid {
            CapabilityLevel::Limited
        } else {
            CapabilityLevel::High
        },
    }
}
