#![allow(clippy::result_large_err)]

use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::json;
use shadow_zip_app_core::{
    AppCore, ArchiveUseCases, CatEntryRequest, CreateRequest, DiagnoseRequest, EntrySelection,
    ExtractRequest, InspectRequest, ListRequest, PreviewEntryRequest, TestArchiveRequest,
    TreeRequest,
};
use shadow_zip_domain::*;
use shadow_zip_platform::PlatformConfig;

const RESULT_SCHEMA: &str = "shadow-zip.cli.result.v1";

#[derive(Parser)]
#[command(
    name = "shadow-zip",
    version,
    about = "Shadow Zip command line interface"
)]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    locale: Option<String>,
    #[arg(long, global = true)]
    password: Option<String>,
    #[arg(long, global = true)]
    password_file: Option<PathBuf>,
    #[arg(long, global = true)]
    password_env: Option<String>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true)]
    ndjson: bool,
    #[arg(long, global = true)]
    quiet: bool,
    #[arg(long, global = true)]
    verbose: bool,
    #[arg(long, global = true)]
    no_progress: bool,
    #[arg(long, global = true)]
    no_interaction: bool,
    #[arg(long, global = true, default_value = "auto")]
    color: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Info {
        archive: PathBuf,
    },
    List {
        archive: PathBuf,
        #[arg(long, default_value = "")]
        query: String,
        #[arg(long)]
        kind: Vec<EntryKindArg>,
        #[arg(long)]
        only_encrypted: bool,
        #[arg(long)]
        only_unsafe: bool,
        #[arg(long, default_value = "name")]
        sort: SortArg,
        #[arg(long)]
        desc: bool,
        #[arg(long)]
        columns: Option<String>,
    },
    Tree {
        archive: PathBuf,
        #[arg(long)]
        depth: Option<usize>,
    },
    Preflight {
        #[command(subcommand)]
        command: PreflightCommand,
    },
    Extract {
        archive: PathBuf,
        #[arg(long)]
        to: PathBuf,
        #[arg(long = "entry")]
        entries: Vec<String>,
        #[arg(long = "id")]
        ids: Vec<u64>,
        #[arg(long)]
        include: Vec<String>,
        #[arg(long)]
        exclude: Vec<String>,
        #[arg(long)]
        all_matches: bool,
        #[arg(long)]
        overwrite: bool,
        #[arg(long)]
        skip_existing: bool,
        #[arg(long)]
        rename_existing: bool,
        #[arg(long)]
        keep_newer: bool,
        #[arg(long, default_value = "conservative")]
        symlink_policy: SymlinkArg,
    },
    Create {
        output: PathBuf,
        inputs: Vec<PathBuf>,
        #[arg(long)]
        format: Option<FormatArg>,
        #[arg(long)]
        method: Option<String>,
        #[arg(long)]
        level: Option<u8>,
        #[arg(long)]
        solid: bool,
        #[arg(long)]
        no_solid: bool,
        #[arg(long)]
        encrypt_file_names: bool,
        #[arg(long)]
        volume_size: Option<u64>,
        #[arg(long, default_value = "conservative")]
        symlink_policy: SymlinkArg,
        #[arg(long)]
        archive_path: Option<String>,
    },
    Test {
        archive: PathBuf,
    },
    Cat {
        archive: PathBuf,
        entry: String,
        #[arg(long = "id")]
        id: Option<u64>,
    },
    Preview {
        archive: PathBuf,
        entry: String,
        #[arg(long = "id")]
        id: Option<u64>,
        #[arg(long, default_value = "metadata")]
        mode: PreviewModeArg,
        #[arg(long, default_value_t = 0)]
        width: u32,
        #[arg(long, default_value_t = 0)]
        height: u32,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Backends,
    Helpers,
    Diagnose {
        archive: PathBuf,
    },
    Cache {
        #[command(subcommand)]
        command: CacheCommand,
    },
    Recent {
        #[command(subcommand)]
        command: RecentCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Subcommand)]
enum PreflightCommand {
    Extract {
        archive: PathBuf,
        #[arg(long)]
        to: PathBuf,
        #[arg(long = "entry")]
        entries: Vec<String>,
        #[arg(long = "id")]
        ids: Vec<u64>,
        #[arg(long)]
        include: Vec<String>,
        #[arg(long)]
        exclude: Vec<String>,
        #[arg(long)]
        all_matches: bool,
    },
}

#[derive(Subcommand)]
enum CacheCommand {
    Status,
    Cleanup {
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum RecentCommand {
    List,
    Clear,
}

#[derive(Subcommand)]
enum ConfigCommand {
    Get { key: Option<String> },
    Set { key: String, value: String },
    Path,
}

#[derive(Clone, ValueEnum)]
enum EntryKindArg {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, ValueEnum)]
enum SortArg {
    Name,
    Size,
    PackedSize,
    Type,
    Modified,
    Method,
    Encrypted,
    Path,
}

#[derive(Clone, ValueEnum)]
enum SymlinkArg {
    Conservative,
    PreserveLinks,
    FollowWithinDestination,
}

#[derive(Clone, ValueEnum)]
enum FormatArg {
    Zip,
    SevenZip,
    Tar,
    TarGz,
    TarXz,
    TarZst,
    Rar,
}

#[derive(Clone, ValueEnum)]
enum PreviewModeArg {
    Metadata,
    Thumbnail,
    Fit,
    Full,
    Text,
    External,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if cli.json {
                let payload = json!({
                    "schema": RESULT_SCHEMA,
                    "command": command_name(&cli.command),
                    "ok": false,
                    "error": error,
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload)
                        .unwrap_or_else(|_| "{\"ok\":false}".into())
                );
            } else {
                eprintln!("{}", error.message);
            }
            ExitCode::from(exit_code(error.kind))
        }
    }
}

fn run(cli: &Cli) -> Result<(), ArchiveError> {
    let config = cli
        .config
        .clone()
        .or_else(|| env::var_os("SHADOW_ZIP_CONFIG").map(PathBuf::from))
        .map(AppCore::load_config)
        .unwrap_or_default();
    let core = AppCore::new(config.clone(), PlatformConfig::default());
    let password = resolve_password(cli)?;
    let open_options = OpenOptions {
        password: password.clone(),
        prefer_cached_index: true,
    };

    match &cli.command {
        Command::Info { archive } => {
            let result = core.inspect(InspectRequest {
                archive: archive.clone(),
                open_options,
            })?;
            output(cli, "info", &result, || {
                println!("Archive: {}", result.info.display_name);
                println!("Format: {}", result.info.format);
                println!(
                    "Entries: {}",
                    result
                        .info
                        .entry_count
                        .map_or("-".into(), |n| n.to_string())
                );
            })
        }
        Command::List {
            archive,
            query,
            kind,
            only_encrypted,
            only_unsafe,
            sort,
            desc,
            columns: _,
        } => {
            let result = core.list(ListRequest {
                archive: archive.clone(),
                open_options,
                filter: EntryFilter {
                    query: query.clone(),
                    kinds: kind.iter().map(map_kind).collect(),
                    only_encrypted: *only_encrypted,
                    only_unsafe: *only_unsafe,
                },
                sort: EntrySort {
                    column: map_sort(sort),
                    direction: if *desc {
                        SortDirection::Descending
                    } else {
                        SortDirection::Ascending
                    },
                },
                listing_mode: ListingMode::Full,
            })?;
            output(cli, "list", &result, || {
                print_entries(&result.visible_entries)
            })
        }
        Command::Tree { archive, depth } => {
            let result = core.tree(TreeRequest {
                archive: archive.clone(),
                open_options,
                filter: EntryFilter::default(),
                sort: EntrySort::default(),
                listing_mode: ListingMode::Full,
            })?;
            output(cli, "tree", &result, || {
                for node in result.tree.nodes.values() {
                    let node_depth = tree_node_depth(&node.path);
                    if depth.is_none_or(|max| node_depth <= max) {
                        println!("{}", node.path);
                    }
                }
            })
        }
        Command::Preflight {
            command:
                PreflightCommand::Extract {
                    archive,
                    to,
                    entries,
                    ids,
                    include,
                    exclude,
                    all_matches,
                },
        } => {
            let request = extract_request(
                archive,
                to,
                entries,
                ids,
                include,
                exclude,
                *all_matches,
                open_options,
                ExtractOptions::default(),
            );
            let result = core.preflight_extract(&request)?;
            output(cli, "preflight extract", &result, || {
                println!("Destination: {}", result.destination.display());
                println!("Entries: {}", result.total_entries);
                println!("Conflicts: {}", result.conflicts.len());
                println!("Blocked: {}", result.blocked_entries.len());
            })
        }
        Command::Extract {
            archive,
            to,
            entries,
            ids,
            include,
            exclude,
            all_matches,
            overwrite,
            skip_existing,
            rename_existing,
            keep_newer,
            symlink_policy,
        } => {
            let options = ExtractOptions {
                password,
                overwrite_policy: overwrite_policy(
                    *overwrite,
                    *skip_existing,
                    *rename_existing,
                    *keep_newer,
                ),
                symlink_policy: map_symlink(symlink_policy),
                preserve_permissions: true,
            };
            let result = core.extract(
                extract_request(
                    archive,
                    to,
                    entries,
                    ids,
                    include,
                    exclude,
                    *all_matches,
                    open_options,
                    options,
                ),
                None,
            )?;
            output(cli, "extract", &result, || {
                println!("Task: {}", result.task_id);
                println!("Processed entries: {}", result.summary.processed_entries);
            })
        }
        Command::Create {
            output: output_path,
            inputs,
            format,
            method,
            level,
            solid,
            no_solid,
            encrypt_file_names,
            volume_size,
            symlink_policy,
            archive_path,
        } => {
            let archive_format = format
                .as_ref()
                .map(map_format)
                .unwrap_or_else(|| infer_format(output_path));
            let input_paths = inputs
                .iter()
                .map(|path| InputPath {
                    path: path.clone(),
                    archive_path: archive_path.clone(),
                })
                .collect::<Vec<_>>();
            let result = core.create(
                CreateRequest {
                    inputs: input_paths,
                    output: output_path.clone(),
                    options: CreateOptions {
                        format: archive_format,
                        compression_method: method.clone(),
                        compression_level: *level,
                        solid: *solid && !*no_solid,
                        encrypt_file_names: *encrypt_file_names,
                        password,
                        volume_size: *volume_size,
                        symlink_policy: map_symlink(symlink_policy),
                    },
                },
                None,
            )?;
            output(cli, "create", &result, || {
                println!("Created: {}", output_path.display());
                println!("Task: {}", result.task_id);
            })
        }
        Command::Test { archive } => {
            let result = core.test_archive(
                TestArchiveRequest {
                    archive: archive.clone(),
                    open_options,
                    options: TestOptions { password },
                },
                None,
            )?;
            output(cli, "test", &result, || {
                println!("Task: {}", result.task_id)
            })
        }
        Command::Cat { archive, entry, id } => {
            let result = core.cat(CatEntryRequest {
                archive: archive.clone(),
                entry: if let Some(id) = id {
                    EntrySelection::Ids(vec![EntryId(*id)])
                } else {
                    EntrySelection::Paths {
                        paths: vec![entry.clone()],
                        all_matches: false,
                    }
                },
                open_options,
                stream_options: StreamOptions { password },
            })?;
            if cli.json {
                let payload = json!({
                    "schema": RESULT_SCHEMA,
                    "command": "cat",
                    "ok": true,
                    "result": {
                        "entry": result.entry,
                        "bytes": result.bytes.len(),
                        "access_cost": format!("{:?}", result.access_cost),
                    }
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).map_err(json_error)?
                );
            } else {
                io::stdout().write_all(&result.bytes).map_err(io_error)?;
            }
            Ok(())
        }
        Command::Preview {
            archive,
            entry,
            id,
            mode,
            width,
            height,
            output,
        } => {
            let preview_mode = map_preview_mode(mode);
            let result = core.preview(PreviewEntryRequest {
                archive: archive.clone(),
                entry: if let Some(id) = id {
                    EntrySelection::Ids(vec![EntryId(*id)])
                } else {
                    EntrySelection::Paths {
                        paths: vec![entry.clone()],
                        all_matches: false,
                    }
                },
                open_options,
                mode: preview_mode,
                target_size: shadow_zip_preview::PixelSize {
                    width: *width,
                    height: *height,
                },
            })?;
            if let Some(output) = output {
                fs::write(output, format!("{:?}", result.result)).map_err(io_error)?;
            }
            output_preview(cli, &result)
        }
        Command::Backends => {
            let result = core.backends();
            output(cli, "backends", &result, || {
                for backend in &result {
                    println!("{}", backend.backend_name);
                }
            })
        }
        Command::Helpers => {
            let result = core.helpers();
            output(cli, "helpers", &result, || {
                for helper in &result {
                    println!(
                        "{:?}: {}",
                        helper.kind,
                        if helper.available {
                            "available"
                        } else {
                            "missing"
                        }
                    );
                }
            })
        }
        Command::Diagnose { archive } => {
            let result = core.diagnose(DiagnoseRequest {
                archive: archive.clone(),
            });
            output(cli, "diagnose", &result, || {
                for backend in &result.backends {
                    println!("{}", backend.backend_name);
                }
            })
        }
        Command::Cache { command } => match command {
            CacheCommand::Status => {
                let result = core.cache_summary();
                output(cli, "cache status", &result, || {
                    println!("Indexes: {}", result.index_records);
                    println!("Thumbnails: {}", result.thumbnail_count);
                    println!("Temp files: {}", result.temp_count);
                })
            }
            CacheCommand::Cleanup { dry_run } => {
                if *dry_run {
                    let result = core.cache_summary();
                    output(
                        cli,
                        "cache cleanup",
                        &json!({ "dry_run": true, "before": result }),
                        || {
                            println!("Dry run: cache entries remain unchanged");
                        },
                    )
                } else {
                    let result = core.cleanup_cache()?;
                    output(cli, "cache cleanup", &result, || {
                        println!("Cleanup task: {}", result.task_id);
                    })
                }
            }
        },
        Command::Recent { command } => match command {
            RecentCommand::List => {
                let result = core.recent_files();
                output(cli, "recent list", &result, || {
                    for item in &result {
                        println!("{}", item.display_name);
                    }
                })
            }
            RecentCommand::Clear => {
                core.clear_recent_files();
                output(cli, "recent clear", &json!({ "cleared": true }), || {
                    println!("Recent files cleared");
                })
            }
        },
        Command::Config { command } => match command {
            ConfigCommand::Path => {
                let result = cli.config.clone().unwrap_or_else(default_config_path);
                output(cli, "config path", &json!({ "path": result }), || {
                    println!("{}", result.display());
                })
            }
            ConfigCommand::Get { key } => {
                let value = serde_json::to_value(&config).map_err(json_error)?;
                let result = if let Some(key) = key {
                    get_dotted(&value, key).cloned().ok_or_else(|| {
                        ArchiveError::new(
                            ArchiveErrorKind::Internal,
                            format!("Unknown config key '{key}'"),
                        )
                    })?
                } else {
                    value
                };
                output(cli, "config get", &result, || println!("{result}"))
            }
            ConfigCommand::Set { key, value } => {
                let mut json_config = serde_json::to_value(&config).map_err(json_error)?;
                set_dotted(&mut json_config, key, parse_config_value(value))?;
                let new_config: AppConfig =
                    serde_json::from_value(json_config).map_err(json_error)?;
                let path = cli.config.clone().unwrap_or_else(default_config_path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(io_error)?;
                }
                fs::write(
                    &path,
                    serde_json::to_string_pretty(&new_config).map_err(json_error)?,
                )
                .map_err(io_error)?;
                output(
                    cli,
                    "config set",
                    &json!({ "key": key, "path": path }),
                    || {
                        println!("Updated {key}");
                    },
                )
            }
        },
    }
}

fn output<T: Serialize>(
    cli: &Cli,
    command: &str,
    value: &T,
    human: impl FnOnce(),
) -> Result<(), ArchiveError> {
    if cli.json {
        let payload = json!({
            "schema": RESULT_SCHEMA,
            "command": command,
            "ok": true,
            "result": value,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).map_err(json_error)?
        );
    } else {
        human();
    }
    Ok(())
}

fn tree_node_depth(path: &str) -> usize {
    if path == "/" {
        0
    } else {
        path.trim_matches('/')
            .split('/')
            .filter(|part| !part.is_empty())
            .count()
    }
}

fn output_preview(
    cli: &Cli,
    result: &shadow_zip_app_core::PreviewEntryResult,
) -> Result<(), ArchiveError> {
    let payload = json!({
        "schema": RESULT_SCHEMA,
        "command": "preview",
        "ok": true,
        "result": {
            "preview": format!("{:?}", result.result),
            "access_cost": format!("{:?}", result.access_cost),
            "warnings": result.warnings,
        }
    });
    if cli.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).map_err(json_error)?
        );
    } else {
        println!("{:?}", result.result);
    }
    Ok(())
}

fn print_entries(entries: &[ArchiveEntry]) {
    println!("ID\tType\tSize\tPacked\tMethod\tEnc\tPath");
    for entry in entries {
        println!(
            "{}\t{:?}\t{}\t{}\t{}\t{}\t{}",
            entry.id.0,
            entry.kind,
            entry.size.map_or("-".into(), |value| value.to_string()),
            entry
                .compressed_size
                .map_or("-".into(), |value| value.to_string()),
            entry.method.as_deref().unwrap_or("-"),
            if entry.encrypted { "yes" } else { "no" },
            entry.display_path
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_request(
    archive: &Path,
    to: &Path,
    entries: &[String],
    ids: &[u64],
    include: &[String],
    exclude: &[String],
    all_matches: bool,
    open_options: OpenOptions,
    extract_options: ExtractOptions,
) -> ExtractRequest {
    let selection = if !ids.is_empty() {
        EntrySelection::Ids(ids.iter().map(|id| EntryId(*id)).collect())
    } else if !entries.is_empty() {
        EntrySelection::Paths {
            paths: entries.to_vec(),
            all_matches,
        }
    } else if !include.is_empty() || !exclude.is_empty() {
        EntrySelection::Globs {
            include: include.to_vec(),
            exclude: exclude.to_vec(),
        }
    } else {
        EntrySelection::All
    };
    ExtractRequest {
        archive: archive.to_path_buf(),
        destination: to.to_path_buf(),
        selection,
        open_options,
        extract_options,
        require_preflight_clear: true,
    }
}

fn resolve_password(cli: &Cli) -> Result<Option<String>, ArchiveError> {
    if let Some(password) = &cli.password {
        return Ok(Some(password.clone()));
    }
    if let Some(path) = &cli.password_file {
        return fs::read_to_string(path)
            .map(|text| Some(text.trim_end_matches(['\r', '\n']).to_string()))
            .map_err(io_error);
    }
    if let Some(name) = &cli.password_env {
        return Ok(env::var(name).ok());
    }
    Ok(env::var("SHADOW_ZIP_PASSWORD").ok())
}

fn overwrite_policy(
    overwrite: bool,
    skip: bool,
    rename: bool,
    keep_newer: bool,
) -> OverwritePolicy {
    if overwrite {
        OverwritePolicy::Overwrite
    } else if skip {
        OverwritePolicy::Skip
    } else if rename {
        OverwritePolicy::Rename
    } else if keep_newer {
        OverwritePolicy::KeepNewer
    } else {
        OverwritePolicy::AskBatch
    }
}

fn map_kind(kind: &EntryKindArg) -> EntryKind {
    match kind {
        EntryKindArg::File => EntryKind::File,
        EntryKindArg::Directory => EntryKind::Directory,
        EntryKindArg::Symlink => EntryKind::Symlink,
        EntryKindArg::Other => EntryKind::Other,
    }
}

fn map_sort(sort: &SortArg) -> EntrySortColumn {
    match sort {
        SortArg::Name => EntrySortColumn::Name,
        SortArg::Size => EntrySortColumn::Size,
        SortArg::PackedSize => EntrySortColumn::PackedSize,
        SortArg::Type => EntrySortColumn::Type,
        SortArg::Modified => EntrySortColumn::Modified,
        SortArg::Method => EntrySortColumn::Method,
        SortArg::Encrypted => EntrySortColumn::Encrypted,
        SortArg::Path => EntrySortColumn::Path,
    }
}

fn map_symlink(policy: &SymlinkArg) -> SymlinkPolicy {
    match policy {
        SymlinkArg::Conservative => SymlinkPolicy::Conservative,
        SymlinkArg::PreserveLinks => SymlinkPolicy::PreserveLinks,
        SymlinkArg::FollowWithinDestination => SymlinkPolicy::FollowWithinDestination,
    }
}

fn map_format(format: &FormatArg) -> ArchiveFormat {
    match format {
        FormatArg::Zip => ArchiveFormat::Zip,
        FormatArg::SevenZip => ArchiveFormat::SevenZip,
        FormatArg::Tar => ArchiveFormat::Tar,
        FormatArg::TarGz => ArchiveFormat::TarGz,
        FormatArg::TarXz => ArchiveFormat::TarXz,
        FormatArg::TarZst => ArchiveFormat::TarZst,
        FormatArg::Rar => ArchiveFormat::Rar,
    }
}

fn map_preview_mode(mode: &PreviewModeArg) -> shadow_zip_preview::PreviewMode {
    match mode {
        PreviewModeArg::Metadata => shadow_zip_preview::PreviewMode::Metadata,
        PreviewModeArg::Thumbnail => shadow_zip_preview::PreviewMode::Thumbnail,
        PreviewModeArg::Fit => shadow_zip_preview::PreviewMode::FitWindow,
        PreviewModeArg::Full => shadow_zip_preview::PreviewMode::FullResolution,
        PreviewModeArg::Text => shadow_zip_preview::PreviewMode::Text,
        PreviewModeArg::External => shadow_zip_preview::PreviewMode::ExternalOpen,
    }
}

fn infer_format(path: &Path) -> ArchiveFormat {
    let name = path.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        ArchiveFormat::TarGz
    } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
        ArchiveFormat::TarXz
    } else if name.ends_with(".tar.zst") || name.ends_with(".tzst") {
        ArchiveFormat::TarZst
    } else if name.ends_with(".tar") {
        ArchiveFormat::Tar
    } else if name.ends_with(".7z") {
        ArchiveFormat::SevenZip
    } else if name.ends_with(".rar") {
        ArchiveFormat::Rar
    } else {
        ArchiveFormat::Zip
    }
}

fn get_dotted<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for part in key.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn set_dotted(
    value: &mut serde_json::Value,
    key: &str,
    new_value: serde_json::Value,
) -> Result<(), ArchiveError> {
    let parts = key.split('.').collect::<Vec<_>>();
    let mut current = value;
    for part in &parts[..parts.len().saturating_sub(1)] {
        current = current.get_mut(part).ok_or_else(|| {
            ArchiveError::new(
                ArchiveErrorKind::Internal,
                format!("Unknown config key '{key}'"),
            )
        })?;
    }
    let Some(last) = parts.last() else {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Internal,
            "Config key cannot be empty",
        ));
    };
    let Some(slot) = current.get_mut(last) else {
        return Err(ArchiveError::new(
            ArchiveErrorKind::Internal,
            format!("Unknown config key '{key}'"),
        ));
    };
    *slot = new_value;
    Ok(())
}

fn parse_config_value(value: &str) -> serde_json::Value {
    serde_json::from_str(value).unwrap_or_else(|_| serde_json::Value::String(value.to_string()))
}

fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Info { .. } => "info",
        Command::List { .. } => "list",
        Command::Tree { .. } => "tree",
        Command::Preflight { .. } => "preflight",
        Command::Extract { .. } => "extract",
        Command::Create { .. } => "create",
        Command::Test { .. } => "test",
        Command::Cat { .. } => "cat",
        Command::Preview { .. } => "preview",
        Command::Backends => "backends",
        Command::Helpers => "helpers",
        Command::Diagnose { .. } => "diagnose",
        Command::Cache { .. } => "cache",
        Command::Recent { .. } => "recent",
        Command::Config { .. } => "config",
    }
}

fn default_config_path() -> PathBuf {
    PathBuf::from("shadow-zip.json")
}

fn exit_code(kind: ArchiveErrorKind) -> u8 {
    match kind {
        ArchiveErrorKind::UnsupportedFormat => 3,
        ArchiveErrorKind::UnsupportedCodec | ArchiveErrorKind::UnsupportedFilter => 4,
        ArchiveErrorKind::PasswordRequired => 5,
        ArchiveErrorKind::InvalidPassword => 6,
        ArchiveErrorKind::CorruptArchive => 7,
        ArchiveErrorKind::InsufficientDiskSpace => 8,
        ArchiveErrorKind::PermissionDenied => 9,
        ArchiveErrorKind::PathTooLong => 10,
        ArchiveErrorKind::PathTraversalBlocked => 11,
        ArchiveErrorKind::SymlinkPolicyBlocked => 12,
        ArchiveErrorKind::BackendUnavailable | ArchiveErrorKind::ExternalHelperFailed => 13,
        ArchiveErrorKind::Cancelled => 14,
        ArchiveErrorKind::Io => 15,
        ArchiveErrorKind::Internal => 1,
    }
}

fn io_error(error: std::io::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Io, "I/O operation failed")
        .with_technical_detail(error.to_string())
}

fn json_error(error: serde_json::Error) -> ArchiveError {
    ArchiveError::new(ArchiveErrorKind::Internal, "JSON operation failed")
        .with_technical_detail(error.to_string())
}
