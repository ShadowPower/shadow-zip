# 资料来源、事实依据与工程假设

文档日期：2026-05-07

## 1. 文档依据说明

本文档记录本方案所依赖的公开资料、已经核验的事实、基于资料形成的工程判断，以及后续必须通过原型或许可证审计继续验证的问题。其目的在于避免将“已经确认的库能力”“合理工程推断”和“尚需验证的产品承诺”混为一谈。

资料核验覆盖 Flutter 桌面能力、Rust/Flutter 桥接方案、minizip-ng、sevenz-rust2、libarchive、unrar.rs、7-Zip-zstd、NanaZip、7-Zip SDK、RARLAB 资料、图片解码库以及平台集成候选库。由于相关项目在持续演进，进入开发阶段后应定期重新核对关键依赖版本和许可证条款。

## 2. Flutter 与三平台能力

桌面端计划迁移为 Flutter + Rust，UI 层仍必须被封装，业务逻辑不应直接依赖具体 UI 框架对象和临时模式。

Flutter 官方桌面支持已经覆盖 Windows、Linux 和 macOS，配合 Rust FFI、平台通道或代码生成式桥接方案，可以作为该产品的 UI 技术路线。但 Flutter 桌面端的文件对话框、原生菜单、窗口行为、渲染性能、平台集成和 Rust bridge 方案仍然构成工程风险，需要原型验证。

相关资料包括 Flutter 桌面端文档、Rust FFI/桥接方案以及平台集成库。Flutter 的列表虚拟化和平台通道能力需要在桌面原型中验证。

## 3. ZIP 后端依据

minizip-ng 是 ZIP 后端的推荐主选。公开资料显示它支持 ZIP64、AES、分卷、buffered streaming 以及多种压缩方法。对于一个产品级桌面归档工具，这些能力比简单 ZIP 读写更关键，因为用户会自然期望大文件、密码包、分卷包和 Unicode 文件名能够稳定工作。

基于该资料，本方案将 minizip-ng 定位为 ZIP 主后端，而不是仅用 Rust ZIP crate 承担全部 ZIP 能力。该选择会带来 C/C++ 构建和分发成本，但可以换取更成熟的 ZIP 功能覆盖。

## 4. 7z 后端依据

sevenz-rust2 是 7z 主后端的推荐选择。其公开文档显示，它支持 LZMA、LZMA2、Brotli、BZIP2、Deflate、PPMD、LZ4、ZSTD 等 codec，并具备 AES 相关能力。这一事实直接影响产品定位：7z + ZSTD、7z + LZ4 和 7z + Brotli 不应被视为异常归档，而应被纳入正式兼容目标。

7-Zip-zstd 和 NanaZip 的资料进一步证明，Zstandard、LZ4、Brotli 等 codec 在 7z 生态中具有现实存在和用户价值。7-Zip-zstd README 中关于 7z 与 7zz 外部 codec/plugin 行为差异的说明也表明，产品不能依赖用户系统中已有 7z 命令具备一致能力。若要提供三平台一致体验，应用需要自带后端能力或受控 helper。

本方案据此将 sevenz-rust2 作为主路径，将 libarchive 和 helper 作为 fallback 路径。该判断仍需通过样本库验证，尤其是复杂组合，如 ZSTD + BCJ + solid + encryption + multipart。

## 5. libarchive 依据

libarchive 支持广泛的归档格式，并具备流式处理能力。其资料显示，它支持 7-Zip archives，包括使用 zstandard compression 的 7z 归档。这使其适合作为 tar.* fallback、边缘格式识别和部分 7z 读取 fallback。

然而，libarchive 不应作为唯一主后端。它的模型更适合流式归档处理，而不是所有交互式随机访问场景。此外，公开 issue 显示其在加密 7z/RAR 上存在已知限制。因此，本方案将 libarchive 定位为兼容层和 fallback，而非统一归档核心。

## 6. RAR 依据

unrar.rs 以及底层 UnRAR 能够支持 RAR 的列表、测试和解压，但不支持创建 RAR。RAR 创建涉及 RARLAB 官方工具或授权方案，这一边界不能通过普通 Rust crate 规避。

因此，产品应明确区分 RAR 解压和 RAR 创建。RAR/RAR5 解压可以作为内建目标；RAR 创建应被设计为外部引擎能力，并在许可证、分发和 UI 上清楚说明。

## 7. 图片预览依据

image crate 提供常见图片格式解码能力和 ImageDecoder 接口，fast_image_resize 提供高性能缩放能力，kamadak-exif 可读取 EXIF 信息，zune-image 可作为可选补充。基于这些资料，图片预览管线可以在 Rust 侧实现大部分基础能力。

需要注意的是，图片预览的难点不只是解码格式数量。产品必须处理大图、动图、损坏图片、EXIF 方向、ICC、局部解码、缩略图缓存、内存上限和预览取消。ZIP 内图片可以较快读取，而 solid 7z、solid RAR 和 tar.xx 中的图片受归档访问模型限制，预览延迟可能显著增加。

## 8. 工程假设

本方案包含若干需要原型验证的工程假设。第一，Flutter 可以在三平台上支撑高密度工具型界面和大列表虚拟化。第二，sevenz-rust2 对常见 7z 扩展 codec 的支持足以作为主路径。第三，minizip-ng 的三平台构建和 FFI 维护成本可控。第四，tar.zst 单遍流式任务可以在保持取消响应的同时获得良好吞吐。第五，图片预览的内存和磁盘策略可以通过分级加载与 LRU 缓存控制在合理范围内。

这些假设都不能仅凭文档最终确认。进入 Phase 0 后，应通过原型、样本库和性能基线逐项验证。

## 9. 许可证关注点

许可证审计必须在稳定版本前完成。重点包括 Flutter 相关许可证、minizip-ng、sevenz-rust2、libarchive、UnRAR、RAR CLI、7-Zip SDK、7-Zip-zstd、图片 codec 依赖、静态链接和动态链接义务、third-party notices 以及安装包中 helper 的分发条款。

其中 RAR 创建是最敏感事项。产品不得在未完成授权确认前承诺内建 RAR 创建能力。UnRAR 的读取能力也应确认其分发条款，避免将“可解压”误解为“可自由创建 RAR”。

## 10. 主要资料链接

本方案参考的主要资料包括：Flutter 文档、Flutter Rust 桥接方案、minizip-ng、sevenz-rust2、libarchive、libarchive encrypted 7z/RAR issue、unrar.rs、RARLAB downloads、RARLAB addons、RARLAB license、7-Zip SDK、7-Zip GitHub、7-Zip-zstd、NanaZip、image docs、fast_image_resize、zune-image、kamadak-exif、windows-rs、objc2、ashpd、rfd、notify-rust 和 tray-icon。
