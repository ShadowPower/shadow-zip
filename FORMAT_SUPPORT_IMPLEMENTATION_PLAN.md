# Shadow Zip 格式支持实现方案

文档日期：2026-05-18

## 1. 目标和选型原则

本文档基于 `archive-product-docs/09-bandizip-format-gap-checklist.md`，说明 Shadow Zip 应如何用 Rust 实现各格式支持。核心目标是尽量把格式能力做成进程内库调用，而不是依赖外部二进制程序。

选型顺序固定如下：

1. 优先使用纯 Rust crate，并把能力封装成 `ArchiveBackend` 或压缩流 adapter。
2. 纯 Rust crate 覆盖不足时，使用 C/C++ 库的 Rust binding 或项目自建 FFI wrapper，仍保持进程内调用。
3. 只有在前两者不可用、授权不可接受、格式过于冷门或实现成本明显失衡时，才允许回退到受控二进制 helper。

通用实现边界：

- 后端必须输出统一的 `ArchiveInfo`、`ArchiveListing`、`ArchiveEntry`、`ArchiveCapabilities`。
- 解压必须经由现有 `SafeWriter`，继续复用路径穿越、UNC、device path、符号链接和冲突策略。
- 支持 entry byte stream 的格式应实现 `open_entry_reader`，避免预览和 `cat` 只能走整包解压。
- 顺序格式必须标记 `AccessCost::SequentialFromStart`，solid 或块压缩格式必须标记较高访问成本。
- 密码、损坏包、不支持 codec/filter、分卷缺失、CRC 错误要映射成结构化 `ArchiveErrorKind`。
- 新格式必须进入 CLI fixture、真实样本集、错误样本集和性能基线。

## 2. 全流式组合格式约束

`tar.gz`、`tar.xz`、`tar.zst`、`tar.bz2`、`tar.lzma`、`tar.Z`、`tar.lz`、`tar.br` 必须作为一个链式流处理，不能拆成“先生成 tar，再解压/压缩 tar”的两次操作。

解压管线：

```text
File -> compression decoder -> tar::Archive -> SafeWriter
```

创建管线：

```text
InputScanner -> tar::Builder -> compression encoder -> File
```

实现要求：

- 所有 decoder/encoder 必须实现或包装为 `Read`/`Write`。
- 进度统计挂在 stream pump 或 tar entry 读写循环上。
- 取消任务要能中断压缩流、tar reader/writer 和目标写入。
- 临时目录不能出现接近未压缩 tar 总大小的中间文件。

## 3. 已支持格式的实现完善

### 3.1 ZIP / ZIPX

首选方案：继续使用 Rust `zip` crate。

实现计划：

- 保留现有 `crates/archive-zip` 后端。
- 开启并验证 `deflate`、`deflate64`、`bzip2`、`aes-crypto`、`zstd`、`lzma`、`xz` 等 feature。
- ZIPX 仍可归入 ZIP 后端，但 `ArchiveInfo.codecs` 必须列出真实 method。
- `create_zip_archive` 应根据 `CreateOptions.compression_method` 选择 Store、Deflate、BZIP2、ZSTD、XZ 等 method。
- AES 写入需要补齐实际实现，不能只在 capability 中声明。
- 分卷 ZIP 读取/写入若 `zip` crate 覆盖不足，可评估 `unarc-rs` 的 split ZIP 能力；仍不足再考虑 minizip-ng FFI。

备选方案：minizip-ng C library FFI，用于高级 ZIP64、AES、split ZIP、兼容性恢复。

最后 fallback：7-Zip/libarchive 二进制 helper，只作为诊断或用户手动兼容模式。

### 3.2 7Z

首选方案：继续使用纯 Rust `sevenz-rust2`。

实现计划：

- 补齐真实创建路径，使用 `ArchiveWriter`/compress API 创建 `.7z`。
- 默认创建 `LZMA2`，高级选项开放 `ZSTD`、`LZ4`、`Brotli`、`BZIP2`、`DEFLATE`、`PPMD`。
- 读取时解析 folder/coder chain，把 codec/filter 写入 `ArchiveInfo.codecs` 和 entry method。
- solid 归档必须标记 selected extract 和 preview 的高访问成本。
- header encryption、AES256、分卷 `.7z.001` 必须建立样本测试。
- 对 unsupported codec/filter，优先尝试进程内 libarchive FFI，而不是调用二进制。

备选方案：libarchive C library FFI，覆盖部分 7z 兼容场景。

最后 fallback：受控 7zz/7z helper。

### 3.3 TAR

首选方案：继续使用 Rust `tar` crate。

实现计划：

- 保持 `tar::Archive` 顺序读取和 `tar::Builder` 顺序写入。
- 对 symlink、hardlink、device node、fifo 等 entry type 做策略化映射。
- 未压缩 tar 在 `Read + Seek` 可用时可增加轻量索引缓存，但不能承诺 ZIP 式随机访问。
- 创建时保留 Unix mode、mtime、uid/gid 的能力要以跨平台策略暴露。

备选方案：libarchive FFI，用于 pax/GNU sparse/ACL 等边缘 tar 特性。

最后 fallback：无默认二进制 fallback。

### 3.4 TGZ / tar.gz

首选方案：`tar` + `flate2`，配置 `rust_backend` 时尽量走纯 Rust miniz_oxide。

实现计划：

- 解压：`File -> GzDecoder -> tar::Archive -> SafeWriter`。
- 创建：`InputScanner -> tar::Builder<GzEncoder<File>>`。
- 独立 `.gz` 与 `.tar.gz` 必须分开识别。
- 读取 gzip header 中原始文件名和 mtime，写入 `ArchiveInfo` 或 metadata。
- 增加大包取消、损坏 gzip header、CRC 错误样本。

备选方案：zlib-ng/minizip-ng FFI，只用于性能专项或兼容性问题。

最后 fallback：无默认二进制 fallback。

### 3.5 TXZ / tar.xz

首选方案：`tar` + `xz2`。`xz2` 是 liblzma binding，不是纯 Rust，但成熟度和兼容性较好。

实现计划：

- 解压：`File -> XzDecoder -> tar::Archive -> SafeWriter`。
- 创建：`InputScanner -> tar::Builder<XzEncoder<File>>`。
- 若要进一步减少 C 依赖，可评估 `lzma-rust2` 对 xz container 的覆盖程度；未确认前不替换主路径。
- 显示 xz 解压 CPU 成本较高的访问提示。

备选方案：libarchive FFI。

最后 fallback：无默认二进制 fallback。

### 3.6 TZST / tar.zst

首选方案：`tar` + `zstd` crate。`zstd` crate 绑定 libzstd，跨平台成熟。

实现计划：

- 解压：`File -> zstd::stream::read::Decoder -> tar::Archive -> SafeWriter`。
- 创建：`InputScanner -> tar::Builder<zstd::stream::write::Encoder<File>>`。
- 支持 compression level。
- 对 skippable frame、字典缺失、损坏 frame 做错误映射。

备选方案：libarchive FFI。

最后 fallback：无默认二进制 fallback。

### 3.7 RAR / RAR5

首选方案：继续使用 `unrar` crate。它依赖 UnRAR 原生库，但进程内调用优先于外部二进制。

实现计划：

- 完善 helper availability 与实际 crate 动态/静态链接状态的诊断。
- 建立 RAR4、RAR5、solid、password、header encrypted、多卷样本。
- `open_entry_reader` 若 UnRAR API 难以直接流式提供单 entry，可使用受控临时文件策略，但必须有大小限制和清理策略。
- RAR 创建不作为内建能力；若未来支持，只能作为 RARLAB 授权外部能力。

备选方案：libarchive FFI 读取 RAR/RAR5，但注意 proprietary limitation。

最后 fallback：unrar/rar 二进制 helper。

### 3.8 libarchive fallback

首选方案：把当前 `bsdtar` 二进制 fallback 改造成 `libarchive` C library FFI。

实现计划：

- 新建 `crates/archive-libarchive-sys` 或采用维护良好的 binding。
- 使用 `archive_read_support_format_all` 和显式 filter support，进程内读取 entry。
- 把 libarchive entry 转成 `ArchiveEntry`，把 data block 流进 `SafeWriter`。
- 只把 libarchive 用作兼容后端，不让它掩盖主后端的错误。
- 记录 fallback 来源、原始错误、libarchive format/filter 名称。

最后 fallback：保留 `bsdtar` 作为可选诊断工具，不作为默认路径。

## 4. 创建格式缺口

### 4.1 LZH(lh7)

首选方案：读取使用纯 Rust `delharc`，创建评估 `oxiarc-lzhuf` 作为 lh/LZH 压缩算法基础。

实现计划：

- 先实现 LHA/LZH 读取后端，复用第 5.6 和 5.7 的读取方案。
- 创建 `.lzh` 时需要自己写 LHA header、CRC、method 标记和目录 entry 映射。
- `lh7` 创建若 `oxiarc-lzhuf` 不能覆盖完整 LHA method，先降级为不承诺创建。
- 文件名编码需要提供 Shift-JIS、CP437、本地代码页和 UTF-8 策略。

备选方案：libarchive FFI 可读 LHA/LZH，但其公开支持列表主要是读取，不适合作为创建主路径。

最后 fallback：lha/lha32 等二进制 helper，仅可选。

### 4.2 ISO(joliet)

首选方案：读取用 `hadris` 的 ISO 模块；创建优先评估 libarchive ISO9660 writer FFI。

实现计划：

- 读取 `.iso` 时优先使用 `hadris`，覆盖 ISO9660、Joliet、Rock Ridge。
- 创建 Joliet ISO 时，纯 Rust 生态需要评估是否有成熟 writer；若没有，使用 libarchive ISO9660 writer。
- 创建选项包括卷标、Joliet 开关、Rock Ridge 开关、路径长度策略。
- ISO 是文件系统镜像，不压缩；UI/CLI 必须区别于压缩归档。

备选方案：自研最小 ISO9660/Joliet writer，仅支持普通目录和文件。

最后 fallback：mkisofs/xorriso helper，不作为默认能力。

### 4.3 GZ

首选方案：`flate2`，配置 Rust backend。

实现计划：

- 新建单文件压缩流 backend 或 stream codec service。
- `.gz` 只接受单文件输入；目录输入必须提示使用 `tar.gz`。
- 读取 header 的 filename 和 mtime。
- 写入 header、CRC 和 ISIZE。
- `ArchiveListing` 生成单个虚拟 entry。

备选方案：zlib-ng FFI 用于性能。

最后 fallback：无默认二进制 fallback。

### 4.4 XZ

首选方案：`xz2`/liblzma FFI。

实现计划：

- `.xz` 只作为单文件压缩流，目录输入提示使用 `tar.xz`。
- `ArchiveListing` 生成单个虚拟 entry。
- 支持压缩级别和 check type。
- 解压和创建都走 `Read`/`Write` stream。

备选方案：评估 `lzma-rust2` 是否足以替换解码路径。

最后 fallback：无默认二进制 fallback。

## 5. 旧式归档格式缺口

### 5.1 ACE

首选方案：评估纯 Rust `unarc-rs` 的 ACE 模块。

实现计划：

- 使用 `unarc-rs` unified API 快速建立读取、列目录、解压、密码读取。
- 验证 ACE 1.0、ACE 2.0、Blowfish encryption、CRC 错误和不支持版本。
- 多卷 ACE 若库不支持，capability 标记为 unsupported，并给出清晰错误。
- 若 `unarc-rs` 的抽象不能满足 entry stream，针对 ACE module 写薄 adapter。

备选方案：libarchive 不在公开列表中承诺 ACE；不作为首选。

最后 fallback：unar/7zz helper，需安全沙箱和输出解析。

### 5.2 ALZ

首选方案：没有已确认成熟纯 Rust crate。建议先做格式探测和 unsupported diagnostic。

实现计划：

- 实现扩展名和魔数探测，能明确显示 ALZ unsupported。
- 调研 ALZip SDK、The Unarchiver/unar 源码或 7-Zip 相关 codec 是否可库化。
- 若存在可接受 C/C++ 库，封装为进程内 FFI 后端。
- 文件名编码优先支持韩文代码页和 UTF-8。
- 分卷 ALZ 作为必须测试项。

备选方案：自研读取器，成本较高，需格式文档和样本库。

最后 fallback：unar 或 7zz helper。

### 5.3 ARJ

首选方案：评估纯 Rust `unarc-rs` ARJ 模块。

实现计划：

- 支持列目录、解压、CRC、Garble/GOST40 密码读取。
- 多卷 ARJ 若库不支持，capability 标记为 unsupported。
- GOST-256/ARJCRYPT 只检测并提示不支持。
- 文件名编码支持 CP437 和本地代码页。

备选方案：libarchive FFI。libarchive 公开列表未把 ARJ 作为核心格式承诺，应作为兼容补充。

最后 fallback：7zz/unar helper。

### 5.4 BH

首选方案：没有已确认成熟纯 Rust crate。先实现探测和 unsupported diagnostic。

实现计划：

- 收集 BlakHole 样本，确认 header、method、加密标记。
- 若格式简单，考虑自研只读解析器。
- 若有 C/C++ 解码实现，封装进程内 FFI。
- 不支持 method 必须映射为 `UnsupportedCodec`。

备选方案：The Unarchiver/unar 源码库化评估。

最后 fallback：unar helper。

### 5.5 EGG

首选方案：没有已确认成熟纯 Rust crate。建议优先调研 ALZip EGG 公开实现或 The Unarchiver。

实现计划：

- 先实现 `.egg` 探测和 unsupported diagnostic。
- 样本必须覆盖普通、solid、分卷、密码、Unicode/韩文文件名。
- 若找到可接受 C/C++ 库，封装 FFI 后端。
- 若只能通过 helper 支持，必须显式标为 external。

备选方案：自研只读解析器，需较大格式逆向成本。

最后 fallback：unar helper。

### 5.6 LHA

首选方案：纯 Rust `delharc`。

实现计划：

- 新建 `archive-lha` 后端，支持 `.lha`。
- 用 `delharc` 读取 header、method、mtime、CRC 和 entry stream。
- 解压时逐 entry 流进 `SafeWriter`。
- 创建不在 LHA 读取后端里承诺，避免和 LZH(lh7) 创建混淆。

备选方案：libarchive FFI 读取。

最后 fallback：lha/unar helper。

### 5.7 LZH

首选方案：纯 Rust `delharc`。

实现计划：

- 与 LHA 共享后端，扩展名覆盖 `.lzh`。
- 显示 `lh0`、`lh5`、`lh6`、`lh7` 等 method。
- 文件名编码按样本提供 CP932/Shift-JIS 策略。
- CRC 错误映射为 `CorruptArchive`。

备选方案：libarchive FFI 读取。

最后 fallback：lha/unar helper。

### 5.8 PMA

首选方案：没有已确认成熟纯 Rust crate。先实现探测和 unsupported diagnostic。

实现计划：

- 收集 PMarc 样本，明确 header、method、CRC。
- 如果格式范围可控，考虑自研只读解析器。
- 否则寻找 C/C++ 实现并封装 FFI。
- 旧编码路径必须经过统一 filename decoder。

备选方案：The Unarchiver/unar 源码库化评估。

最后 fallback：unar helper。

## 6. 安装包、软件包和应用容器

### 6.1 CAB

首选方案：纯 Rust `cab` crate。

实现计划：

- 新建 CAB 后端，使用 `cab::Cabinet` 列 folder/file entries。
- 支持 uncompressed、MSZIP、LZX decode；Quantum 标记为 unsupported codec。
- 解压时每个 file entry 流进 `SafeWriter`。
- 创建 CAB 可作为二阶段能力，先支持 MSZIP 或 uncompressed。
- 多 cabinet/cab chain 先检测并提示，不默认跨文件查找。

备选方案：libarchive FFI 读取 Microsoft CAB。

最后 fallback：expand.exe/cabextract helper，仅诊断模式。

### 6.2 Compound(MSI)

首选方案：纯 Rust `cfb` crate 读取 OLE Compound File。

实现计划：

- 新建 Compound/MSI 后端，把 storage 映射为目录，stream 映射为文件。
- 支持列 stream、提取 stream。
- MSI 数据库表解析可作为 metadata preview，不阻塞基础提取。
- 内嵌 CAB 提取：先从 CFB stream 中识别 CAB magic，再转交 CAB 后端。
- 创建 MSI 不承诺。

备选方案：Windows MSI API 不跨平台，不作为主路径；libmsi 可评估但优先级较低。

最后 fallback：lessmsi/msiextract helper。

### 6.3 DEB

首选方案：纯 Rust `ar` + tar/compression stream pipeline。

实现计划：

- `.deb` 先用 `ar::Archive` 解析 `debian-binary`、`control.tar.*`、`data.tar.*`。
- `control` 和 `data` payload 根据后缀选择 gzip/xz/zstd/bzip2 decoder。
- payload 必须直接进入 `tar::Archive`，全流式解压。
- listing 可以展示两层：包成员和 payload 内文件。
- metadata preview 读取 `control` 文件。

备选方案：libarchive FFI 可读 ar/deb 组合，但纯 Rust 足够。

最后 fallback：dpkg-deb helper，不作为默认路径。

### 6.4 XPI

首选方案：复用 ZIP 后端。

实现计划：

- `.xpi` 作为 ZIP 派生格式注册独立 `ArchiveFormat::Xpi` 或 alias。
- 后端直接调用 ZIP reader/writer，能力与 ZIP 一致。
- metadata preview 解析 `manifest.json` 或 legacy install manifest。
- 创建 XPI 只是在 ZIP 基础上增加文件名和 manifest 校验，不需要独立压缩实现。

备选方案：无。

最后 fallback：无默认二进制 fallback。

### 6.5 ASAR

首选方案：纯 Rust `asar` crate。

实现计划：

- 新建 ASAR 后端，解析 header JSON 生成 listing。
- 普通 packed file 支持 `open_entry_reader`。
- unpacked file 引用需要把 archive 路径旁的 unpacked 目录纳入 source model。
- integrity feature 打开后支持校验。
- 创建 ASAR 可用 `AsarWriter`，但需要处理 executable 标记和 unpacked 策略。

备选方案：`hive-asar` 可用于异步/Tokio 路径，但当前功能缺口需评估。

最后 fallback：asar npm CLI 不作为默认路径。

### 6.6 NSIS

首选方案：纯 Rust `nsis` crate。

实现计划：

- 新建 NSIS 后端，从 PE overlay 解析 NSIS 数据。
- 列 section、embedded file 和脚本 metadata。
- 若 `nsis` crate 已暴露文件数据，直接流式提取；若只可 inspect，则先作为 list/metadata 支持。
- solid 压缩标记高访问成本。
- 不支持版本明确报 `UnsupportedFormat` 或 `UnsupportedCodec`。

备选方案：libarchive FFI 有时可处理 NSIS installer，作为兼容路径。

最后 fallback：7zz/UniExtract helper。

## 7. 镜像和文件系统容器

### 7.1 ISO

首选方案：纯 Rust `hadris` ISO 模块。

实现计划：

- 支持 ISO9660、Joliet、Rock Ridge 和 El Torito metadata。
- 把文件系统目录映射为 archive listing。
- entry reader 直接读取 extent 数据。
- 创建 ISO 使用 libarchive ISO9660 writer 或自研最小 writer。
- 混合 ISO/UDF 先探测 UDF，再根据更丰富的文件名/metadata 选择展示。

备选方案：libarchive FFI 读取/创建 ISO9660。

最后 fallback：xorriso/7zz helper。

### 7.2 UDF

首选方案：纯 Rust `hadris` UDF feature。

实现计划：

- 支持 UDF 1.02 到 2.60 的读取路径。
- 列目录、提取文件、Unicode 文件名、大文件。
- 与 ISO 混合镜像共享 block device reader。
- 创建 UDF 暂不承诺，除非 `hadris` 或其他 crate 出现成熟 writer。

备选方案：libarchive FFI 若能读取目标样本，则作为兼容补充。

最后 fallback：7zz helper。

### 7.3 BIN

首选方案：纯 Rust自研 CUE parser + ISO/UDF reader。

实现计划：

- `.bin` 单独很难判断文件系统，优先查找同名 `.cue`。
- 解析 CUE 找到 MODE1/2048 或 MODE1/2352 数据轨。
- 对数据轨做 sector adapter，暴露为 ISO/UDF reader。
- 多轨音频不作为归档能力承诺。

备选方案：libcdio C library FFI，用于复杂光盘布局。

最后 fallback：bchunk/7zz helper。

### 7.4 IMG

首选方案：纯 Rust多探测：ISO/UDF/FAT。

实现计划：

- 对 `.img` 做 magic/offset 探测，不只看扩展名。
- ISO/UDF 交给 `hadris`。
- FAT 可用 `hadris` FAT 模块读取。
- 对未知文件系统只给诊断，不做块设备挂载。

备选方案：libarchive FFI 或 filesystem-specific C library。

最后 fallback：7zz helper。

### 7.5 ISZ

首选方案：无成熟纯 Rust方案。先实现探测和 unsupported diagnostic。

实现计划：

- 收集 UltraISO ISZ 样本，覆盖压缩、分段、加密。
- 调研 ISZ SDK 或 7-Zip/其他开源实现是否可库化。
- 若可 FFI，先解出虚拟 ISO block reader，再交给 ISO/UDF 后端。
- 加密 ISZ 先只做检测和错误提示。

备选方案：自研 decoder，成本较高。

最后 fallback：7zz/UltraISO helper。

### 7.6 DAA(1.0)

首选方案：无成熟纯 Rust方案。先实现 DAA 1.0 探测和 unsupported diagnostic。

实现计划：

- 收集 PowerISO DAA 1.0 样本。
- 若存在可接受 C/C++ decoder，封装为 virtual ISO reader。
- 分卷和加密先作为外部能力。
- 不支持非 1.0 变体必须明确提示。

备选方案：自研 decoder，成本较高且样本不足。

最后 fallback：PowerISO/7zz helper。

## 8. 单文件压缩流

### 8.1 BR

首选方案：纯 Rust `brotli` crate。

实现计划：

- `.br` listing 生成单个虚拟 entry。
- 解压使用 Brotli decoder stream。
- 创建使用 Brotli encoder stream，目录输入提示使用 `tar.br`。
- 截断流和参数错误映射为结构化错误。

备选方案：brotli C library FFI，通常不需要。

最后 fallback：无默认二进制 fallback。

### 8.2 BZ

首选方案：先确认 `.bz` legacy bzip 实际格式需求；若不是 bzip2，优先使用 libarchive FFI。

实现计划：

- `.bz` 与 `.bz2` 分开探测。
- 若样本实际为 bzip2，转交 BZ2 后端。
- 若为 legacy bzip，纯 Rust生态不稳定时使用 libarchive filter。
- 与 `.tar.bz` 组合时必须进入全流式 tar pipeline。

备选方案：自研 legacy bzip decoder。

最后 fallback：无默认二进制 fallback。

### 8.3 BZ2

首选方案：Rust `bzip2` crate。新版生态已有 Rust backend 方向，应优先配置 Rust backend；否则接受 libbz2 FFI。

实现计划：

- `.bz2` listing 生成单个虚拟 entry。
- `.tar.bz2`、`.tbz2` 转交 tar+bzip2 pipeline。
- 创建 `.bz2` 只接受单文件。
- CRC 和截断错误映射。

备选方案：libarchive FFI。

最后 fallback：无默认二进制 fallback。

### 8.4 GZ

首选方案：`flate2` Rust backend。

实现计划：

- 与第 4.3 共用单文件压缩流 adapter。
- `.tar.gz` 和 `.tgz` 不进入单文件 GZ 后端。

备选方案：zlib-ng FFI。

最后 fallback：无默认二进制 fallback。

### 8.5 XZ

首选方案：`xz2`/liblzma FFI。

实现计划：

- 与第 4.4 共用单文件压缩流 adapter。
- `.tar.xz` 和 `.txz` 不进入单文件 XZ 后端。

备选方案：`lzma-rust2` 评估。

最后 fallback：无默认二进制 fallback。

### 8.6 ZSTD

首选方案：`zstd` crate。

实现计划：

- `.zst`/`.zstd` listing 生成单个虚拟 entry。
- 解压和创建都使用 stream API。
- 字典缺失、skippable frame、checksum failure 做错误映射。
- `.tar.zst`/`.tzst` 不进入单文件 ZSTD 后端。

备选方案：libarchive FFI。

最后 fallback：无默认二进制 fallback。

### 8.7 LZ4

首选方案：纯 Rust `lz4_flex`，若需要完整 frame 兼容可评估 `lz4`/liblz4 FFI。

实现计划：

- 支持 LZ4 frame 格式，不把 raw block 误认为 archive。
- listing 生成单个虚拟 entry。
- 支持 block/content checksum 校验。
- 创建 `.lz4` 只接受单文件。

备选方案：liblz4 FFI。

最后 fallback：无默认二进制 fallback。

### 8.8 LZ

首选方案：`lzip` crate，它是 lzlib binding，提供 reader/writer stream。

实现计划：

- `.lz` listing 生成单个虚拟 entry。
- 解压和创建都走 stream。
- 支持 multi-member。
- `.tar.lz` 进入 tar+lzip 全流式 pipeline。

备选方案：libarchive FFI 支持 lzip filter。

最后 fallback：无默认二进制 fallback。

### 8.9 LZMA

首选方案：纯 Rust `lzma-rust2`。

实现计划：

- 支持 LZMA-alone stream。
- listing 生成单个虚拟 entry。
- `.tar.lzma`/`.tlz` 进入 tar+lzma 全流式 pipeline。
- 参数不兼容映射为 `UnsupportedCodec` 或 `CorruptArchive`。

备选方案：`xz2`/liblzma FFI。

最后 fallback：无默认二进制 fallback。

### 8.10 Z

首选方案：评估 `unarc-rs` 的 `z`/`tarz` 模块；若不足，使用 libarchive compress/LZW filter FFI。

实现计划：

- `.Z` listing 生成单个虚拟 entry。
- `.tar.Z` 进入 tar+Unix compress 全流式 pipeline。
- 支持 LZW reset/clear code 错误诊断。
- 创建 `.Z` 通常不建议作为新能力，除非用户明确需要旧系统兼容。

备选方案：移植 Rust coreutils/posixutils 的 LZW compress 实现。

最后 fallback：无默认二进制 fallback。

## 9. tar 派生格式

### 9.1 TBZ

首选方案：`tar` + BZ/BZ2 decoder。实际样本常等同 bzip2。

实现计划：

- `.tbz` 和 `.tar.bz` 进入 tar+bzip decoder。
- 解压、列表、创建都必须全流式。
- 若 `.tar.bz` 使用 legacy bzip，不支持时明确报错。

备选方案：libarchive FFI。

最后 fallback：无默认二进制 fallback。

### 9.2 TBZ2

首选方案：`tar` + `bzip2` crate。

实现计划：

- `.tbz2` 和 `.tar.bz2` 进入 tar+bzip2 decoder。
- 创建时 `tar::Builder<BzEncoder<File>>`。
- CRC/截断错误映射。

备选方案：libarchive FFI。

最后 fallback：无默认二进制 fallback。

### 9.3 TLZ

首选方案：`tar` + `lzma-rust2` 或 `xz2` LZMA-alone decoder。

实现计划：

- `.tlz` 和 `.tar.lzma` 进入 tar+lzma decoder。
- 创建时 `tar::Builder<LzmaEncoder<File>>`。
- LZMA 参数错误要在打开阶段尽早提示。

备选方案：libarchive FFI。

最后 fallback：无默认二进制 fallback。

## 10. 高级归档和备份格式

### 10.1 WIM

首选方案：`wimlib` Rust binding。它绑定 C library wimlib，支持 WIM 创建、修改、提取和挂载相关能力。

实现计划：

- 新建 WIM 后端，打开 WIM 后列 image index。
- listing 需要带 image id/name，路径层次为 `/image-name/path` 或 UI 虚拟根。
- 提取指定 image 或 selected entries 使用 wimlib extraction API。
- split WIM/SWM、solid resource、pipable WIM 建样本测试。
- 创建/修改 WIM 可作为高级能力，不进入 MVP。

备选方案：libarchive FFI 若读取能力满足样本。

最后 fallback：wimlib-imagex helper。

### 10.2 ZPAQ

首选方案：`zpaq_rs`。它是 Rust safe binding，构建时编译 C++ shim 和 libzpaq，运行时不需要动态库。

实现计划：

- 新建 ZPAQ 后端，支持 list、extract、test。
- 快照/版本模型需要映射到 virtual directory 或 metadata。
- 创建/append 可作为高级能力，但默认不建议生成 ZPAQ。
- 密码和多线程选项必须在 capability 中表达。

备选方案：自建 libzpaq FFI。

最后 fallback：zpaq helper。

### 10.3 PEA

首选方案：未发现成熟 Rust crate。先实现探测和 unsupported diagnostic。

实现计划：

- 调研 PeaZip/PEA 格式源代码是否可作为 C/C++ library 嵌入。
- PEA 强调 Pack/Encrypt/Authenticate，错误模型必须区分密码错误、认证失败、损坏数据。
- 若无法进程内库化，标记为 external-only。

备选方案：自研读取器，前提是格式文档和测试样本充分。

最后 fallback：pea/PeaZip helper。

### 10.4 AES

首选方案：先明确 Bandizip 列表中的 `AES` 具体容器。不要把它和 ZIP AES 混为一谈。

实现计划：

- 增加格式探测 spike：收集 Bandizip 可解的 `.aes` 样本。
- 若是标准 AES Crypt 格式，可使用 Rust `aes`、`cbc`/`ctr`、`sha2`/`hmac` 等 crypto crates 自研 reader。
- listing 生成单个虚拟 entry。
- 密码错误、认证失败、padding 错误必须区分。

备选方案：C library FFI 仅在格式确认后评估。

最后 fallback：aescrypt helper。

## 11. 编码和传输封装

### 11.1 UU

首选方案：自研纯 Rust decoder，格式简单。

实现计划：

- 解析 `begin mode filename` header。
- 支持多段文本、不同换行。
- listing 生成 header 中的 filename。
- 解码输出到单个虚拟 entry。
- 创建 uuencode 可作为可选编码功能，不进入归档主线。

备选方案：libarchive FFI 支持 uuencoded pre-filter。

最后 fallback：无默认二进制 fallback。

### 11.2 UUE

首选方案：与 UU 共用自研 decoder。

实现计划：

- `.uue` 作为 uuencode 扩展名别名。
- 支持文本编码容错和换行容错。
- 错误块映射为 `CorruptArchive`。

备选方案：libarchive FFI。

最后 fallback：无默认二进制 fallback。

### 11.3 XXE

首选方案：自研纯 Rust decoder。

实现计划：

- 解析 xxencode alphabet 和 header。
- listing 生成单个虚拟 entry。
- 支持多段文本。
- 损坏块映射为 `CorruptArchive`。

备选方案：寻找小型 C implementation 并内嵌 FFI。

最后 fallback：无默认二进制 fallback。

## 12. 实施分层建议

建议新增三类 crate，而不是把所有格式塞进现有后端。

- `archive-stream-codecs`：单文件压缩流和 tar filter 链，覆盖 `gz`、`bz2`、`xz`、`zst`、`lz4`、`br`、`lz`、`lzma`、`Z`。
- `archive-container-extra`：纯 Rust 容器格式，覆盖 `cab`、`deb`、`asar`、`cfb/msi`、`iso/udf`、`lha/lzh`、`ace/arj`。
- `archive-ffi-extra`：进程内 C/C++ wrapper，覆盖 `libarchive`、`wimlib`、`zpaq`、潜在 ALZ/EGG/ISZ/DAA/PEA 库。

后端选择顺序：

1. 专用纯 Rust 后端。
2. 专用 FFI 后端。
3. libarchive FFI 兼容后端。
4. 外部二进制 helper。

## 13. 优先实现批次

第一批，收益高且库生态清晰：

- CAB：`cab`。
- DEB：`ar` + tar stream。
- ASAR：`asar`.
- MSI/Compound：`cfb`。
- ISO/UDF/IMG：`hadris`。
- LHA/LZH：`delharc`。
- BR/BZ2/GZ/XZ/ZSTD/LZ4/LZ/LZMA/Z：stream codec。
- TBZ/TBZ2/TLZ：tar + stream codec。

第二批，使用 FFI 但价值较高：

- WIM：`wimlib`。
- ZPAQ：`zpaq_rs`。
- ISO creation：libarchive writer 或自研 writer。
- ZIP advanced/split：minizip-ng 或 `unarc-rs` 补充。

第三批，需样本和格式调研：

- ACE/ARJ：先评估 `unarc-rs`，满足则进主线。
- NSIS：`nsis` 可先提供 inspect/list，再确认提取能力。
- BIN：CUE parser + ISO/UDF bridge。

第四批，暂列 external 或 unsupported：

- ALZ、EGG、BH、PMA、ISZ、DAA、PEA、AES。

这些格式在没有可维护库和样本库前，不应伪装成正式支持。正确做法是先实现准确探测、清晰错误、诊断日志和可选 helper 配置。

## 14. 参考来源

- libarchive README: 支持读取 tar、ISO9660、ZIP/ZIPX、7z、CAB、LHA/LZH、RAR/RAR5，并支持 gzip、bzip2、compress/LZW、lzma/lzip/xz、lz4、zstd filters；也支持创建 ZIP/ZIPX、ISO9660、7z 等。
- `sevenz-rust2` docs: 支持 COPY、LZMA、LZMA2、Brotli、BZIP2、DEFLATE、PPMD、LZ4、ZSTD 等 codec 和 BCJ/Delta filters。
- `hadris` docs: 提供 ISO9660/Joliet/Rock Ridge/El Torito、FAT、UDF、CPIO 支持。
- `cab` docs: 纯 Rust CAB 读写库，支持 uncompressed、MSZIP，并支持 LZX 解码。
- `cfb` docs: 纯 Rust Compound File Binary 读写库。
- `ar` docs: 纯 Rust Unix ar 读写库，支持 Debian package 使用的 common variant。
- `asar` docs: 纯 Rust ASAR 读写库，支持 Electron ASAR。
- `nsis` docs: 纯 Rust NSIS installer parser。
- `delharc` docs: 纯 Rust LHA/LZH 解析和解压库。
- `unarc-rs` docs: unified API 覆盖 ACE、ARJ、LHA/LZH、BZ2、GZ、TAR.Z、TBZ、TGZ、ZIP、7z、RAR 等。
- `wimlib` docs: Rust binding 到 wimlib C library，支持 WIM 创建、修改、提取等。
- `zpaq_rs` docs: Rust safe binding 到 libzpaq，通过 C++ shim 静态链接。
- `lzip` docs: lzlib binding，提供 lzip reader/writer streams。
