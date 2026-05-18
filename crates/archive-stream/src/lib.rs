use std::{
    fs::File,
    io::{self, Cursor, Read, Write},
    path::Path,
};

use brotli::{CompressorWriter, Decompressor};
use bzip2::{Compression as Bzip2Compression, read::BzDecoder, write::BzEncoder};
use flate2::{Compression as GzipCompression, read::GzDecoder, write::GzEncoder};
use fs_err as fs;
use lz4_flex::frame::{FrameDecoder as Lz4Decoder, FrameEncoder as Lz4Encoder};
use shadow_zip_archive_core::{
    ArchiveBackend, ByteSource, EntryReader, InputScanner, OpenArchive, SafeWriter, ScannedInput,
    StreamLimits, StreamPump, create_pipeline, quick_test_pipeline, sequential_extract_pipeline,
};
use shadow_zip_domain::*;
use tar::{Archive as TarReader, Builder as TarBuilder};
use xz2::{
    read::XzDecoder,
    stream::{LzmaOptions, Stream as XzStream},
    write::XzEncoder,
};

pub struct StreamBackend;

impl ArchiveBackend for StreamBackend {
    fn name(&self) -> &'static str {
        "stream-codec"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        let spec = detect_stream_format(&source.display_name());
        Ok(ProbeResult {
            format: spec.map_or(ArchiveFormat::Unknown, |spec| spec.archive_format()),
            confidence: if spec.is_some() {
                ProbeConfidence::Extension
            } else {
                ProbeConfidence::Impossible
            },
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        _options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        let spec = detect_stream_format(&source.display_name()).ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "stream backend does not recognize this extension",
            )
        })?;
        Ok(Box::new(StreamArchive {
            source,
            spec,
            listing_cache: None,
        }))
    }

    fn create_plan(
        &self,
        inputs: &[InputPath],
        output: &Path,
        options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let spec = detect_create_format(output, &options)?;
        Ok(TaskPlan::new(
            TaskKind::Create,
            format!("Stream create {}", output.display()),
        )
        .estimated_entries(inputs.len())
        .native(create_pipeline())
        .warn(
            "stream-codec",
            format!("create path uses streaming {}", spec.label()),
        ))
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![
                ArchiveFormat::TarGz,
                ArchiveFormat::TarXz,
                ArchiveFormat::TarZst,
                ArchiveFormat::TarBz,
                ArchiveFormat::TarBz2,
                ArchiveFormat::TarLzma,
                ArchiveFormat::TarBr,
                ArchiveFormat::Gz,
                ArchiveFormat::Xz,
                ArchiveFormat::Bz,
                ArchiveFormat::Bz2,
                ArchiveFormat::Zstd,
                ArchiveFormat::Lz4,
                ArchiveFormat::Br,
                ArchiveFormat::Lzma,
            ],
            capabilities: stream_capabilities(),
        }
    }
}

struct StreamArchive {
    source: ArchiveSource,
    spec: StreamFormatSpec,
    listing_cache: Option<ArchiveListing>,
}

impl OpenArchive for StreamArchive {
    fn info(&self) -> ArchiveInfo {
        ArchiveInfo {
            format: self.spec.archive_format(),
            display_name: self.source.display_name(),
            total_bytes: self
                .source
                .path()
                .and_then(|path| fs::metadata(path).ok())
                .map(|metadata| metadata.len()),
            entry_count: (self.spec.container == StreamContainer::Single).then_some(1),
            codecs: vec![self.spec.codec.name().into()],
            filters: Vec::new(),
            is_solid: false,
            is_encrypted: false,
            has_header_encryption: false,
            is_multi_volume: false,
        }
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        stream_capabilities()
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        if let Some(listing) = &self.listing_cache {
            return Ok(listing.clone());
        }
        let listing = match self.spec.container {
            StreamContainer::Single => self.single_listing()?,
            StreamContainer::Tar => self.tar_listing()?,
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
            .map(|plan| {
                plan.warn(
                    "sequential-access",
                    "streamed codecs are sequential and may scan from the beginning",
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

    fn open_entry_reader(
        &mut self,
        entry: EntryId,
        _options: StreamOptions,
    ) -> Result<EntryReader, ArchiveError> {
        match self.spec.container {
            StreamContainer::Single => {
                if entry != EntryId(0) {
                    return Err(entry_not_found());
                }
                Ok(EntryReader {
                    entry,
                    access_cost: AccessCost::SequentialFromStart,
                    source: Box::new(ReadSource::new(self.open_decoder()?)),
                    size: None,
                })
            }
            StreamContainer::Tar => self.open_tar_entry_reader(entry),
        }
    }

    fn test(&mut self, _options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        Ok(
            TaskPlan::new(TaskKind::Test, format!("Stream test {}", self.spec.label()))
                .native(quick_test_pipeline(read_steps(self.spec))),
        )
    }
}

impl StreamArchive {
    fn source_path(&self) -> Result<&Path, ArchiveError> {
        self.source.path().ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "stream backend requires a local path",
            )
        })
    }

    fn open_decoder(&self) -> Result<Box<dyn Read + Send>, ArchiveError> {
        let file = File::open(self.source_path()?).map_err(io_error)?;
        decoder_for(self.spec.codec, file)
    }

    fn single_listing(&self) -> Result<ArchiveListing, ArchiveError> {
        let path = self.source_path()?;
        let compressed_size = fs::metadata(path).ok().map(|metadata| metadata.len());
        let raw_path = self.spec.single_entry_name(path);
        Ok(ArchiveListing {
            entries: vec![ArchiveEntry {
                id: EntryId(0),
                raw_path: raw_path.clone(),
                normalized_path: raw_path.replace('\\', "/"),
                display_path: raw_path.clone(),
                kind: EntryKind::File,
                size: None,
                compressed_size,
                modified_at: None,
                method: Some(self.spec.codec.name().into()),
                encrypted: false,
                safety: classify_entry_path(&raw_path),
            }],
            directories: Default::default(),
            is_complete: true,
        })
    }

    fn tar_listing(&self) -> Result<ArchiveListing, ArchiveError> {
        let reader = self.open_decoder()?;
        let mut archive = TarReader::new(reader);
        let mut entries = Vec::new();
        for (index, entry) in archive.entries().map_err(io_error)?.enumerate() {
            let entry = entry.map_err(io_error)?;
            let path = entry
                .path()
                .map_err(io_error)?
                .to_string_lossy()
                .into_owned();
            entries.push(ArchiveEntry {
                id: EntryId(index as u64),
                raw_path: path.clone(),
                normalized_path: path.replace('\\', "/"),
                display_path: path.clone(),
                kind: tar_entry_kind(entry.header().entry_type()),
                size: entry.header().size().ok(),
                compressed_size: None,
                modified_at: None,
                method: Some(self.spec.codec.name().into()),
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
        match self.spec.container {
            StreamContainer::Single => self.extract_single(destination, selected, options)?,
            StreamContainer::Tar => self.extract_tar(destination, selected, options)?,
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
        .native(sequential_extract_pipeline(read_steps(self.spec))))
    }

    fn extract_single(
        &self,
        destination: &Path,
        selected: Option<&[EntryId]>,
        options: ExtractOptions,
    ) -> Result<(), ArchiveError> {
        if selected
            .map(|ids| !ids.contains(&EntryId(0)))
            .unwrap_or(false)
        {
            return Ok(());
        }
        let writer = SafeWriter::new(destination.to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(options.overwrite_policy);
        let reader = self.open_decoder()?;
        let mut source = ReadSource::new(reader);
        let name = self.spec.single_entry_name(self.source_path()?);
        writer.write_stream(&name, &mut source, |_| Ok(()))?;
        Ok(())
    }

    fn extract_tar(
        &self,
        destination: &Path,
        selected: Option<&[EntryId]>,
        options: ExtractOptions,
    ) -> Result<(), ArchiveError> {
        let writer = SafeWriter::new(destination.to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(options.overwrite_policy);
        let reader = self.open_decoder()?;
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
            } else if entry.header().entry_type().is_symlink() {
                if matches!(options.symlink_policy, SymlinkPolicy::Conservative) {
                    return Err(ArchiveError::new(
                        ArchiveErrorKind::SymlinkPolicyBlocked,
                        "Symlink extraction is blocked by policy",
                    )
                    .with_entry_path(path));
                }
            } else {
                write_tar_entry_streaming(&writer, &path, &mut entry)?;
            }
        }
        Ok(())
    }

    fn open_tar_entry_reader(&self, entry: EntryId) -> Result<EntryReader, ArchiveError> {
        let reader = self.open_decoder()?;
        let mut archive = TarReader::new(reader);
        for (index, tar_entry) in archive.entries().map_err(io_error)?.enumerate() {
            let mut tar_entry = tar_entry.map_err(io_error)?;
            if EntryId(index as u64) != entry {
                continue;
            }
            if tar_entry.header().entry_type().is_dir() {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::Internal,
                    "Cannot open a directory entry as a byte stream",
                ));
            }
            let size = tar_entry.header().size().ok();
            let mut bytes =
                Vec::with_capacity(size.unwrap_or_default().min(16 * 1024 * 1024) as usize);
            tar_entry.read_to_end(&mut bytes).map_err(io_error)?;
            return Ok(EntryReader {
                entry,
                access_cost: AccessCost::SequentialFromStart,
                source: Box::new(Cursor::new(bytes)),
                size,
            });
        }
        Err(entry_not_found())
    }
}

struct ReadSource {
    inner: Box<dyn Read + Send>,
}

impl ReadSource {
    fn new(inner: Box<dyn Read + Send>) -> Self {
        Self { inner }
    }
}

impl ByteSource for ReadSource {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

pub fn create_stream_archive(
    inputs: &[InputPath],
    output: &Path,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    let spec = detect_create_format(output, &options)?;
    match spec.container {
        StreamContainer::Single => create_single_stream(inputs, output, spec, options),
        StreamContainer::Tar => create_tar_stream(inputs, output, spec, options),
    }
}

fn create_single_stream(
    inputs: &[InputPath],
    output: &Path,
    spec: StreamFormatSpec,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    if inputs.len() != 1 || inputs[0].path.is_dir() {
        return Err(ArchiveError::new(
            ArchiveErrorKind::UnsupportedFormat,
            "single-file compression requires exactly one file input",
        ));
    }
    let mut source = File::open(&inputs[0].path).map_err(io_error)?;
    let mut sink = encoder_for(
        spec.codec,
        File::create(output).map_err(io_error)?,
        &options,
    )?;
    StreamPump::new(StreamLimits::default()).copy(&mut source, &mut sink, |_| Ok(()))?;
    finish_encoder(sink)?;
    Ok(())
}

fn create_tar_stream(
    inputs: &[InputPath],
    output: &Path,
    spec: StreamFormatSpec,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    let scanned = InputScanner::scan(inputs)?;
    let sink = encoder_for(
        spec.codec,
        File::create(output).map_err(io_error)?,
        &options,
    )?;
    let sink = write_tar_entries(sink, &scanned)?;
    finish_encoder(sink)?;
    Ok(())
}

fn write_tar_entries<W: Write>(writer: W, inputs: &[ScannedInput]) -> Result<W, ArchiveError> {
    let mut builder = TarBuilder::new(writer);
    for input in inputs {
        if input.is_dir {
            builder
                .append_dir(&input.archive_path, &input.source_path)
                .map_err(io_error)?;
        } else {
            builder
                .append_path_with_name(&input.source_path, &input.archive_path)
                .map_err(io_error)?;
        }
    }
    builder.finish().map_err(io_error)?;
    builder.into_inner().map_err(io_error)
}

fn write_tar_entry_streaming<R: Read>(
    writer: &SafeWriter,
    entry_path: &str,
    source: &mut R,
) -> Result<u64, ArchiveError> {
    writer.write_stream(entry_path, source, |_| Ok(()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamContainer {
    Single,
    Tar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamCodec {
    Gzip,
    Xz,
    Bzip2,
    Zstd,
    Lz4,
    Brotli,
    Lzma,
    Lzip,
    Compress,
}

impl StreamCodec {
    fn name(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Xz => "xz",
            Self::Bzip2 => "bzip2",
            Self::Zstd => "zstd",
            Self::Lz4 => "lz4",
            Self::Brotli => "brotli",
            Self::Lzma => "lzma",
            Self::Lzip => "lzip",
            Self::Compress => "compress",
        }
    }

    fn compression_method(self) -> CompressionMethod {
        match self {
            Self::Gzip => CompressionMethod::Gzip,
            Self::Xz | Self::Lzma | Self::Lzip => CompressionMethod::Xz,
            Self::Bzip2 => CompressionMethod::Deflate,
            Self::Zstd => CompressionMethod::Zstandard,
            Self::Lz4 => CompressionMethod::Lz4,
            Self::Brotli => CompressionMethod::Brotli,
            Self::Compress => CompressionMethod::Store,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct StreamFormatSpec {
    container: StreamContainer,
    codec: StreamCodec,
    suffix: &'static str,
}

impl StreamFormatSpec {
    fn archive_format(self) -> ArchiveFormat {
        match (self.container, self.codec) {
            (StreamContainer::Tar, StreamCodec::Gzip) => ArchiveFormat::TarGz,
            (StreamContainer::Tar, StreamCodec::Xz) => ArchiveFormat::TarXz,
            (StreamContainer::Tar, StreamCodec::Zstd) => ArchiveFormat::TarZst,
            (StreamContainer::Tar, StreamCodec::Bzip2) => {
                if self.suffix.ends_with('2') {
                    ArchiveFormat::TarBz2
                } else {
                    ArchiveFormat::TarBz
                }
            }
            (StreamContainer::Tar, StreamCodec::Lzma) => ArchiveFormat::TarLzma,
            (StreamContainer::Tar, StreamCodec::Lzip) => ArchiveFormat::TarLz,
            (StreamContainer::Tar, StreamCodec::Compress) => ArchiveFormat::TarZ,
            (StreamContainer::Tar, StreamCodec::Brotli) => ArchiveFormat::TarBr,
            (StreamContainer::Tar, StreamCodec::Lz4) => ArchiveFormat::Unknown,
            (StreamContainer::Single, StreamCodec::Gzip) => ArchiveFormat::Gz,
            (StreamContainer::Single, StreamCodec::Xz) => ArchiveFormat::Xz,
            (StreamContainer::Single, StreamCodec::Bzip2) => {
                if self.suffix.ends_with('2') {
                    ArchiveFormat::Bz2
                } else {
                    ArchiveFormat::Bz
                }
            }
            (StreamContainer::Single, StreamCodec::Zstd) => ArchiveFormat::Zstd,
            (StreamContainer::Single, StreamCodec::Lz4) => ArchiveFormat::Lz4,
            (StreamContainer::Single, StreamCodec::Brotli) => ArchiveFormat::Br,
            (StreamContainer::Single, StreamCodec::Lzma) => ArchiveFormat::Lzma,
            (StreamContainer::Single, StreamCodec::Lzip) => ArchiveFormat::Lz,
            (StreamContainer::Single, StreamCodec::Compress) => ArchiveFormat::Z,
        }
    }

    fn label(self) -> String {
        match self.container {
            StreamContainer::Single => self.codec.name().into(),
            StreamContainer::Tar => format!("tar.{}", self.codec.name()),
        }
    }

    fn single_entry_name(self, path: &Path) -> String {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map_or_else(|| path.to_string_lossy().into_owned(), ToOwned::to_owned);
        strip_suffix_ignore_ascii_case(&name, self.suffix)
            .filter(|stripped| !stripped.is_empty())
            .unwrap_or(&name)
            .to_string()
    }
}

fn detect_create_format(
    output: &Path,
    options: &CreateOptions,
) -> Result<StreamFormatSpec, ArchiveError> {
    match options.format {
        ArchiveFormat::TarGz => Ok(StreamFormatSpec {
            container: StreamContainer::Tar,
            codec: StreamCodec::Gzip,
            suffix: ".tar.gz",
        }),
        ArchiveFormat::TarXz => Ok(StreamFormatSpec {
            container: StreamContainer::Tar,
            codec: StreamCodec::Xz,
            suffix: ".tar.xz",
        }),
        ArchiveFormat::TarZst => Ok(StreamFormatSpec {
            container: StreamContainer::Tar,
            codec: StreamCodec::Zstd,
            suffix: ".tar.zst",
        }),
        ArchiveFormat::TarBz => Ok(StreamFormatSpec {
            container: StreamContainer::Tar,
            codec: StreamCodec::Bzip2,
            suffix: ".tar.bz",
        }),
        ArchiveFormat::TarBz2 => Ok(StreamFormatSpec {
            container: StreamContainer::Tar,
            codec: StreamCodec::Bzip2,
            suffix: ".tar.bz2",
        }),
        ArchiveFormat::TarLzma => Ok(StreamFormatSpec {
            container: StreamContainer::Tar,
            codec: StreamCodec::Lzma,
            suffix: ".tar.lzma",
        }),
        ArchiveFormat::TarLz => Ok(StreamFormatSpec {
            container: StreamContainer::Tar,
            codec: StreamCodec::Lzip,
            suffix: ".tar.lz",
        }),
        ArchiveFormat::TarZ => Ok(StreamFormatSpec {
            container: StreamContainer::Tar,
            codec: StreamCodec::Compress,
            suffix: ".tar.Z",
        }),
        ArchiveFormat::TarBr => Ok(StreamFormatSpec {
            container: StreamContainer::Tar,
            codec: StreamCodec::Brotli,
            suffix: ".tar.br",
        }),
        ArchiveFormat::Gz => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Gzip,
            suffix: ".gz",
        }),
        ArchiveFormat::Xz => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Xz,
            suffix: ".xz",
        }),
        ArchiveFormat::Bz => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Bzip2,
            suffix: ".bz",
        }),
        ArchiveFormat::Bz2 => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Bzip2,
            suffix: ".bz2",
        }),
        ArchiveFormat::Zstd => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Zstd,
            suffix: ".zst",
        }),
        ArchiveFormat::Lz4 => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Lz4,
            suffix: ".lz4",
        }),
        ArchiveFormat::Br => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Brotli,
            suffix: ".br",
        }),
        ArchiveFormat::Lzma => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Lzma,
            suffix: ".lzma",
        }),
        ArchiveFormat::Lz => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Lzip,
            suffix: ".lz",
        }),
        ArchiveFormat::Z => Ok(StreamFormatSpec {
            container: StreamContainer::Single,
            codec: StreamCodec::Compress,
            suffix: ".Z",
        }),
        _ => detect_stream_format(&output.to_string_lossy()).ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "stream backend cannot infer the requested output format",
            )
        }),
    }
}

fn detect_stream_format(name: &str) -> Option<StreamFormatSpec> {
    let candidates = [
        (".tar.gz", StreamContainer::Tar, StreamCodec::Gzip),
        (".tgz", StreamContainer::Tar, StreamCodec::Gzip),
        (".tar.xz", StreamContainer::Tar, StreamCodec::Xz),
        (".txz", StreamContainer::Tar, StreamCodec::Xz),
        (".tar.zst", StreamContainer::Tar, StreamCodec::Zstd),
        (".tar.zstd", StreamContainer::Tar, StreamCodec::Zstd),
        (".tzst", StreamContainer::Tar, StreamCodec::Zstd),
        (".tar.bz2", StreamContainer::Tar, StreamCodec::Bzip2),
        (".tar.bz", StreamContainer::Tar, StreamCodec::Bzip2),
        (".tbz2", StreamContainer::Tar, StreamCodec::Bzip2),
        (".tbz", StreamContainer::Tar, StreamCodec::Bzip2),
        (".tar.lz4", StreamContainer::Tar, StreamCodec::Lz4),
        (".tlz4", StreamContainer::Tar, StreamCodec::Lz4),
        (".tar.br", StreamContainer::Tar, StreamCodec::Brotli),
        (".tbr", StreamContainer::Tar, StreamCodec::Brotli),
        (".tar.lzma", StreamContainer::Tar, StreamCodec::Lzma),
        (".tlzma", StreamContainer::Tar, StreamCodec::Lzma),
        (".tar.lz", StreamContainer::Tar, StreamCodec::Lzip),
        (".tlz", StreamContainer::Tar, StreamCodec::Lzip),
        (".tar.z", StreamContainer::Tar, StreamCodec::Compress),
        (".gz", StreamContainer::Single, StreamCodec::Gzip),
        (".xz", StreamContainer::Single, StreamCodec::Xz),
        (".bz2", StreamContainer::Single, StreamCodec::Bzip2),
        (".bz", StreamContainer::Single, StreamCodec::Bzip2),
        (".zst", StreamContainer::Single, StreamCodec::Zstd),
        (".zstd", StreamContainer::Single, StreamCodec::Zstd),
        (".lz4", StreamContainer::Single, StreamCodec::Lz4),
        (".br", StreamContainer::Single, StreamCodec::Brotli),
        (".lzma", StreamContainer::Single, StreamCodec::Lzma),
        (".lz", StreamContainer::Single, StreamCodec::Lzip),
        (".z", StreamContainer::Single, StreamCodec::Compress),
    ];
    candidates
        .into_iter()
        .find(|(suffix, _, _)| ends_with_ignore_ascii_case(name, suffix))
        .map(|(suffix, container, codec)| StreamFormatSpec {
            container,
            codec,
            suffix,
        })
}

fn decoder_for<R: Read + Send + 'static>(
    codec: StreamCodec,
    reader: R,
) -> Result<Box<dyn Read + Send>, ArchiveError> {
    Ok(match codec {
        StreamCodec::Gzip => Box::new(GzDecoder::new(reader)),
        StreamCodec::Xz => Box::new(XzDecoder::new(reader)),
        StreamCodec::Bzip2 => Box::new(BzDecoder::new(reader)),
        StreamCodec::Zstd => Box::new(zstd::stream::read::Decoder::new(reader).map_err(io_error)?),
        StreamCodec::Lz4 => Box::new(Lz4Decoder::new(reader)),
        StreamCodec::Brotli => Box::new(Decompressor::new(reader, 128 * 1024)),
        StreamCodec::Lzma => Box::new(XzDecoder::new_stream(
            reader,
            XzStream::new_lzma_decoder(u64::MAX).map_err(xz_error)?,
        )),
        StreamCodec::Lzip => return unsupported_codec("lzip"),
        StreamCodec::Compress => return unsupported_codec("compress .Z"),
    })
}

enum CodecEncoder<W: Write> {
    Gzip(GzEncoder<W>),
    Xz(XzEncoder<W>),
    Bzip2(BzEncoder<W>),
    Zstd(zstd::stream::write::Encoder<'static, W>),
    Lz4(Lz4Encoder<W>),
    Brotli(CompressorWriter<W>),
    Lzma(XzEncoder<W>),
}

impl<W: Write> Write for CodecEncoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Gzip(writer) => writer.write(buf),
            Self::Xz(writer) => writer.write(buf),
            Self::Bzip2(writer) => writer.write(buf),
            Self::Zstd(writer) => writer.write(buf),
            Self::Lz4(writer) => writer.write(buf),
            Self::Brotli(writer) => writer.write(buf),
            Self::Lzma(writer) => writer.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Gzip(writer) => writer.flush(),
            Self::Xz(writer) => writer.flush(),
            Self::Bzip2(writer) => writer.flush(),
            Self::Zstd(writer) => writer.flush(),
            Self::Lz4(writer) => writer.flush(),
            Self::Brotli(writer) => writer.flush(),
            Self::Lzma(writer) => writer.flush(),
        }
    }
}

fn encoder_for<W: Write>(
    codec: StreamCodec,
    writer: W,
    options: &CreateOptions,
) -> Result<CodecEncoder<W>, ArchiveError> {
    let level = options.compression_level.unwrap_or(6);
    Ok(match codec {
        StreamCodec::Gzip => CodecEncoder::Gzip(GzEncoder::new(
            writer,
            GzipCompression::new(level.min(9) as u32),
        )),
        StreamCodec::Xz => CodecEncoder::Xz(XzEncoder::new(writer, level.min(9) as u32)),
        StreamCodec::Bzip2 => CodecEncoder::Bzip2(BzEncoder::new(
            writer,
            Bzip2Compression::new(level.min(9) as u32),
        )),
        StreamCodec::Zstd => CodecEncoder::Zstd(
            zstd::stream::write::Encoder::new(writer, level.min(22) as i32).map_err(io_error)?,
        ),
        StreamCodec::Lz4 => CodecEncoder::Lz4(Lz4Encoder::new(writer)),
        StreamCodec::Brotli => CodecEncoder::Brotli(CompressorWriter::new(
            writer,
            128 * 1024,
            level.min(11) as u32,
            22,
        )),
        StreamCodec::Lzma => CodecEncoder::Lzma(XzEncoder::new_stream(
            writer,
            XzStream::new_lzma_encoder(
                &LzmaOptions::new_preset(level.min(9) as u32).map_err(xz_error)?,
            )
            .map_err(xz_error)?,
        )),
        StreamCodec::Lzip => return unsupported_codec("lzip"),
        StreamCodec::Compress => return unsupported_codec("compress .Z"),
    })
}

fn finish_encoder<W: Write>(encoder: CodecEncoder<W>) -> Result<W, ArchiveError> {
    match encoder {
        CodecEncoder::Gzip(writer) => writer.finish().map_err(io_error),
        CodecEncoder::Xz(writer) => writer.finish().map_err(io_error),
        CodecEncoder::Bzip2(writer) => writer.finish().map_err(io_error),
        CodecEncoder::Zstd(writer) => writer.finish().map_err(io_error),
        CodecEncoder::Lz4(writer) => writer.finish().map_err(|error| {
            ArchiveError::new(ArchiveErrorKind::Io, "lz4 stream finalization failed")
                .with_backend("stream-codec")
                .with_technical_detail(error.to_string())
        }),
        CodecEncoder::Brotli(mut writer) => {
            writer.flush().map_err(io_error)?;
            Ok(writer.into_inner())
        }
        CodecEncoder::Lzma(writer) => writer.finish().map_err(io_error),
    }
}

fn read_steps(spec: StreamFormatSpec) -> Vec<PipelineStep> {
    let mut steps = vec![PipelineStep::StreamDecompress {
        codec: spec.codec.compression_method(),
    }];
    if spec.container == StreamContainer::Tar {
        steps.push(PipelineStep::StreamTarEntries);
    }
    steps
}

fn tar_entry_kind(entry_type: tar::EntryType) -> EntryKind {
    if entry_type.is_dir() {
        EntryKind::Directory
    } else if entry_type.is_symlink() {
        EntryKind::Symlink
    } else if entry_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn strip_suffix_ignore_ascii_case<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    name.get(..name.len().checked_sub(suffix.len())?)
        .filter(|_| ends_with_ignore_ascii_case(name, suffix))
}

fn ends_with_ignore_ascii_case(value: &str, suffix: &str) -> bool {
    value
        .get(value.len().saturating_sub(suffix.len())..)
        .is_some_and(|tail| tail.eq_ignore_ascii_case(suffix))
}

fn stream_capabilities() -> ArchiveCapabilities {
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

fn unsupported_codec<T>(codec: &str) -> Result<T, ArchiveError> {
    Err(ArchiveError::new(
        ArchiveErrorKind::UnsupportedCodec,
        format!("{codec} is detected but not implemented by the pure Rust stream backend"),
    )
    .with_backend("stream-codec"))
}

fn entry_not_found() -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Internal, "Archive entry id was not found")
        .with_backend("stream-codec")
}

fn io_error(error: io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "stream codec I/O operation failed")
        .with_backend("stream-codec")
        .with_technical_detail(error.to_string())
}

fn xz_error(error: xz2::stream::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::UnsupportedCodec, "LZMA stream setup failed")
        .with_backend("stream-codec")
        .with_technical_detail(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_tar_and_single_stream_suffixes() {
        assert_eq!(
            detect_stream_format("backup.tbz2").unwrap().container,
            StreamContainer::Tar
        );
        assert_eq!(
            detect_stream_format("backup.tar.br").unwrap().codec,
            StreamCodec::Brotli
        );
        assert_eq!(
            detect_stream_format("readme.txt.gz").unwrap().container,
            StreamContainer::Single
        );
    }

    #[test]
    fn gzip_single_file_roundtrip_is_streamed() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("hello.txt");
        let output = dir.path().join("hello.txt.gz");
        let extract_dir = dir.path().join("out");
        fs::write(&input, b"hello stream").unwrap();

        create_stream_archive(
            &[InputPath {
                path: input.clone(),
                archive_path: None,
            }],
            &output,
            CreateOptions {
                format: ArchiveFormat::Unknown,
                compression_method: None,
                compression_level: Some(6),
                solid: false,
                encrypt_file_names: false,
                password: None,
                volume_size: None,
                symlink_policy: SymlinkPolicy::Conservative,
            },
        )
        .unwrap();

        let mut archive = StreamBackend
            .open(ArchiveSource::LocalPath(output), OpenOptions::default())
            .unwrap();
        archive
            .extract_all(
                &extract_dir,
                ExtractOptions {
                    overwrite_policy: OverwritePolicy::Overwrite,
                    ..ExtractOptions::default()
                },
            )
            .unwrap();

        assert_eq!(
            fs::read(extract_dir.join("hello.txt")).unwrap(),
            b"hello stream"
        );
    }

    #[test]
    fn bzip2_tar_roundtrip_is_streamed() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("hello.txt");
        let output = dir.path().join("bundle.tar.bz2");
        let extract_dir = dir.path().join("out");
        fs::write(&input, b"hello tar stream").unwrap();

        create_stream_archive(
            &[InputPath {
                path: input,
                archive_path: Some("data/hello.txt".into()),
            }],
            &output,
            CreateOptions {
                format: ArchiveFormat::Unknown,
                compression_method: None,
                compression_level: Some(6),
                solid: false,
                encrypt_file_names: false,
                password: None,
                volume_size: None,
                symlink_policy: SymlinkPolicy::Conservative,
            },
        )
        .unwrap();

        let mut archive = StreamBackend
            .open(ArchiveSource::LocalPath(output), OpenOptions::default())
            .unwrap();
        let listing = archive.listing(ListingMode::Full).unwrap();
        assert_eq!(listing.entries[0].display_path, "data/hello.txt");
        archive
            .extract_all(
                &extract_dir,
                ExtractOptions {
                    overwrite_policy: OverwritePolicy::Overwrite,
                    ..ExtractOptions::default()
                },
            )
            .unwrap();

        assert_eq!(
            fs::read(extract_dir.join("data/hello.txt")).unwrap(),
            b"hello tar stream"
        );
    }
}
