mod fixtures;

use assert_cmd::Command;
use predicates::prelude::*;

use fixtures::FixtureSet;

fn shadow_zip() -> Command {
    Command::cargo_bin("shadow-zip").unwrap()
}

fn json_output(assert: assert_cmd::assert::Assert) -> serde_json::Value {
    serde_json::from_slice(&assert.get_output().stdout).unwrap()
}

fn assert_result_schema(json: &serde_json::Value, command: &str) {
    assert_eq!(json["schema"], "shadow-zip.cli.result.v1");
    assert_eq!(json["command"], command);
    assert_eq!(json["ok"], true);
}

#[test]
fn cli_global_001_root_help_lists_commands() {
    shadow_zip()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Shadow Zip command line interface",
        ))
        .stdout(predicate::str::contains("info"))
        .stdout(predicate::str::contains("extract"))
        .stdout(predicate::str::contains("create"));
}

#[test]
fn cli_global_002_extract_help_lists_policies() {
    shadow_zip()
        .args(["extract", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--overwrite"))
        .stdout(predicate::str::contains("--skip-existing"))
        .stdout(predicate::str::contains("--rename-existing"));
}

#[test]
fn cli_global_003_version_works() {
    shadow_zip().arg("--version").assert().success();
}

#[test]
fn cli_global_004_unknown_command_exits_2() {
    shadow_zip()
        .arg("nope")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn cli_global_005_missing_required_arg_exits_2() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--to"));
}

#[test]
fn cli_global_006_info_json_schema() {
    let fixtures = FixtureSet::create();
    let json = json_output(
        shadow_zip()
            .args(["info"])
            .arg(&fixtures.basic_zip)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_result_schema(&json, "info");
}

#[test]
fn cli_global_007_json_error_schema_for_unsupported() {
    let fixtures = FixtureSet::create();
    let json = json_output(
        shadow_zip()
            .args(["info"])
            .arg(&fixtures.unsupported_bin)
            .arg("--json")
            .assert()
            .failure(),
    );
    assert_eq!(json["schema"], "shadow-zip.cli.result.v1");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["kind"], "UnsupportedFormat");
}

#[test]
fn cli_global_009_no_progress_keeps_stderr_clean() {
    let fixtures = FixtureSet::create();
    let out = fixtures.out_dir("no-progress");
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .arg("--to")
        .arg(&out)
        .arg("--no-progress")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}

#[test]
fn cli_global_010_quiet_test_succeeds() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["test"])
        .arg(&fixtures.basic_zip)
        .arg("--quiet")
        .assert()
        .success();
}

#[test]
fn cli_global_012_color_never_has_no_ansi() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["list"])
        .arg(&fixtures.basic_zip)
        .args(["--color", "never"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}").not());
}

#[test]
fn cli_global_014_custom_config_loads() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .arg("--config")
        .arg(&fixtures.config_minimal)
        .args(["info"])
        .arg(&fixtures.basic_zip)
        .arg("--json")
        .assert()
        .success();
}

#[test]
fn cli_backend_001_backends_json_lists_all_backends() {
    let json = json_output(shadow_zip().args(["backends", "--json"]).assert().success());
    let text = json.to_string();
    assert!(text.contains("zip"));
    assert!(text.contains("7z"));
    assert!(text.contains("tar-stream"));
    assert!(text.contains("unrar"));
    assert!(text.contains("libarchive-fallback"));
}

#[test]
fn cli_backend_002_to_006_info_reports_zip_and_tar_formats() {
    let fixtures = FixtureSet::create();
    for (path, expected) in [
        (&fixtures.basic_zip, "Zip"),
        (&fixtures.basic_tar, "Tar"),
        (&fixtures.basic_targz, "TarGz"),
        (&fixtures.basic_tarxz, "TarXz"),
        (&fixtures.basic_tarzst, "TarZst"),
    ] {
        let json = json_output(
            shadow_zip()
                .args(["info"])
                .arg(path)
                .arg("--json")
                .assert()
                .success(),
        );
        assert_eq!(json["result"]["info"]["format"], expected);
    }
}

#[test]
fn cli_backend_012_unsupported_exits_3() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["info"])
        .arg(&fixtures.unsupported_bin)
        .assert()
        .code(3);
}

#[test]
fn cli_backend_013_diagnose_lists_backend_probes() {
    let fixtures = FixtureSet::create();
    let json = json_output(
        shadow_zip()
            .args(["diagnose"])
            .arg(&fixtures.unsupported_bin)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_result_schema(&json, "diagnose");
    assert!(json["result"]["backends"].as_array().unwrap().len() >= 4);
}

#[test]
fn cli_info_001_basic_human_output() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["info"])
        .arg(&fixtures.basic_zip)
        .assert()
        .success()
        .stdout(predicate::str::contains("Archive:"))
        .stdout(predicate::str::contains("Format: ZIP"))
        .stdout(predicate::str::contains("Entries:"));
}

#[test]
fn cli_info_003_empty_zip_success() {
    let fixtures = FixtureSet::create();
    let json = json_output(
        shadow_zip()
            .args(["info"])
            .arg(&fixtures.empty_zip)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(json["result"]["info"]["entry_count"], 0);
}

#[test]
fn cli_info_004_unicode_display_name_round_trips() {
    let fixtures = FixtureSet::create();
    let json = json_output(
        shadow_zip()
            .args(["info"])
            .arg(&fixtures.unicode_zip)
            .arg("--json")
            .assert()
            .success(),
    );
    assert!(
        json["result"]["info"]["display_name"]
            .as_str()
            .unwrap()
            .contains("unicode.zip")
    );
}

#[test]
fn cli_info_005_corrupt_zip_exits_7() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["info"])
        .arg(&fixtures.corrupt_zip)
        .arg("--json")
        .assert()
        .code(7)
        .stdout(predicate::str::contains("CorruptArchive"));
}

#[test]
fn cli_list_001_default_table_outputs_entries() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["list"])
        .arg(&fixtures.basic_zip)
        .assert()
        .success()
        .stdout(predicate::str::contains("docs/readme.txt"))
        .stdout(predicate::str::contains("images/pixel.png"));
}

#[test]
fn cli_list_002_json_entries_have_core_fields() {
    let fixtures = FixtureSet::create();
    let json = json_output(
        shadow_zip()
            .args(["list"])
            .arg(&fixtures.basic_zip)
            .arg("--json")
            .assert()
            .success(),
    );
    let entries = json["result"]["visible_entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry["display_path"] == "docs/readme.txt")
    );
    assert!(entries[0].get("id").is_some());
    assert!(entries[0].get("kind").is_some());
}

#[test]
fn cli_list_003_to_011_filters_and_sorts() {
    let fixtures = FixtureSet::create();
    let query = json_output(
        shadow_zip()
            .args(["list"])
            .arg(&fixtures.basic_zip)
            .args(["--query", "readme", "--json"])
            .assert()
            .success(),
    );
    let entries = query["result"]["visible_entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["display_path"], "docs/readme.txt");

    let files = json_output(
        shadow_zip()
            .args(["list"])
            .arg(&fixtures.basic_zip)
            .args(["--kind", "file", "--json"])
            .assert()
            .success(),
    );
    assert!(
        files["result"]["visible_entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["kind"] == "File")
    );

    shadow_zip()
        .args(["list"])
        .arg(&fixtures.basic_zip)
        .args(["--sort", "size", "--desc", "--json"])
        .assert()
        .success();

    shadow_zip()
        .args(["list"])
        .arg(&fixtures.basic_zip)
        .args(["--columns", "id,path,size"])
        .assert()
        .success()
        .stdout(predicate::str::contains("docs/readme.txt"));
}

#[test]
fn cli_list_007_only_unsafe_and_unicode_and_many() {
    let fixtures = FixtureSet::create();
    let unsafe_json = json_output(
        shadow_zip()
            .args(["list"])
            .arg(&fixtures.unsafe_zip)
            .args(["--only-unsafe", "--json"])
            .assert()
            .success(),
    );
    assert!(
        unsafe_json["result"]["visible_entries"]
            .as_array()
            .unwrap()
            .len()
            >= 2
    );

    shadow_zip()
        .args(["list"])
        .arg(&fixtures.unicode_zip)
        .arg("--json")
        .assert()
        .success()
        .stdout(predicate::str::contains("说明"));

    let many = json_output(
        shadow_zip()
            .args(["list"])
            .arg(&fixtures.many_zip)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(
        many["result"]["visible_entries"].as_array().unwrap().len(),
        32
    );
}

#[test]
fn cli_list_012_duplicate_paths_are_not_lost() {
    let fixtures = FixtureSet::create();
    let json = json_output(
        shadow_zip()
            .args(["list"])
            .arg(&fixtures.duplicate_zip)
            .arg("--json")
            .assert()
            .success(),
    );
    assert!(json["result"]["visible_entries"].as_array().unwrap().len() >= 2);
}

#[test]
fn cli_tree_001_default_tree_outputs_directories() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["tree"])
        .arg(&fixtures.basic_zip)
        .assert()
        .success()
        .stdout(predicate::str::contains("/docs"))
        .stdout(predicate::str::contains("/images"));
}

#[test]
fn cli_tree_002_depth_limits_output() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["tree"])
        .arg(&fixtures.basic_zip)
        .args(["--depth", "0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("/docs").not());
}

#[test]
fn cli_tree_003_to_005_json_duplicate_and_unsafe() {
    let fixtures = FixtureSet::create();
    let json = json_output(
        shadow_zip()
            .args(["tree"])
            .arg(&fixtures.basic_zip)
            .arg("--json")
            .assert()
            .success(),
    );
    assert!(json["result"]["tree"]["nodes"].get("/").is_some());

    shadow_zip()
        .args(["tree"])
        .arg(&fixtures.duplicate_zip)
        .arg("--json")
        .assert()
        .success();
    shadow_zip()
        .args(["tree"])
        .arg(&fixtures.unsafe_zip)
        .arg("--json")
        .assert()
        .success();
}

#[test]
fn cli_preflight_001_002_clean_target() {
    let fixtures = FixtureSet::create();
    let out = fixtures.root.path().join("new-out");
    let json = json_output(
        shadow_zip()
            .args(["preflight", "extract"])
            .arg(&fixtures.basic_zip)
            .arg("--to")
            .arg(&out)
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(json["result"]["conflicts"].as_array().unwrap().len(), 0);
    assert_eq!(
        json["result"]["blocked_entries"].as_array().unwrap().len(),
        0
    );
}

#[test]
fn cli_preflight_003_target_is_file() {
    let fixtures = FixtureSet::create();
    let file = fixtures.root.path().join("target-file");
    std::fs::write(&file, b"x").unwrap();
    shadow_zip()
        .args(["preflight", "extract"])
        .arg(&fixtures.basic_zip)
        .arg("--to")
        .arg(&file)
        .assert()
        .code(9);
}

#[test]
fn cli_preflight_005_006_conflict_detection() {
    let fixtures = FixtureSet::create();
    let out = fixtures.out_dir("conflict-out");
    std::fs::create_dir_all(out.join("docs")).unwrap();
    std::fs::write(out.join("docs/readme.txt"), b"old").unwrap();
    let json = json_output(
        shadow_zip()
            .args(["preflight", "extract"])
            .arg(&fixtures.basic_zip)
            .arg("--to")
            .arg(&out)
            .arg("--entry")
            .arg("docs/readme.txt")
            .arg("--json")
            .assert()
            .success(),
    );
    assert_eq!(json["result"]["conflicts"].as_array().unwrap().len(), 1);
}

#[test]
fn cli_preflight_007_to_011_blocks_unsafe_paths() {
    let fixtures = FixtureSet::create();
    let out = fixtures.out_dir("unsafe-out");
    let json = json_output(
        shadow_zip()
            .args(["preflight", "extract"])
            .arg(&fixtures.unsafe_zip)
            .arg("--to")
            .arg(&out)
            .arg("--json")
            .assert()
            .success(),
    );
    assert!(json["result"]["blocked_entries"].as_array().unwrap().len() >= 2);
}

#[test]
fn cli_extract_001_zip_writes_files() {
    let fixtures = FixtureSet::create();
    let out = fixtures.out_dir("extract-out");
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .arg("--to")
        .arg(&out)
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(out.join("docs/readme.txt")).unwrap(),
        "hello shadow zip\n"
    );
}

#[test]
fn cli_extract_002_to_005_tar_variants_write_files() {
    let fixtures = FixtureSet::create();
    for archive in [
        &fixtures.basic_tar,
        &fixtures.basic_targz,
        &fixtures.basic_tarxz,
        &fixtures.basic_tarzst,
    ] {
        let out = fixtures.out_dir(&format!(
            "out-{}",
            archive
                .file_name()
                .unwrap()
                .to_string_lossy()
                .replace('.', "-")
        ));
        shadow_zip()
            .args(["extract"])
            .arg(archive)
            .arg("--to")
            .arg(&out)
            .assert()
            .success();
        assert_eq!(
            std::fs::read_to_string(out.join("docs/readme.txt")).unwrap(),
            "hello shadow zip\n"
        );
    }
}

#[test]
fn cli_extract_006_007_selected_id_and_path() {
    let fixtures = FixtureSet::create();
    let path_out = fixtures.out_dir("selected-path");
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .args(["--entry", "docs/readme.txt"])
        .arg("--to")
        .arg(&path_out)
        .assert()
        .success();
    assert!(path_out.join("docs/readme.txt").exists());
    assert!(!path_out.join("docs/manual.md").exists());

    let id_out = fixtures.out_dir("selected-id");
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .args(["--id", "0"])
        .arg("--to")
        .arg(&id_out)
        .assert()
        .success();
    assert!(id_out.join("docs/readme.txt").exists());
}

#[test]
fn cli_extract_010_011_include_exclude_globs() {
    let fixtures = FixtureSet::create();
    let include_out = fixtures.out_dir("include");
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .args(["--include", "docs/*.txt"])
        .arg("--to")
        .arg(&include_out)
        .assert()
        .success();
    assert!(include_out.join("docs/readme.txt").exists());
    assert!(!include_out.join("docs/manual.md").exists());

    let exclude_out = fixtures.out_dir("exclude");
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .args(["--exclude", "*.bin"])
        .arg("--to")
        .arg(&exclude_out)
        .assert()
        .success();
    assert!(!exclude_out.join("data.bin").exists());
}

#[test]
fn cli_extract_012_to_015_conflict_policies() {
    let fixtures = FixtureSet::create();
    let out = fixtures.out_dir("conflict");
    std::fs::create_dir_all(out.join("docs")).unwrap();
    std::fs::write(out.join("docs/readme.txt"), b"old").unwrap();
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .arg("--to")
        .arg(&out)
        .assert()
        .failure();
    assert_eq!(std::fs::read(out.join("docs/readme.txt")).unwrap(), b"old");

    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .arg("--to")
        .arg(&out)
        .arg("--overwrite")
        .assert()
        .success();
    assert_eq!(
        std::fs::read_to_string(out.join("docs/readme.txt")).unwrap(),
        "hello shadow zip\n"
    );

    std::fs::write(out.join("docs/readme.txt"), b"old").unwrap();
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .arg("--to")
        .arg(&out)
        .arg("--rename-existing")
        .assert()
        .success();
    assert!(out.join("docs/readme (1).txt").exists());
}

#[test]
fn cli_extract_017_030_unsafe_never_writes_outside() {
    let fixtures = FixtureSet::create();
    let out = fixtures.out_dir("unsafe-extract");
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.unsafe_zip)
        .arg("--to")
        .arg(&out)
        .assert()
        .code(11);
    assert!(!fixtures.root.path().join("escape.txt").exists());
}

#[test]
fn cli_create_001_to_007_zip_tar_variants_round_trip() {
    let fixtures = FixtureSet::create();
    for (name, format) in [
        ("created.zip", "zip"),
        ("created.tar", "tar"),
        ("created.tar.gz", "tar-gz"),
        ("created.tar.xz", "tar-xz"),
        ("created.tar.zst", "tar-zst"),
    ] {
        let output = fixtures.root.path().join(name);
        shadow_zip()
            .args(["create"])
            .arg(&output)
            .arg(&fixtures.input_dir)
            .args(["--format", format])
            .assert()
            .success();
        shadow_zip()
            .args(["list"])
            .arg(&output)
            .assert()
            .success()
            .stdout(predicate::str::contains("readme.txt"));
    }
}

#[test]
fn cli_create_010_011_multi_input_and_archive_path() {
    let fixtures = FixtureSet::create();
    let output = fixtures.root.path().join("multi.zip");
    shadow_zip()
        .args(["create"])
        .arg(&output)
        .arg(&fixtures.readme)
        .arg(&fixtures.binary)
        .assert()
        .success();
    shadow_zip()
        .args(["list"])
        .arg(&output)
        .assert()
        .success()
        .stdout(predicate::str::contains("readme.txt"))
        .stdout(predicate::str::contains("data.bin"));

    let mapped = fixtures.root.path().join("mapped.zip");
    shadow_zip()
        .args(["create"])
        .arg(&mapped)
        .arg(&fixtures.readme)
        .args(["--archive-path", "docs/mapped.txt"])
        .assert()
        .success();
    shadow_zip()
        .args(["list"])
        .arg(&mapped)
        .assert()
        .success()
        .stdout(predicate::str::contains("docs/mapped.txt"));
}

#[test]
fn cli_create_020_021_022_023_validation_errors() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["create"])
        .arg(fixtures.root.path().join("no-input.zip"))
        .assert()
        .failure();

    shadow_zip()
        .args(["create"])
        .arg(fixtures.root.path().join("empty-password.zip"))
        .arg(&fixtures.input_dir)
        .args(["--password", ""])
        .assert()
        .code(5);

    shadow_zip()
        .args(["create"])
        .arg(fixtures.root.path().join("small-volume.zip"))
        .arg(&fixtures.input_dir)
        .args(["--volume-size", "1024"])
        .assert()
        .failure();

    shadow_zip()
        .args(["create"])
        .arg(fixtures.root.path().join("out.rar"))
        .arg(&fixtures.input_dir)
        .args(["--format", "rar", "--json"])
        .assert()
        .code(3)
        .stdout(predicate::str::contains("UnsupportedFormat"));
}

#[test]
fn cli_test_001_002_005_basic_and_corrupt() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["test"])
        .arg(&fixtures.basic_zip)
        .assert()
        .success();
    shadow_zip()
        .args(["test"])
        .arg(&fixtures.basic_targz)
        .arg("--json")
        .assert()
        .success();
    shadow_zip()
        .args(["test"])
        .arg(&fixtures.corrupt_zip)
        .arg("--json")
        .assert()
        .code(7);
}

#[test]
fn cli_cat_001_002_003_text_binary_id() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["cat"])
        .arg(&fixtures.basic_zip)
        .arg("docs/readme.txt")
        .assert()
        .success()
        .stdout(predicate::eq("hello shadow zip\n"));

    let binary = shadow_zip()
        .args(["cat"])
        .arg(&fixtures.basic_zip)
        .arg("data.bin")
        .assert()
        .success();
    assert_eq!(binary.get_output().stdout.len(), 256);

    shadow_zip()
        .args(["cat"])
        .arg(&fixtures.basic_zip)
        .arg("ignored")
        .args(["--id", "0"])
        .assert()
        .success()
        .stdout(predicate::eq("hello shadow zip\n"));
}

#[test]
fn cli_cat_005_006_directory_and_unsafe_errors() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["cat"])
        .arg(&fixtures.basic_zip)
        .arg("docs/")
        .assert()
        .failure();
    shadow_zip()
        .args(["cat"])
        .arg(&fixtures.unsafe_zip)
        .arg("../escape.txt")
        .assert()
        .code(11);
}

#[test]
fn cli_preview_001_002_005_009_010_modes() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["preview"])
        .arg(&fixtures.basic_zip)
        .arg("images/pixel.png")
        .args(["--mode", "metadata", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Metadata"));

    shadow_zip()
        .args(["preview"])
        .arg(&fixtures.basic_zip)
        .arg("docs/readme.txt")
        .args(["--mode", "text"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello shadow zip"));

    let output = fixtures.root.path().join("thumb.out");
    shadow_zip()
        .args(["preview"])
        .arg(&fixtures.basic_zip)
        .arg("images/pixel.png")
        .args(["--mode", "thumbnail", "--output"])
        .arg(&output)
        .assert()
        .success();
    assert!(output.exists());

    shadow_zip()
        .args(["preview"])
        .arg(&fixtures.basic_zip)
        .arg("data.bin")
        .args(["--mode", "thumbnail", "--json"])
        .assert()
        .failure();

    shadow_zip()
        .args(["preview"])
        .arg(&fixtures.basic_zip)
        .arg("docs/readme.txt")
        .args(["--mode", "external", "--json"])
        .assert()
        .success();
}

#[test]
fn cli_helper_001_and_cache_recent_config_commands() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["--json", "helpers"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shadow-zip.cli.result.v1"));

    shadow_zip()
        .args(["cache", "status", "--json"])
        .assert()
        .success();
    shadow_zip()
        .args(["cache", "cleanup", "--dry-run", "--json"])
        .assert()
        .success();
    shadow_zip()
        .args(["recent", "list", "--json"])
        .assert()
        .success();
    shadow_zip()
        .args(["recent", "clear", "--json"])
        .assert()
        .success();
    shadow_zip().args(["config", "path"]).assert().success();
    shadow_zip()
        .args(["--config"])
        .arg(&fixtures.config_minimal)
        .args(["config", "get", "preview.max_input_bytes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("16777216"));
}

#[test]
fn cli_config_set_unknown_invalid_and_precedence() {
    let fixtures = FixtureSet::create();
    let config = fixtures.root.path().join("config.json");
    std::fs::copy(&fixtures.config_minimal, &config).unwrap();
    shadow_zip()
        .arg("--config")
        .arg(&config)
        .args(["config", "set", "default_compression_level", "7"])
        .assert()
        .success();
    shadow_zip()
        .arg("--config")
        .arg(&config)
        .args(["config", "get", "default_compression_level"])
        .assert()
        .success()
        .stdout(predicate::str::contains("7"));
    shadow_zip()
        .arg("--config")
        .arg(&config)
        .args(["config", "get", "nope.key"])
        .assert()
        .failure();
    shadow_zip()
        .arg("--config")
        .arg(&fixtures.config_invalid)
        .args(["config", "get"])
        .assert()
        .success();
}

#[test]
fn cli_err_and_security_cases() {
    let fixtures = FixtureSet::create();
    shadow_zip()
        .args(["extract"])
        .arg(&fixtures.basic_zip)
        .assert()
        .code(2);
    shadow_zip()
        .args(["info"])
        .arg(&fixtures.unsupported_bin)
        .assert()
        .code(3);
    shadow_zip()
        .args(["info"])
        .arg(&fixtures.corrupt_zip)
        .assert()
        .code(7);
    shadow_zip()
        .args(["cat"])
        .arg(&fixtures.unsafe_zip)
        .arg("../escape.txt")
        .assert()
        .code(11);
}
