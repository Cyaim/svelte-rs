# sv-pag

**纯 Rust 读腾讯 PAG(Portable Animated Graphics)容器。零依赖,不绑 libpag。**

只做一件事:把 `.pag` 的**容器**拆开 —— 取出宽高/帧率/时长,判定它是矢量档还是
位图序列帧档,并在是序列帧档时把**每帧的编码后字节**原样交出来。

---

## 头号缺口:从未在真实 `.pag` 上验证过

**本解析器没有在 AE 插件真实导出的 `.pag` 文件上跑过,一次都没有。**

全部测试固件都是按 libpag 源码**手工构造**的字节序列(仓库里不存任何 `.pag`
二进制资产)。所有字段布局都能指到 libpag 的源码行(见下面的核实表),而且
其中绝大多数还被 libpag **自己的第二份实现**(TypeScript 版)独立印证过 ——
但"照着两份源码读对了"和"真文件能打开"仍然是两件事。

**接入前必须做的第一件事:拿一个真实导出的 `.pag`(矢量档和位图序列帧档各一个)
对拍。** 在那之前,本 crate 的结论都带这个前提。

单独标出**只有一个来源**的那一条:`BitmapSequence`(标签 46)的字段布局
只有 C++ 的 `src/codec/tags/BitmapSequence.cpp` 一份出处 —— TypeScript 版
(libpag-lite)**没有** `bitmap-sequence.ts`,因为它自述"仅支持播放包含单独一个
BMP **视频**序列帧的 PAG 动效文件"。也就是说本 crate 最核心的那个结构,
交叉印证是缺位的。

---

## 为什么不绑 libpag(以及这条论据管到哪为止)

`docs/plans/pag-2-integration.md` §0 的裁决:**运行期绑定 libpag 是硬否。**

- libpag → tgfx 的 C++ 依赖闭包里有 **`pathkit`**(仓库自述 *"This library is
  extracted from the Skia library"*)与 **`skcms`** —— ADR-3 排除 skia-safe 的
  理由在这里原样复现且更重;
- 官方**不发 Windows/Linux 预编译库**(README 原文:"We currently only publish
  precompiled libraries for iOS, Android, macOS, Web, and OpenHarmony"),
  而 Windows 是本仓库的第一顺位平台;
- 依赖同步靠 Node.js 写的 `depsync` 在**构建期联网 git clone** 二十多个仓库 ——
  `cargo vendor` / 离线构建 / 发 crates.io 全部失效。

### ⚠️ 这条论据只管**运行期** —— 本节早期版本漏了这个限定,现补上

计划文件对上面第二条**自己加了限定**,原文(`docs/plans/pag-2-integration.md:28`):

> **注意这条只管运行期**:构建期另有 wasm 通道,见 §3.3 c2′

同文件 `:175` 记着 npm **`libpag` 4.5.81 是预编译 wasm、Apache-2.0、吃真 `.pag`、
走完整 codec、不需要任何 C++ 工具链**,并明写这是「"Windows/Linux 无预编译库"
这条结论**在构建期不成立**的原因」。

**所以本 crate 不能说"c2 根本不需要 libpag"。** 早期版本的这句话越过了计划文件
支持的范围。它能说的是这句窄得多的话:

> 计划里 c2′ 的首选方案(Node + 无头 GL / puppeteer 跑 wasm libpag,逐帧 seek
> 出帧)带着一个**计划自己标为"开工前第一个 spike,不许跳过"的未核实前提**:
> npm `libpag` 的 wasm 构建在纯 Node 下能否离屏渲染。
> 而对**已经按位图序列帧模式导出**的素材,帧数据本来就在 `.pag` 里 ——
> 这一档可以完全绕开那条链路,只需要我们自己会读这个容器。

**这一档有多大,由素材决定,不由本 crate 决定。** 能不能绕开,一个文件一个文件地
用 `PagFile::is_pure_bitmap()` 问(**不是** `kind()`,理由见下一节)。

**格式侧的核实结论:能读。** 结构完全公开在 Apache-2.0 的 `src/codec` 里,
布局并不复杂,帧字节是完整的、自描述的编码图片(实测导出侧写 WebP),
取出来直接能喂解码器。

---

## `kind()` 不是判据,`is_pure_bitmap()` 才是

`kind()` 回答的是「文件里**出现过**哪些序列类型」。它对下面两个文件返回
**同一个** `FileKind::Bitmap`:

| 文件 | `kind()` | `is_pure_bitmap()` |
|---|---|---|
| 只有一个 `BitmapCompositionBlock` | `Bitmap` | `true` |
| 矢量根 composition + 位图子 composition | `Bitmap` | **`false`** |

第二种上,矢量根可以对位图子档施加**变换 / 蒙版 / 混合模式** —— 而这些全在
`LayerBlock` 里,本 crate **有意不解析 `LayerBlock`**,于是在**原理上**无从判断
能不能忽略它们。同一个文件上 `attributes()`(主 = 矢量根,比如 1920x1080@30)
与 `bitmap_sequences()`(子档,比如 4x4@24)返回的还是**两个不同 composition
的数据**。

`is_pure_bitmap()` 取最保守的一档:**文件里每一个 composition 都是位图档**才为真。
只要出现过任何矢量或视频 composition 就返回 `false`,哪怕它其实是个空壳。
宁可少支持。

---

## 能读什么 / 不能读什么

| 能 | 不能 |
|---|---|
| 文件头:魔数 / 版本 / 体长度 / 压缩标志 | 加密档(版本 3)—— **显式报错**,不尝试解析 |
| 标签流(SWF 风格 TLV),含 uint32 长体逃逸 | ZLIB / LZMA 压缩体 —— **显式报错**(libpag 自己也只接受未压缩) |
| `CompositionAttributes`:宽 / 高 / 时长(帧)/ 帧率 / 背景色 | 矢量图层:形状、文本、蒙版、效果、变换、关键帧 —— **完全不解析** |
| 文件里有哪些序列类型(`kind()`);能否只靠重放序列还原(`is_pure_bitmap()`) | 视频序列(H.264)—— 只计数,**不解析** |
| 位图序列:每帧 `isKeyframe` + 每个脏矩形的 `(x, y)` + **编码字节** | `ImageBytes` / `FontTables` / `EditableIndices` 等 —— 按长度跳过 |
| 帧字节的编码嗅探(WebP / PNG / JPEG / 未知) | **不解码图片** —— 本 crate 零图像依赖,解码留给上层 |

**设计纪律:没核实过布局的标签一律按长度跳过,绝不猜着解析。**
这不是偷懒 —— 编造字段偏移会产出"能编译、能跑、悄悄解错"的结果,
比解析失败坏得多。

---

## 外部事实核实表

**核实日期 2026-07-22。** 主来源是 libpag 仓库 `main` 分支
commit **`7bdb1380976ee6294038c2814e69a6d0cfb7a535`**(2026-07-22T08:40:55Z),
最新 release v4.5.81。全部源码经 GitHub API 取原文后**逐字阅读**,不是搜索摘要。

「交叉印证」一列指 libpag 自带的第二份独立实现:
`web/lite/src/`(纯 TypeScript 的 libpag-lite)。两份实现由同一个团队维护但代码
互不共享,同一事实两边一致时可信度显著更高。

### 文件头

| 事实 | 出处 | 交叉印证 | 状态 |
|---|---|---|---|
| 文件最短 11 字节,否则拒绝 | `src/codec/Codec.cpp` `ReadBodyBytes`:`if (stream->length() < 11)` | `web/lite/src/pag-codec.ts:73` `if (byteArray.length < 11)` | ✅ |
| 魔数是 3 个字节 `'P' 'A' 'G'` | 同上:`readInt8() ×3`,`if (P != 'P' \|\| A != 'A' \|\| G != 'G')` | 同上 `:77` `if (P !== 80 \|\| A !== 65 \|\| G !== 71)` | ✅ |
| 版本 = 1 字节 uint8 | 同上:`auto version = stream->readUint8();` | 同上 `:78` `byteArray.readInt8(); // version` | ✅ |
| **版本 3 = 加密档**,开源解码器拒绝 | 同上:`static const uint8_t EncryptedVersion = 3;` → `PAGThrowError(..., "Encrypted PAG file")` | lite 版不校验版本 | ✅(单源) |
| 已知最大版本 = 3,更大即拒 | 同上:`static const uint8_t KnownVersion = 3;` → `if (version > KnownVersion)` | lite 版不校验 | ✅(单源) |
| 编码器当前写版本 **1** | `src/codec/Version.h`:`static const uint8_t Version = 1;` | — | ✅ |
| 体长度 = uint32 **小端** | `Codec.cpp`:`auto bodyLength = stream->readUint32();` | `pag-codec.ts:79` `byteArray.readUint32()` | ✅ |
| 压缩标志 = 1 字节;只接受 `'U'` | `Codec.cpp`:`if (compression != CompressionAlgorithm::UNCOMPRESSED)` | `pag-codec.ts:80`(读但不校验) | ✅ |
| `UNCOMPRESSED='U'` / `ZLIB='Z'` / `LZMA='L'` | `src/codec/CompressionAlgorithm.h` | — | ✅ |
| 体长度**大于**剩余时按剩余截断,**不报错** | `Codec.cpp`:`bodyLength = std::min(bodyLength, stream->bytesAvailable());` | — | ✅ |
| 全文件字节序恒为**小端** | `src/codec/utils/DecodeStream.h` 类注释:"The byte order of DecodeStream is always little-endian." | TS `ByteArray` 带 `littleEndian` 标志 | ✅ |
| 空 composition 列表 = 无效文件 | `Codec.cpp` `VerifyAndMake`:`bool success = !compositions.empty();` | `pag-codec.ts` `verifyAndMake` | ✅ |

### 标签(TLV)编码

| 事实 | 出处 | 交叉印证 | 状态 |
|---|---|---|---|
| 标签头 = uint16 小端;`length = v & 63`,`code = v >> 6` | `src/codec/TagHeader.cpp` `ReadTagHeader` | `web/lite/src/codec/tags/tag-header.ts:11-13` 逐字一致 | ✅ |
| `length == 63` 是**逃逸标记**,再读一个 uint32 作真长度 | 同上:`if (length == 63) { length = stream->readUint32(); }` | 同上 `:14-16` | ✅ |
| 写侧判据是 `length < 63`(体长恰好 63 也走逃逸) | `TagHeader.cpp` `WriteTypeAndLength` | — | ✅ |
| 结束标签 = uint16 `0`(code 0 + length 0) | `TagHeader.cpp` `WriteEndTag`:`stream->writeUint16(0);` + `TagCode::End = 0` | `tag-header.ts:25` `while (header.code !== TagCode.End)` | ✅ |
| 标签循环:读头 → 切出 length 字节**子流** → 分发 → 直到 End | `src/codec/TagHeader.h` 模板 `ReadTags` | `tag-header.ts:23-33` 结构一致 | ✅ |
| 标签号最大 1023(10 位) | 由 `code = v >> 6` 推出,uint16 高 10 位 | — | ✅ |
| `TagCode` 全表(End=0 … ImageScaleModes=94,34~44 保留,无 9) | `include/pag/file.h` `enum class TagCode` | — | ✅ |

### 数值原语

| 事实 | 出处 | 交叉印证 | 状态 |
|---|---|---|---|
| `EncodedUint32` = LEB128 变长,每字节低 7 位数据(**低位在前**),高位=续位,**最多 5 组** | `src/codec/utils/DecodeStream.cpp` `readEncodedUint32`:`for (int i = 0; i < 32; i += 7)` | `web/lite/src/codec/utils/byte-array.ts:171-189` 逐字一致 | ✅ |
| 第 5 组仍带续位时**不报错**,高位丢弃,游标已推进 5 字节 | 同上(for 循环自然结束,无异常分支) | 同上 | ✅ |
| `EncodedInt32` = **最低位是符号位(1=负)**,其余位是绝对值 —— **不是标准 zigzag** | 同上 `readEncodedInt32`:`value = data >> 1; return (data & 1) > 0 ? -value : value;` | `byte-array.ts:191-195` 逐字一致 | ✅ |
| `EncodedUint64` 最多 10 组 | 同上 `readEncodedUint64`:`for (int i = 0; i < 64; i += 7)` | `byte-array.ts:197-` | ✅ |
| 位读 `readUBits` **字节内低位在前**;掩码表 `{0,1,3,7,15,31,63,127,255}` | 同上 `readUBits` | `byte-array.ts:232-249` 逐字一致 | ✅ |
| `readBitBoolean() = readUBits(1) != 0` | `DecodeStream.h` 内联定义 | `byte-array.ts:280-282` | ✅ |
| **字节读之后位游标对齐回字节边界**:`_bitPosition = _position * 8` | `DecodeStream.cpp` `positionChanged` | `byte-array.ts:297-299` `positonChanged` | ✅ |
| **位读之后字节游标向上取整**:`_position = ceil(bits/8)` | `DecodeStream.cpp` `bitPositionChanged` + `src/codec/utils/StreamContext.h`:`BitsToBytes(c) = ceil(c * 0.125)` | `byte-array.ts:293-295` `Math.ceil(this.bitPosition * 0.125)` | ✅ |
| `readBoolean` 读**整整 1 字节**(不是 1 bit) | `DecodeStream.cpp` `readBoolean`:`getBoolean(_position); positionChanged(1);` | — | ✅ |
| `readByteData` = `EncodedUint32` 长度 + 该长度的原始字节 | `DecodeStream.cpp` `readByteData` | — | ✅ |
| `float` = 4 字节小端 | `DecodeStream.cpp` `readFloat`:`getFloat(_position); positionChanged(4);` | `byte-array.ts` `readFloat32` | ✅ |

### composition 与序列帧

| 事实 | 出处 | 交叉印证 | 状态 |
|---|---|---|---|
| `VectorCompositionBlock`(2)体 = `id:EncodedUint32` + 子标签流 | `src/codec/tags/VectorCompositionTag.cpp` `ReadVectorComposition` | `web/lite/.../vector-composition-tag.ts:16` | ✅ |
| `BitmapCompositionBlock`(45)体 = `id:EncodedUint32` + 子标签流 | `src/codec/tags/BitmapCompositionTag.cpp` `ReadBitmapComposition` | — | ✅(单源) |
| `VideoCompositionBlock`(50)体 = `id:EncodedUint32` + **`hasAlpha:1 字节 Boolean`** + 子标签流 | `src/codec/tags/VideoCompositionTag.cpp` `ReadVideoComposition` | `.../video-composition-tag.ts:10-11` 逐字一致 | ✅ |
| `CompositionAttributes`(3)= `EncInt32 宽` / `EncInt32 高` / `EncUint64 时长` / `float32 帧率` / `3×uint8 RGB` | `src/codec/tags/CompositionAttributes.cpp` + `src/codec/DataTypes.cpp` `ReadTime`(`readEncodedUint64`)、`ReadColor`(3×`readUint8`) | `.../composition-attributes.ts:5-11` 逐字一致 | ✅ |
| 时长单位是**帧**不是毫秒 | `ReadTime` 返回 `Frame` 类型 | — | ✅ |
| 背景色**只有 RGB 三分量**,无 alpha | `DataTypes.cpp` `ReadColor` 只读 3 个 uint8 | — | ✅ |
| **主 composition = 列表最后一个**,`File::width/height/duration/frameRate` 都取自它 | `src/base/File.cpp:85`:`mainComposition = compositions.back();` 及 `:150-168` | — | ✅ |
| `BitmapSequence`(46)= `EncInt32 宽` / `EncInt32 高` / `float32 帧率` / `EncUint32 帧数` / **帧数个 bit 的 isKeyframe** / 然后每帧 `EncUint32 块数` + 每块 `EncInt32 x` `EncInt32 y` `ByteData` | `src/codec/tags/BitmapSequence.cpp` `ReadBitmapSequence`(全文逐字读过) | **无** —— libpag-lite 没有 bitmap-sequence.ts | ⚠️ **单源** |
| isKeyframe 是**先连着写完所有帧**,再写各帧块数据(两趟,不是交错) | 同上:两个独立的 `for (uint32_t i = 0; i < count; i++)` 循环 | — | ⚠️ 单源 |
| 一个 composition 可有**多档**分辨率的序列,按 width 升序 | `BitmapCompositionTag.cpp` `lessFirst`:`item1->width < item2->width` | — | ✅ |
| 帧字节由 AE 导出插件编成 **WebP** | `exporter/src/export/sequence/BitmapSequence.cpp`:`#include <webp/encode.h>`、`EncodeImageData(...)`、失败报 `AlertInfoType::WebpEncodeError` | — | ✅ |
| 但**容器不记录编码**,播放侧走泛用嗅探 | `src/rendering/sequences/BitmapSequenceReader.cpp:77`:`tgfx::ImageCodec::MakeFrom(imageBytes)` | — | ✅ |
| 块的**宽高不在容器里**,要解码后才知道 | 同上:用 `codec->width()` / `codec->height()` | — | ✅ |
| **帧是差分的**:非关键帧只带脏矩形,须从最近关键帧起逐帧重放 | 同上 `findStartFrame` + 贴图循环;导出侧 `diffRect` / `ExpandRectRange` | — | ✅ |
| 关键帧的首块尺寸小于画布时先清屏 | `BitmapSequenceReader.cpp`:`if (firstRead && bitmapFrame->isKeyframe && !(codec->width() == pixmap.width() && ...)) pixmap.clear();` | — | ✅ |
| libpag 有 WebP 解析工具:`RIFF_HEADER_SIZE 12 // "RIFFnnnnWEBP"` | `src/codec/utils/WebpDecoder.h` | — | ✅ |

### 零长度块与 `verify()` 门(**订正区**)

> 🔴 **早期版本这里有一条错的 ✅**:「零长度块 = 合法的"空帧",不是错误」,
> 出处引的是 `BitmapSequenceReader.cpp:78` 的注释
> *"The returned image could be nullptr if the frame is an empty frame."*
> —— 打开原文后那句注的是**第 77 行 `tgfx::ImageCodec::MakeFrom` 的返回值**
> (解码器可能返回 null),它没有、也不可能在说"零长度 `ByteData` 是合法数据"。
> **那条 ✅ 是没挣来的,已删。** 真实行为是下面这几行,链条方向恰好相反。

| 事实 | 出处 | 交叉印证 | 状态 |
|---|---|---|---|
| `readByteData` 对 `length == 0` 返回 **`nullptr`**(与"越界"同一个返回值) | `src/codec/utils/DecodeStream.cpp:147`:`if (length == 0 \|\| length > bytes.length() \|\| ...) return nullptr;` | — | ✅ |
| `BitmapFrame::verify()` 要求每个块 `fileBytes != nullptr` ⇒ **零长度块让整个文件不合法** | `src/base/BitmapSequence.cpp:34-39` | — | ✅ |
| 写侧**从不产出**零长度块 | `src/codec/tags/BitmapSequence.cpp:64-80`:计数与写入两处都 `if (bitmap->fileBytes->length() == 0) continue;` | — | ✅ |
| 真正的"空帧"是导出器 bug 导出的 **1×1 WebP**,不是零长度块 | `BitmapSequence::isEmptyBitmapFrame`(`src/base/BitmapSequence.cpp:47-72`):`length() > 150` 或 `WebPGetInfo` 宽高 > 1 就不算空帧 | — | ✅ |
| `Codec::VerifyAndMake`:任一 composition `verify()` 不过 ⇒ delete 全部、返回 `nullptr`(**整个文件被拒**) | `src/codec/Codec.cpp:150-176` | — | ✅ |
| `Composition::verify()`:`width > 0 && height > 0 && duration > 0 && frameRate > 0` | `src/base/Composition.cpp:54-60` | — | ✅ |
| `Sequence::verify()`:`composition != nullptr && width > 0 && height > 0 && frameRate > 0` | `src/base/Sequence.cpp:39-41` | — | ✅ |
| `BitmapSequence::verify()`:额外要求 `frames` 非空 | `src/base/BitmapSequence.cpp:74-85` | — | ✅ |
| `BitmapComposition::verify()` / `VideoComposition::verify()`:额外要求 `sequences` 非空 | `src/base/BitmapComposition.cpp`、`src/base/VideoComposition.cpp` | — | ✅ |
| `VectorComposition::verify()`:额外要求**每个 `Layer` 都 verify 通过** | `src/base/VectorComposition.cpp` | — | ✅(**我们检不了,见差异表**) |

### 未核实 / 明确不做

| 项 | 状态 |
|---|---|
| `VideoSequence`(51)的字段布局 | **未核实** —— 只计数,不解析 |
| `ImageBytes` / `ImageBytesV2` / `ImageBytesV3`(47/48/49)布局 | **未核实** —— 按长度跳过 |
| `LayerBlock`(5)及全部矢量子标签(形状/文本/蒙版/效果)布局 | **未核实** —— 按长度跳过 |
| `FontTables` / `EditableIndices` / `FileAttributes` / `ImageScaleModes` 等布局 | **未核实** —— 按长度跳过 |
| ZLIB / LZMA 压缩体的封装方式 | **未核实**(libpag 自己也不接受)—— 显式报错 |
| 加密档(版本 3)的加密方式 | **未核实**(企业版特性)—— 显式报错 |
| 官方 `pag.io/docs/en/pag-spec.html` 的 **PDF 规范正文** | **未读** —— 本 crate 的依据全部是源码,不是那份 PDF |
| 真实 `.pag` 是否与源码推出的布局一致 | **未验证** —— 见顶部头号缺口 |
| 现实中是否存在版本 0 / 2 的 `.pag` | **未核实** —— 我们照 libpag 一并接受 |

---

## 与 libpag 的行为差异(有意为之,全部列出)

> 上一版这张表自称"全部列出"却漏了三条(`verify()` 门整个缺位、
> `start_frame_for` 的 `None`、`read_ubits(0)`),还有一条(零长度 `ByteData`)
> 把方向写反了。已全部补上/订正 —— 订正过的行标 🔴。

| 处 | libpag | sv-pag | 为什么 |
|---|---|---|---|
| 越界读 | 记一条 exception + **返回 0 继续跑**,靠调用方在循环边界补查 | **立刻返回 `Err`** | 半路返回零值再事后补查,会让"解出来的东西"和真实字节悄悄脱钩,是二进制解析器最难查的一类 bug。仓库 R4 去 panic 纪律也要求有明确失败点 |
| 帧数上界 | **无检查**,畸形 `count` 会先分配一大批对象再退出 | 分配前先卡:`ceil(count/8) + count ≤ 剩余字节` | 一个 5 字节的畸形 varint 否则能让我们为 40 亿个帧结构体分配内存 |
| 每帧块数上界 | 无检查 | 分配前先卡:`块数 × 3 ≤ 剩余字节` | 同上;每块至少 3 字节(x/y/长度三个 varint) |
| 预留容量 | 无 | `Vec::with_capacity` 按 4096 个元素封顶 | **声称的**数字不能直接换成一次大分配;真要长到那么大,得先逐字节拿出真实数据 |
| 标签体长溢出 | `readBytes` 抛异常后返回空流,继续走一轮才退出 | 单独的 `TagLengthOverflow` 错误,立刻退出 | 标签头的长度是攻击者可控的 u32,是最经典的越界入口,值得单列 |
| 🔴 零长度 `ByteData` | `readByteData` 返回 `nullptr` → `BitmapFrame::verify()` 失败 → **整个文件被拒**;写侧也从不产出零长度块 | **照做**:报 `VerifyFailed { reason: EmptyFrameBytes }` | 上一版这行写反了(说 libpag 把零长度和越界"混在同一个返回值里,我们分开"),于是把 libpag 眼里的**致命错误**当成正常数据放行,上层会静默画个空而不是报"文件损坏" |
| 🔴 `verify()` 门 | 逐 composition 查宽高/时长/帧率/序列非空/帧非空/块字节非空,任一不过整个文件被拒 | **照做**(`PagError::VerifyFailed`) | 上一版整个缺位。不拒的后果很具体:`seq.width` 是裸 `i32`,`-4i32 as usize` = 18446744073709551612 —— 我们的"不 panic"不能靠把 panic 转移给调用方实现 |
| `verify()` 里**我们检不了**的两条 | `VectorComposition::verify()` 还要每个 `Layer` 都过;`Composition::verify()` 还查 `audioBytes` 长度 | **不检**(矢量档上我们的门比 libpag **松**) | `LayerBlock` / `AudioBytes` 的布局本 crate 未核实、不解析。这是"只解容器"的直接后果,如实记在这里 |
| 🔴 `start_frame_for` 找不到关键帧 | `findStartFrame`(`BitmapSequenceReader.cpp:113-122`)把 `startFrame` 初始化为 `0`,不命中就**从第 0 帧重放** | 返回 `None` | 真实文件第 0 帧必是关键帧,这条在合法输入上不可见;但拿 0 硬顶等于对畸形序列悄悄编一个答案 |
| 🔴 `readUBits(0)` | 返回 0 并照常重算游标 | 报 `BadBitCount` | 目前不可达(内部只有 `read_bit_bool` 调它,固定传 1),但 0 位宽是调用错误不是数据,不该有静默返回值 |
| varint 超长(第 5 组仍带续位) | 静默截断,游标推进 5 字节 | **照抄** | 报错会拒掉 libpag 能打开的文件;照抄能保证消耗字节数逐字节一致 —— 对容器解析器,游标不跑偏比拒绝畸形值更重要 |

### 内存放大:线性,但常数是约 32×(如实写出来)

上面的帧数上界只保证**线性**,不保证常数小:

- `size_of::<BitmapFrame>() == 32`(`bool` + `Vec` 的 24 字节),
  `size_of::<BitmapRect>() == 24`(`i32` + `i32` + `&[u8]` 胖指针);
- `ceil(count/8) + count ≤ 剩余字节` 只把 `count` 压到 ≤ 剩余**字节数**,
  于是 N 字节的标签体最多能驱动 **32N 字节**的 `frames`。

也就是说一个精心构造的 10 MB `.pag` 能在第一个 EOF 之前吃掉几百 MB。
**我们不再往下加绝对帧数上界** —— 那要么拍一个没有依据的数字,要么冒拒掉合法
长动画的风险。做的是两件事:把常数写在这里和 `parse.rs` 的注释里;
预留容量按 4096 封顶,让分配跟着**实读**走而不是跟着声称的数字走。

---

## 用法

```rust
use sv_pag::PagFile;

let bytes = std::fs::read("loading.pag")?;
let pag = PagFile::parse(&bytes)?;

if let Some(a) = pag.attributes() {
    // 注意这是**主 composition** 的属性,不一定是下面某条序列的属性
    println!("{}x{} @ {}fps, {} 帧", a.width, a.height, a.frame_rate, a.duration_frames);
}

// 是 is_pure_bitmap() 而**不是** kind() == FileKind::Bitmap ——
// 后者对"矢量根套位图子 composition"的混合档同样为真,而那种档
// 不能只靠重放序列还原(矢量根的变换/蒙版在 LayerBlock 里,我们不解析)。
if pag.is_pure_bitmap() {
    for seq in pag.bitmap_sequences() {
        for (i, frame) in seq.frames.iter().enumerate() {
            for rect in &frame.bitmaps {
                // rect.bytes 是**编码后**的图片字节(通常是 WebP),零拷贝借用。
                // 本 crate 不解码 —— 交给上层。
                println!("帧 {i} 块 ({},{}) {:?} {} 字节",
                    rect.x, rect.y, rect.encoding(), rect.bytes.len());
            }
        }
        // 差分语义:要画第 10 帧,得从这一帧开始重放
        let start = seq.start_frame_for(10);
    }
}
```

### 画序列帧的正确姿势(**别踩这两个坑**)

PAG 的位图序列**不是**"每帧一张完整图片",而是**关键帧 + 脏矩形差分**。
直接解第 N 帧的字节贴上去,会得到一块孤零零的碎片。正确流程:

0. **先 `pag.is_pure_bitmap()`。** 为 `false` 就别往下走 —— 序列只是文件里的
   一个图层,矢量根还会对它做变换/蒙版/混合,而那些我们读不到。
   (这一步在早期版本里是缺的,而 `seq.width × seq.height` 的画布对混合档
   只覆盖那一个图层,不是文件的画面。)
1. `seq.start_frame_for(n)` 找到最近的关键帧 `s`;
2. 准备一张 `seq.width × seq.height` 的画布 —— **序列自己的**宽高,
   不是 `attributes()` 的宽高。纯位图档上两者相等,混合档上不等;
3. 从 `s` 到 `n` **逐帧**把每个 `BitmapRect` 的解码结果贴到 `(x, y)`;
4. 关键帧那一帧,若首块尺寸小于画布,先清屏。

宽高一定是正数:负宽高 / 零帧率 / 空序列的文件在 `parse` 就被 `verify()` 门
拒掉了(与 libpag 一致),不会流到这里。

(libpag 播放器还有一个"上次解到第几帧"的缓存快进,那是播放器优化,与容器格式无关。)

---

## 纪律

- **零依赖**;`#![forbid(unsafe_code)]`。
- **不 panic**,而且**不靠把 panic 转移给调用方**来实现:截断 / 畸形 /
  声称长度越界一律 `Err`;libpag 的 `verify()` 门(负宽高 / 零帧率 / 空序列 /
  零长度块)也照样把文件拒掉,而不是把畸形值原样递出去。
- **模糊测试**:
  - 一条把合法文件从**每一个字节位置**切断都要求返回 `Err` 的用例;
  - 一条 20000 样本的**种子变异**模糊测试(拿合法固件做原地字节变异 + 随机截断),
    并带**覆盖率断言** —— 必须真的走进 `parse_composition` /
    `parse_bitmap_sequence` / 位读 / `read_byte_data`,否则测试直接失败。
    🔴 上一版只有"随机体 + 合法头"那一条,自述说它"让样本真能走进标签解析",
    **实测 2000 个样本里 0 个构造出过 composition**(1895 个死在第一个标签头的
    `TagLengthOverflow`)—— 也就是它声称在保护的解析器一次都没被跑到。
    那条测试留着(它守的是标签头那一层的守卫),但覆盖率由新的那条负责,
    断言钉死,防止再退化回零覆盖。
- **不猜格式**:没核实过布局的标签一律按长度跳过。
- 测试固件全部**手工构造在代码里**,仓库不存二进制资产。

```sh
cargo test -p sv-pag
```
