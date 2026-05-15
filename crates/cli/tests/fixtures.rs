use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use flate2::{Compression, write::GzEncoder};
use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
use tar::Builder;
use xz2::write::XzEncoder;
use zip::{ZipWriter, write::SimpleFileOptions};

pub struct FixtureSet {
    pub root: tempfile::TempDir,
    pub basic_zip: PathBuf,
    pub unsafe_zip: PathBuf,
    pub empty_zip: PathBuf,
    pub basic_tar: PathBuf,
    pub basic_targz: PathBuf,
    pub basic_tarxz: PathBuf,
    pub basic_tarzst: PathBuf,
    pub duplicate_zip: PathBuf,
    pub unicode_zip: PathBuf,
    pub many_zip: PathBuf,
    pub corrupt_zip: PathBuf,
    pub unsupported_bin: PathBuf,
    pub input_dir: PathBuf,
    pub readme: PathBuf,
    pub binary: PathBuf,
    pub config_minimal: PathBuf,
    pub config_invalid: PathBuf,
}

impl FixtureSet {
    pub fn create() -> Self {
        let root = tempfile::tempdir().unwrap();
        let input_dir = root.path().join("input");
        fs::create_dir_all(input_dir.join("docs")).unwrap();
        fs::create_dir_all(input_dir.join("images")).unwrap();
        let readme = input_dir.join("docs/readme.txt");
        let binary = input_dir.join("data.bin");
        fs::write(&readme, b"hello shadow zip\n").unwrap();
        fs::write(input_dir.join("docs/manual.md"), b"# Manual\n").unwrap();
        fs::write(input_dir.join("images/pixel.png"), pixel_png()).unwrap();
        fs::write(&binary, (0_u8..=255).collect::<Vec<_>>()).unwrap();

        let basic_zip = root.path().join("basic.zip");
        write_zip(
            &basic_zip,
            &[
                ("docs/readme.txt", b"hello shadow zip\n".as_slice()),
                ("docs/manual.md", b"# Manual\n".as_slice()),
                ("images/pixel.png", pixel_png().as_slice()),
                ("data.bin", &(0_u8..=255).collect::<Vec<_>>()),
            ],
        );

        let unsafe_zip = root.path().join("unsafe-paths.zip");
        write_zip(
            &unsafe_zip,
            &[
                ("../escape.txt", b"escape".as_slice()),
                ("/absolute.txt", b"absolute".as_slice()),
                ("C:/drive.txt", b"drive".as_slice()),
            ],
        );

        let empty_zip = root.path().join("empty.zip");
        write_zip(&empty_zip, &[]);

        let basic_tar = root.path().join("basic.tar");
        write_tar(&basic_tar, false);
        let basic_targz = root.path().join("basic.tar.gz");
        write_tar(&basic_targz, true);
        let basic_tarxz = root.path().join("basic.tar.xz");
        write_tar_xz(&basic_tarxz);
        let basic_tarzst = root.path().join("basic.tar.zst");
        write_tar_zst(&basic_tarzst);

        let duplicate_zip = root.path().join("duplicate-paths.zip");
        write_zip(
            &duplicate_zip,
            &[
                ("same.txt", b"one".as_slice()),
                ("folder/../same.txt", b"two".as_slice()),
            ],
        );

        let unicode_zip = root.path().join("unicode.zip");
        write_zip(
            &unicode_zip,
            &[
                ("文档/说明.txt", "你好\n".as_bytes()),
                ("space name.txt", b"space".as_slice()),
            ],
        );

        let many_zip = root.path().join("many-small-files.zip");
        let many_entries = (0..32)
            .map(|index| (format!("files/{index:03}.txt"), vec![index as u8]))
            .collect::<Vec<_>>();
        write_zip_owned(&many_zip, &many_entries);

        let corrupt_zip = root.path().join("corrupt.zip");
        fs::write(&corrupt_zip, b"not a zip file").unwrap();
        let unsupported_bin = root.path().join("unsupported.bin");
        fs::write(&unsupported_bin, b"not an archive").unwrap();

        let config_minimal = root.path().join("minimal.json");
        fs::write(
            &config_minimal,
            serde_json::to_string_pretty(&shadow_zip_domain::AppConfig::default()).unwrap(),
        )
        .unwrap();
        let config_invalid = root.path().join("invalid.json");
        fs::write(&config_invalid, b"{not json").unwrap();

        Self {
            root,
            basic_zip,
            unsafe_zip,
            empty_zip,
            basic_tar,
            basic_targz,
            basic_tarxz,
            basic_tarzst,
            duplicate_zip,
            unicode_zip,
            many_zip,
            corrupt_zip,
            unsupported_bin,
            input_dir,
            readme,
            binary,
            config_minimal,
            config_invalid,
        }
    }

    pub fn out_dir(&self, name: &str) -> PathBuf {
        let path = self.root.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
}

fn write_zip_owned(path: &Path, entries: &[(String, Vec<u8>)]) {
    let borrowed = entries
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect::<Vec<_>>();
    write_zip(path, &borrowed);
}

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = File::create(path).unwrap();
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in entries {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

fn write_tar(path: &Path, gzip: bool) {
    let file = File::create(path).unwrap();
    if gzip {
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = Builder::new(encoder);
        append_tar_entries(&mut builder);
        builder.finish().unwrap();
    } else {
        let mut builder = Builder::new(file);
        append_tar_entries(&mut builder);
        builder.finish().unwrap();
    }
}

fn write_tar_xz(path: &Path) {
    let file = File::create(path).unwrap();
    let encoder = XzEncoder::new(file, 6);
    let mut builder = Builder::new(encoder);
    append_tar_entries(&mut builder);
    builder.finish().unwrap();
}

fn write_tar_zst(path: &Path) {
    let file = File::create(path).unwrap();
    let encoder = zstd::stream::write::Encoder::new(file, 3).unwrap();
    let mut builder = Builder::new(encoder);
    append_tar_entries(&mut builder);
    builder.finish().unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

fn append_tar_entries<W: Write>(builder: &mut Builder<W>) {
    let mut header = tar::Header::new_gnu();
    header.set_path("docs/readme.txt").unwrap();
    header.set_size(b"hello shadow zip\n".len() as u64);
    header.set_cksum();
    builder
        .append(&header, b"hello shadow zip\n".as_slice())
        .unwrap();

    let mut header = tar::Header::new_gnu();
    header.set_path("docs/manual.md").unwrap();
    header.set_size(b"# Manual\n".len() as u64);
    header.set_cksum();
    builder.append(&header, b"# Manual\n".as_slice()).unwrap();
}

fn pixel_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes)
        .write_image(&[255, 0, 0, 255], 1, 1, ColorType::Rgba8.into())
        .unwrap();
    bytes
}
