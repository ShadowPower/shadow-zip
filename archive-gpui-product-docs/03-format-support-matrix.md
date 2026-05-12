# 格式与能力支持矩阵

文档日期：2026-05-07

## 1. 文档说明

本文件用于精确定义各类归档格式在本产品中的支持边界。由于归档能力涉及读取、写入、加密、分卷、随机访问、预览、fallback 和平台分发，使用矩阵是必要的。矩阵并非简单功能清单，而是产品承诺、后端选型和测试范围之间的合同。

阅读本文件时应注意两点。第一，同一格式在不同 codec、solid 状态、加密方式和分卷方式下的体验可能完全不同。例如，7z + LZMA2 non-solid 与 7z + ZSTD solid 都是 7z，但单文件预览成本不同。第二，表中的“支持”不只表示能够完成操作，也包含用户体验和可维护性判断。某些能力虽然技术上可以通过外部工具完成，但不适合作为内建默认能力承诺。

## 2. 标记说明

本文档使用以下标记：

- F：完整支持。
- H：高质量支持，存在少量边缘限制。
- M：中等支持，功能可用但体验或能力有限。
- L：有限支持，仅覆盖部分场景。
- E：依赖外部组件或商业授权。
- N：不支持或不建议支持。

## 3. 总体格式矩阵

总体矩阵给出每种格式在产品中的基本定位。ZIP 和 7z 是交互式浏览体验的核心格式；tar.* 是开发者与跨平台分发场景的重要格式，必须保证单遍流式处理；RAR/RAR5 以解压和浏览为目标，创建能力外部化。

| 格式 | 打开列表 | 解压全部 | 解压选中 | 创建 | 更新已有归档 | 密码读取 | 密码写入 | 分卷读取 | 分卷写入 | 图片预览 | 随机访问体验 | 主后端 | fallback |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| ZIP | F | F | F | F | H | F | F | F | F | F | F | minizip-ng | libarchive |
| ZIPX | H | H | H | H | M | H | H | H | M | H | H | minizip-ng/libarchive | libarchive |
| 7z LZMA/LZMA2 | F | F | H | F | M | H | H | H | H | H | H | sevenz-rust2 | libarchive/helper |
| 7z ZSTD | F | F | H | H | M | H | H | H | H | H | H/M | sevenz-rust2 | libarchive/helper |
| 7z LZ4 | F | F | H | H | M | H | H | H | H | H | H | sevenz-rust2 | helper |
| 7z Brotli | F | F | H | H | M | H | H | H | H | H | H/M | sevenz-rust2 | helper |
| 7z solid | F | F | M | F | M | H | H | H | H | M | L | sevenz-rust2 | helper |
| tar | F | F | H | F | N | N | N | N | N | M | L | tar crate | libarchive |
| tar.gz | H | F | M | F | N | N | N | N | N | L/M | L | tar+flate2 | libarchive |
| tar.xz | H | F | M | F | N | N | N | N | N | L/M | L | tar+xz | libarchive |
| tar.zst | H | F | M | F | N | N | N | N | N | L/M | L | tar+zstd | libarchive |
| RAR/RAR5 | H | F | M | E | N | H | N | H | E | M | L | unrar | libarchive limited |
| RAR encrypted | M | M | L | E | N | H | N | H | E | L | L | unrar | libarchive limited |

## 4. ZIP 能力矩阵

ZIP 是本产品最适合提供即时浏览和快速预览的格式。minizip-ng 应作为主后端，以覆盖 ZIP64、AES、分卷和高级压缩方法。默认创建策略应以兼容性为主，高级方法应提供但不应默认启用。

| 能力 | 支持等级 | 实现方案 | 产品说明 |
|---|---:|---|---|
| 普通 ZIP 读取 | F | minizip-ng | 必须作为核心能力 |
| 普通 ZIP 创建 | F | minizip-ng | 默认压缩方法建议 Deflate |
| ZIP64 | F | minizip-ng | 大文件必需 |
| AES 加密读取 | F | minizip-ng | 密码包主线能力 |
| AES 加密写入 | F | minizip-ng | 推荐密码 ZIP 默认方案 |
| Traditional PKWARE | H | minizip-ng | 兼容旧包，但安全性弱 |
| 分卷读取 | F | minizip-ng | 需要样本测试 |
| 分卷写入 | F | minizip-ng | UI 需提供分卷大小 |
| ZSTD method | H | minizip-ng | 高级选项，兼容性提示 |
| BZIP2/LZMA/XZ method | H | minizip-ng | 高级选项 |
| 图片预览 | F | minizip-ng + preview service | 随机访问体验最佳 |
| 更新已有归档 | H | minizip-ng | 需谨慎处理损坏恢复 |

## 5. 7z codec 矩阵

7z 的支持边界必须以 codec 和 filter 维度描述。只声明“支持 7z”不足以表达真实能力，因为 ZSTD、LZ4、Brotli、BCJ、Delta、solid、header encryption 和 multipart 都会改变访问成本和兼容性。

| 7z codec/filter | 列目录 | 解压 | 创建 | 图片预览 | 单文件提取 | solid 组合体验 | 兼容性风险 | 主路径 |
|---|---:|---:|---:|---:|---:|---:|---|---|
| COPY | F | F | F | F | F | H | 低 | sevenz-rust2 |
| LZMA | F | F | F | H | H | M | 低 | sevenz-rust2 |
| LZMA2 | F | F | F | H | H | M | 低 | sevenz-rust2 |
| ZSTD | F | F | H | H | H | M | 中 | sevenz-rust2 |
| LZ4 | F | F | H | H | H | M/H | 中 | sevenz-rust2 |
| Brotli | F | F | H | H | H | M | 中 | sevenz-rust2 |
| BZIP2 | F | F | H | H | H | M | 中低 | sevenz-rust2 |
| DEFLATE | F | F | H | H | H | M | 中低 | sevenz-rust2 |
| PPMD | F | F | H | H | M | M | 中 | sevenz-rust2 |
| BCJ x86 | F | F | H | H | H | M | 中 | sevenz-rust2 |
| BCJ ARM | F | F | H | H | H | M | 中 | sevenz-rust2 |
| BCJ ARM64 | F | F | H | H | H | M | 中 | sevenz-rust2 |
| Delta | F | F | H | H | H | M | 中 | sevenz-rust2 |

## 6. 7z 特性矩阵

产品应将 7z + ZSTD、7z + LZ4 和 7z + Brotli 纳入正式读取目标。这些组合在 sevenz-rust2、7-Zip-zstd 和 NanaZip 生态中都有现实依据。不过，创建默认值仍应保守，避免默认生成旧工具无法打开的归档。

| 特性 | 支持等级 | 实现说明 | UI 说明 |
|---|---:|---|---|
| 非 solid 7z | H | sevenz-rust2 | 单文件提取和预览较好 |
| solid 7z | M/H | sevenz-rust2 | 单文件预览可能较慢 |
| header encryption | H | sevenz-rust2 | 输入密码前可能无法列目录 |
| AES256 | H | sevenz-rust2 | 需完整测试 |
| 分卷读取 | H | sevenz-rust2/helper | 需样本覆盖 |
| 分卷写入 | H | sevenz-rust2/helper | 高级选项 |
| 7z + ZSTD | H | sevenz-rust2/libarchive/helper | 正式支持目标 |
| 7z + LZ4 | H | sevenz-rust2/helper | 高级 codec |
| 7z + Brotli | H | sevenz-rust2/helper | 高级 codec |
| 旧工具兼容 | M | 创建默认 LZMA2 | 高级 codec 需提示风险 |

## 7. tar.* 矩阵

tar.* 的核心要求是单遍流式处理。表中的随机访问等级较低，并不代表产品价值低，而是反映格式结构本身不适合 ZIP 式随机预览。索引缓存可以改善浏览体验，但不能改变压缩流需要顺序读取的事实。

| 格式 | 单遍解压 | 单遍创建 | 列目录 | 解压选中 | 图片预览 | 索引缓存 | 随机访问 | 说明 |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| tar | F | F | F | H | M | M | L/M | 未压缩 tar 可 seek 时体验较好 |
| tar.gz | F | F | H | M | L/M | F | L | 必须避免中间 tar |
| tar.xz | F | F | H | M | L/M | F | L | 解压 CPU 成本较高 |
| tar.zst | F | F | H | M | L/M | F | L | 速度与压缩比平衡好 |

## 8. RAR 矩阵

RAR 的产品定位应谨慎。解压、列表、测试和密码读取可以作为内建目标；创建能力应被视为外部引擎或商业授权能力。该边界应反映在 UI、文档和发布声明中。

| 能力 | 支持等级 | 实现方案 | 说明 |
|---|---:|---|---|
| RAR 列表 | H | unrar | 支持目标 |
| RAR5 列表 | H | unrar | 支持目标 |
| RAR 解压 | F | unrar | 支持目标 |
| RAR5 解压 | F | unrar | 支持目标 |
| 密码 RAR 读取 | H | unrar | 支持目标 |
| RAR 测试 | H | unrar | 支持目标 |
| RAR 分卷读取 | H | unrar | 支持目标 |
| RAR 创建 | E | rar CLI/商业授权 | 不作为内建默认能力 |
| RAR 分卷创建 | E | rar CLI/商业授权 | 外部能力 |
| RAR 图片预览 | M | unrar + preview | 受顺序读取影响 |

## 9. 加密矩阵

加密能力不能跨格式简单等价。ZIP AES、7z AES 和 RAR encrypted 在后端、header 可见性、创建能力和 fallback 支持上都不同。产品应将密码读取、密码写入和 header encryption 分开呈现。

| 格式 | 读取密码包 | 创建密码包 | header encryption 读取 | header encryption 创建 | 备注 |
|---|---:|---:|---:|---:|---|
| ZIP Traditional | F | F | N/A | N/A | 兼容旧工具但安全性弱 |
| ZIP AES | F | F | N/A | N/A | 推荐默认密码 ZIP |
| 7z AES | H | H | H | H | 需覆盖 header encrypted 样本 |
| tar.* | N | N | N | N | 建议外层加密而非格式级加密 |
| RAR encrypted | H | N | M/H | N | 读取可做，创建外部化 |
| libarchive on encrypted 7z/RAR | L | N | L | N | 存在已知限制 |

## 10. 浏览体验矩阵

浏览体验矩阵用于指导 UI 状态设计。格式固有限制应转化为明确状态，例如正在扫描、正在定位压缩块、正在顺序读取，而不是表现为无说明的等待。

| 格式 | 快速列目录 | 快速打开单文件 | 连续图片预览 | 搜索 | 排序 | 体验等级 |
|---|---:|---:|---:|---:|---:|---|
| ZIP | F | F | F | F | F | 最佳 |
| ZIPX | H | H | H | F | F | 很好 |
| 7z 非 solid | F | H | H | F | F | 很好 |
| 7z solid | F | M | M | F | F | 中等 |
| tar | H | M | L/M | H | H | 中等偏低 |
| tar.gz | M/H | L/M | L | H | H | 低 |
| tar.xz | M/H | L/M | L | H | H | 低 |
| tar.zst | M/H | L/M | L | H | H | 低 |
| RAR 非 solid | H | M | M | H | H | 中等 |
| RAR solid | H | L/M | L/M | H | H | 较低 |

## 11. 图片格式预览矩阵

图片预览支持应采用分级策略。metadata 和缩略图是首要目标，原图和高级格式支持应受资源限制和后端能力控制。HEIC、PSD 等格式可通过平台解码器或 helper 后续扩展。

| 图片格式 | metadata | 缩略图 | 适应窗口 | 原图 | EXIF/ICC | 实现建议 |
|---|---:|---:|---:|---:|---:|---|
| JPEG | F | F | F | F | F | image + kamadak-exif |
| PNG | F | F | F | F | H | image |
| GIF | F | H | H | M | L | image |
| WebP | F | H | H | H | M/H | image 或 zune-image |
| BMP | F | F | F | F | L | image |
| TIFF | H | H | H | M | F | image + kamadak-exif |
| ICO | H | H | H | M | L | image |
| AVIF | H | H | H | M/H | M | image feature 或专用解码 |
| QOI | H | H | H | H | L | image |
| HEIC | L/E | L/E | L/E | L/E | M | 平台或外部 helper |
| PSD | L/E | L/E | L/E | L/E | L | 可后续扩展 |

## 12. shell 集成矩阵

shell 集成是产品化能力，而不是归档核心能力。Windows 和 macOS 可以获得较高一致性，Linux 需要面对文件管理器生态差异，因此应分阶段实现。

| 能力 | Windows | macOS | Linux | MVP |
|---|---:|---:|---:|---:|
| 双击归档打开 | F | F | F | H |
| 文件关联 | F | F | H | M |
| 右键解压到此处 | F | H | M | N |
| 右键解压到同名目录 | F | H | M | N |
| 右键创建归档 | F | H | M | N |
| 从归档拖出 | H | H | M | N |
| 系统通知 | F | F | H | M |
| 最近文件 | F | F | H | M |
| 自动更新 | H | H | M | N |

## 13. 失败与降级矩阵

失败与降级矩阵用于定义错误处理策略。产品应尽可能将后端失败转化为结构化、可解释、可恢复的用户体验。

| 场景 | 检测方式 | 用户提示 | 降级策略 |
|---|---|---|---|
| 不支持的 7z codec | method chain probe | 显示 codec 名称 | 尝试 libarchive/helper |
| 不支持的 7z filter | filter chain probe | 显示 filter 名称 | 尝试 helper |
| 加密 header 需要密码 | open header fail | 请求密码 | 重试打开 |
| libarchive 不能读加密数据 | backend error | 标明 fallback 不支持 | 切换主后端/helper |
| tar.xx 索引未完成 | index state | 显示扫描中 | 允许先解压全部 |
| solid 7z 预览慢 | archive info | 显示读取压缩块中 | 邻近预取 |
| 大图超限 | decoder guard | 显示保护模式 | 低清预览或外部查看 |
| RAR 创建不可用 | capability probe | 显示需要外部组件 | 引导配置 rar CLI |
| 磁盘空间不足 | preflight | 显示所需与可用空间 | 停止任务 |
| 路径穿越 | sanitizer | 阻止危险 entry | 继续安全 entry |
