# 接口与数据模型设计

文档日期：2026-05-07

## 1. 设计意图

本系统的接口设计应服务于长期可维护性。归档工具面对的格式差异极大，如果 UI 直接依赖具体后端，后续会很难处理 fallback、错误映射、平台差异和能力差异。因此，接口层的核心任务是把底层库能力转换成稳定的领域模型，并让 UI 只依据领域模型进行交互决策。

设计时必须避免几个常见错误。第一，不能假设所有格式都能随机访问。tar.gz 和 solid 7z 的访问模型与 ZIP 完全不同。第二，不能假设所有格式都支持更新已有归档。第三，不能假设所有格式都使用相同加密模型。第四，不能把底层错误码直接传给 UI。第五，不能让 FFI 对象跨越后端边界进入业务层。

## 2. 归档会话模型

用户每打开一个归档，系统都应创建一个归档会话。会话持有归档来源、后端实例、listing、能力信息、缓存状态、密码状态和当前任务状态。会话的生命周期由窗口或标签页管理，但会话内部不应持有 UI 对象。

归档来源可以是本地路径、文件句柄、内存映射文件或受限 stream。实际使用时，大部分后端会要求 seek，因此 stream 只能在后端明确支持时启用。会话打开后，系统应产生 `ArchiveInfo` 和 `ArchiveCapabilities`。`ArchiveInfo` 描述归档是什么，`ArchiveCapabilities` 描述它能做什么。

Entry 不应单纯以路径作为唯一标识。归档内可能存在重复路径，路径大小写规则也会因平台不同产生歧义。更稳妥的做法是使用 session 内部稳定的 `EntryId`，并将原始路径、规范化路径、安全状态和显示路径分开保存。

## 3. 能力模型

能力模型是 UI 与后端之间最重要的契约。一个 ZIP 归档和一个 tar.gz 归档都可以“解压”，但它们在列目录、预览单文件、更新归档和随机访问方面完全不同。如果没有能力模型，UI 只能基于格式名做硬编码判断，后续会很难支持 7z + ZSTD、solid 7z、分卷 RAR 或 libarchive fallback。

能力模型应表达列目录、解压全部、解压选中、创建、更新已有归档、近似随机访问、密码读取、密码写入、header encryption、分卷读取、分卷写入和 entry stream 预览等能力。每项能力不应只是布尔值，而应允许 Full、High、Medium、Limited、External 和 Unsupported 等等级。这样 UI 可以区分“完整支持”“可用但较慢”“依赖外部组件”和“不支持”。

## 4. 后端接口

后端接口应分为两个层次。第一层是 `ArchiveBackend`，负责 probe、open、create plan 和后端能力声明。第二层是 `OpenArchive`，表示已经打开的归档实例，负责 listing、extract、open entry stream 和 test。

一个典型后端接口可以表达为：

```rust
pub trait ArchiveBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn probe(&self, source: &ArchiveSource) -> Result<ProbeResult, ArchiveError>;
    fn open(&self, source: ArchiveSource, options: OpenOptions) -> Result<Box<dyn OpenArchive>, ArchiveError>;
    fn create_plan(&self, inputs: &[InputPath], output: &Path, options: CreateOptions) -> Result<TaskPlan, ArchiveError>;
    fn backend_capabilities(&self) -> BackendCapabilities;
}
```

```rust
pub trait OpenArchive: Send {
    fn info(&self) -> ArchiveInfo;
    fn capabilities(&self) -> ArchiveCapabilities;
    fn listing(&mut self, mode: ListingMode) -> Result<ArchiveListing, ArchiveError>;
    fn extract_all(&mut self, destination: &Path, options: ExtractOptions) -> Result<TaskPlan, ArchiveError>;
    fn extract_selected(&mut self, entries: &[EntryId], destination: &Path, options: ExtractOptions) -> Result<TaskPlan, ArchiveError>;
    fn open_entry_stream(&mut self, entry: EntryId, options: StreamOptions) -> Result<EntryStream, ArchiveError>;
    fn test(&mut self, options: TestOptions) -> Result<TaskPlan, ArchiveError>;
}
```

这些接口不要求所有后端都完整实现每个操作。若某项能力受限，后端应通过 capability 和结构化错误表达，而不是在运行时返回含糊失败。

## 5. 任务模型

后端不应直接执行长任务并阻塞调用方。压缩、解压、测试、索引构建和预览都应生成 `TaskPlan`，再交由任务引擎调度。`TaskPlan` 应包含任务类型、估算字节数、估算 entry 数、是否需要密码、是否需要外部 helper、潜在 warning 和实际 executor。

任务进度应统一表示当前阶段、当前路径、已处理字节、总字节、已处理 entry、总 entry、速度、ETA 和 warning。UI 只订阅聚合后的任务状态，不接收后端原始事件流。这样可以避免大量小文件场景下 UI 被事件淹没。

## 6. 预览模型

预览请求应包含归档会话、entry id、目标模式、目标尺寸和优先级。预览结果可以是 metadata、bitmap、文本、外部查看信息或不支持原因。预览服务不应直接知道 ZIP、7z 或 RAR 的内部细节，它只通过 Archive Service 获取 entry stream，并根据 capability 决定是否需要降级或提示等待。

缓存键必须足够稳定。一个缩略图缓存项应至少绑定归档 fingerprint、entry 路径、entry 大小、mtime、预览尺寸、decoder 版本和方向信息。如果只用路径作为 key，归档更新后很容易产生错误缓存。

## 7. 错误模型

错误模型应将底层差异转化为稳定语义。后端可以返回不同的 C errno、7z 状态码、libarchive code 或 unrar code，但进入领域层后必须映射为统一的 `ArchiveErrorKind`。UI 只处理领域错误，并根据错误类型展示密码重试、fallback、空间不足、权限不足、路径风险或后端缺失等操作。

错误对象应包含用户消息、技术详情、后端名称、归档路径、entry 路径和建议动作。对于 fallback 失败，错误链应保留原始后端失败和 fallback 后端失败，便于诊断。

## 8. FFI 边界

C/C++ 后端必须被严格封装。FFI 层负责生命周期、错误码转换、字符串编码、路径编码、资源释放和 panic 隔离。任何 C 指针、后端句柄或不安全资源都不应越过后端 crate 边界。Rust 领域层只接收安全对象、稳定错误和受控 stream。

外部 helper 也应视为一种 FFI 边界。调用时必须使用参数数组，不得拼接 shell 字符串；必须限制工作目录、输出大小和执行时间；取消任务时必须能够终止进程并清理临时文件。

## 9. 配置模型

配置应分为应用配置、压缩默认配置、预览配置、缓存配置、平台配置和外部 helper 配置。配置文件必须包含 schema version，以便后续迁移。涉及安全和隐私的配置，例如密码会话记忆、日志级别、外部 helper 路径和缓存保留策略，应有明确默认值，并在 UI 中以正式文案说明影响。

