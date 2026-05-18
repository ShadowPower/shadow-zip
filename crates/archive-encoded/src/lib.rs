use std::{
    fs::File,
    io::{Cursor, Read, Write},
    path::Path,
};

use shadow_zip_archive_core::{
    ArchiveBackend, EntryReader, OpenArchive, SafeWriter, StreamLimits, create_pipeline,
    quick_test_pipeline, random_access_extract_pipeline,
};
use shadow_zip_domain::*;

const AES_CRYPT_MAGIC: &[u8; 3] = b"AES";
const XX_ALPHABET: &[u8; 64] = b"+-0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

pub struct EncodedBackend;

impl ArchiveBackend for EncodedBackend {
    fn name(&self) -> &'static str {
        "encoded"
    }

    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError> {
        let detected = detect_source(source)?;
        Ok(ProbeResult {
            format: detected.format,
            confidence: detected.confidence,
            backend_name: self.name(),
        })
    }

    fn open(
        &self,
        source: ArchiveSource,
        options: OpenOptions,
    ) -> Result<Box<dyn OpenArchive>, ArchiveError> {
        let detected = detect_source(&source)?;
        if detected.confidence == ProbeConfidence::Impossible {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Encoded backend could not identify this source",
            )
            .with_backend(self.name()));
        }
        Ok(Box::new(EncodedArchive {
            source,
            format: detected.format,
            password: options.password,
            listing_cache: None,
        }))
    }

    fn create_plan(
        &self,
        inputs: &[InputPath],
        output: &Path,
        options: CreateOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        if !matches!(
            options.format,
            ArchiveFormat::Uu | ArchiveFormat::Uue | ArchiveFormat::Xxe
        ) {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Encoded backend can create UU/UUE/XXE files",
            )
            .with_backend(self.name()));
        }
        if inputs.len() != 1 || inputs[0].path.is_dir() {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "UU/UUE/XXE creation requires exactly one file input",
            )
            .with_backend(self.name()));
        }
        Ok(
            TaskPlan::new(TaskKind::Create, format!("Create encoded {}", output.display()))
                .estimated_entries(inputs.len())
                .native(create_pipeline()),
        )
    }

    fn backend_capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            formats: vec![ArchiveFormat::Uu, ArchiveFormat::Uue, ArchiveFormat::Xxe],
            capabilities: encoded_capabilities(),
        }
    }
}

pub fn create_encoded_archive(
    inputs: &[InputPath],
    output: &Path,
    options: CreateOptions,
) -> Result<(), ArchiveError> {
    if inputs.len() != 1 || inputs[0].path.is_dir() {
        return Err(ArchiveError::new(
            ArchiveErrorKind::UnsupportedFormat,
            "UU/UUE/XXE creation requires exactly one file input",
        )
        .with_backend("encoded"));
    }
    let xx = match options.format {
        ArchiveFormat::Uu | ArchiveFormat::Uue => false,
        ArchiveFormat::Xxe => true,
        _ => {
            return Err(ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Encoded backend can create UU/UUE/XXE files",
            )
            .with_backend("encoded"));
        }
    };
    let input = &inputs[0];
    let name = input
        .archive_path
        .clone()
        .or_else(|| {
            input
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "data.bin".into())
        .replace('\\', "/");
    let mut source = File::open(&input.path).map_err(io_error)?;
    let mut sink = File::create(output).map_err(io_error)?;
    encode_text_file(&mut source, &mut sink, &name, xx)
}

struct EncodedArchive {
    source: ArchiveSource,
    format: ArchiveFormat,
    password: Option<String>,
    listing_cache: Option<ArchiveListing>,
}

impl OpenArchive for EncodedArchive {
    fn info(&self) -> ArchiveInfo {
        ArchiveInfo {
            format: self.format,
            display_name: self.source.display_name(),
            total_bytes: self.source_size().ok(),
            entry_count: Some(1),
            codecs: match self.format {
                ArchiveFormat::Aes => vec!["AES Crypt v2".into()],
                ArchiveFormat::Xxe => vec!["xxencode".into()],
                ArchiveFormat::Uu | ArchiveFormat::Uue => vec!["uuencode".into()],
                _ => Vec::new(),
            },
            filters: Vec::new(),
            is_solid: false,
            is_encrypted: self.format == ArchiveFormat::Aes,
            has_header_encryption: false,
            is_multi_volume: false,
        }
    }

    fn capabilities(&self) -> ArchiveCapabilities {
        match self.format {
            ArchiveFormat::Aes => aes_capabilities(),
            _ => encoded_capabilities(),
        }
    }

    fn listing(&mut self, _mode: ListingMode) -> Result<ArchiveListing, ArchiveError> {
        if let Some(listing) = &self.listing_cache {
            return Ok(listing.clone());
        }
        let listing = match self.format {
            ArchiveFormat::Aes => self.aes_listing()?,
            ArchiveFormat::Xxe => self.text_listing(true)?,
            ArchiveFormat::Uu | ArchiveFormat::Uue => self.text_listing(false)?,
            _ => {
                return Err(ArchiveError::new(
                    ArchiveErrorKind::UnsupportedFormat,
                    "Encoded backend received an unsupported format",
                )
                .with_backend("encoded"));
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
        ensure_entry(entry)?;
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
        ensure_entry(entry)?;
        if self.format == ArchiveFormat::Aes {
            return Err(self.aes_error(options.password.as_ref()));
        }
        let decoded = self.decode_text_entry()?;
        let size = Some(decoded.bytes.len() as u64);
        Ok(EntryReader {
            entry,
            access_cost: AccessCost::Random,
            source: Box::new(Cursor::new(decoded.bytes)),
            size,
        })
    }

    fn test(&mut self, options: TestOptions) -> Result<TaskPlan, ArchiveError> {
        if self.format == ArchiveFormat::Aes {
            return Err(self.aes_error(options.password.as_ref()));
        }
        let _ = self.decode_text_entry()?;
        Ok(
            TaskPlan::new(TaskKind::Test, "Test encoded file").native(quick_test_pipeline(vec![
                PipelineStep::ProbeArchive,
                PipelineStep::ValidateEntryPath,
            ])),
        )
    }
}

impl EncodedArchive {
    fn source_path(&self) -> Result<&Path, ArchiveError> {
        self.source.path().ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedFormat,
                "Encoded backend requires a local path",
            )
            .with_backend("encoded")
        })
    }

    fn source_size(&self) -> Result<u64, ArchiveError> {
        self.source_path()?
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(io_error)
    }

    fn read_bytes(&self) -> Result<Vec<u8>, ArchiveError> {
        let mut file = File::open(self.source_path()?).map_err(io_error)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(io_error)?;
        Ok(bytes)
    }

    fn aes_listing(&self) -> Result<ArchiveListing, ArchiveError> {
        let name = strip_final_extension(&self.source.display_name(), "aes");
        Ok(single_entry_listing(
            name,
            None,
            self.source_size().ok(),
            Some("AES Crypt v2".into()),
            true,
        ))
    }

    fn text_listing(&self, xx: bool) -> Result<ArchiveListing, ArchiveError> {
        let decoded = decode_encoded_text(&self.read_bytes()?, xx)?;
        Ok(single_entry_listing(
            decoded.name,
            Some(decoded.bytes.len() as u64),
            self.source_size().ok(),
            Some(if xx { "xxencode" } else { "uuencode" }.into()),
            false,
        ))
    }

    fn decode_text_entry(&self) -> Result<DecodedFile, ArchiveError> {
        decode_encoded_text(&self.read_bytes()?, self.format == ArchiveFormat::Xxe)
    }

    fn extract(
        &mut self,
        destination: &Path,
        selected: Option<&[EntryId]>,
        options: ExtractOptions,
    ) -> Result<TaskPlan, ArchiveError> {
        let listing = self.listing(ListingMode::Full)?;
        if selected.is_some_and(|entries| !entries.contains(&EntryId(0))) {
            return Ok(extract_plan(destination, 0));
        }
        if self.format == ArchiveFormat::Aes {
            return Err(self.aes_error(options.password.as_ref()));
        }
        let decoded = self.decode_text_entry()?;
        let writer = SafeWriter::new(destination.to_path_buf(), StreamLimits::default())
            .with_overwrite_policy(options.overwrite_policy);
        let mut source = Cursor::new(decoded.bytes);
        writer.write_stream(&decoded.name, &mut source, |_| Ok(()))?;
        Ok(extract_plan(destination, listing.entries.len()))
    }

    fn aes_error(&self, password: Option<&String>) -> ArchiveError {
        if password.or(self.password.as_ref()).is_none() {
            ArchiveError::new(
                ArchiveErrorKind::PasswordRequired,
                "AES Crypt v2 files require a password before they can be decrypted",
            )
            .with_backend("encoded")
        } else {
            ArchiveError::new(
                ArchiveErrorKind::UnsupportedCodec,
                "AES Crypt v2 decryption is not implemented",
            )
            .with_backend("encoded")
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DetectedFormat {
    format: ArchiveFormat,
    confidence: ProbeConfidence,
}

#[derive(Debug, Clone)]
struct DecodedFile {
    name: String,
    bytes: Vec<u8>,
}

fn detect_source(source: &ArchiveSource) -> Result<DetectedFormat, ArchiveError> {
    let extension_format = source
        .path()
        .and_then(|path| path.extension())
        .and_then(|extension| extension.to_str())
        .and_then(format_from_extension);
    let signature_format = source
        .path()
        .and_then(|path| detect_signature(path).ok().flatten());

    Ok(match (signature_format, extension_format) {
        (Some(format), _) => DetectedFormat {
            format,
            confidence: ProbeConfidence::Signature,
        },
        (None, Some(format)) => DetectedFormat {
            format,
            confidence: ProbeConfidence::Extension,
        },
        (None, None) => DetectedFormat {
            format: ArchiveFormat::Unknown,
            confidence: ProbeConfidence::Impossible,
        },
    })
}

fn detect_signature(path: &Path) -> Result<Option<ArchiveFormat>, ArchiveError> {
    let mut file = File::open(path).map_err(io_error)?;
    let mut buffer = [0_u8; 1024];
    let read = file.read(&mut buffer).map_err(io_error)?;
    let bytes = &buffer[..read];
    if bytes.len() >= 4 && &bytes[..3] == AES_CRYPT_MAGIC && bytes[3] == 2 {
        return Ok(Some(ArchiveFormat::Aes));
    }
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        if line.starts_with("begin ") {
            return Ok(Some(
                match path.extension().and_then(|extension| extension.to_str()) {
                    Some(extension) if extension.eq_ignore_ascii_case("xxe") => ArchiveFormat::Xxe,
                    Some(extension) if extension.eq_ignore_ascii_case("uue") => ArchiveFormat::Uue,
                    _ => ArchiveFormat::Uu,
                },
            ));
        }
    }
    Ok(None)
}

fn format_from_extension(extension: &str) -> Option<ArchiveFormat> {
    if extension.eq_ignore_ascii_case("aes") {
        Some(ArchiveFormat::Aes)
    } else if extension.eq_ignore_ascii_case("uu") {
        Some(ArchiveFormat::Uu)
    } else if extension.eq_ignore_ascii_case("uue") {
        Some(ArchiveFormat::Uue)
    } else if extension.eq_ignore_ascii_case("xxe") {
        Some(ArchiveFormat::Xxe)
    } else {
        None
    }
}

fn decode_encoded_text(bytes: &[u8], xx: bool) -> Result<DecodedFile, ArchiveError> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        ArchiveError::new(
            ArchiveErrorKind::CorruptArchive,
            "Encoded text is not valid UTF-8",
        )
        .with_backend("encoded")
        .with_technical_detail(error.to_string())
    })?;
    let mut lines = text.lines();
    let header = lines
        .find(|line| line.starts_with("begin "))
        .ok_or_else(|| corrupt("Encoded text is missing a begin header"))?;
    let name = parse_begin_name(header)?;
    let mut output = Vec::new();
    let mut saw_zero = false;

    for line in lines {
        if saw_zero {
            if line.trim() == "end" {
                return Ok(DecodedFile {
                    name,
                    bytes: output,
                });
            }
            if line.trim().is_empty() {
                continue;
            }
            return Err(corrupt("Encoded text has data after the zero-length line"));
        }

        if line.is_empty() {
            continue;
        }
        let line_bytes = line.as_bytes();
        let expected = decode_len(line_bytes[0], xx)?;
        if expected == 0 {
            saw_zero = true;
            continue;
        }
        let decoded = decode_data_line(&line_bytes[1..], expected, xx)?;
        output.extend_from_slice(&decoded);
    }

    Err(corrupt("Encoded text is missing the end marker"))
}

fn encode_text_file<R: Read, W: Write>(
    source: &mut R,
    sink: &mut W,
    name: &str,
    xx: bool,
) -> Result<(), ArchiveError> {
    writeln!(sink, "begin 644 {name}").map_err(io_error)?;
    let mut buffer = [0_u8; 45];
    loop {
        let read = source.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        write_encoded_line(sink, &buffer[..read], xx)?;
    }
    let zero = encode_len(0, xx);
    writeln!(sink, "{zero}").map_err(io_error)?;
    writeln!(sink, "end").map_err(io_error)
}

fn write_encoded_line<W: Write>(sink: &mut W, bytes: &[u8], xx: bool) -> Result<(), ArchiveError> {
    let mut line = Vec::with_capacity(1 + bytes.len().div_ceil(3) * 4);
    line.push(encode_len(bytes.len() as u8, xx));
    for chunk in bytes.chunks(3) {
        let a = chunk.first().copied().unwrap_or(0);
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        let values = [a >> 2, ((a << 4) | (b >> 4)) & 0x3f, ((b << 2) | (c >> 6)) & 0x3f, c & 0x3f];
        for value in values {
            line.push(encode_sixbit(value, xx));
        }
    }
    sink.write_all(&line).map_err(io_error)?;
    sink.write_all(b"\n").map_err(io_error)
}

fn encode_len(len: u8, xx: bool) -> u8 {
    encode_sixbit(len & 0x3f, xx)
}

fn encode_sixbit(value: u8, xx: bool) -> u8 {
    if xx {
        XX_ALPHABET[value as usize]
    } else {
        let encoded = (value & 0x3f) + 0x20;
        if encoded == 0x20 { b'`' } else { encoded }
    }
}

fn parse_begin_name(header: &str) -> Result<String, ArchiveError> {
    let mut parts = header.splitn(3, ' ');
    let _begin = parts.next();
    let _mode = parts
        .next()
        .filter(|mode| !mode.is_empty())
        .ok_or_else(|| corrupt("Encoded text begin header is missing a mode"))?;
    let name = parts
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| corrupt("Encoded text begin header is missing a file name"))?;
    if !matches!(classify_entry_path(name), EntrySafety::Safe) {
        return Err(ArchiveError::new(
            ArchiveErrorKind::PathTraversalBlocked,
            "Encoded output path was blocked by the extraction safety policy",
        )
        .with_backend("encoded")
        .with_entry_path(name));
    }
    Ok(name.replace('\\', "/"))
}

fn decode_data_line(data: &[u8], expected: usize, xx: bool) -> Result<Vec<u8>, ArchiveError> {
    let required_chars = expected.div_ceil(3) * 4;
    if data.len() < required_chars {
        return Err(corrupt(
            "Encoded text line is shorter than its declared length",
        ));
    }
    let mut decoded = Vec::with_capacity(expected);
    for chunk in data[..required_chars].chunks(4) {
        let a = decode_six_bit(chunk[0], xx)?;
        let b = decode_six_bit(chunk[1], xx)?;
        let c = decode_six_bit(chunk[2], xx)?;
        let d = decode_six_bit(chunk[3], xx)?;
        decoded.push((a << 2) | (b >> 4));
        decoded.push((b << 4) | (c >> 2));
        decoded.push((c << 6) | d);
    }
    decoded.truncate(expected);
    Ok(decoded)
}

fn decode_len(byte: u8, xx: bool) -> Result<usize, ArchiveError> {
    Ok(decode_six_bit(byte, xx)? as usize)
}

fn decode_six_bit(byte: u8, xx: bool) -> Result<u8, ArchiveError> {
    if xx {
        XX_ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
            .map(|index| index as u8)
            .ok_or_else(|| corrupt("XXE text contains a non-xxencode character"))
    } else if (b' '..=b'`').contains(&byte) {
        Ok((byte.wrapping_sub(b' ')) & 0x3f)
    } else {
        Err(corrupt("UU text contains a non-uuencode character"))
    }
}

fn single_entry_listing(
    name: String,
    size: Option<u64>,
    compressed_size: Option<u64>,
    method: Option<String>,
    encrypted: bool,
) -> ArchiveListing {
    ArchiveListing {
        entries: vec![ArchiveEntry {
            id: EntryId(0),
            raw_path: name.clone(),
            normalized_path: name.replace('\\', "/"),
            display_path: name.clone(),
            kind: EntryKind::File,
            size,
            compressed_size,
            modified_at: None,
            method,
            encrypted,
            safety: classify_entry_path(&name),
        }],
        directories: Default::default(),
        is_complete: true,
    }
}

fn extract_plan(destination: &Path, entries: usize) -> TaskPlan {
    TaskPlan::new(
        TaskKind::Extract,
        format!("Decode encoded file to {}", destination.display()),
    )
    .estimated_entries(entries)
    .native(random_access_extract_pipeline(PipelineStep::ProbeArchive))
}

fn ensure_entry(entry: EntryId) -> Result<(), ArchiveError> {
    if entry == EntryId(0) {
        Ok(())
    } else {
        Err(
            ArchiveError::new(ArchiveErrorKind::Internal, "Archive entry id was not found")
                .with_backend("encoded"),
        )
    }
}

fn strip_final_extension(name: &str, extension: &str) -> String {
    let suffix = format!(".{extension}");
    if name
        .to_ascii_lowercase()
        .ends_with(&suffix.to_ascii_lowercase())
    {
        name[..name.len() - suffix.len()].to_string()
    } else {
        name.into()
    }
}

fn aes_capabilities() -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Limited,
        extract_all: CapabilityLevel::Unsupported,
        extract_selected: CapabilityLevel::Unsupported,
        create: CapabilityLevel::Unsupported,
        update: CapabilityLevel::Unsupported,
        random_access: CapabilityLevel::Unsupported,
        password_read: CapabilityLevel::Limited,
        password_write: CapabilityLevel::Unsupported,
        header_encryption: CapabilityLevel::Unsupported,
        multi_volume_read: CapabilityLevel::Unsupported,
        multi_volume_write: CapabilityLevel::Unsupported,
        entry_stream_preview: CapabilityLevel::Unsupported,
    }
}

fn encoded_capabilities() -> ArchiveCapabilities {
    ArchiveCapabilities {
        list: CapabilityLevel::Full,
        extract_all: CapabilityLevel::Full,
        extract_selected: CapabilityLevel::Full,
        create: CapabilityLevel::Full,
        update: CapabilityLevel::Unsupported,
        random_access: CapabilityLevel::Full,
        password_read: CapabilityLevel::Unsupported,
        password_write: CapabilityLevel::Unsupported,
        header_encryption: CapabilityLevel::Unsupported,
        multi_volume_read: CapabilityLevel::Unsupported,
        multi_volume_write: CapabilityLevel::Unsupported,
        entry_stream_preview: CapabilityLevel::Full,
    }
}

fn corrupt(message: impl Into<String>) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::CorruptArchive, message).with_backend("encoded")
}

fn io_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "Encoded backend I/O operation failed")
        .with_backend("encoded")
        .with_technical_detail(error.to_string())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use shadow_zip_archive_core::ArchiveBackend;

    use super::*;

    #[test]
    fn probe_detects_aes_crypt_v2_signature() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.bin");
        std::fs::write(&path, b"AES\x02\x00payload").unwrap();

        let probe = EncodedBackend
            .probe(&ArchiveSource::LocalPath(path))
            .unwrap();

        assert_eq!(probe.format, ArchiveFormat::Aes);
        assert_eq!(probe.confidence, ProbeConfidence::Signature);
    }

    #[test]
    fn aes_requires_password_then_reports_unsupported_codec() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.aes");
        std::fs::write(&path, b"AES\x02\x00payload").unwrap();
        let mut archive = EncodedBackend
            .open(ArchiveSource::LocalPath(path), OpenOptions::default())
            .unwrap();

        let no_password = archive
            .extract_all(dir.path(), ExtractOptions::default())
            .unwrap_err();
        assert_eq!(no_password.kind, ArchiveErrorKind::PasswordRequired);

        let with_password = archive
            .extract_all(
                dir.path(),
                ExtractOptions {
                    password: Some("secret".into()),
                    ..ExtractOptions::default()
                },
            )
            .unwrap_err();
        assert_eq!(with_password.kind, ArchiveErrorKind::UnsupportedCodec);
    }

    #[test]
    fn uu_lists_and_extracts_virtual_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.uu");
        std::fs::write(&path, b"begin 644 hello.txt\n#0V%T\n`\nend\n").unwrap();
        let mut archive = EncodedBackend
            .open(ArchiveSource::LocalPath(path), OpenOptions::default())
            .unwrap();

        let listing = archive.listing(ListingMode::Full).unwrap();
        assert_eq!(listing.entries[0].raw_path, "hello.txt");
        assert_eq!(listing.entries[0].size, Some(3));

        archive
            .extract_all(
                dir.path(),
                ExtractOptions {
                    overwrite_policy: OverwritePolicy::Overwrite,
                    ..ExtractOptions::default()
                },
            )
            .unwrap();
        assert_eq!(std::fs::read(dir.path().join("hello.txt")).unwrap(), b"Cat");
    }

    #[test]
    fn uue_open_entry_reader_decodes_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.uue");
        std::fs::write(&path, b"begin 644 hello.txt\n#0V%T\n`\nend\n").unwrap();
        let mut archive = EncodedBackend
            .open(ArchiveSource::LocalPath(path), OpenOptions::default())
            .unwrap();

        let mut reader = archive
            .open_entry_reader(EntryId(0), StreamOptions::default())
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8];
        loop {
            let read = reader.source.read_chunk(&mut buffer).unwrap();
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }

        assert_eq!(bytes, b"Cat");
    }

    #[test]
    fn xxe_lists_and_extracts_virtual_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.xxe");
        std::fs::write(&path, xxe_fixture(b"Cat", "hello.txt")).unwrap();
        let mut archive = EncodedBackend
            .open(ArchiveSource::LocalPath(path), OpenOptions::default())
            .unwrap();

        assert_eq!(
            archive.listing(ListingMode::Full).unwrap().entries[0].method,
            Some("xxencode".into())
        );
        archive
            .extract_selected(
                &[EntryId(0)],
                dir.path(),
                ExtractOptions {
                    overwrite_policy: OverwritePolicy::Overwrite,
                    ..ExtractOptions::default()
                },
            )
            .unwrap();

        assert_eq!(std::fs::read(dir.path().join("hello.txt")).unwrap(), b"Cat");
    }

    #[test]
    fn corrupt_text_maps_to_structured_error() {
        let error = decode_encoded_text(b"begin 644 bad.txt\n#bad\n`\nend\n", false).unwrap_err();

        assert_eq!(error.kind, ArchiveErrorKind::CorruptArchive);
    }

    #[test]
    fn uu_accepts_space_zero_length_line() {
        let decoded = decode_encoded_text(b"begin 644 hello.txt\n#0V%T\n \nend\n", false).unwrap();

        assert_eq!(decoded.bytes, b"Cat");
    }

    fn xxe_fixture(bytes: &[u8], name: &str) -> Vec<u8> {
        let mut output = format!("begin 644 {name}\n").into_bytes();
        output.push(XX_ALPHABET[bytes.len()]);
        let mut chunk = [0_u8; 3];
        chunk[..bytes.len()].copy_from_slice(bytes);
        let groups = [
            chunk[0] >> 2,
            ((chunk[0] << 4) | (chunk[1] >> 4)) & 0x3f,
            ((chunk[1] << 2) | (chunk[2] >> 6)) & 0x3f,
            chunk[2] & 0x3f,
        ];
        for group in groups {
            output.push(XX_ALPHABET[group as usize]);
        }
        output.extend_from_slice(b"\n+\nend\n");
        output
    }
}
