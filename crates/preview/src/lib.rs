use std::io;

use chardetng::EncodingDetector;
use fast_image_resize::{PixelType, ResizeOptions, Resizer, images::Image};
use image::ImageReader;
use shadow_zip_archive_core::{ByteSink, ByteSource, StreamLimits, StreamPump};
use shadow_zip_domain::*;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PreviewRequest {
    pub id: Uuid,
    pub session_id: SessionId,
    pub entry_id: EntryId,
    pub entry_name: String,
    pub entry_size: Option<u64>,
    pub mode: PreviewMode,
    pub target_size: PixelSize,
    pub priority: TaskPriority,
}

impl PreviewRequest {
    pub fn metadata(session_id: SessionId, entry_id: EntryId) -> Self {
        Self {
            id: Uuid::new_v4(),
            session_id,
            entry_id,
            entry_name: format!("entry-{}", entry_id.0),
            entry_size: None,
            mode: PreviewMode::Metadata,
            target_size: PixelSize::default(),
            priority: TaskPriority::UserBlocking,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    Metadata,
    Thumbnail,
    FitWindow,
    FullResolution,
    Text,
    ExternalOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

impl PixelSize {
    pub fn pixels(self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

#[derive(Debug, Clone)]
pub enum PreviewResult {
    Metadata(ImageMetadata),
    Bitmap(BitmapPreview),
    Text(TextPreview),
    External(ExternalPreview),
    Unsupported(UnsupportedPreview),
}

#[derive(Debug, Clone)]
pub struct ImageMetadata {
    pub file_name: String,
    pub file_size: Option<u64>,
    pub dimensions: Option<PixelSize>,
    pub orientation: Option<u16>,
    pub color_profile: Option<String>,
    pub exif_summary: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct BitmapPreview {
    pub dimensions: PixelSize,
    pub cache_key: Option<String>,
    pub memory_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct TextPreview {
    pub text: String,
    pub truncated: bool,
    pub encoding: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExternalPreview {
    pub display_path_hint: Option<String>,
    pub requires_temp_file: bool,
}

#[derive(Debug, Clone)]
pub struct UnsupportedPreview {
    pub reason: String,
    pub access_cost: Option<AccessCost>,
}

#[derive(Debug, Clone)]
pub struct PreviewLimits {
    pub max_input_bytes: u64,
    pub max_output_pixels: u64,
    pub max_animation_frames: u32,
    pub text_preview_bytes: u64,
}

impl Default for PreviewLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024,
            max_output_pixels: 32_000_000,
            max_animation_frames: 128,
            text_preview_bytes: 512 * 1024,
        }
    }
}

pub struct PreviewService {
    limits: PreviewLimits,
}

impl PreviewService {
    pub fn new(limits: PreviewLimits) -> Self {
        Self { limits }
    }

    pub fn limits(&self) -> &PreviewLimits {
        &self.limits
    }

    pub fn plan(&self, request: &PreviewRequest, access_cost: AccessCost) -> TaskPlan {
        let plan = TaskPlan::new(TaskKind::Preview, format!("Preview {}", request.entry_name))
            .native(NativePipelinePlan::new(
                self.pipeline(request, access_cost).task_steps(),
            ));

        if matches!(
            access_cost,
            AccessCost::SequentialFromStart | AccessCost::SolidBlockScan
        ) {
            plan.warn(
                "preview-access-cost",
                "This archive may need sequential reads before preview data is available",
            )
        } else {
            plan
        }
    }

    pub fn pipeline(&self, request: &PreviewRequest, access_cost: AccessCost) -> PreviewPipeline {
        let mut stages = vec![PreviewStage::OpenEntryStream, PreviewStage::ReadMetadata];
        match request.mode {
            PreviewMode::Metadata => {}
            PreviewMode::Thumbnail => {
                stages.extend([
                    PreviewStage::ReadBoundedInput,
                    PreviewStage::DecodeImage,
                    PreviewStage::ResizeThumbnail,
                ]);
            }
            PreviewMode::FitWindow => {
                stages.extend([
                    PreviewStage::ReadBoundedInput,
                    PreviewStage::DecodeImage,
                    PreviewStage::ResizeToFit,
                ]);
            }
            PreviewMode::FullResolution => {
                stages.extend([
                    PreviewStage::ReadBoundedInput,
                    PreviewStage::DecodeImage,
                    PreviewStage::ApplyOrientation,
                ]);
            }
            PreviewMode::Text => {
                stages.extend([PreviewStage::ReadTextPrefix, PreviewStage::DecodeText])
            }
            PreviewMode::ExternalOpen => stages.extend([
                PreviewStage::ReadBoundedInput,
                PreviewStage::MaterializeTempFile,
            ]),
        }

        PreviewPipeline {
            request: request.clone(),
            access_cost,
            stages,
            limits: self.limits.clone(),
        }
    }

    pub fn process(
        &self,
        request: &PreviewRequest,
        access_cost: AccessCost,
        source: &mut dyn ByteSource,
    ) -> Result<PreviewResult, ArchiveError> {
        let pipeline = self.pipeline(request, access_cost);
        PreviewProcessor::new(pipeline).run(source)
    }
}

#[derive(Debug, Clone)]
pub struct PreviewPipeline {
    pub request: PreviewRequest,
    pub access_cost: AccessCost,
    pub stages: Vec<PreviewStage>,
    pub limits: PreviewLimits,
}

impl PreviewPipeline {
    fn task_steps(&self) -> Vec<PipelineStep> {
        self.stages
            .iter()
            .map(|stage| match stage {
                PreviewStage::OpenEntryStream => PipelineStep::ProbeArchive,
                PreviewStage::ReadMetadata => PipelineStep::DecodeImageMetadata,
                PreviewStage::ReadBoundedInput | PreviewStage::ReadTextPrefix => {
                    PipelineStep::ProbeArchive
                }
                PreviewStage::DecodeImage => PipelineStep::DecodeImageBitmap,
                PreviewStage::ResizeThumbnail | PreviewStage::ResizeToFit => {
                    PipelineStep::ResizeImage
                }
                PreviewStage::ApplyOrientation => PipelineStep::ResizeImage,
                PreviewStage::DecodeText => PipelineStep::ProbeArchive,
                PreviewStage::MaterializeTempFile => PipelineStep::WriteFile,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewStage {
    OpenEntryStream,
    ReadMetadata,
    ReadBoundedInput,
    ReadTextPrefix,
    DecodeImage,
    DecodeText,
    ApplyOrientation,
    ResizeThumbnail,
    ResizeToFit,
    MaterializeTempFile,
}

struct PreviewProcessor {
    pipeline: PreviewPipeline,
}

impl PreviewProcessor {
    fn new(pipeline: PreviewPipeline) -> Self {
        Self { pipeline }
    }

    fn run(&self, source: &mut dyn ByteSource) -> Result<PreviewResult, ArchiveError> {
        match self.pipeline.request.mode {
            PreviewMode::Metadata => Ok(PreviewResult::Metadata(self.metadata_only())),
            PreviewMode::Text => self.text_preview(source),
            PreviewMode::ExternalOpen => self.external_preview(source),
            PreviewMode::Thumbnail | PreviewMode::FitWindow | PreviewMode::FullResolution => {
                self.image_preview(source)
            }
        }
    }

    fn metadata_only(&self) -> ImageMetadata {
        ImageMetadata {
            file_name: self.pipeline.request.entry_name.clone(),
            file_size: self.pipeline.request.entry_size,
            dimensions: None,
            orientation: None,
            color_profile: None,
            exif_summary: Vec::new(),
        }
    }

    fn text_preview(&self, source: &mut dyn ByteSource) -> Result<PreviewResult, ArchiveError> {
        let mut sink = BoundedMemorySink::new(self.pipeline.limits.text_preview_bytes as usize);
        StreamPump::new(StreamLimits {
            buffer_size: 32 * 1024,
            max_entry_bytes: self.pipeline.limits.text_preview_bytes,
            max_total_bytes: self.pipeline.limits.text_preview_bytes,
        })
        .copy(source, &mut sink, |_| Ok(()))?;
        let mut detector = EncodingDetector::new();
        detector.feed(&sink.bytes, true);
        let encoding = detector.guess(None, true);
        let (text, _, had_errors) = encoding.decode(&sink.bytes);

        Ok(PreviewResult::Text(TextPreview {
            text: text.into_owned(),
            truncated: sink.truncated,
            encoding: Some(if had_errors {
                format!("{} with replacement", encoding.name())
            } else {
                encoding.name().to_string()
            }),
        }))
    }

    fn external_preview(&self, source: &mut dyn ByteSource) -> Result<PreviewResult, ArchiveError> {
        let mut sink = CountingPreviewSink::default();
        StreamPump::new(StreamLimits {
            buffer_size: 128 * 1024,
            max_entry_bytes: self.pipeline.limits.max_input_bytes,
            max_total_bytes: self.pipeline.limits.max_input_bytes,
        })
        .copy(source, &mut sink, |_| Ok(()))?;

        Ok(PreviewResult::External(ExternalPreview {
            display_path_hint: Some(self.pipeline.request.entry_name.clone()),
            requires_temp_file: true,
        }))
    }

    fn image_preview(&self, source: &mut dyn ByteSource) -> Result<PreviewResult, ArchiveError> {
        let mut sink = BoundedMemorySink::new(self.pipeline.limits.max_input_bytes as usize);
        StreamPump::new(StreamLimits {
            buffer_size: 128 * 1024,
            max_entry_bytes: self.pipeline.limits.max_input_bytes,
            max_total_bytes: self.pipeline.limits.max_input_bytes,
        })
        .copy(source, &mut sink, |_| Ok(()))?;

        let reader = ImageReader::new(std::io::Cursor::new(&sink.bytes))
            .with_guessed_format()
            .map_err(image_io_error)?;
        let format = reader.format();
        let image = reader.decode().map_err(image_error)?.to_rgba8();
        let source_dimensions = PixelSize {
            width: image.width(),
            height: image.height(),
        };
        if source_dimensions.pixels() > self.pipeline.limits.max_output_pixels {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Image exceeds configured pixel limit",
            ));
        }

        let dimensions = requested_dimensions(
            source_dimensions,
            self.pipeline.request.target_size,
            self.pipeline.request.mode,
        );
        if dimensions.pixels() > self.pipeline.limits.max_output_pixels {
            return Err(ArchiveError::new(
                ArchiveErrorKind::Internal,
                "Preview exceeds configured output pixel limit",
            ));
        }

        let resized_bytes = resize_rgba(image.as_raw(), source_dimensions, dimensions)?;

        Ok(PreviewResult::Bitmap(BitmapPreview {
            dimensions,
            cache_key: Some(format!(
                "{}:{:?}:{}x{}",
                self.pipeline.request.entry_id.0, format, dimensions.width, dimensions.height
            )),
            memory_bytes: resized_bytes.len(),
        }))
    }
}

fn resize_rgba(
    bytes: &[u8],
    source: PixelSize,
    target: PixelSize,
) -> Result<Vec<u8>, ArchiveError> {
    if source == target {
        return Ok(bytes.to_vec());
    }
    let source_image =
        Image::from_vec_u8(source.width, source.height, bytes.to_vec(), PixelType::U8x4)
            .map_err(resize_error)?;
    let mut target_image = Image::new(target.width, target.height, PixelType::U8x4);
    let mut resizer = Resizer::new();
    resizer
        .resize(&source_image, &mut target_image, &ResizeOptions::new())
        .map_err(resize_error)?;
    Ok(target_image.into_vec())
}

fn requested_dimensions(source: PixelSize, target: PixelSize, mode: PreviewMode) -> PixelSize {
    match mode {
        PreviewMode::Thumbnail => PixelSize {
            width: 256,
            height: 256,
        },
        PreviewMode::FitWindow if target.width > 0 && target.height > 0 => target,
        PreviewMode::FullResolution if target.width > 0 && target.height > 0 => target,
        _ => source,
    }
}

struct BoundedMemorySink {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}

impl BoundedMemorySink {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(64 * 1024)),
            limit,
            truncated: false,
        }
    }
}

impl ByteSink for BoundedMemorySink {
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if bytes.len() > remaining {
            self.bytes.extend_from_slice(&bytes[..remaining]);
            self.truncated = true;
        } else {
            self.bytes.extend_from_slice(bytes);
        }
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct CountingPreviewSink {
    bytes: u64,
}

impl ByteSink for CountingPreviewSink {
    fn write_chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.bytes += bytes.len() as u64;
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn image_error(error: image::ImageError) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Internal, "Image decode failed")
        .with_technical_detail(error.to_string())
}

fn image_io_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "Image input failed")
        .with_technical_detail(error.to_string())
}

fn resize_error(error: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Internal, "Image resize failed")
        .with_technical_detail(error.to_string())
}
