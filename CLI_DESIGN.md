# Shadow Zip CLI 设计文档

文档日期：2026-05-14

## 1. 目标与边界

Shadow Zip 需要以 CLI 和 Rust 核心为稳定基础，同时为未来 Flutter + Rust 桌面端保留复用边界。CLI 不是 TUI，不提供 curses、全屏交互、键盘导航式文件管理器或常驻终端界面。它应像 `git`、`tar`、`7z` 一样，通过子命令、参数、标准输入输出、退出码和机器可读格式完成操作。

CLI 的设计目标是：除纯图形交互能力外，核心功能都应可由命令行触发，并且和未来桌面端共用同一套领域逻辑、后端选择、预检查、安全策略、任务执行、错误模型和配置模型。CLI 不应重新实现一套压缩、解压、预览或安全检查逻辑。

CLI 应覆盖以下能力：

- 打开、探测和查看归档信息。
- 列目录、搜索、过滤、排序和输出条目元数据。
- 解压全部或选中条目。
- 创建归档。
- 测试归档完整性。
- 预检查解压风险和冲突。
- 读取条目内容到 stdout 或文件。
- 生成预览元数据、文本预览或图片预览文件。
- 查询后端能力、外部 helper 状态、配置和诊断。
- 管理缓存、最近文件和任务恢复记录。

CLI 不覆盖以下内容：

- 图形化浏览、目录树拖拽、图片查看器缩放交互、文件对话框。
- 未来 Flutter 桌面端的窗口状态、布局状态、菜单、工具栏和 overlay。
- 平台 shell 扩展安装向导的图形流程。

## 2. 核心原则

### 2.1 CLI 和未来桌面端共用 app-core

当前代码已经将大量稳定能力放在 `crates/domain`、`crates/archive-core`、`crates/preview`、`crates/task-engine`、`crates/cache`、`crates/platform` 中，并由非 UI 编排层 `crates/app-core` 对外提供 use case。当前仓库不再包含旧桌面 app/ui crate；后续 Flutter 桌面端应作为独立适配层接入 `app-core`。

目标结构：

```text
crates/
  cli/          # CLI binary，只负责参数解析、stdout/stderr、退出码
  app-core/     # 应用编排层，CLI 和未来桌面端共用
  desktop/      # 未来 Flutter/Rust 桌面适配层，不承载核心业务
  domain/
  archive-core/
  preview/
  task-engine/
  cache/
  platform/
```

`app-core` 应负责：

- 构造后端列表。
- 加载和保存配置。
- 打开归档会话。
- 生成 listing、preflight、extract、create、test、preview、cache cleanup 等操作。
- 调用任务引擎并执行任务。
- 聚合进度、警告、诊断和错误。

桌面端和 CLI 的差异只应体现在适配层：

- GUI 将领域状态映射为窗口、面板、列表、overlay 和按钮。
- CLI 将领域状态映射为文本、JSON、NDJSON、退出码和 stderr。

必须建立一个硬性边界：桌面端不能直接调用具体 archive backend、preflight、task runtime 或 preview processor 来做业务决策；CLI 也不能直接绕过 app-core 拼装后端调用。桌面端和 CLI 都只能调用 app-core 暴露的 use case。这样 CLI 集成测试跑通时，测到的不是“另一个命令行实现”，而是未来桌面端实际依赖的同一条核心逻辑链路。

### 2.2 不以文件扩展名硬编码行为

CLI 不应因为参数来自终端就绕过能力模型。所有操作都必须走 `ArchiveService` 的 probe、backend selection 和 `ArchiveCapabilities`。例如 `shadow-zip extract a.tar.gz` 与 GUI 中点击解压应使用同一套 `OpenArchive::extract_all` 或 `extract_selected` 逻辑。

### 2.3 默认安全，显式覆盖

CLI 常用于脚本和批处理，不能弹窗确认。因此危险行为必须通过参数显式表达。

默认策略：

- 解压前运行 preflight。
- 阻止路径穿越、绝对路径、Windows drive path、UNC path、device path。
- 符号链接采用 conservative 策略。
- 覆盖冲突默认失败，而不是询问。
- 密码不写入日志。
- 机器可读输出中不包含明文密码。

用户可以通过参数改变策略，例如 `--overwrite`、`--skip-existing`、`--rename-existing`、`--allow-symlinks`、`--allow-risky-paths`。其中安全降级参数必须在帮助文本中清楚说明风险。

### 2.4 可脚本化优先

CLI 必须保证：

- stdout 用于主结果。
- stderr 用于进度、警告和人类可读错误。
- 退出码稳定。
- JSON 输出 schema 稳定并带版本。
- 支持 `--quiet`、`--verbose`、`--json`、`--ndjson`。
- 支持无交互模式，缺少必要信息时返回错误。

### 2.5 CLI 作为核心逻辑验收入口

CLI 应被设计成核心逻辑的 headless 验收入口。也就是说，CLI 测通应能证明以下 GUI 核心能力已经被覆盖：

- 后端探测和 fallback。
- 归档打开、listing、目录树构建、过滤、排序、搜索。
- 解压 preflight、安全阻断、冲突检测、覆盖策略。
- 解压全部、解压选中、任务计划、任务执行、进度聚合。
- 创建归档的输入扫描、profile 默认值、格式能力选择。
- 测试归档、密码处理、错误映射。
- 预览 pipeline、访问成本降级、资源限制。
- helper 发现、配置加载、缓存清理和诊断输出。

为实现这一点，CLI 测试不应只验证参数解析或输出格式。关键集成测试必须通过编译后的 `shadow-zip` binary 触发 app-core，并使用真实 fixture 文件执行完整 use case。除输出适配外，不允许在 CLI 测试路径上替换掉 GUI 也会使用的核心服务。

### 2.6 GUI 只保留表现逻辑

GUI 中允许存在的逻辑：

- 将用户事件转换为 app-core request。
- 将 app-core response 转换为 `WorkbenchState`。
- 控制窗口、菜单、文件对话框、拖拽、selection highlight、overlay。
- 做轻量展示格式化，例如列宽、图标、颜色和本地化文本。

GUI 中不应存在的逻辑：

- 根据扩展名选择后端。
- 自己判断某格式是否支持解压、创建、预览或密码。
- 自己实现路径安全检查、冲突策略、密码重试、helper fallback。
- 自己扫描输入目录或生成压缩任务。
- 自己解释 task plan 或维护另一套错误分类。

如果未来发现某个 GUI 行为无法用 CLI 或 app-core 测试覆盖，优先调整 app-core use case，而不是在 GUI 中补业务分支。

## 3. Binary 与命名

推荐新增 binary crate：

```text
crates/cli
```

Cargo package 建议命名为：

```toml
name = "shadow-zip-cli"
```

安装后的可执行文件建议为：

```text
shadow-zip
```

如果未来桌面端也需要命令入口，则发布包中可以采用：

- `shadow-zip`：CLI。
- 桌面快捷方式或单独的 Flutter 应用入口：图形界面。

原因是 CLI 更适合作为 PATH 中的稳定命令，GUI 启动通常由桌面入口、文件关联或 `shadow-zip open --gui` 完成。

## 4. 命令总览

```text
shadow-zip info <archive>
shadow-zip list <archive>
shadow-zip tree <archive>
shadow-zip extract <archive> --to <dir>
shadow-zip extract <archive> --entry <path-or-id> --to <dir>
shadow-zip create <output> <input>...
shadow-zip test <archive>
shadow-zip preflight extract <archive> --to <dir>
shadow-zip cat <archive> <entry>
shadow-zip preview <archive> <entry>
shadow-zip backends
shadow-zip helpers
shadow-zip config get [key]
shadow-zip config set <key> <value>
shadow-zip cache status
shadow-zip cache cleanup
shadow-zip recent list
shadow-zip diagnose <archive>
```

全局参数：

```text
--config <file>             指定配置文件
--locale <tag>              覆盖语言，例如 zh-CN、en-US
--password <value>          直接提供密码，不推荐在共享环境使用
--password-file <file>      从文件读取密码
--password-env <name>       从环境变量读取密码
--json                      输出 JSON
--ndjson                    输出换行分隔 JSON 事件流
--quiet                     只输出必要结果
--verbose                   输出更多诊断
--no-progress               禁止进度输出
--color <auto|always|never> 控制颜色
--help
--version
```

密码参数优先级：

```text
--password > --password-file > --password-env > 交互输入
```

在非 TTY 或指定 `--no-interaction` 时，如果需要密码但没有提供，命令必须失败并返回 `PasswordRequired` 对应退出码。

## 5. 输出模式

### 5.1 人类可读输出

默认输出应适合终端阅读。例如 `list` 默认输出表格：

```text
ID    Type  Size       Packed     Method    Enc  Modified              Path
1     dir   -          -          -         no   2026-05-01 10:30      docs/
2     file  12.4 KiB   5.1 KiB    deflate   no   2026-05-01 10:31      docs/readme.txt
```

进度默认写入 stderr：

```text
extract: Writing docs/readme.txt  18.2 MiB / 120.0 MiB  15%
```

### 5.2 JSON 输出

`--json` 输出单个完整 JSON 对象，适合脚本消费：

```json
{
  "schema": "shadow-zip.cli.result.v1",
  "command": "list",
  "ok": true,
  "archive": {
    "path": "example.zip",
    "format": "Zip"
  },
  "entries": []
}
```

### 5.3 NDJSON 事件流

长任务可使用 `--ndjson` 输出事件流：

```json
{"schema":"shadow-zip.cli.event.v1","type":"task-started","task_id":"...","kind":"Extract"}
{"schema":"shadow-zip.cli.event.v1","type":"progress","stage":"Writing","processed_bytes":1048576}
{"schema":"shadow-zip.cli.event.v1","type":"task-completed","task_id":"..."}
```

NDJSON 适合上层应用集成，例如 VS Code 插件、构建脚本、CI 或其它 GUI wrapper。

## 6. 退出码

CLI 应将 `ArchiveErrorKind` 映射为稳定退出码。建议：

| 退出码 | 含义 |
|---:|---|
| 0 | 成功 |
| 1 | 一般错误或未分类内部错误 |
| 2 | 参数错误 |
| 3 | 不支持的格式 |
| 4 | 不支持的 codec/filter |
| 5 | 需要密码 |
| 6 | 密码错误 |
| 7 | 归档损坏 |
| 8 | 磁盘空间不足 |
| 9 | 权限不足 |
| 10 | 路径过长 |
| 11 | 路径安全策略阻止 |
| 12 | 符号链接策略阻止 |
| 13 | 外部 helper 不可用或执行失败 |
| 14 | 任务取消 |
| 15 | I/O 错误 |
| 16 | 部分成功，存在跳过或失败条目 |

JSON 错误对象应包含领域错误：

```json
{
  "schema": "shadow-zip.cli.result.v1",
  "ok": false,
  "error": {
    "kind": "PathTraversalBlocked",
    "message": "Extraction was blocked by safety preflight",
    "technical_detail": null,
    "backend": "zip",
    "archive_path": "bad.zip",
    "entry_path": "../escape.txt"
  }
}
```

## 7. 命令设计

### 7.1 `info`

用途：显示归档基本信息、格式、后端、压缩方法、加密、solid、分卷和能力摘要。

```text
shadow-zip info <archive>
shadow-zip info <archive> --json
```

输出字段应来自 `ArchiveInfo` 和 `ArchiveCapabilities`，不由 CLI 自己推断。

示例：

```text
shadow-zip info release.7z
```

建议 JSON 结构：

```json
{
  "schema": "shadow-zip.cli.result.v1",
  "command": "info",
  "ok": true,
  "info": {
    "format": "SevenZip",
    "display_name": "release.7z",
    "entry_count": 128,
    "is_solid": true,
    "is_encrypted": false,
    "has_header_encryption": false,
    "is_multi_volume": false
  },
  "capabilities": {
    "list": "Full",
    "extract_all": "Full",
    "extract_selected": "High",
    "entry_stream_preview": "Medium"
  }
}
```

### 7.2 `list`

用途：列出归档条目。

```text
shadow-zip list <archive>
shadow-zip list <archive> --query readme
shadow-zip list <archive> --kind file
shadow-zip list <archive> --only-encrypted
shadow-zip list <archive> --only-unsafe
shadow-zip list <archive> --sort size --desc
shadow-zip list <archive> --columns id,path,size,modified
shadow-zip list <archive> --json
```

参数映射：

- `--query` -> `EntryFilter.query`
- `--kind` -> `EntryFilter.kinds`
- `--only-encrypted` -> `EntryFilter.only_encrypted`
- `--only-unsafe` -> `EntryFilter.only_unsafe`
- `--sort` -> `EntrySortColumn`
- `--desc` -> `SortDirection::Descending`

条目选择必须支持两种方式：

- Entry ID：稳定适合脚本。
- 路径或 glob：适合人类使用，但可能匹配多个条目。

路径重复时，CLI 应提示使用 Entry ID，除非用户指定 `--all-matches`。

### 7.3 `tree`

用途：按目录树显示归档。

```text
shadow-zip tree <archive>
shadow-zip tree <archive> --depth 2
shadow-zip tree <archive> --json
```

内部使用 `DirectoryTree::from_listing`。该命令只输出目录结构，不应重新扫描或重新解析路径。

### 7.4 `preflight extract`

用途：只做解压预检查，不执行写入。

```text
shadow-zip preflight extract <archive> --to <dir>
shadow-zip preflight extract <archive> --entry docs/readme.txt --to <dir>
shadow-zip preflight extract <archive> --to <dir> --json
```

输出内容来自 `ExtractPreflight`：

- 目标目录。
- 总条目数。
- 估算写入字节。
- 冲突列表。
- 被安全策略阻止的条目。
- warning 列表。

默认存在 blocked entries 时退出码为 11；存在冲突但未指定冲突策略时退出码为 1 或专用冲突码。为保持简单，可先使用 1，并在 JSON error 中表达 `requires_conflict_resolution`。

### 7.5 `extract`

用途：解压全部或选中条目。

```text
shadow-zip extract <archive> --to <dir>
shadow-zip extract <archive> --to <dir> --entry docs/readme.txt
shadow-zip extract <archive> --to <dir> --id 42
shadow-zip extract <archive> --to <dir> --include "images/*.png"
shadow-zip extract <archive> --to <dir> --exclude "*.tmp"
```

冲突策略：

```text
--overwrite
--skip-existing
--rename-existing
--keep-newer
--fail-on-conflict
```

映射到 `OverwritePolicy`：

- `--overwrite` -> `Overwrite`
- `--skip-existing` -> `Skip`
- `--rename-existing` -> `Rename`
- `--keep-newer` -> `KeepNewer`
- 默认 -> CLI 层使用 fail-on-conflict，不进入 `AskBatch`

符号链接策略：

```text
--symlink-policy conservative
--symlink-policy preserve-links
--symlink-policy follow-within-destination
```

默认值为 `conservative`。

解压流程：

1. 通过 `app-core` 打开归档。
2. 获取 listing。
3. 根据 `--entry`、`--id`、`--include`、`--exclude` 解析选中条目。
4. 运行 `PreflightService::check_listing`。
5. 如果存在安全阻断，失败，除非未来明确实现安全覆盖策略。
6. 如果存在冲突且未指定冲突策略，失败并输出冲突详情。
7. 调用 `OpenArchive::extract_all` 或 `OpenArchive::extract_selected` 生成 `TaskPlan`。
8. 交给 `TaskEngine` 和 runtime executor 执行。
9. 输出进度和最终摘要。

### 7.6 `create`

用途：创建新归档。

```text
shadow-zip create output.zip file1 dir2
shadow-zip create output.7z src --format 7z --method lzma2 --level 7 --solid
shadow-zip create output.tar.zst src --format tar.zst --method zstd --level 3
shadow-zip create output.zip src --password-env ZIP_PASSWORD --encrypt-file-names
```

参数：

```text
--format <zip|7z|tar|tar.gz|tar.xz|tar.zst>
--method <store|deflate|lzma2|zstd|lz4|brotli|xz|gzip>
--level <0-9>
--solid
--no-solid
--password <value>
--password-file <file>
--password-env <name>
--encrypt-file-names
--volume-size <size>
--symlink-policy <policy>
--archive-path <path>       单输入时指定归档内路径
--paths <relative|preserve-root|flatten>
```

输入扫描使用 `InputScanner::scan`。创建选项使用 `CreateArchiveDraft` 和 `CreateOptions`，避免 CLI 维护另一套默认值。

默认值应来自 `AppConfig`：

- 默认格式：`default_create_format`
- 默认压缩级别：`default_compression_level`
- 创建 profile：`creation_profiles`

### 7.7 `test`

用途：测试归档完整性。

```text
shadow-zip test <archive>
shadow-zip test <archive> --password-env ARCHIVE_PASSWORD
shadow-zip test <archive> --json
```

内部调用 `OpenArchive::test(TestOptions)`。输出应包含处理条目数、失败条目数、warning 和后端名称。

### 7.8 `cat`

用途：将单个条目内容写入 stdout，适合脚本管道。

```text
shadow-zip cat <archive> docs/readme.txt
shadow-zip cat <archive> --id 42 > readme.txt
```

约束：

- 只能选择一个 file entry。
- 默认拒绝目录、symlink 和 unsafe entry。
- stdout 是二进制数据，不混入进度。
- 进度和错误只写 stderr。

`cat` 需要后端提供实际 entry stream。当前 `EntryStream` 还只是访问成本模型，后续应在 `archive-core` 中扩展为可读取的受控 stream，或新增 `open_entry_reader` API。

### 7.9 `preview`

用途：生成预览结果。

```text
shadow-zip preview <archive> image.png --mode metadata
shadow-zip preview <archive> image.png --mode text
shadow-zip preview <archive> image.png --mode thumbnail --output thumb.png
shadow-zip preview <archive> image.png --mode fit --width 1024 --height 768 --output preview.png
```

模式映射到 `PreviewMode`：

- `metadata`
- `thumbnail`
- `fit`
- `full`
- `text`
- `external`

设计要求：

- metadata/text 可输出 stdout。
- bitmap 预览默认要求 `--output`，避免把二进制图片混入终端。
- `--json` 时输出 `PreviewResult` 的结构化摘要。
- 大文件和大图限制来自 `PreviewConfig` 和 `PreviewLimits`。

### 7.10 `backends`

用途：显示内置后端和能力。

```text
shadow-zip backends
shadow-zip backends --json
```

输出来自 `ArchiveBackend::backend_capabilities`。

### 7.11 `helpers`

用途：显示外部 helper 检测结果。

```text
shadow-zip helpers
shadow-zip helpers --json
```

内部使用 `HelperDiscovery`，输出 unrar、libarchive 等路径、版本、可用性和支持格式。

### 7.12 `config`

用途：读写配置。

```text
shadow-zip config get
shadow-zip config get preview.max_input_bytes
shadow-zip config set default_create_format Zip
shadow-zip config path
```

配置写入必须：

- 保留 schema version。
- 解析失败时不覆盖原文件。
- 写入前可创建备份，例如 `.bak`。
- 对未知 key 返回参数错误。

### 7.13 `cache`

用途：查看或清理缓存。

```text
shadow-zip cache status
shadow-zip cache cleanup
shadow-zip cache cleanup --dry-run
```

缓存清理应复用 `CacheService::cleanup_plan()`。

### 7.14 `recent`

用途：查看最近文件。

```text
shadow-zip recent list
shadow-zip recent clear
```

最近文件使用 app-core 统一配置和持久化逻辑，不能 CLI 单独维护。

### 7.15 `diagnose`

用途：诊断某个归档为什么无法打开或能力受限。

```text
shadow-zip diagnose <archive>
shadow-zip diagnose <archive> --json
```

诊断应包含：

- 每个 backend 的 probe result。
- open 失败原因链。
- helper 状态。
- capability 降级原因。
- 可能的建议动作。

该命令面向 bug report 和用户自助排错。

## 8. 交互与无交互模式

CLI 可以在 TTY 中进行有限行式交互，例如安全输入密码：

```text
Password for encrypted.7z:
```

但这不是 TUI。它不能进入全屏、不能持久接管终端、不能提供文件列表选择器。

交互规则：

- 默认在 TTY 中可提示密码。
- 非 TTY 中不提示，直接失败。
- `--no-interaction` 禁止所有 prompt。
- 冲突不通过逐项 prompt 解决，必须由参数提供批量策略。
- 安全阻断不通过 prompt 解除。

## 9. 配置与环境变量

建议支持环境变量：

```text
SHADOW_ZIP_CONFIG
SHADOW_ZIP_LOCALE
SHADOW_ZIP_PASSWORD
SHADOW_ZIP_NO_COLOR
SHADOW_ZIP_LOG
SHADOW_ZIP_UNRAR
SHADOW_ZIP_LIBARCHIVE
```

优先级：

```text
命令行参数 > 环境变量 > 配置文件 > 默认值
```

配置文件格式继续使用 `AppConfig` 的 serde JSON 表示。CLI 不定义独立配置 schema。

## 10. 任务执行与进度

CLI 不应只 enqueue 任务后立即退出，除非显式指定后台模式。默认行为是同步执行并等待完成。

建议模式：

```text
--wait              默认，等待任务完成
--background        入队后返回任务 id，未来可用于 daemon 模式
--ndjson            输出进度事件
--no-progress       不显示进度
```

MVP 阶段可以只实现同步执行：

1. app-core 生成 `TaskPlan`。
2. CLI 调用 `TaskEngine::enqueue`。
3. CLI 调用 `TaskEngine::run_next` 或 app-core 封装的 `run_task_to_completion`。
4. `ProgressSink` 根据输出模式写 stderr 或 NDJSON。

后续若实现 daemon，可扩展：

```text
shadow-zip task list
shadow-zip task status <id>
shadow-zip task cancel <id>
shadow-zip task retry <id>
```

但 daemon 不是首版必要条件。

## 11. 安全策略

CLI 的安全策略必须和 GUI 一致，并且在批处理场景更保守。

### 11.1 路径安全

默认阻止：

- `../x`
- `/x`
- `C:/x`
- UNC path
- Windows device path
- 解压后逃出目标目录的 symlink
- 超出平台限制的路径

安全检查来自 `classify_entry_path`、`safe_join`、`PreflightService` 和 `SafeWriter`。

### 11.2 覆盖策略

CLI 不使用 GUI 的 `AskBatch` 作为运行时默认，因为 CLI 无法弹出批量冲突面板。CLI 层应将默认策略解释为：

```text
preflight 发现冲突 -> 失败并输出冲突列表
```

用户必须显式指定覆盖策略。

### 11.3 密码

要求：

- 不在日志、错误链、进度事件和 JSON 输出中打印密码。
- `--password` 帮助文本提示可能进入 shell history。
- 推荐 `--password-env` 或安全 prompt。
- 会话密码记忆只在进程内有效。

### 11.4 外部 helper

外部 helper 调用必须使用 `Command` 参数数组，不拼接 shell 字符串。必须设置：

- 超时。
- 输出大小限制。
- 工作目录。
- 取消时 kill 子进程。
- redacted args。

这部分复用 `HelperRunner` 和 `ExternalHelperPlan`。

## 12. 数据结构与 API 调整建议

### 12.1 新增 `app-core`

建议新增：

```rust
pub struct AppCore {
    // 原 AppController 的非 UI 字段
}

impl AppCore {
    pub fn new(config: AppConfig, platform_config: PlatformConfig) -> Self;
    pub fn open_archive(&self, path: PathBuf, options: OpenOptions) -> Result<ArchiveSessionSnapshot, ArchiveError>;
    pub fn list_entries(&self, request: ListRequest) -> Result<ListResult, ArchiveError>;
    pub fn preflight_extract(&self, request: ExtractRequest) -> Result<ExtractPreflight, ArchiveError>;
    pub fn extract(&self, request: ExtractRequest, sink: &dyn ProgressSink) -> Result<TaskSummary, ArchiveError>;
    pub fn create(&self, request: CreateRequest, sink: &dyn ProgressSink) -> Result<TaskSummary, ArchiveError>;
    pub fn test(&self, request: TestRequest, sink: &dyn ProgressSink) -> Result<TaskSummary, ArchiveError>;
    pub fn preview(&self, request: PreviewCliRequest) -> Result<PreviewResult, ArchiveError>;
}
```

未来桌面端 adapter 和 CLI command handler 都依赖 `AppCore`。

`AppCore` 的方法应当是产品 use case，而不是底层 helper 的薄封装。每个 use case 都应完整包含打开归档、能力判断、选择解析、preflight、任务计划、执行和错误映射中属于该操作的必要步骤。桌面端用户动作和 CLI 子命令都只能进入这些 use case。

### 12.2 共享 Use Case 合同

建议将每个用户可见动作定义为稳定 request/response：

```rust
pub trait ArchiveUseCases {
    fn inspect(&self, request: InspectRequest) -> Result<InspectResult, ArchiveError>;
    fn list(&self, request: ListRequest) -> Result<ListResult, ArchiveError>;
    fn extract(&self, request: ExtractRequest, sink: &dyn ProgressSink) -> Result<TaskSummary, ArchiveError>;
    fn create(&self, request: CreateRequest, sink: &dyn ProgressSink) -> Result<TaskSummary, ArchiveError>;
    fn test(&self, request: TestRequest, sink: &dyn ProgressSink) -> Result<TaskSummary, ArchiveError>;
    fn preview(&self, request: PreviewCliRequest) -> Result<PreviewResult, ArchiveError>;
    fn diagnose(&self, request: DiagnoseRequest) -> Result<DiagnoseResult, ArchiveError>;
}
```

桌面端 adapter 只做：

```text
UI event -> request -> ArchiveUseCases -> response -> desktop state
```

CLI adapter 只做：

```text
argv/env/stdin -> request -> ArchiveUseCases -> stdout/stderr/exit code
```

这个合同是“CLI 测通等于桌面端核心逻辑测通”的关键。如果桌面端需要一个核心动作，但该动作没有对应 use case 或无法通过 CLI/headless test 调用，应视为架构缺口。

### 12.3 请求/响应模型

建议将 CLI 和桌面端都需要的请求模型放入 `app-core` 或 `domain`：

```rust
pub struct ListRequest {
    pub archive: PathBuf,
    pub open_options: OpenOptions,
    pub filter: EntryFilter,
    pub sort: EntrySort,
    pub listing_mode: ListingMode,
}

pub struct ExtractRequest {
    pub archive: PathBuf,
    pub destination: PathBuf,
    pub selection: EntrySelection,
    pub open_options: OpenOptions,
    pub extract_options: ExtractOptions,
    pub require_preflight_clear: bool,
}

pub enum EntrySelection {
    All,
    Ids(Vec<EntryId>),
    Paths(Vec<String>),
    Globs { include: Vec<String>, exclude: Vec<String> },
}
```

### 12.4 Entry stream 扩展

当前 `OpenArchive::open_entry_stream` 返回 `EntryStream`，它表达 access cost，但不包含可读数据源。为了支持 `cat` 和真实 preview，建议调整为：

```rust
pub struct EntryReader {
    pub entry: EntryId,
    pub access_cost: AccessCost,
    pub source: Box<dyn ByteSource>,
    pub size: Option<u64>,
}
```

或者新增方法：

```rust
fn open_entry_reader(
    &mut self,
    entry: EntryId,
    options: StreamOptions,
) -> Result<EntryReader, ArchiveError>;
```

保留原 `open_entry_stream` 可作为轻量 plan/probe API。

### 12.5 Task runtime

当前 `TaskEngine` 能解释 `TaskPlan`，但部分后端仍是 task-plan producer skeleton。CLI 首版可以先执行当前可执行的 plan，并在输出中清楚标记 skeleton 后端的限制。随着后端真实执行能力增强，CLI 不应修改命令契约。

## 13. 参数解析实现建议

Rust CLI 建议使用 `clap` derive：

```rust
#[derive(clap::Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    #[arg(long)]
    json: bool,
    #[arg(long)]
    ndjson: bool,
    #[arg(long)]
    quiet: bool,
    #[arg(long)]
    verbose: bool,
}
```

新增依赖建议：

```toml
clap = { version = "4", features = ["derive"] }
rpassword = "7"
globset = "0.4"
anstream = "0.6"
```

如需保持依赖极简，可以先只引入 `clap` 和 `rpassword`。

## 14. 测试策略

测试策略的核心目标是：CLI 集成测试通过，必须能证明 GUI 的绝大多数核心逻辑也通过。这里的“核心逻辑”指归档能力、数据模型、安全策略、任务执行、配置、缓存、预览、错误处理和 helper 诊断，不包括 Flutter 渲染、鼠标键盘事件、窗口布局和文件对话框。

### 14.1 核心验收原则

CLI 测试要有三条约束：

- CLI 测试必须调用编译后的 `shadow-zip` binary 或 app-core use case，不能只测内部函数。
- 除少数错误注入测试外，核心流程应使用真实小归档 fixture 和真实后端。
- GUI 不能有 CLI 无法触达的核心分支；如果 GUI 需要特殊业务逻辑，应把它下沉到 app-core 并增加 headless 测试。

推荐建立两类测试：

- `app-core` contract tests：直接调用 use case，验证 request/response、状态变化和错误。
- `cli` end-to-end tests：通过命令行触发同一 use case，验证 stdout/stderr/exit code/schema。

### 14.2 GUI 核心覆盖矩阵

| GUI 核心动作 | CLI 覆盖命令 | 共享 app-core 路径 | 覆盖内容 |
|---|---|---|---|
| 打开归档 | `info`、`list` | `inspect`、`list` | probe、backend selection、open options、错误映射 |
| 显示文件列表 | `list` | `list` | listing mode、entry model、大小/时间/加密/安全状态 |
| 目录树 | `tree` | `list` + `DirectoryTree` | normalized path、目录聚合、重复路径处理 |
| 搜索和过滤 | `list --query --kind --only-encrypted --only-unsafe` | `list` | `EntryFilter` 行为 |
| 排序 | `list --sort --desc` | `list` | `EntrySort` 行为 |
| 解压前检查 | `preflight extract` | `preflight_extract` | 目标目录、空间、冲突、危险路径、symlink 策略 |
| 解压全部 | `extract --to` | `extract` | extract plan、task runtime、SafeWriter、进度和摘要 |
| 解压选中 | `extract --id`、`extract --entry` | `extract` | entry selection、重复路径处理、selected extraction |
| 冲突策略 | `extract --overwrite/--skip-existing/--rename-existing/--keep-newer` | `extract` | `OverwritePolicy` 映射与写入行为 |
| 创建归档 | `create` | `create` | input scan、create profile、format capabilities、task plan |
| 测试归档 | `test` | `test` | `TestOptions`、密码、完整性错误 |
| 密码读取 | `info/list/extract/test --password-*` | open/test/extract use case | 密码来源、错误重试、日志脱敏 |
| 条目内容读取 | `cat` | entry reader use case | entry stream、stdout 二进制安全、访问成本 |
| 预览 | `preview` | `preview` | preview pipeline、文本编码、图片限制、访问成本 warning |
| helper 诊断 | `helpers`、`diagnose` | diagnose/helper use case | helper discovery、fallback 提示、redacted args |
| 配置 | `config get/set` | config service | schema version、默认值、配置优先级 |
| 缓存 | `cache status/cleanup` | cache service | cleanup plan、缓存根目录、清理策略 |
| 最近文件 | `recent list/clear` | recent service | recent file persistence、max items |

未来桌面端仍需自己的测试，但范围应很薄：

- 桌面端状态能正确呈现 app-core response。
- 点击按钮能发出正确 request。
- overlay、列表 selection、键盘快捷键、窗口 resize 不崩溃。
- 渲染在最小窗口尺寸下不重叠。

这些桌面端测试不重新验证归档业务正确性。

### 14.3 纯解析测试

验证命令参数解析到 request model：

- `extract --overwrite` 映射到 `OverwritePolicy::Overwrite`
- `list --only-encrypted` 映射到 `EntryFilter.only_encrypted`
- `create --format tar.zst --method zstd` 映射到 `CreateOptions`

### 14.4 app-core 合同测试

使用 tempfile 和真实小归档：

- ZIP list 输出条目，目录、文件、大小、mtime 和加密标记正确。
- tar.gz listing 使用顺序扫描路径，仍产生一致 `ArchiveListing`。
- extract preflight 检测冲突。
- unsafe path 被阻止。
- symlink 默认被 conservative policy 阻止或降级。
- create draft validation 覆盖空输入、空密码、过小分卷。
- helper 缺失返回结构化诊断，不 panic。

### 14.5 CLI 端到端测试

使用 `assert_cmd` 或 cargo integration test：

- stdout JSON 可解析。
- stderr 包含进度但 stdout 不混入进度。
- 错误退出码稳定。
- `cat` 输出二进制内容不被污染。
- `extract` 真实写出文件，并通过文件内容校验。
- `preflight extract --json` 对冲突和 blocked entry 输出稳定 schema。
- `create` 后立刻用 `list` 和 `test` 验证生成归档。
- `--password-env` 成功打开加密 fixture，错误密码返回稳定退出码。

### 14.6 桌面端薄适配测试

桌面端测试应刻意避免重复 app-core 的归档测试。推荐使用 fake `ArchiveUseCases` 返回固定 response，只验证 UI adapter：

- 打开成功后桌面端 session、目录树、状态栏更新。
- app-core 返回 `ArchiveError` 后显示错误 overlay。
- 解压 preflight 返回冲突后显示冲突面板。
- 点击测试归档按钮发出 `TestRequest`。

### 14.7 golden output

人类可读表格可以使用少量 golden snapshot，但不要让列宽变化导致大量脆弱测试。JSON schema 测试更重要。

### 14.8 CI 门禁

CI 至少应包含：

```text
cargo test -p shadow-zip-domain
cargo test -p shadow-zip-archive-core
cargo test -p shadow-zip-preview
cargo test -p shadow-zip-task-engine
cargo test -p shadow-zip-app-core
cargo test -p shadow-zip-cli
```

未来桌面端落地后再运行对应 smoke test。发布前必须运行 CLI end-to-end fixture suite。该 suite 通过时，可以认为桌面端的核心业务链路已经被覆盖；桌面端测试只负责证明表现层没有把这些能力接错。

## 15. 发布与兼容

CLI 是外部契约，一旦发布应尽量保持兼容。

兼容策略：

- 子命令和参数名遵循语义版本。
- JSON schema 使用 `schema` 字段标识版本。
- 新增字段允许，删除或改名需要 major 版本。
- 退出码保持稳定。
- 默认安全策略可以更严格，但不能悄悄变得更宽松。

## 16. 实施路线

### 阶段 1：抽出共享编排层

- 新增 `crates/app-core`。
- 保持核心业务逻辑集中在 `app-core`。
- 未来 Flutter 桌面端只负责 bootstrap、UI 状态和事件适配。
- 桌面端通过稳定 request/response 调用 app-core adapter。

验收标准：

- app-core 可在无 Flutter 环境下测试。
- 仓库不包含旧桌面 UI 框架依赖或专用 crate。
- 未来桌面端不持有后端选择、preflight、extract、create、test、preview 等业务编排。
- 每个桌面端核心动作都能映射到一个 app-core use case。

### 阶段 2：CLI MVP

实现：

- `info`
- `list`
- `preflight extract`
- `extract`
- `create`
- `test`
- `backends`
- `helpers`

支持：

- 人类可读输出。
- `--json`。
- 稳定退出码。
- 密码 env/file/prompt。

验收标准：

- CLI fixture suite 覆盖打开、列表、preflight、解压、创建、测试、helper 诊断。
- GUI 对应动作不包含额外业务分支，只负责把用户事件转换为同一批 request。
- 任一 CLI 核心流程失败时，应视为 GUI 对应核心流程也不可发布。

### 阶段 3：流式与预览

实现：

- `cat`
- `preview metadata`
- `preview text`
- `preview thumbnail --output`

需要先补齐真实 entry reader API。

### 阶段 4：运维与诊断

实现：

- `config`
- `cache`
- `recent`
- `diagnose`
- `--ndjson` progress events

### 阶段 5：长期能力

可考虑：

- task daemon。
- shell context menu 调用 CLI。
- CI 中的 archive validation。
- 更完整的 JSON schema 文档。

## 17. 示例

查看归档信息：

```text
shadow-zip info app.7z
```

列出所有图片：

```text
shadow-zip list photos.zip --query .jpg --sort modified --desc
```

JSON 列目录：

```text
shadow-zip list photos.zip --json
```

解压到目录，遇到冲突自动改名：

```text
shadow-zip extract photos.zip --to ./out --rename-existing
```

只解压一个条目：

```text
shadow-zip extract docs.zip --entry docs/readme.txt --to ./out
```

创建 zip：

```text
shadow-zip create release.zip target/release README.md --format zip --method deflate --level 6
```

创建加密 7z：

```text
shadow-zip create private.7z secrets --format 7z --method lzma2 --solid --password-env SHADOW_ZIP_PASSWORD --encrypt-file-names
```

测试归档：

```text
shadow-zip test release.zip
```

输出文件内容到管道：

```text
shadow-zip cat docs.zip docs/readme.txt | more
```

生成缩略图：

```text
shadow-zip preview photos.zip cover.jpg --mode thumbnail --output cover-thumb.png
```

诊断 helper：

```text
shadow-zip helpers --json
```

## 18. 关键结论

Shadow Zip CLI 的核心不是“给 GUI 补一套命令行”，而是把产品核心能力沉淀为稳定的 app-core，然后让 GUI 和 CLI 成为两个薄适配器。CLI 应成为核心逻辑的 headless 验收入口：当 CLI fixture suite 跑通时，应能证明 GUI 的归档打开、列表、过滤、排序、preflight、解压、创建、测试、预览、helper、配置和缓存等绝大多数业务链路已经通过同一套逻辑验证。

首版应优先实现脚本最需要的能力：`info`、`list`、`preflight extract`、`extract`、`create`、`test`、`backends` 和 `helpers`。随后再补齐 `cat`、`preview`、`diagnose`、`config`、`cache` 等高级命令。

GUI 自身测试仍然必要，但它的职责应收窄为表现层验证：事件是否发出正确 request、response 是否呈现为正确状态、窗口和 overlay 是否正常渲染。归档业务正确性应主要由 app-core contract tests 和 CLI end-to-end tests 承担。
