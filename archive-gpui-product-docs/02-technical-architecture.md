# 技术架构文档

文档日期：2026-05-07

## 1. 架构总览

本项目应采用分层、分后端、能力驱动的架构。归档格式之间的差异过大，不能以一个“万能归档接口”掩盖底层限制。ZIP 的 central directory 使其非常适合快速列目录和单文件预览；tar.gz、tar.xz、tar.zst 则是顺序流，适合单遍解压和创建，但不天然适合随机浏览；7z 在 non-solid 与 solid 两种情况下具有完全不同的访问成本；RAR 解压可以实现，但创建能力涉及专有授权和外部工具。

因此，推荐架构是：GPUI 负责桌面界面与交互；Rust 领域层负责归档 session、任务、缓存、索引、错误和能力模型；Archive Service 根据格式和能力选择后端；各后端分别适配 minizip-ng、sevenz-rust2、libarchive、Rust tar 流式管线和 UnRAR。UI 不直接理解后端细节，而是依据归档能力模型决定呈现什么操作、提示什么限制、允许什么 fallback。

逻辑结构如下：

```text
GPUI Interface
  -> UI State
  -> Domain State
  -> Task Engine
  -> Archive Service
  -> Preview Service
  -> Cache and Index Service
  -> Platform Integration
  -> Backend Adapters
```

## 2. 模块边界

工程上建议采用 Rust workspace。`app` crate 负责应用入口、依赖组装和配置加载；`ui` crate 封装 GPUI 组件；`domain` crate 定义归档 entry、能力、错误和任务领域模型；`task-engine` crate 管理任务队列、优先级、取消和进度聚合；`archive-core` crate 定义后端 trait；`archive-zip`、`archive-7z`、`archive-tar`、`archive-rar` 和 `archive-libarchive` 分别承担格式后端适配；`preview` crate 负责图片和文本预览；`cache` crate 负责索引、缩略图和临时文件；`platform` crate 隔离三平台系统集成。

这种拆分的目的不是增加抽象数量，而是控制变化边界。GPUI 仍处于 pre-1.0，UI 层应被隔离；C/C++ FFI 存在生命周期、错误码和路径编码问题，应被限制在后端 crate 内；平台集成差异大，应避免在业务逻辑中到处出现条件编译。

## 3. 能力模型

系统的中心对象不应是“格式名”，而应是能力。一个归档 session 打开后，应产生 `ArchiveInfo` 与 `ArchiveCapabilities`。前者描述格式、大小、entry 数量、codec、filter、solid、加密和分卷信息；后者描述是否支持列目录、解压全部、解压选中、创建、更新、随机访问、密码读取、密码写入、分卷读写、header encryption 和预览 stream。

能力模型使 UI 能够做正确判断。例如，ZIP 内图片可以直接预览；solid 7z 中的图片虽然也可以预览，但可能需要解码同一 solid block 中的前序数据；tar.gz 中的单文件预览可能需要从压缩流起点顺序读取到目标 entry。三者都可以表现为“预览图片”，但交互状态和等待解释必须不同。

## 4. 后端设计

ZIP 后端建议采用 minizip-ng。该库具备产品级 ZIP 能力，包括 ZIP64、AES、分卷、buffered streaming 和多压缩方法支持。ZIP 是最适合作为即时浏览体验的格式，打开时可以读取 central directory 构建 listing，单文件提取和图片预览也可以通过 entry 定位高效完成。创建 ZIP 时默认应使用 Deflate，AES 应作为推荐加密方式，高级方法如 ZSTD、LZMA、XZ 应在 UI 中标注兼容性风险。

7z 后端建议以 sevenz-rust2 为主。其重要价值在于不仅支持 LZMA/LZMA2，也支持 ZSTD、LZ4、Brotli、BZIP2、Deflate、PPMD 等 codec，并具备 AES 相关能力。这使得应用可以在三平台上提供受控且一致的 7z 扩展 codec 支持，而不依赖用户系统中安装的 7z 或 7zz。打开 7z 时，后端必须解析 codec chain、filter chain、solid 状态、header encryption 和分卷信息。对于不支持的边缘组合，应尝试 libarchive 或受控 helper fallback。

tar.* 后端应优先采用 Rust 原生流式管线，必要时使用 libarchive fallback。tar、tar.gz、tar.xz、tar.zst 的正确处理方式是链式流。解压路径为文件输入、解压器、tar reader、安全磁盘写入器；创建路径为文件系统遍历、tar writer、压缩器、输出文件。该设计严格避免中间 tar 文件，能够降低磁盘占用和 I/O 成本。

RAR 后端应以 UnRAR 为基础实现列表、测试和解压。RAR 创建不应作为内建默认能力承诺，因为该能力涉及 RARLAB 授权和外部工具。产品层面应将其设计为外部 helper 或商业授权扩展，并在 UI 中通过 capability probe 决定是否展示入口。

libarchive 的角色应定位为 fallback 和广谱处理层。它适合流式解包、tar.* fallback、部分 7z 读取和边缘格式识别，但不宜作为唯一主后端。尤其在加密 7z/RAR 上，libarchive 存在已知限制，主路径仍应由更适合的专用后端承担。

## 5. 任务系统

任务系统是保证产品体感的关键。所有压缩、解压、索引构建、图片预览、缩略图生成、测试归档和缓存清理都必须以任务形式执行。任务应支持优先级、取消、暂停、重试、进度聚合和错误汇总。

优先级应体现用户当前意图。用户刚点击的图片预览应高于后台缩略图预取；用户主动启动的解压或压缩应高于索引缓存构建；缓存清理应始终处于最低优先级。进度事件不应逐文件直接进入 UI，而应在 30 至 60 ms 的窗口内聚合，并同时报告字节进度、文件数进度、当前阶段、当前文件、速度和 ETA。

取消能力必须深入后端执行过程。对流式任务，取消点应出现在 buffer 读写边界和 entry 边界；对图片解码，取消点应出现在 metadata、解码、缩放和缓存写入之间；对外部 helper，取消必须能够终止进程，并清理临时文件。

## 6. 缓存与索引

缓存系统应分为索引缓存、缩略图缓存、预览临时缓存和任务恢复记录。索引缓存主要服务于 tar.gz、tar.xz、tar.zst 等顺序格式。首次打开这些归档时，系统可以通过单遍扫描构建 entry metadata，并将结果写入缓存；后续再次打开相同归档时，可以快速呈现目录骨架。缓存只应保存 metadata，不应保存中间 tar。

缩略图缓存应按 archive fingerprint、entry 标识、entry 路径、entry 大小、mtime、目标尺寸、decoder 版本和方向信息生成 key。缓存必须有容量上限和 LRU 策略。预览临时文件只允许用于系统外部查看器、拖出到文件管理器、大图高分辨率预览或特殊解码器需要真实路径的场景。

## 7. 预览管线

预览管线应采取分级加载策略。用户选中图片后，系统先读取 metadata，包括尺寸、文件大小、方向、EXIF 和 ICC 信息；随后生成缩略图；再生成适应窗口预览；只有当用户请求 100% 或更高倍率时，才生成高分辨率结果。这样可以将首屏反馈压到较低延迟，同时避免默认解码超大图片导致内存飙升。

基础实现可采用 image、fast_image_resize 和 kamadak-exif。image 提供常见格式解码能力和 ImageDecoder 接口，fast_image_resize 可用于高性能缩放，kamadak-exif 可用于读取 EXIF。对于 HEIC、PSD 等更复杂格式，可以在后续版本引入平台原生解码或外部 helper。

## 8. 平台集成

平台集成应独立于核心归档能力。Windows 需要处理文件关联、Explorer 右键菜单、Jump List、通知、长路径和 NTFS metadata；macOS 需要处理 app bundle、签名、公证、Finder 集成、Dock 最近文件和 APFS metadata；Linux 需要处理 XDG desktop file、MIME association、portal、通知和不同文件管理器扩展差异。

MVP 不应被 shell 深度集成阻塞。合理策略是先确保三平台核心应用可用，再逐步实现文件关联，最后处理右键菜单和拖出到系统文件管理器。

## 9. 错误模型

错误必须结构化，并从底层后端错误映射为稳定领域错误。UI 不应直接处理 C errno、7z 内部错误码、libarchive 状态码或 unrar 返回码。领域错误应至少区分不支持格式、不支持 codec、不支持 filter、需要密码、密码错误、归档损坏、磁盘空间不足、权限不足、路径过长、路径穿越被阻止、符号链接策略阻止、后端不可用、外部 helper 失败、任务取消和普通 I/O 错误。

每个错误应包含用户可读消息、技术详情、后端名称、归档路径、entry 路径和可恢复建议。这样既能提高用户体验，也能为诊断和回归测试提供稳定依据。

