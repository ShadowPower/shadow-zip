# Shadow Zip CLI 开发任务清单

文档日期：2026-05-14

## 1. 使用说明

本清单用于把 `CLI_DESIGN.md` 和 `CLI_TEST_CASES.md` 落地为可执行开发任务。每个任务都必须满足以下格式：

- 任务项可勾选。
- 任务内容明确到文件、模块、接口或命令。
- 完成标准说明代码层面必须做到什么。
- 验收标准说明如何验证任务完成。
- 未满足验收标准时，不得勾选任务。

执行原则：

- 未来桌面端和 CLI 都必须调用同一个 `app-core` use case。
- CLI 不得直接绕过 app-core 调用具体 backend。
- 未来桌面端不得保留后端选择、preflight、extract、create、test、preview 等核心业务编排。
- 所有 CLI 输出、退出码、JSON/NDJSON schema 必须稳定。
- 每个对外暴露的 CLI 功能都必须有测试。

## 2. 全局完成定义

全部任务完成后，必须满足：

- `cargo test --workspace` 通过。
- CLI 必跑 E2E 套件通过。
- 当前阶段不要求桌面端 smoke test；未来 Flutter 桌面端落地后补充对应 smoke test。
- `CLI_TEST_CASES.md` 中已实现功能对应的用例不再标记 pending。
- `shadow-zip --help` 能列出所有已实现命令。
- `shadow-zip info/list/preflight extract/extract/create/test/backends/helpers --json` 输出合法 JSON。
- CLI 测通能覆盖未来桌面端的主要核心逻辑。

## 3. 阶段 0：准备与约束

- [ ] **CLI-DEV-0001：确认当前 workspace 状态**
  - 任务：运行 `git status --short`，确认当前未提交文件和工作树状态。
  - 完成标准：记录当前新增/修改文件；不覆盖用户已有改动。
  - 验收标准：开发记录或 PR 描述中列出开始开发时的工作树状态。

- [ ] **CLI-DEV-0002：建立 CLI 开发分支**
  - 任务：从当前分支创建开发分支，例如 `feature/cli-app-core`。
  - 完成标准：所有 CLI 相关改动都在该分支完成。
  - 验收标准：`git branch --show-current` 输出开发分支名。

- [ ] **CLI-DEV-0003：确认文档基线**
  - 任务：确认根目录存在 `CLI_DESIGN.md` 和 `CLI_TEST_CASES.md`。
  - 完成标准：两个文件能被打开，内容包含 app-core、CLI 测试和 GUI 覆盖映射。
  - 验收标准：`Test-Path CLI_DESIGN.md` 和 `Test-Path CLI_TEST_CASES.md` 均为 true。

- [ ] **CLI-DEV-0004：新增任务追踪说明**
  - 任务：在 PR 描述或开发记录中声明本清单为开发验收依据。
  - 完成标准：每个 PR 子任务引用本文件任务 ID。
  - 验收标准：代码评审时能按任务 ID 查到对应改动。

## 4. 阶段 1：抽出 app-core

- [ ] **CLI-DEV-0101：新增 `crates/app-core` crate**
  - 任务：创建 `crates/app-core/Cargo.toml` 和 `crates/app-core/src/lib.rs`。
  - 完成标准：crate 名称为 `shadow-zip-app-core`；edition 继承 workspace；依赖 domain、archive-core、archive backends、preview、cache、platform、task-engine。
  - 验收标准：`cargo check -p shadow-zip-app-core` 成功。

- [ ] **CLI-DEV-0102：将 app-core 加入 workspace**
  - 任务：在根 `Cargo.toml` 的 workspace members 中加入 `crates/app-core`，并在 workspace dependencies 中加入 `shadow-zip-app-core` path 依赖。
  - 完成标准：workspace 能识别 `shadow-zip-app-core`。
  - 验收标准：`cargo metadata` 输出包含 `shadow-zip-app-core`。

- [ ] **CLI-DEV-0103：定义 app-core 公共入口 `AppCore`**
  - 任务：在 `crates/app-core/src/lib.rs` 中定义 `pub struct AppCore`。
  - 完成标准：`AppCore::new(config: AppConfig, platform_config: PlatformConfig) -> Self` 可构造所有后端、task engine、preview service、preflight service、cache service。
  - 验收标准：新增单元测试 `app_core_constructs_with_default_config` 通过。

- [ ] **CLI-DEV-0104：定义 app-core use case trait**
  - 任务：定义 `pub trait ArchiveUseCases`，包含 `inspect`、`list`、`tree`、`preflight_extract`、`extract`、`create`、`test`、`preview`、`helpers`、`diagnose`、`cache_status`、`cache_cleanup`、`recent_list`。
  - 完成标准：trait 方法只接收 request model 和 progress sink，不接收 GUI 类型。
  - 验收标准：`crates/app-core` 不依赖 Flutter 或已移除的 UI crate。

- [ ] **CLI-DEV-0105：定义 inspect request/response**
  - 任务：新增 `InspectRequest` 和 `InspectResult`。
  - 完成标准：`InspectRequest` 至少包含 `archive: PathBuf`、`open_options: OpenOptions`；`InspectResult` 至少包含 `info: ArchiveInfo`、`capabilities: ArchiveCapabilities`、`backend_name: Option<String>`。
  - 验收标准：`AppCore::inspect` 能打开 `basic.zip` 并返回 `ArchiveFormat::Zip`。

- [ ] **CLI-DEV-0106：定义 list request/response**
  - 任务：新增 `ListRequest` 和 `ListResult`。
  - 完成标准：request 包含 archive、open_options、filter、sort、listing_mode；result 包含 info、capabilities、listing、visible_entries。
  - 验收标准：`AppCore::list` 支持 query、kind、only_encrypted、only_unsafe、sort、desc。

- [ ] **CLI-DEV-0107：定义 tree request/response**
  - 任务：新增 `TreeRequest` 和 `TreeResult`。
  - 完成标准：result 包含 `DirectoryTree` 和可选 depth 限制后的展示节点。
  - 验收标准：`basic.zip` 的 tree 包含 `/`、`/docs`、`/images`。

- [ ] **CLI-DEV-0108：定义 entry selection model**
  - 任务：新增 `EntrySelection` enum：`All`、`Ids(Vec<EntryId>)`、`Paths(Vec<String>)`、`Globs { include, exclude }`。
  - 完成标准：支持路径重复检测；重复路径默认返回参数错误或结构化错误；支持 `all_matches` 选项。
  - 验收标准：`duplicate-paths.zip` 用 path 选择默认失败，用 id 选择成功。

- [ ] **CLI-DEV-0109：定义 extract request/response**
  - 任务：新增 `ExtractRequest` 和 `ExtractResult`。
  - 完成标准：request 包含 archive、destination、selection、open_options、extract_options、require_preflight_clear；result 包含 task id、summary、warnings、preflight。
  - 验收标准：`AppCore::extract` 在默认冲突策略下遇到冲突失败，不覆盖文件。

- [ ] **CLI-DEV-0110：定义 create request/response**
  - 任务：新增 `CreateRequest` 和 `CreateResult`。
  - 完成标准：request 包含 output、inputs、format、method、level、solid、password、volume_size、path_mode、symlink_policy；result 包含 task id、summary、warnings。
  - 验收标准：`CreateRequest` 能从 `CreateArchiveDraft::default_for` 复用默认 profile。

- [ ] **CLI-DEV-0111：定义 test request/response**
  - 任务：新增 `TestArchiveRequest` 和 `TestArchiveResult`。
  - 完成标准：request 包含 archive 和 open/test password options；result 包含 task id、summary、warnings。
  - 验收标准：`AppCore::test` 对 `basic.zip` 返回成功。

- [ ] **CLI-DEV-0112：定义 preview request/response**
  - 任务：新增 `PreviewEntryRequest` 和 `PreviewEntryResult`。
  - 完成标准：request 包含 archive、entry selection、mode、target size、output path、open options；result 包含 `PreviewResult`、warnings、access_cost。
  - 验收标准：metadata 模式不要求 output path；bitmap 模式无 output path 时返回参数错误。

- [ ] **CLI-DEV-0113：定义 helper/diagnose request/response**
  - 任务：新增 `HelpersResult`、`DiagnoseRequest`、`DiagnoseResult`。
  - 完成标准：diagnose result 包含每个 backend 的 probe 结果、open 结果、helper 状态和 causes。
  - 验收标准：`diagnose unsupported.bin` 返回所有 backend probe 信息。

- [ ] **CLI-DEV-0114：保持核心业务逻辑集中在 app-core**
  - 任务：确认后端构造、open、preflight、extract、create、test、preview、recent、diagnostics、config 操作都在 `crates/app-core` 或更底层核心 crate。
  - 完成标准：CLI 和未来桌面端都可以通过 app-core use case 触发核心流程。
  - 验收标准：workspace 中不存在旧桌面专用 app/ui crate，且 `crates/app-core` 不依赖 UI 框架。

- [ ] **CLI-DEV-0115：预留未来 Flutter adapter 边界**
  - 任务：确保 app-core request/response 不包含 CLI 专用 stdout/stderr 类型，也不包含 Flutter/平台窗口对象。
  - 完成标准：adapter 只需要做 request 转换和 response 转换，不需要做后端选择、安全检查或任务执行。
  - 验收标准：代码搜索确认 app-core 不出现 UI 框架依赖。

- [ ] **CLI-DEV-0117：新增 app-core 基础测试**
  - 任务：在 `crates/app-core/tests/use_cases.rs` 中新增 inspect/list/preflight/extract/create/test 基础测试。
  - 完成标准：测试使用 tempfile 和真实小 fixture，不依赖 Flutter。
  - 验收标准：`cargo test -p shadow-zip-app-core` 通过。

## 5. 阶段 2：补齐真实执行边界

- [ ] **CLI-DEV-0201：定义统一 task runner**
  - 任务：在 app-core 中实现 `run_task_to_completion(plan, priority, progress_sink)`。
  - 完成标准：方法负责 enqueue、run_next、返回 `TaskSummary` 或 `ArchiveError`。
  - 验收标准：extract/create/test use case 不直接暴露未执行的 task plan，除非显式 background 模式。

- [ ] **CLI-DEV-0202：定义 progress sink adapter trait**
  - 任务：在 app-core 中定义 CLI/GUI 可共用的 progress callback 或复用 `ProgressSink`。
  - 完成标准：use case 能把 `TaskProgress` 发给调用方。
  - 验收标准：CLI `--ndjson` 能收到 progress event。

- [ ] **CLI-DEV-0203：修正 ZIP extract 执行语义**
  - 任务：确认 `ZipArchive::extract_to` 的实际文件写入和返回 task plan 语义不会导致重复执行或假进度。
  - 完成标准：extract use case 对 ZIP 只写一次文件；progress 与实际执行一致或明确分阶段。
  - 验收标准：`extract basic.zip --to out` 运行两次时，第二次默认冲突失败且文件未被重复破坏。

- [ ] **CLI-DEV-0204：修正 tar extract 内存边界**
  - 任务：替换 tar extract 中对普通文件 `read_to_end` 的无界读法，改为 bounded stream 写入。
  - 完成标准：tar entry 通过 `StreamPump` 或等价 bounded stream 写出。
  - 验收标准：大 tar entry 解压时内存不会随 entry size 线性增长。

- [ ] **CLI-DEV-0205：实现 entry reader API**
  - 任务：在 `archive-core` 中新增 `EntryReader { entry, access_cost, source, size }` 和 `OpenArchive::open_entry_reader`。
  - 完成标准：ZIP 和 tar 至少实现真实 reader；7z/RAR/libarchive 可先返回 Unsupported 或 ExternalHelper pending。
  - 验收标准：`cat basic.zip docs/readme.txt` 输出真实内容。

- [ ] **CLI-DEV-0206：兼容旧 `open_entry_stream`**
  - 任务：保留 `open_entry_stream` 或用默认实现从 `open_entry_reader` 提取 access_cost。
  - 完成标准：现有 preview plan 编译通过。
  - 验收标准：`cargo test --workspace` 通过。

- [ ] **CLI-DEV-0207：实现 selected entry resolver**
  - 任务：在 app-core 中实现 `resolve_selection(listing, EntrySelection, all_matches) -> Vec<EntryId>`。
  - 完成标准：支持 id、path、glob include/exclude；重复 path 默认失败。
  - 验收标准：覆盖 `CLI-EXTRACT-006` 到 `CLI-EXTRACT-011`。

- [ ] **CLI-DEV-0208：实现 CLI 默认冲突策略**
  - 任务：在 app-core extract 中将 CLI 默认策略设为 preflight conflict fail，不传 `AskBatch` 进入写入阶段。
  - 完成标准：未指定覆盖策略时，发现 conflicts 直接返回结构化错误。
  - 验收标准：`CLI-EXTRACT-012` 通过。

- [ ] **CLI-DEV-0209：实现 partial success summary**
  - 任务：定义并返回 skipped、blocked、failed、processed entries 的 `TaskSummary`。
  - 完成标准：skip existing、blocked entry、partial failure 都能反映在 summary。
  - 验收标准：`CLI-EXTRACT-029` 通过。

- [ ] **CLI-DEV-0210：完善错误到退出码映射**
  - 任务：在 app-core 或 cli 中实现 `ArchiveErrorKind -> ExitCode` 映射。
  - 完成标准：映射与 `CLI_DESIGN.md` 退出码表一致。
  - 验收标准：`CLI-ERR-001` 到 `CLI-ERR-017` 全部通过。

## 6. 阶段 3：新增 CLI crate

- [ ] **CLI-DEV-0301：新增 `crates/cli` crate**
  - 任务：创建 `crates/cli/Cargo.toml` 和 `crates/cli/src/main.rs`。
  - 完成标准：package 名称为 `shadow-zip-cli`；binary 名称为 `shadow-zip`。
  - 验收标准：`cargo run -p shadow-zip-cli -- --help` 成功。

- [ ] **CLI-DEV-0302：将 cli 加入 workspace**
  - 任务：根 `Cargo.toml` workspace members 加入 `crates/cli`，workspace dependencies 加入 `shadow-zip-cli` 不需要；cli 依赖 `shadow-zip-app-core`。
  - 完成标准：workspace 能构建 CLI。
  - 验收标准：`cargo check -p shadow-zip-cli` 成功。

- [ ] **CLI-DEV-0303：添加 CLI 依赖**
  - 任务：在 `crates/cli/Cargo.toml` 添加 `clap`、`serde_json`、`rpassword`、可选 `globset`、`anstream`。
  - 完成标准：依赖版本固定在 workspace 或 crate dependency 中。
  - 验收标准：`cargo check -p shadow-zip-cli` 成功。

- [ ] **CLI-DEV-0304：定义根 CLI parser**
  - 任务：实现 `Cli` struct，包含 `command` 和全局参数 `--config`、`--locale`、`--password`、`--password-file`、`--password-env`、`--json`、`--ndjson`、`--quiet`、`--verbose`、`--no-progress`、`--no-interaction`、`--color`。
  - 完成标准：`--help` 输出所有全局参数。
  - 验收标准：`CLI-GLOBAL-001`、`CLI-GLOBAL-003` 通过。

- [ ] **CLI-DEV-0305：定义所有一级子命令**
  - 任务：实现 subcommands：`info`、`list`、`tree`、`preflight`、`extract`、`create`、`test`、`cat`、`preview`、`backends`、`helpers`、`config`、`cache`、`recent`、`diagnose`。
  - 完成标准：未实现子命令返回清楚的 `Not implemented` 错误，不能 panic。
  - 验收标准：`shadow-zip --help` 列出所有子命令。

- [ ] **CLI-DEV-0306：实现 CLI bootstrap**
  - 任务：实现配置加载、平台配置加载、locale 覆盖、password source 解析、AppCore 构造。
  - 完成标准：全局参数优先级为 CLI 参数 > 环境变量 > 配置文件 > 默认值。
  - 验收标准：`CLI-GLOBAL-014`、`CLI-CONFIG-009`、`CLI-CONFIG-010` 通过。

- [ ] **CLI-DEV-0307：实现 stdout/stderr 分离**
  - 任务：封装 output writer，主结果写 stdout，进度/warning/人类错误写 stderr。
  - 完成标准：`cat` 二进制输出不混入进度或 warning。
  - 验收标准：`CLI-CAT-002` 通过。

- [ ] **CLI-DEV-0308：实现 JSON renderer**
  - 任务：所有命令成功或失败时，`--json` 输出单个 JSON 对象。
  - 完成标准：顶层字段包含 `schema`、`command`、`ok`。
  - 验收标准：`CLI-GLOBAL-006`、`CLI-GLOBAL-007` 通过。

- [ ] **CLI-DEV-0309：实现 NDJSON renderer**
  - 任务：长任务 `--ndjson` 输出 event stream。
  - 完成标准：包含 `task-started`、`progress`、`task-completed` 或 `task-failed`。
  - 验收标准：`CLI-GLOBAL-008`、`CLI-EXTRACT-027` 通过。

- [ ] **CLI-DEV-0310：实现 exit code handler**
  - 任务：main 返回稳定退出码，不使用 panic 作为用户错误。
  - 完成标准：所有 `ArchiveErrorKind` 都有明确 exit code。
  - 验收标准：`CLI-ERR-001` 到 `CLI-ERR-017` 通过。

## 7. 阶段 4：实现 MVP 命令

- [ ] **CLI-DEV-0401：实现 `info` 命令**
  - 任务：解析 `archive` 参数，调用 `AppCore::inspect`。
  - 完成标准：支持人类可读和 `--json`。
  - 验收标准：`CLI-INFO-001` 到 `CLI-INFO-005` 通过。

- [ ] **CLI-DEV-0402：实现 `backends` 命令**
  - 任务：调用 app-core backend registry，输出所有 backend 名称、formats、capabilities。
  - 完成标准：包含 zip、7z、tar-stream、unrar、libarchive-fallback。
  - 验收标准：`CLI-BACKEND-001` 通过。

- [ ] **CLI-DEV-0403：实现 `helpers` 命令**
  - 任务：调用 `HelperDiscovery`，输出 unrar 和 libarchive 诊断。
  - 完成标准：支持 `--json`；helper 缺失不失败，除非命令执行异常。
  - 验收标准：`CLI-HELPER-001` 到 `CLI-HELPER-003` 通过。

- [ ] **CLI-DEV-0404：实现 `list` 命令参数**
  - 任务：支持 `--query`、`--kind`、`--only-encrypted`、`--only-unsafe`、`--sort`、`--desc`、`--columns`。
  - 完成标准：参数全部映射到 `ListRequest`。
  - 验收标准：`CLI-LIST-003` 到 `CLI-LIST-011` 通过。

- [ ] **CLI-DEV-0405：实现 `list` 输出**
  - 任务：实现表格输出和 JSON 输出。
  - 完成标准：表格包含 id、type、size、packed、method、enc、modified、path；JSON 输出完整 entries。
  - 验收标准：`CLI-LIST-001`、`CLI-LIST-002` 通过。

- [ ] **CLI-DEV-0406：实现 `tree` 命令**
  - 任务：支持 `archive`、`--depth`、`--json`。
  - 完成标准：内部使用 app-core tree use case，不重复构建逻辑。
  - 验收标准：`CLI-TREE-001` 到 `CLI-TREE-005` 通过。

- [ ] **CLI-DEV-0407：实现 `preflight extract` 命令**
  - 任务：支持 `archive`、`--to`、`--entry`、`--id`、`--include`、`--exclude`、`--json`。
  - 完成标准：调用 `AppCore::preflight_extract`，输出 conflicts、blocked_entries、warnings。
  - 验收标准：`CLI-PREFLIGHT-001` 到 `CLI-PREFLIGHT-012` 通过。

- [ ] **CLI-DEV-0408：实现 `extract` 参数**
  - 任务：支持 `--to`、`--entry`、`--id`、`--include`、`--exclude`、`--all-matches`、`--overwrite`、`--skip-existing`、`--rename-existing`、`--keep-newer`、`--symlink-policy`。
  - 完成标准：参数全部映射到 `ExtractRequest` 和 `ExtractOptions`。
  - 验收标准：`CLI-EXTRACT-006` 到 `CLI-EXTRACT-016` 通过。

- [ ] **CLI-DEV-0409：实现 `extract` 执行**
  - 任务：调用 `AppCore::extract`，执行任务到完成。
  - 完成标准：成功写出文件；失败时不留下危险路径写入。
  - 验收标准：`CLI-EXTRACT-001` 到 `CLI-EXTRACT-005`、`CLI-EXTRACT-017` 通过。

- [ ] **CLI-DEV-0410：实现 `create` 参数**
  - 任务：支持 output、inputs、`--format`、`--method`、`--level`、`--solid`、`--no-solid`、`--password-*`、`--encrypt-file-names`、`--volume-size`、`--symlink-policy`、`--archive-path`、`--paths`。
  - 完成标准：参数全部映射到 `CreateRequest`。
  - 验收标准：`CLI-CREATE-020` 到 `CLI-CREATE-023` 通过。

- [ ] **CLI-DEV-0411：实现 `create` 执行**
  - 任务：调用 `AppCore::create`，创建归档并返回 summary。
  - 完成标准：ZIP、tar、tar.gz、tar.xz、tar.zst 创建后能被 `list`、`test`、`extract` 验证。
  - 验收标准：`CLI-CREATE-001` 到 `CLI-CREATE-007`、`CLI-CREATE-025` 通过。

- [ ] **CLI-DEV-0412：实现 `test` 命令**
  - 任务：支持 archive、password source、JSON、NDJSON。
  - 完成标准：调用 `AppCore::test` 并执行到完成。
  - 验收标准：`CLI-TEST-001`、`CLI-TEST-002`、`CLI-TEST-005`、`CLI-TEST-008` 通过。

## 8. 阶段 5：实现高级命令

- [ ] **CLI-DEV-0501：实现 `cat` 命令**
  - 任务：支持 archive、entry path、`--id`、password source。
  - 完成标准：stdout 输出 entry 原始 bytes；stderr 不混入主数据。
  - 验收标准：`CLI-CAT-001` 到 `CLI-CAT-009` 通过。

- [ ] **CLI-DEV-0502：实现 `preview metadata`**
  - 任务：支持 `preview <archive> <entry> --mode metadata --json`。
  - 完成标准：返回 `PreviewResult::Metadata`。
  - 验收标准：`CLI-PREVIEW-001` 通过。

- [ ] **CLI-DEV-0503：实现 `preview text`**
  - 任务：支持 `--mode text`，文本输出到 stdout 或 JSON。
  - 完成标准：支持编码检测和截断标记。
  - 验收标准：`CLI-PREVIEW-002` 到 `CLI-PREVIEW-004` 通过。

- [ ] **CLI-DEV-0504：实现 `preview thumbnail/fit/full`**
  - 任务：支持 `--mode thumbnail|fit|full`、`--width`、`--height`、`--output`。
  - 完成标准：bitmap 模式必须要求 `--output`；输出图片可被解码。
  - 验收标准：`CLI-PREVIEW-005` 到 `CLI-PREVIEW-009` 通过。

- [ ] **CLI-DEV-0505：实现 `preview external`**
  - 任务：支持 `--mode external --json`。
  - 完成标准：返回 temp file 需求或 external preview metadata，不直接打开 GUI 程序。
  - 验收标准：`CLI-PREVIEW-010` 通过。

- [ ] **CLI-DEV-0506：实现 preview access cost warning**
  - 任务：对 tar、solid 7z、external helper access cost 输出 warning。
  - 完成标准：warning 在 JSON 和人类输出中都可见。
  - 验收标准：`CLI-PREVIEW-011`、`CLI-PREVIEW-012` 通过。

- [ ] **CLI-DEV-0507：实现 `diagnose` 命令**
  - 任务：支持 archive、`--json`、`--verbose`。
  - 完成标准：输出每个 backend 的 probe/open 结果、helper 状态、失败 causes。
  - 验收标准：`CLI-DIAG-001` 到 `CLI-DIAG-006` 通过。

- [ ] **CLI-DEV-0508：实现 `config path/get/set`**
  - 任务：支持 `config path`、`config get [key]`、`config set <key> <value>`。
  - 完成标准：支持 dotted key；未知 key 和类型错误返回参数错误；写入不破坏原文件。
  - 验收标准：`CLI-CONFIG-001` 到 `CLI-CONFIG-010` 通过。

- [ ] **CLI-DEV-0509：实现 `cache status/cleanup`**
  - 任务：支持 `cache status`、`cache cleanup`、`cache cleanup --dry-run`。
  - 完成标准：使用 app-core/cache service，不直接删除未知目录。
  - 验收标准：`CLI-CACHE-001` 到 `CLI-CACHE-008` 通过。

- [ ] **CLI-DEV-0510：实现 `recent list/clear`**
  - 任务：支持 `recent list`、`recent clear`。
  - 完成标准：使用 app-core recent store，遵守 `RecentFilesConfig`。
  - 验收标准：`CLI-RECENT-001` 到 `CLI-RECENT-006` 通过。

## 9. 阶段 6：密码、安全和隐私

- [ ] **CLI-DEV-0601：实现 password source resolver**
  - 任务：实现 `--password`、`--password-file`、`--password-env`、TTY prompt。
  - 完成标准：优先级为 `--password` > `--password-file` > `--password-env` > prompt。
  - 验收标准：`CLI-EXTRACT-020` 到 `CLI-EXTRACT-023` 通过。

- [ ] **CLI-DEV-0602：实现 `--no-interaction`**
  - 任务：非交互模式禁止 password prompt 和任何 confirm prompt。
  - 完成标准：缺密码时返回 `PasswordRequired`，不阻塞。
  - 验收标准：`CLI-GLOBAL-013` 通过。

- [ ] **CLI-DEV-0603：实现输出 redaction**
  - 任务：所有错误、diagnose、helper args、JSON/NDJSON 输出都使用 `RedactionPolicy`。
  - 完成标准：明文密码不出现在任何输出。
  - 验收标准：`CLI-SEC-003` 到 `CLI-SEC-006` 通过。

- [ ] **CLI-DEV-0604：实现危险路径硬阻断**
  - 任务：extract/cat/preview 对 unsafe entry 使用统一安全策略。
  - 完成标准：默认阻止 ParentTraversal、AbsolutePath、WindowsDrivePath、UncPath、DevicePath。
  - 验收标准：`CLI-PREFLIGHT-007` 到 `CLI-PREFLIGHT-011`、`CLI-SEC-001` 通过。

- [ ] **CLI-DEV-0605：实现 security policy config 覆盖**
  - 任务：从 AppConfig 读取 max_entries、max_total、max_single、max_ratio、max_depth、max_path、block_recursive_archives。
  - 完成标准：preflight 根据配置产生 warning/block。
  - 验收标准：`CLI-PREFLIGHT-013` 到 `CLI-PREFLIGHT-018` 通过。

- [ ] **CLI-DEV-0606：实现 symlink policy**
  - 任务：extract/create 支持 conservative、preserve-links、follow-within-destination。
  - 完成标准：默认 conservative；逃出 destination 的 symlink 永远阻止。
  - 验收标准：`CLI-EXTRACT-018`、`CLI-EXTRACT-019`、`CLI-CREATE-015`、`CLI-CREATE-016` 通过。

- [ ] **CLI-DEV-0607：实现 stream limits**
  - 任务：cat、preview、extract 使用 `StreamLimits` 或 `PreviewLimits`。
  - 完成标准：超限失败并返回结构化错误。
  - 验收标准：`CLI-SEC-008`、`CLI-SEC-009` 通过。

## 10. 阶段 7：fixture 和测试基础设施

- [ ] **CLI-DEV-0701：创建 fixture 目录**
  - 任务：新增 `tests/fixtures/archives`、`tests/fixtures/inputs`、`tests/fixtures/configs`。
  - 完成标准：目录结构与 `CLI_TEST_CASES.md` 第 3 节一致。
  - 验收标准：`Test-Path tests/fixtures/archives` 为 true。

- [ ] **CLI-DEV-0702：实现 fixture 生成器**
  - 任务：新增 `tests/fixtures/generate.rs` 或 `xtask fixtures`。
  - 完成标准：能生成 basic.zip、basic.tar、basic.tar.gz、basic.tar.xz、basic.tar.zst、empty.zip、unsafe-paths.zip、duplicate-paths.zip、unicode.zip、images.zip、text-encodings.zip。
  - 验收标准：从空 fixture 目录运行生成器后，必需 fixture 全部存在。

- [ ] **CLI-DEV-0703：新增 CLI E2E 测试 crate 或 tests**
  - 任务：在 `crates/cli/tests/e2e.rs` 或 workspace `tests/cli_e2e.rs` 中创建测试入口。
  - 完成标准：测试能调用编译后的 `shadow-zip` binary。
  - 验收标准：一个 smoke test `shadow_zip_help_works` 通过。

- [ ] **CLI-DEV-0704：添加测试依赖**
  - 任务：添加 `assert_cmd`、`assert_fs`、`predicates`、`insta`、`tempfile`。
  - 完成标准：依赖只用于 dev-dependencies。
  - 验收标准：`cargo test -p shadow-zip-cli --tests` 编译通过。

- [ ] **CLI-DEV-0705：实现测试 helper**
  - 任务：实现 `shadow_zip()`、`fixture()`、`temp_workspace()`、`parse_json()`、`assert_no_secret()`、`assert_file_hash()`。
  - 完成标准：helper 放在测试公共模块，不复制粘贴。
  - 验收标准：至少 3 个 E2E 测试复用 helper。

- [ ] **CLI-DEV-0706：实现 JSON schema 检查 helper**
  - 任务：实现对 `schema`、`command`、`ok`、`error.kind` 的统一断言。
  - 完成标准：成功和失败 JSON 都有 helper。
  - 验收标准：`CLI-GLOBAL-006`、`CLI-GLOBAL-007` 测试复用该 helper。

- [ ] **CLI-DEV-0707：实现 NDJSON 检查 helper**
  - 任务：实现逐行 parse 和 event type 检查。
  - 完成标准：能断言事件顺序和每行 schema。
  - 验收标准：`CLI-GLOBAL-008` 通过。

## 11. 阶段 8：实现测试套件

- [ ] **CLI-DEV-0801：实现 global 测试**
  - 任务：覆盖 `CLI-GLOBAL-001` 到 `CLI-GLOBAL-015`。
  - 完成标准：每个测试 ID 有对应测试函数或参数化 case。
  - 验收标准：global 测试全部通过。

- [ ] **CLI-DEV-0802：实现 backend/info/list/tree 测试**
  - 任务：覆盖 `CLI-BACKEND-001` 到 `CLI-BACKEND-013`、`CLI-INFO-001` 到 `CLI-INFO-007`、`CLI-LIST-001` 到 `CLI-LIST-017`、`CLI-TREE-001` 到 `CLI-TREE-005`。
  - 完成标准：helper 条件测试在缺 helper 时 skipped，不误报 passed。
  - 验收标准：本组测试全部通过或正确 skipped。

- [ ] **CLI-DEV-0803：实现 preflight/extract 测试**
  - 任务：覆盖 `CLI-PREFLIGHT-001` 到 `CLI-PREFLIGHT-020`、`CLI-EXTRACT-001` 到 `CLI-EXTRACT-030`。
  - 完成标准：所有文件系统副作用在 temp dir 中执行。
  - 验收标准：本组测试全部通过。

- [ ] **CLI-DEV-0804：实现 create/test 测试**
  - 任务：覆盖 `CLI-CREATE-001` 到 `CLI-CREATE-025`、`CLI-TEST-001` 到 `CLI-TEST-008`。
  - 完成标准：create 后必须用 list/test/extract round-trip 验证。
  - 验收标准：本组测试全部通过。

- [ ] **CLI-DEV-0805：实现 cat/preview 测试**
  - 任务：覆盖 `CLI-CAT-001` 到 `CLI-CAT-009`、`CLI-PREVIEW-001` 到 `CLI-PREVIEW-012`。
  - 完成标准：二进制 stdout 测试检查 hash，不用字符串比较。
  - 验收标准：本组测试全部通过。

- [ ] **CLI-DEV-0806：实现 helpers/diagnose 测试**
  - 任务：覆盖 `CLI-HELPER-001` 到 `CLI-HELPER-005`、`CLI-DIAG-001` 到 `CLI-DIAG-006`。
  - 完成标准：fake helper 用临时可执行脚本或测试 binary，不依赖开发者机器状态。
  - 验收标准：本组测试全部通过或条件 skipped。

- [ ] **CLI-DEV-0807：实现 config/cache/recent 测试**
  - 任务：覆盖 `CLI-CONFIG-001` 到 `CLI-CONFIG-010`、`CLI-CACHE-001` 到 `CLI-CACHE-008`、`CLI-RECENT-001` 到 `CLI-RECENT-006`。
  - 完成标准：每个测试使用独立配置文件和 cache root。
  - 验收标准：测试不会修改真实用户配置。

- [ ] **CLI-DEV-0808：实现 task/error/security/platform 测试**
  - 任务：覆盖 `CLI-TASK-001` 到 `CLI-TASK-009`、`CLI-ERR-001` 到 `CLI-ERR-017`、`CLI-SEC-001` 到 `CLI-SEC-010`、`CLI-PLAT-001` 到 `CLI-PLAT-008`。
  - 完成标准：平台特定测试用 `cfg` 或运行时 skip 明确处理。
  - 验收标准：Windows、macOS、Linux 上不存在误失败。

## 12. 阶段 9：未来桌面端薄适配验证

- [ ] **CLI-DEV-0901：定义桌面端 fake use cases**
  - 任务：未来 Flutter 桌面端落地后，为 UI adapter 测试定义 fake app-core adapter。
  - 完成标准：fake 返回固定 session、error、preflight、task id。
  - 验收标准：桌面端测试不需要真实归档文件。

- [ ] **CLI-DEV-0902：测试打开归档 UI 适配**
  - 任务：验证 open 成功后桌面端 session、tree、status 更新。
  - 完成标准：不测试 backend，只测试 response 到 UI state 的映射。
  - 验收标准：桌面端测试通过。

- [ ] **CLI-DEV-0903：测试错误 overlay**
  - 任务：fake 返回 `ArchiveError`，验证 error overlay 显示。
  - 完成标准：错误 title/message/detail 来自 `ErrorPresentation`。
  - 验收标准：桌面端测试通过。

- [ ] **CLI-DEV-0904：测试解压任务按钮适配**
  - 任务：点击或直接调用 extract action，验证发出正确 request 并显示 task id。
  - 完成标准：桌面端不自行做 preflight 或 backend 选择。
  - 验收标准：代码搜索确认桌面端无核心业务分支。

- [ ] **CLI-DEV-0905：测试创建/设置/helper overlay 映射**
  - 任务：验证 create draft、settings、helper diagnostics 的状态展示。
  - 完成标准：overlay 数据来自 app-core/domain model。
  - 验收标准：桌面端 smoke tests 通过。

## 13. 阶段 10：CI 与发布门禁

- [ ] **CLI-DEV-1001：新增 CI job：workspace tests**
  - 任务：CI 中运行 `cargo test --workspace`。
  - 完成标准：PR 必跑。
  - 验收标准：CI 配置文件中存在该 job。

- [ ] **CLI-DEV-1002：新增 CI job：CLI 必跑 E2E**
  - 任务：CI 中运行 CLI 必跑套件。
  - 完成标准：覆盖 `CLI_TEST_CASES.md` 第 25.1 节。
  - 验收标准：PR 必跑且失败阻止合并。

- [ ] **CLI-DEV-1003：新增 CI job：fixture generation check**
  - 任务：CI 从空目录重新生成 fixture，并运行 smoke tests。
  - 完成标准：fixture 生成可重复。
  - 验收标准：删除 fixture 后生成器能恢复必需 fixture。

- [ ] **CLI-DEV-1004：新增 CI job：format/lint**
  - 任务：运行 `cargo fmt --check` 和 `cargo clippy --workspace --all-targets`。
  - 完成标准：无 clippy deny 违规。
  - 验收标准：CI 中格式或 lint 失败会阻止合并。

- [ ] **CLI-DEV-1005：新增夜间 E2E 套件**
  - 任务：配置 scheduled/nightly 运行完整 CLI E2E，包括 helper 条件测试。
  - 完成标准：夜间报告区分 passed、failed、skipped、pending。
  - 验收标准：nightly workflow 可手动触发。

- [ ] **CLI-DEV-1006：定义发布前命令**
  - 任务：在 README 或 release checklist 中写明发布前命令。
  - 完成标准：至少包含 workspace tests 和 CLI full E2E；未来桌面端落地后增加 desktop smoke。
  - 验收标准：发布流程文档可直接复制执行。

## 14. 阶段 11：文档与维护

- [ ] **CLI-DEV-1101：更新 README**
  - 任务：README 添加 CLI 简介和基础命令示例。
  - 完成标准：包含 info/list/extract/create/test 示例。
  - 验收标准：新用户能按 README 运行一个基本 CLI 流程。

- [ ] **CLI-DEV-1102：新增 CLI 用户文档**
  - 任务：新增 `CLI.md` 或 README CLI 章节，说明所有命令、参数、退出码。
  - 完成标准：文档与 `--help` 保持一致。
  - 验收标准：抽查任意命令，文档参数和 `--help` 一致。

- [ ] **CLI-DEV-1103：新增 JSON schema 文档**
  - 任务：记录 result schema 和 event schema。
  - 完成标准：包含成功、错误、progress、task completed 示例。
  - 验收标准：schema 示例能被测试中的 JSON parser 解析。

- [ ] **CLI-DEV-1104：维护测试用例状态**
  - 任务：在 `CLI_TEST_CASES.md` 或单独 tracking 文件中标记 implemented/pending/skipped。
  - 完成标准：每个测试 ID 都能映射到测试函数或 pending 原因。
  - 验收标准：没有“已发布功能但测试 pending”的条目。

- [ ] **CLI-DEV-1105：记录已知限制**
  - 任务：文档明确哪些功能依赖 helper、哪些后端仍是 skeleton、哪些用例 pending。
  - 完成标准：限制不隐藏在测试 skip 中。
  - 验收标准：发布说明包含 CLI 已知限制。

## 15. MVP 最小可交付清单

MVP 可以只完成以下任务，但必须全部通过验收：

- [ ] CLI-DEV-0101 到 CLI-DEV-0117
- [ ] CLI-DEV-0201 到 CLI-DEV-0210
- [ ] CLI-DEV-0301 到 CLI-DEV-0310
- [ ] CLI-DEV-0401 到 CLI-DEV-0412
- [ ] CLI-DEV-0601 到 CLI-DEV-0605
- [ ] CLI-DEV-0701 到 CLI-DEV-0707
- [ ] CLI-DEV-0801 到 CLI-DEV-0804
- [ ] CLI-DEV-0901 到 CLI-DEV-0904
- [ ] CLI-DEV-1001 到 CLI-DEV-1004

MVP 发布前必须通过：

- [ ] `cargo check --workspace`
- [ ] `cargo test --workspace`
- [ ] CLI global/backend/info/list/tree/preflight/extract/create/test/error/security 必跑用例
- [ ] Future desktop smoke tests

## 16. 完整版交付清单

完整版必须额外完成：

- [ ] CLI-DEV-0501 到 CLI-DEV-0510
- [ ] CLI-DEV-0606 到 CLI-DEV-0607
- [ ] CLI-DEV-0805 到 CLI-DEV-0808
- [ ] CLI-DEV-0905
- [ ] CLI-DEV-1005 到 CLI-DEV-1006
- [ ] CLI-DEV-1101 到 CLI-DEV-1105

完整版发布前必须通过：

- [ ] `cargo test --workspace`
- [ ] CLI full E2E fixture suite
- [ ] helper 条件套件在可用环境通过，不可用环境 skipped
- [ ] 桌面端覆盖映射中所有已实现功能对应 CLI 用例全部通过
- [ ] 文档、`--help`、JSON schema 三者一致
