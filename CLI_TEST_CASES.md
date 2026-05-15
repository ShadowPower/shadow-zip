# Shadow Zip CLI 测试用例设计

文档日期：2026-05-14

## 1. 测试目标

本文件基于 `CLI_DESIGN.md`，定义 Shadow Zip CLI 的完整测试用例。目标不是只验证命令行参数，而是把 CLI 作为核心逻辑的 headless 验收入口：CLI 全量 fixture suite 通过时，应能证明 GUI 的绝大多数核心业务逻辑也通过了同一套 app-core、domain、archive-core、preview、task-engine、cache 和 platform 逻辑。

测试覆盖范围包括：

- CLI 全局参数、输出模式、退出码和错误格式。
- app-core use case 合同。
- backend probe、fallback、能力模型和格式支持。
- 归档信息、列表、目录树、过滤、搜索、排序。
- 解压 preflight、安全策略、冲突策略、覆盖策略。
- 解压全部、解压选中、任务执行、进度、取消和摘要。
- 创建归档、输入扫描、压缩选项、路径存储、加密、分卷。
- 测试归档完整性。
- entry stream、`cat`、preview pipeline。
- helper 发现、外部进程、redaction 和诊断。
- 配置、缓存、最近文件和恢复记录。

## 2. 测试分层

### 2.1 CLI E2E 测试

通过编译后的 `shadow-zip` binary 执行真实命令，验证 stdout、stderr、退出码、文件系统副作用和 JSON/NDJSON schema。所有核心 GUI 业务能力都必须至少有一个 CLI E2E 用例覆盖。

### 2.2 app-core 合同测试

直接调用 app-core use case，验证 request/response、错误、任务状态和状态转换。该层用于覆盖 CLI 不方便制造的边界，例如取消点、磁盘空间不足、helper 超时、权限错误注入。

### 2.3 crate 级单元测试

验证纯逻辑：

- `domain`: path safety、security scan、filter/sort、config validation、error mapping。
- `archive-core`: backend selection、preflight、SafeWriter、StreamPump、InputScanner。
- `task-engine`: priority、progress aggregation、cancellation、recovery。
- `preview`: metadata/text/image pipeline、limits。
- `cache`: fingerprint、LRU、schema migration、cleanup。
- `platform`: helper discovery、helper runner、redaction。

### 2.4 GUI 薄适配测试

GUI 只验证表现层，不重复归档业务测试：

- 用户事件能生成正确 app-core request。
- app-core response 能更新 `WorkbenchState`。
- 错误、冲突、密码、设置、helper overlay 能显示。
- 渲染在最小窗口尺寸不崩溃。

## 3. Fixture 目录

建议在实现 CLI 后新增：

```text
tests/fixtures/
  archives/
    basic.zip
    basic.tar
    basic.tar.gz
    basic.tar.xz
    basic.tar.zst
    basic.7z
    solid.7z
    encrypted.zip
    encrypted.7z
    header-encrypted.7z
    duplicate-paths.zip
    unicode.zip
    unsafe-paths.zip
    symlinks.tar
    nested-archive.zip
    corrupt.zip
    unsupported.bin
    empty.zip
    many-small-files.zip
    images.zip
    text-encodings.zip
    volume.7z.001
    sample.rar
  inputs/
    create-src/
    create-src-symlink/
    create-src-unicode/
    binary.bin
    image.png
    readme.txt
  configs/
    minimal.json
    custom-cache.json
    invalid.json
```

最小 `basic.*` 内容应一致：

```text
docs/
docs/readme.txt          "hello shadow zip\n"
docs/manual.md           "# Manual\n"
images/
images/pixel.png         1x1 PNG
bin/data.bin             固定 256 bytes
empty-dir/
```

安全 fixture 内容：

```text
../escape.txt
/absolute.txt
C:/drive.txt
//server/share.txt
\\?\C:\device.txt
very/deep/.../file.txt
huge-ratio.bin
nested.zip
```

所有 fixture 必须可重复生成，建议新增 `tests/fixtures/generate.rs` 或 `xtask fixtures`。二进制 fixture 可提交到仓库，但必须小于测试预算。

## 4. 通用断言

每个 CLI E2E 用例默认断言：

- 命令退出码符合退出码表。
- stdout 只包含主结果。
- stderr 只包含进度、warning、人类可读错误。
- `--json` 输出单个合法 JSON 对象。
- `--ndjson` 每行都是合法 JSON event。
- JSON 顶层包含 `schema`、`command`、`ok`。
- 错误 JSON 包含 `error.kind`、`error.message`。
- 明文密码不得出现在 stdout、stderr、JSON、NDJSON 或临时日志文件。
- 使用 `--no-progress` 时 stderr 不出现进度行。
- 使用 `--quiet` 时只输出必要结果。

## 5. 全局命令与输出模式

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-GLOBAL-001 | 根帮助 | `shadow-zip --help` | exit 0；包含所有一级子命令；不访问配置和后端 | clap/命令注册 |
| CLI-GLOBAL-002 | 子命令帮助 | `shadow-zip extract --help` | exit 0；包含冲突、密码、安全参数 | 子命令参数契约 |
| CLI-GLOBAL-003 | 版本 | `shadow-zip --version` | exit 0；输出版本和构建目标 | binary metadata |
| CLI-GLOBAL-004 | 未知命令 | `shadow-zip nope` | exit 2；stderr 有用法；stdout 为空 | 参数错误退出码 |
| CLI-GLOBAL-005 | 参数缺失 | `shadow-zip extract basic.zip` | exit 2；提示缺少 `--to` | required arg |
| CLI-GLOBAL-006 | JSON 成功格式 | `shadow-zip info basic.zip --json` | exit 0；stdout 单个 JSON；stderr 无主结果 | JSON renderer |
| CLI-GLOBAL-007 | JSON 错误格式 | `shadow-zip info unsupported.bin --json` | 非 0；stdout 错误 JSON；stderr 无技术详情泄漏 | ErrorPresentation |
| CLI-GLOBAL-008 | NDJSON 长任务 | `shadow-zip extract basic.zip --to out --ndjson` | 多行 event；有 start/progress/completed；每行独立可解析 | ProgressSink |
| CLI-GLOBAL-009 | 禁用进度 | `shadow-zip extract basic.zip --to out --no-progress` | stderr 不含 progress；文件仍写出 | progress adapter |
| CLI-GLOBAL-010 | quiet | `shadow-zip test basic.zip --quiet` | 成功时 stdout/stderr 最小化 | output policy |
| CLI-GLOBAL-011 | verbose | `shadow-zip diagnose corrupt.zip --verbose` | 包含 backend、causes、technical detail | diagnostics verbosity |
| CLI-GLOBAL-012 | color never | `shadow-zip list basic.zip --color never` | stdout 不含 ANSI escape | color policy |
| CLI-GLOBAL-013 | no interaction | `shadow-zip list encrypted.zip --no-interaction` | exit 5；不阻塞等待输入 | TTY/password policy |
| CLI-GLOBAL-014 | 自定义配置 | `shadow-zip --config configs/minimal.json info basic.zip --json` | 使用配置中的 locale/cache/security 默认值 | config precedence |
| CLI-GLOBAL-015 | locale 覆盖 | `shadow-zip --locale en-US info basic.zip` | 人类可读标签使用英文 | i18n adapter |

## 6. 后端探测与能力模型

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-BACKEND-001 | 列出后端 | `shadow-zip backends --json` | 包含 zip、7z、tar-stream、unrar、libarchive-fallback | backend registration |
| CLI-BACKEND-002 | ZIP 能力 | `shadow-zip info basic.zip --json` | format=Zip；capabilities.extract_all=Full | ZipBackend probe/open |
| CLI-BACKEND-003 | tar 能力 | `shadow-zip info basic.tar --json` | format=Tar；random_access=Unsupported/Limited | TarBackend probe/open |
| CLI-BACKEND-004 | tar.gz 能力 | `shadow-zip info basic.tar.gz --json` | codecs 包含 gzip | tar gzip detect |
| CLI-BACKEND-005 | tar.xz 能力 | `shadow-zip info basic.tar.xz --json` | codecs 包含 xz | tar xz detect |
| CLI-BACKEND-006 | tar.zst 能力 | `shadow-zip info basic.tar.zst --json` | codecs 包含 zstd | tar zstd detect |
| CLI-BACKEND-007 | 7z 能力 | `shadow-zip info basic.7z --json` | format=SevenZip；password_read=Full | SevenZipBackend |
| CLI-BACKEND-008 | solid 7z 能力 | `shadow-zip info solid.7z --json` | is_solid=true；extract_selected=Limited | solid capability downgrade |
| CLI-BACKEND-009 | 7z 分卷识别 | `shadow-zip info volume.7z.001 --json` | is_multi_volume=true | split volume detection |
| CLI-BACKEND-010 | RAR helper 缺失 | `shadow-zip info sample.rar --json` | helper 缺失时 BackendUnavailable 或 fallback 诊断 | RarBackend unavailable |
| CLI-BACKEND-011 | RAR helper 可用 | `shadow-zip helpers --json` + `info sample.rar` | available=true 时 RAR list/test 可运行 | HelperDiscovery |
| CLI-BACKEND-012 | unsupported | `shadow-zip info unsupported.bin --json` | exit 3；UnsupportedFormat | open_best failure |
| CLI-BACKEND-013 | fallback 诊断 | `shadow-zip diagnose unsupported.bin --json` | 包含每个 backend probe result 和失败链 | backend failure chain |

## 7. `info` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-INFO-001 | 基本信息 | `shadow-zip info basic.zip` | 输出 display name、format、entries、capabilities | ArchiveInfo |
| CLI-INFO-002 | JSON schema | `shadow-zip info basic.zip --json` | `schema=shadow-zip.cli.result.v1` | schema stability |
| CLI-INFO-003 | 空归档 | `shadow-zip info empty.zip --json` | entry_count=0 或 listing 空；成功 | empty archive |
| CLI-INFO-004 | unicode 路径 | `shadow-zip info unicode.zip --json` | display_name 不乱码 | path encoding |
| CLI-INFO-005 | 损坏归档 | `shadow-zip info corrupt.zip --json` | exit 7；CorruptArchive | backend error mapping |
| CLI-INFO-006 | 加密 header 缺密码 | `shadow-zip info header-encrypted.7z --json --no-interaction` | exit 5；PasswordRequired | password request |
| CLI-INFO-007 | 加密 header 正确密码 | `shadow-zip info header-encrypted.7z --password-env SZ_PASS --json` | 成功；has_header_encryption=true | OpenOptions.password |

## 8. `list` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-LIST-001 | 默认表格 | `shadow-zip list basic.zip` | 输出 docs/readme.txt、images/pixel.png；列齐全 | listing renderer |
| CLI-LIST-002 | JSON entries | `shadow-zip list basic.zip --json` | entries 数量正确；每项含 id/raw/normalized/display/kind/size/safety | ArchiveEntry |
| CLI-LIST-003 | query | `shadow-zip list basic.zip --query readme --json` | 只包含 readme | EntryFilter.query |
| CLI-LIST-004 | kind=file | `shadow-zip list basic.zip --kind file --json` | 不包含 directory | EntryFilter.kinds |
| CLI-LIST-005 | kind=directory | `shadow-zip list basic.zip --kind directory --json` | 只包含 docs/、images/、empty-dir/ | EntryKind |
| CLI-LIST-006 | only encrypted | `shadow-zip list encrypted.zip --password-env SZ_PASS --only-encrypted --json` | 只返回 encrypted=true entry | encryption filter |
| CLI-LIST-007 | only unsafe | `shadow-zip list unsafe-paths.zip --only-unsafe --json` | 返回 ParentTraversal、AbsolutePath 等条目 | EntrySafety |
| CLI-LIST-008 | sort size asc | `shadow-zip list basic.zip --sort size --json` | entries 按 size 升序 | EntrySort |
| CLI-LIST-009 | sort size desc | `shadow-zip list basic.zip --sort size --desc --json` | entries 按 size 降序 | SortDirection |
| CLI-LIST-010 | sort path | `shadow-zip list basic.zip --sort path --json` | 按 normalized_path 排序 | path sorting |
| CLI-LIST-011 | columns | `shadow-zip list basic.zip --columns id,path,size` | 表格只出现指定列 | output projection |
| CLI-LIST-012 | duplicate paths | `shadow-zip list duplicate-paths.zip --json` | 重复路径有不同 id；不丢条目 | EntryId contract |
| CLI-LIST-013 | unicode | `shadow-zip list unicode.zip --json` | 中文、emoji、空格路径 round-trip | encoding |
| CLI-LIST-014 | tar.gz listing | `shadow-zip list basic.tar.gz --json` | listing complete；method=gzip 或 codec 可见 | streaming listing |
| CLI-LIST-015 | large listing | `shadow-zip list many-small-files.zip --json` | 不超时；内存稳定；entries 数正确 | virtual/large data path |
| CLI-LIST-016 | password missing | `shadow-zip list encrypted.zip --no-interaction --json` | exit 5 | password flow |
| CLI-LIST-017 | password wrong | `shadow-zip list encrypted.zip --password wrong --json` | exit 6；InvalidPassword；不泄漏 wrong | redaction |

## 9. `tree` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-TREE-001 | 默认树 | `shadow-zip tree basic.zip` | 显示 docs、images、empty-dir 层级 | DirectoryTree |
| CLI-TREE-002 | depth | `shadow-zip tree basic.zip --depth 1` | 不显示 docs/readme.txt | depth rendering |
| CLI-TREE-003 | JSON | `shadow-zip tree basic.zip --json` | nodes 包含 `/` 和 children | DirectoryNode |
| CLI-TREE-004 | duplicate paths | `shadow-zip tree duplicate-paths.zip --json` | entry_count 正确；目录不重复异常 | tree aggregation |
| CLI-TREE-005 | unsafe paths | `shadow-zip tree unsafe-paths.zip --json` | unsafe 条目不导致 tree 构建 panic | path safety boundary |

## 10. `preflight extract` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-PREFLIGHT-001 | 干净目标 | `shadow-zip preflight extract basic.zip --to out --json` | `is_clear=true`；blocked/conflicts 为空 | PreflightService |
| CLI-PREFLIGHT-002 | 目标不存在 | 同上，`out` 不存在 | 创建或判定可创建；成功 | destination validation |
| CLI-PREFLIGHT-003 | 目标是文件 | `--to existing-file` | exit 9；PermissionDenied | validate_destination |
| CLI-PREFLIGHT-004 | 目标不可写 | `--to readonly-dir` | exit 9；PermissionDenied | write probe |
| CLI-PREFLIGHT-005 | 冲突检测 | 目标已有 docs/readme.txt | conflicts 含 target/source size | detect_conflicts |
| CLI-PREFLIGHT-006 | 选中条目冲突 | `--entry docs/readme.txt` | 只检测 selected listing | selection preflight |
| CLI-PREFLIGHT-007 | ParentTraversal | `unsafe-paths.zip` | blocked_entries 含 ParentTraversal | classify_entry_path |
| CLI-PREFLIGHT-008 | AbsolutePath | `unsafe-paths.zip` | blocked_entries 含 AbsolutePath | classify_entry_path |
| CLI-PREFLIGHT-009 | WindowsDrivePath | `unsafe-paths.zip` | blocked_entries 含 WindowsDrivePath | classify_entry_path |
| CLI-PREFLIGHT-010 | UncPath | `unsafe-paths.zip` | blocked_entries 含 UncPath | classify_entry_path |
| CLI-PREFLIGHT-011 | DevicePath | `unsafe-paths.zip` | blocked_entries 含 DevicePath | classify_entry_path |
| CLI-PREFLIGHT-012 | PathTooLong | long-path fixture | blocked/warnings 含 path-too-long | security policy |
| CLI-PREFLIGHT-013 | too many entries | security config max_entries=10 + many-small-files.zip | warning/block too-many-entries | scan_listing_security |
| CLI-PREFLIGHT-014 | total too large | low max_total config | too-large-uncompressed | security policy |
| CLI-PREFLIGHT-015 | entry too large | low max_single config | entry-too-large | security policy |
| CLI-PREFLIGHT-016 | compression ratio | huge-ratio fixture | suspicious-compression-ratio | bomb policy |
| CLI-PREFLIGHT-017 | deep directory | deep fixture | directory-too-deep | max_directory_depth |
| CLI-PREFLIGHT-018 | nested archive warning | block_recursive_archives=true | nested-archive warning | recursive archive policy |
| CLI-PREFLIGHT-019 | symlink conservative | `symlinks.tar` | symlink warning/block according to policy | symlink policy |
| CLI-PREFLIGHT-020 | JSON stable | any blocked preflight | schema has destination,total_entries,estimated_bytes,conflicts,blocked_entries,warnings | API schema |

## 11. `extract` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-EXTRACT-001 | 解压全部 ZIP | `shadow-zip extract basic.zip --to out` | 文件全部写出；内容 hash 匹配 | ZipArchive extract_all/SafeWriter |
| CLI-EXTRACT-002 | 解压全部 tar | `shadow-zip extract basic.tar --to out` | 文件全部写出 | TarArchive extract_all |
| CLI-EXTRACT-003 | 解压 tar.gz | `shadow-zip extract basic.tar.gz --to out` | 文件全部写出；progress stage Reading/Writing | streaming decompress |
| CLI-EXTRACT-004 | 解压 tar.xz | `shadow-zip extract basic.tar.xz --to out` | 文件全部写出 | xz path |
| CLI-EXTRACT-005 | 解压 tar.zst | `shadow-zip extract basic.tar.zst --to out` | 文件全部写出 | zstd path |
| CLI-EXTRACT-006 | 选中 id | `shadow-zip extract basic.zip --id 1 --to out` | 只写指定 entry | EntryId selection |
| CLI-EXTRACT-007 | 选中 path | `shadow-zip extract basic.zip --entry docs/readme.txt --to out` | 只写 readme | path selection |
| CLI-EXTRACT-008 | duplicate path 默认失败 | `extract duplicate-paths.zip --entry same.txt --to out` | exit 2 或明确要求 `--all-matches/--id` | ambiguous selection |
| CLI-EXTRACT-009 | duplicate path all matches | `--entry same.txt --all-matches` | 所有同名 entry 按策略处理 | path ambiguity |
| CLI-EXTRACT-010 | include glob | `--include "docs/*.txt"` | 只写 txt | glob include |
| CLI-EXTRACT-011 | exclude glob | `--exclude "*.bin"` | 不写 bin/data.bin | glob exclude |
| CLI-EXTRACT-012 | 默认冲突失败 | 目标已有 readme；`extract basic.zip --to out` | 非 0；不覆盖原文件 | fail-on-conflict |
| CLI-EXTRACT-013 | overwrite | `--overwrite` | 覆盖原文件；内容为归档内容 | OverwritePolicy::Overwrite |
| CLI-EXTRACT-014 | skip existing | `--skip-existing` | 保留原文件；summary.skipped_entries > 0 | OverwritePolicy::Skip |
| CLI-EXTRACT-015 | rename existing | `--rename-existing` | 生成 `readme (1).txt` | OverwritePolicy::Rename |
| CLI-EXTRACT-016 | keep newer | `--keep-newer` | 新目标保留，旧目标覆盖或跳过符合策略 | OverwritePolicy::KeepNewer |
| CLI-EXTRACT-017 | unsafe blocked | `extract unsafe-paths.zip --to out` | exit 11；out 外无写入 | ExtractionGuard/SafeWriter |
| CLI-EXTRACT-018 | symlink conservative | `extract symlinks.tar --to out` | exit 12 或跳过 symlink；文件安全 | SymlinkPolicy::Conservative |
| CLI-EXTRACT-019 | preserve links | `--symlink-policy preserve-links` | 平台支持时创建 link；不逃出 destination | symlink policy |
| CLI-EXTRACT-020 | password env | `extract encrypted.zip --password-env SZ_PASS --to out` | 成功；密码不出现在输出 | password source |
| CLI-EXTRACT-021 | password file | `--password-file pass.txt` | 成功 | password file |
| CLI-EXTRACT-022 | password prompt | TTY 模拟输入 | 成功；输入不 echo | prompt integration |
| CLI-EXTRACT-023 | wrong password | `--password wrong` | exit 6；不写部分文件或按 recovery 清理 | invalid password |
| CLI-EXTRACT-024 | 7z solid selected warning | `extract solid.7z --id 1 --to out --json` | warnings 含 solid-scan | access cost warning |
| CLI-EXTRACT-025 | tar selected warning | `extract basic.tar.gz --id 1 --to out --json` | warnings 含 sequential-access | sequential access warning |
| CLI-EXTRACT-026 | RAR helper missing | `extract sample.rar --to out --json` | exit 13；BackendUnavailable/ExternalHelperFailed | external helper |
| CLI-EXTRACT-027 | NDJSON progress | `extract basic.zip --to out --ndjson` | start/progress/completed；stage 合法 | TaskEngine progress |
| CLI-EXTRACT-028 | cancel | app-core 注入 cancel 或 CLI future `task cancel` | lifecycle Cancelled；partial cleanup 记录 | CancellationToken |
| CLI-EXTRACT-029 | partial success | fixture 含部分 blocked 或 skip | exit 16；summary 统计 skipped/blocked | TaskSummary |
| CLI-EXTRACT-030 | destination traversal regression | unsafe fixture | 断言 out 父目录没有 escape.txt | safe_join |

## 12. `create` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-CREATE-001 | 创建 ZIP | `shadow-zip create out.zip inputs/create-src --format zip` | exit 0；随后 `test out.zip` 成功；`list` 内容正确 | Zip create |
| CLI-CREATE-002 | ZIP deflate level | `--method deflate --level 6` | info/list method 可接受；无错误 | CreateOptions |
| CLI-CREATE-003 | ZIP store | `--method store` | 创建成功；method/store 或能力降级提示 | compression method |
| CLI-CREATE-004 | 创建 tar | `create out.tar src --format tar` | test/list/extract 成功 | Tar create |
| CLI-CREATE-005 | 创建 tar.gz | `--format tar.gz --method gzip` | gzip tar 可 list/extract | flate2 writer |
| CLI-CREATE-006 | 创建 tar.xz | `--format tar.xz --method xz` | xz tar 可 list/extract | xz writer |
| CLI-CREATE-007 | 创建 tar.zst | `--format tar.zst --method zstd` | zstd tar 可 list/extract | zstd writer |
| CLI-CREATE-008 | 创建 7z | `create out.7z src --format 7z` | 后端支持时成功；否则 task plan warning 明确 | 7z create |
| CLI-CREATE-009 | solid 7z | `--format 7z --solid --json` | warnings 含 solid-access-cost | solid create warning |
| CLI-CREATE-010 | 多输入 | `create out.zip a.txt dir` | 两个输入都进入归档 | InputScanner |
| CLI-CREATE-011 | archive path | `create out.zip readme.txt --archive-path docs/readme.txt` | list 中路径为 docs/readme.txt | InputPath.archive_path |
| CLI-CREATE-012 | relative paths | `--paths relative` | 不包含绝对根路径 | PathStorageMode::Relative |
| CLI-CREATE-013 | preserve root | `--paths preserve-root` | 包含根目录名 | PathStorageMode::PreserveRootFolder |
| CLI-CREATE-014 | flatten | `--paths flatten` | 文件在根层；重名处理明确 | PathStorageMode::Flatten |
| CLI-CREATE-015 | symlink conservative | `create out.tar create-src-symlink --symlink-policy conservative` | symlink 被跳过/报错符合策略 | symlink create policy |
| CLI-CREATE-016 | preserve symlink | `--symlink-policy preserve-links` | tar 中 symlink entry 正确或能力 warning | symlink support |
| CLI-CREATE-017 | password zip | `create encrypted.zip src --password-env SZ_PASS` | list 无密码失败；有密码成功 | password_write |
| CLI-CREATE-018 | encrypt file names | `create encrypted.7z src --encrypt-file-names --password-env SZ_PASS` | header encryption 标记或能力 warning | header encryption |
| CLI-CREATE-019 | volume size | `--volume-size 2MiB` | 产生分卷或能力 warning；小于 1MiB 失败 | volume validation |
| CLI-CREATE-020 | 无输入 | `create out.zip` | exit 2 或 ArchiveError Internal；不创建文件 | validation |
| CLI-CREATE-021 | 空密码 | `--password ""` | exit 5；PasswordRequired | CreateArchiveDraft::validate |
| CLI-CREATE-022 | 过小分卷 | `--volume-size 10KiB` | exit 2/1；提示至少 1 MiB | validation |
| CLI-CREATE-023 | 不支持 RAR 创建 | `create out.rar src --format rar --json` | exit 3；UnsupportedFormat；说明 licensing | RarBackend create |
| CLI-CREATE-024 | 输出路径无权限 | output 在不可写目录 | exit 9/15；不留下半文件 | IO/recovery |
| CLI-CREATE-025 | create 后 round-trip | `create out.zip src` -> `extract out.zip` -> diff | 内容完全一致 | end-to-end |

## 13. `test` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-TEST-001 | ZIP 完整性 | `shadow-zip test basic.zip` | exit 0；summary completed | Zip test plan |
| CLI-TEST-002 | tar 完整性 | `test basic.tar.gz --json` | exit 0；stages 包含 StreamTarEntries | Tar test plan |
| CLI-TEST-003 | 7z 完整性 | `test basic.7z --json` | exit 0；stages 包含 ReadSevenZipHeader | 7z test plan |
| CLI-TEST-004 | RAR helper 缺失 | `test sample.rar --json` | exit 13 | helper failure |
| CLI-TEST-005 | 损坏 ZIP | `test corrupt.zip --json` | exit 7；CorruptArchive | corrupt mapping |
| CLI-TEST-006 | 密码缺失 | `test encrypted.zip --no-interaction --json` | exit 5 | TestOptions.password |
| CLI-TEST-007 | 密码正确 | `test encrypted.zip --password-env SZ_PASS` | exit 0 | password test |
| CLI-TEST-008 | NDJSON | `test basic.zip --ndjson` | completed event | progress event |

## 14. `cat` 用例

这些用例依赖真实 entry reader API。

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-CAT-001 | 文本输出 | `shadow-zip cat basic.zip docs/readme.txt` | stdout 精确等于 fixture 内容；stderr 无主结果 | entry reader |
| CLI-CAT-002 | 二进制输出 | `cat basic.zip bin/data.bin > data.bin` | hash 匹配；stdout 不混入进度 | binary stdout |
| CLI-CAT-003 | id 选择 | `cat basic.zip --id 2` | 输出对应 entry | EntryId |
| CLI-CAT-004 | 多匹配路径 | `cat duplicate-paths.zip same.txt` | 失败并要求 id | ambiguous path |
| CLI-CAT-005 | 目录错误 | `cat basic.zip docs/` | exit 2；不能 cat directory | EntryKind validation |
| CLI-CAT-006 | unsafe entry | `cat unsafe-paths.zip ../escape.txt` | exit 11 | safety policy |
| CLI-CAT-007 | 密码 | `cat encrypted.zip secret.txt --password-env SZ_PASS` | 成功；密码不泄漏 | StreamOptions.password |
| CLI-CAT-008 | tar 顺序读取 warning | `cat basic.tar.gz docs/readme.txt --json` | warning/access_cost=SequentialFromStart | access cost |
| CLI-CAT-009 | solid 7z warning | `cat solid.7z image.png --json` | access_cost=SolidBlockScan | access cost |

## 15. `preview` 用例

这些用例覆盖 `shadow-zip-preview` 和 entry reader。

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-PREVIEW-001 | metadata | `preview images.zip images/pixel.png --mode metadata --json` | 返回 file_name、file_size、dimensions 字段 | PreviewMode::Metadata |
| CLI-PREVIEW-002 | text UTF-8 | `preview text-encodings.zip utf8.txt --mode text` | 输出可读文本；encoding=UTF-8 | text decode |
| CLI-PREVIEW-003 | text GBK/legacy | `preview text-encodings.zip legacy.txt --mode text --json` | encoding 有 chardet 结果；不 panic | chardetng |
| CLI-PREVIEW-004 | text 截断 | 大文本 fixture | truncated=true | text_preview_bytes |
| CLI-PREVIEW-005 | thumbnail output | `preview images.zip images/pixel.png --mode thumbnail --output thumb.png` | 生成图片；尺寸不超过 256x256 | image decode/resize |
| CLI-PREVIEW-006 | fit output | `--mode fit --width 1024 --height 768 --output fit.png` | 输出尺寸符合目标 | requested_dimensions |
| CLI-PREVIEW-007 | full | `--mode full --output full.png` | 不超过 limits；内容可解码 | FullResolution |
| CLI-PREVIEW-008 | 大图超限 | huge image fixture + low limits config | exit 1；Image exceeds limit | PreviewLimits |
| CLI-PREVIEW-009 | unsupported file | `preview basic.zip bin/data.bin --mode thumbnail --json` | Unsupported 或 decode error 映射稳定 | preview error |
| CLI-PREVIEW-010 | external | `preview basic.zip docs/readme.txt --mode external --json` | requires_temp_file=true | ExternalPreview |
| CLI-PREVIEW-011 | tar access cost | `preview basic.tar.gz images/pixel.png --mode metadata --json` | warning preview-access-cost | access cost warning |
| CLI-PREVIEW-012 | solid access cost | `preview solid.7z images/pixel.png --mode metadata --json` | warning preview-access-cost | solid scan |

## 16. `helpers` 与 `diagnose` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-HELPER-001 | helper JSON | `shadow-zip helpers --json` | unrar/libarchive 含 configured_path、resolved_path、version、available | HelperDiscovery |
| CLI-HELPER-002 | 配置 helper 路径 | `--config helper-config.json helpers --json` | 使用 configured_path | PlatformConfig |
| CLI-HELPER-003 | 缺失 helper | PATH 清空或配置不存在 | available=false；不 panic | helper unavailable |
| CLI-HELPER-004 | unrar version | helper 可用时 | version 非空首行 | Command output |
| CLI-HELPER-005 | libarchive version | helper 可用时 | version 非空首行 | Command output |
| CLI-DIAG-001 | 正常归档诊断 | `diagnose basic.zip --json` | backend probe zip Strong/Extension；open ok | DiagnoseResult |
| CLI-DIAG-002 | 损坏归档诊断 | `diagnose corrupt.zip --json` | 包含 zip failure cause | error chain |
| CLI-DIAG-003 | unsupported 诊断 | `diagnose unsupported.bin --json` | 每个 backend probe result；最终 UnsupportedFormat | probe all |
| CLI-DIAG-004 | helper 缺失诊断 | `diagnose sample.rar --json` | 建议配置 unrar | suggested action |
| CLI-DIAG-005 | redaction | 构造 helper stderr 含 password=secret | 输出 `<redacted>`，无 secret | RedactionPolicy |
| CLI-DIAG-006 | verbose causes | `diagnose corrupt.zip --verbose` | 人类输出包含 causes 和 technical detail | verbose renderer |

## 17. `config` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-CONFIG-001 | config path | `shadow-zip config path` | 输出默认配置路径 | config locator |
| CLI-CONFIG-002 | get all | `config get --json` | 输出完整 AppConfig，schema_version=1 | AppConfig serde |
| CLI-CONFIG-003 | get key | `config get preview.max_input_bytes` | 输出数值 | dotted key lookup |
| CLI-CONFIG-004 | set key | `config set default_compression_level 7` | 写入后 get=7 | config write |
| CLI-CONFIG-005 | unknown key | `config get nope.key` | exit 2 | key validation |
| CLI-CONFIG-006 | invalid value | `config set default_compression_level abc` | exit 2；配置不变 | type validation |
| CLI-CONFIG-007 | invalid config file | `--config invalid.json config get` | 使用默认或返回 parse error，行为明确 | load_config |
| CLI-CONFIG-008 | backup | `config set ...` | 原文件备份存在或原子写入 | persistence safety |
| CLI-CONFIG-009 | env precedence | `SHADOW_ZIP_CONFIG=custom.json config get` | 使用 env config | precedence |
| CLI-CONFIG-010 | CLI precedence | `--config a.json` with env b.json | 使用 a.json | precedence |

## 18. `cache` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-CACHE-001 | status 空缓存 | `shadow-zip cache status --json` | counts=0 | CacheSummary |
| CLI-CACHE-002 | status 有缓存 | 先运行 preview/list 生成缓存 | index/thumbnail/temp 计数正确 | cache service |
| CLI-CACHE-003 | cleanup dry run | `cache cleanup --dry-run --json` | 输出将清理项；不删除 | cleanup preview |
| CLI-CACHE-004 | cleanup | `cache cleanup --json` | exit 0；缓存目录清空或容量下降 | cleanup_plan |
| CLI-CACHE-005 | schema migration | 写旧 schema index-cache.json | load 时清理或迁移 | CACHE_SCHEMA_VERSION |
| CLI-CACHE-006 | fingerprint changed | 修改归档后查缓存 | 旧 index 不命中 | ArchiveFingerprint |
| CLI-CACHE-007 | thumbnail eviction | 超小 thumbnail capacity | LRU 淘汰旧项 | evict_lru |
| CLI-CACHE-008 | temp stale cleanup | 注册不存在 temp | cleanup 后移除 | cleanup_stale_temp_files |

## 19. `recent` 用例

| ID | 用例 | 命令 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-RECENT-001 | recent 空 | `recent list --json` | 空数组 | recent store |
| CLI-RECENT-002 | 打开后记录 | `info basic.zip` 后 `recent list` | basic.zip 在第一项 | record_recent_file |
| CLI-RECENT-003 | 去重 | 连续打开同一归档 | 只有一项，时间更新 | dedupe |
| CLI-RECENT-004 | max items | 打开超过 max_items | 截断到 max_items | RecentFilesConfig |
| CLI-RECENT-005 | disabled | config recent_files.enabled=false | 不记录 | config policy |
| CLI-RECENT-006 | clear | `recent clear` | 后续 list 为空 | persistence |

## 20. 任务、进度、恢复

| ID | 用例 | 触发 | 断言 | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-TASK-001 | priority | app-core enqueue extract/cache cleanup | UserBlocking 先运行 | TaskEngine priority |
| CLI-TASK-002 | progress aggregation | 大文件 extract --ndjson | progress 频率受 33ms 聚合限制 | ProgressAggregator |
| CLI-TASK-003 | completed | extract 成功 | lifecycle Completed；summary 存在 | TaskState |
| CLI-TASK-004 | failed | corrupt extract | lifecycle Failed；error 保存 | TaskEngine.fail |
| CLI-TASK-005 | cancelled | 注入 cancel | lifecycle Cancelled/Cancelling | CancellationToken |
| CLI-TASK-006 | recovery record | 运行中失败 | recovery_records 包含 plan | TaskRecoveryRecord |
| CLI-TASK-007 | retry | 调用 retry | 新 task id；priority normal | retry logic |
| CLI-TASK-008 | external helper timeout | fake helper sleep | exit 13；子进程被 kill | HelperRunner |
| CLI-TASK-009 | external helper cancel | fake helper + cancel | Cancelled；子进程结束 | HelperRunner cancel |

## 21. 错误与退出码

| ID | 场景 | 命令/触发 | 期望退出码 | 期望错误 |
|---|---|---|---:|---|
| CLI-ERR-001 | 参数错误 | `extract basic.zip` | 2 | 参数解析错误 |
| CLI-ERR-002 | UnsupportedFormat | `info unsupported.bin` | 3 | UnsupportedFormat |
| CLI-ERR-003 | UnsupportedCodec | unsupported 7z codec fixture | 4 | UnsupportedCodec |
| CLI-ERR-004 | UnsupportedFilter | unsupported filter fixture | 4 | UnsupportedFilter |
| CLI-ERR-005 | PasswordRequired | encrypted no password | 5 | PasswordRequired |
| CLI-ERR-006 | InvalidPassword | wrong password | 6 | InvalidPassword |
| CLI-ERR-007 | CorruptArchive | corrupt.zip | 7 | CorruptArchive |
| CLI-ERR-008 | InsufficientDiskSpace | mocked fs2/app-core injection | 8 | InsufficientDiskSpace |
| CLI-ERR-009 | PermissionDenied | unwritable dest | 9 | PermissionDenied |
| CLI-ERR-010 | PathTooLong | long path fixture | 10/11 | PathTooLong |
| CLI-ERR-011 | PathTraversalBlocked | unsafe paths | 11 | PathTraversalBlocked |
| CLI-ERR-012 | SymlinkPolicyBlocked | symlink conservative | 12 | SymlinkPolicyBlocked |
| CLI-ERR-013 | ExternalHelperFailed | fake helper nonzero | 13 | ExternalHelperFailed |
| CLI-ERR-014 | Cancelled | cancel running task | 14 | Cancelled |
| CLI-ERR-015 | Io | missing archive | 15 | Io |
| CLI-ERR-016 | PartialSuccess | mixed skip/fail fixture | 16 | summary.partial |
| CLI-ERR-017 | Internal | injected missing native handler | 1 | Internal |

## 22. 安全与隐私专项

| ID | 用例 | 断言 | 覆盖逻辑 |
|---|---|---|---|
| CLI-SEC-001 | 所有危险路径都不写出目标目录 | out 父目录无新增文件 | `safe_join`、`SafeWriter` |
| CLI-SEC-002 | `--overwrite` 不覆盖目标目录外文件 | 外部 sentinel 文件 hash 不变 | path traversal guard |
| CLI-SEC-003 | 密码不在 stdout | grep secret 无匹配 | redaction |
| CLI-SEC-004 | 密码不在 stderr | grep secret 无匹配 | redaction |
| CLI-SEC-005 | 密码不在 JSON | grep secret 无匹配 | serializer |
| CLI-SEC-006 | helper args redacted | diagnose 不包含 `-psecret` 或 `--password=secret` | RedactionPolicy |
| CLI-SEC-007 | recursive archive warning | nested.zip warning 受 config 控制 | SecurityPolicy |
| CLI-SEC-008 | 大图限制 | preview 不解码超限像素 | ImageSecurityPolicy/PreviewLimits |
| CLI-SEC-009 | stream size limit | cat/extract 超限失败 | StreamLimits |
| CLI-SEC-010 | temp 文件清理 | preview external/cancel 后 temp 策略符合配置 | TempFilePolicy |

## 23. 跨平台用例

| ID | 用例 | Windows | macOS/Linux | 覆盖逻辑 |
|---|---|---|---|---|
| CLI-PLAT-001 | 路径分隔符 | `docs\\readme.txt` 可匹配 normalized path | `docs/readme.txt` | path normalization |
| CLI-PLAT-002 | Windows drive path blocked | 必测 | 可用 fixture 字符串测试 | classify_entry_path |
| CLI-PLAT-003 | UNC path blocked | 必测 | 可用 fixture 字符串测试 | classify_entry_path |
| CLI-PLAT-004 | 权限错误 | readonly dir 行为按平台实现 | chmod readonly | PermissionDenied |
| CLI-PLAT-005 | symlink | 开发者模式/权限不足时跳过或标记 | 正常执行 | symlink policy |
| CLI-PLAT-006 | unicode 文件名 | NTFS | APFS/ext4 | encoding |
| CLI-PLAT-007 | helper discovery | unrar.exe/bsdtar.exe | unrar/bsdtar | which/path |
| CLI-PLAT-008 | no color env | `NO_COLOR`/`SHADOW_ZIP_NO_COLOR` | 同左 | color policy |

## 24. GUI 覆盖映射

| GUI 能力 | 必须通过的 CLI 用例 |
|---|---|
| 打开归档和显示基本信息 | CLI-BACKEND-002..013、CLI-INFO-001..007 |
| 文件列表、搜索、过滤、排序 | CLI-LIST-001..017 |
| 目录树 | CLI-TREE-001..005 |
| 解压对话框 preflight | CLI-PREFLIGHT-001..020 |
| 解压全部/选中 | CLI-EXTRACT-001..030 |
| 冲突面板 | CLI-PREFLIGHT-005、CLI-EXTRACT-012..016 |
| 密码弹窗 | CLI-GLOBAL-013、CLI-EXTRACT-020..023、CLI-TEST-006..007 |
| 创建归档面板 | CLI-CREATE-001..025 |
| 测试归档按钮 | CLI-TEST-001..008 |
| 预览侧栏 | CLI-CAT-001..009、CLI-PREVIEW-001..012 |
| helper 诊断面板 | CLI-HELPER-001..005、CLI-DIAG-001..006 |
| 设置面板 | CLI-CONFIG-001..010 |
| 缓存清理 | CLI-CACHE-001..008 |
| 最近文件 | CLI-RECENT-001..006 |
| 任务中心 | CLI-TASK-001..009 |
| 错误 overlay | CLI-ERR-001..017 |

如果上表某一行的 CLI 用例失败，对应 GUI 核心能力不得发布。GUI 只需要额外证明它把按钮、菜单、快捷键和 overlay 正确接到了同一批 app-core use case。

## 25. CI 套件划分

### 25.1 必跑套件

每次 PR 必跑：

```text
cargo test -p shadow-zip-domain
cargo test -p shadow-zip-archive-core
cargo test -p shadow-zip-preview
cargo test -p shadow-zip-task-engine
cargo test -p shadow-zip-cache
cargo test -p shadow-zip-platform
cargo test -p shadow-zip-app-core
cargo test -p shadow-zip-cli
```

CLI E2E 必跑子集：

```text
CLI-GLOBAL-001..015
CLI-BACKEND-001..009
CLI-INFO-001..005
CLI-LIST-001..015
CLI-TREE-001..005
CLI-PREFLIGHT-001..012
CLI-EXTRACT-001..017
CLI-CREATE-001..007,010,020,021,023,025
CLI-TEST-001,002,005
CLI-ERR-001..017
CLI-SEC-001..006
```

### 25.2 夜间套件

夜间或发布前运行：

```text
CLI-BACKEND-010..013
CLI-INFO-006..007
CLI-LIST-016..017
CLI-PREFLIGHT-013..020
CLI-EXTRACT-018..030
CLI-CREATE-008..019,022,024
CLI-TEST-003..008
CLI-CAT-001..009
CLI-PREVIEW-001..012
CLI-HELPER-001..005
CLI-DIAG-001..006
CLI-CONFIG-001..010
CLI-CACHE-001..008
CLI-RECENT-001..006
CLI-TASK-001..009
CLI-PLAT-001..008
```

### 25.3 外部 helper 条件套件

以下用例只在环境具备 helper 时运行，否则标记 skipped，不标记 passed：

```text
RAR helper: CLI-BACKEND-010..011, CLI-EXTRACT-026, CLI-TEST-004
libarchive helper: CLI-BACKEND-013, CLI-HELPER-005, CLI-DIAG-003
```

## 26. 测试实现建议

Rust 测试工具建议：

```toml
[dev-dependencies]
assert_cmd = "2"
assert_fs = "1"
predicates = "3"
serde_json = "1"
insta = { version = "1", features = ["json"] }
tempfile = "3"
```

测试 helper 建议：

```rust
fn shadow_zip() -> assert_cmd::Command;
fn fixture(path: &str) -> PathBuf;
fn temp_workspace() -> assert_fs::TempDir;
fn assert_json_schema(value: &serde_json::Value, schema: &str);
fn assert_no_secret(output: &[u8], secret: &str);
fn assert_file_hash(path: &Path, expected: &str);
```

每个 E2E 测试应独立创建 temp workspace，不能共享输出目录。涉及配置、缓存、recent 的测试必须设置独立 `SHADOW_ZIP_CONFIG` 和 cache root，避免污染开发者环境。

## 27. 发布验收标准

发布前必须满足：

- 必跑套件 100% 通过。
- 夜间套件中与已实现功能相关的用例 100% 通过。
- helper 条件套件在缺 helper 时 skipped，在有 helper 的发布环境中通过。
- 所有 JSON/NDJSON 输出通过 schema 检查。
- 所有错误退出码稳定。
- 所有安全与隐私专项通过。
- GUI 覆盖映射中对应已实现 GUI 功能的 CLI 用例全部通过。

未实现功能可以暂时标记为 `pending`，但必须保留测试 ID 和验收标准。功能一旦对用户暴露，相关 `pending` 必须转为必跑测试。
