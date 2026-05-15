# Rust + Flutter 跨平台归档工具产品与技术文档

文档日期：2026-05-07

本文件夹收录一组面向产品立项、技术评审和后续工程实施的正式文档。文档对象是一款基于 Rust 与 Flutter 的跨平台图形归档工具，目标是在 Windows、Linux 与 macOS 上提供接近 Bandizip 的归档浏览、压缩、解压、预览和系统集成体验。

本项目的基本判断是：在允许使用 C/C++ 库、外部 helper、平台专用代码以及 Rust 原生库共同构建后端的前提下，该产品具有明确的工程可行性。真正的复杂性并不集中在压缩算法本身，而在于不同归档格式的访问模型差异、三平台桌面集成差异、图片预览的资源控制、solid 归档的交互延迟、7z 扩展 codec 的兼容性，以及 RAR 创建能力背后的授权问题。

因此，本文档不将该产品描述为一个“压缩库图形壳”。更准确的定义是：它是一个以 Flutter 为桌面交互层、以 Rust 为领域编排层、以多后端归档引擎为格式兼容层的跨平台归档工作台。产品需要同时处理用户可感知的响应速度、任务可取消性、内存与磁盘资源上限、格式能力透明呈现，以及长期可维护的后端扩展机制。

文档集由以下部分组成。

[01-product-requirements.md](./01-product-requirements.md) 描述产品定位、目标用户、核心场景、性能目标、资源目标、安全要求和版本范围。该文档用于明确产品究竟要解决什么问题，以及第一阶段应当交付哪些具有实际价值的能力。

[02-technical-architecture.md](./02-technical-architecture.md) 描述总体架构、模块边界、归档后端、任务系统、缓存索引、预览管线、平台集成和错误模型。该文档用于支撑技术评审和工程拆分。

[03-format-support-matrix.md](./03-format-support-matrix.md) 是格式支持矩阵。由于格式能力天然适合用表格表达，该文档保留矩阵形式，但其目的不是罗列功能，而是明确不同格式在读取、写入、加密、分卷、预览、随机访问和 fallback 上的真实边界。

[04-implementation-roadmap.md](./04-implementation-roadmap.md) 描述实施阶段、验证顺序、测试策略、性能基线和风险控制方式。该文档用于指导从原型到稳定版本的开发节奏。

[05-sources-and-assumptions.md](./05-sources-and-assumptions.md) 记录已核验的资料来源、事实依据、工程推断和待验证事项。该文档用于区分“已经确认的能力”和“需要原型验证的判断”。

[06-ux-interaction-spec.md](./06-ux-interaction-spec.md) 描述交互设计，包括主窗口、归档打开、压缩、解压、预览、密码、冲突、错误和设置。该文档强调工具型桌面软件应具备的效率、确定性和低干扰体验。

[07-api-and-data-model.md](./07-api-and-data-model.md) 描述接口级设计和核心数据模型。该文档用于后续定义 Rust crate 边界、后端 trait、任务模型、能力模型、错误模型和缓存键。

[08-distribution-licensing-operations.md](./08-distribution-licensing-operations.md) 描述构建分发、平台差异、helper 管理、许可证、日志、诊断、发布门禁和长期维护策略。

综合所有前期搜索与分析，推荐技术路线是：使用 Flutter 构建桌面界面；使用 Rust 实现领域模型、任务调度、缓存、索引、错误处理和后端适配；使用 minizip-ng 作为 ZIP 主后端；使用 sevenz-rust2 作为 7z 主后端，并通过 libarchive 或受控 helper 覆盖边缘兼容场景；使用 Rust 原生流式管线或 libarchive 处理 tar、tar.gz、tar.xz、tar.zst；使用 UnRAR 处理 RAR 解压；RAR 创建则作为外部组件或商业授权能力处理。

其中，7z + Zstandard、7z + LZ4、7z + Brotli 不应被视为异常归档或坏包。它们已经在 sevenz-rust2、libarchive、7-Zip-zstd 和 NanaZip 生态中体现出实际存在价值。产品应将这些扩展 codec 纳入正式兼容范围，但在创建归档时默认采用更保守的 LZMA2，以避免用户生成旧工具无法打开的归档。

同样，tar.gz、tar.xz、tar.zst 必须按链式流处理。正确路径是从压缩流直接进入 tar reader，再直接写入目标目录；创建时则从文件系统遍历直接进入 tar writer，再进入压缩器输出目标文件。任何默认生成中间 .tar 的方案都会造成额外 I/O、额外磁盘占用和明显的体验损失，因而不应被采用。

本文档集当前定位为立项与设计阶段的正式材料。后续进入实现阶段后，应继续补充 Rust workspace 结构、crate 级 API、FFI 安全边界、样本归档库、性能基线报告和三平台构建脚本。

