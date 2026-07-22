# PAG 接入形态设计与裁决(2026-07-22)

> 问题:假设 svelte-rs 要支持 PAG(Tencent 的 Portable Animated Graphics,
> After Effects 动效格式),它在本仓库里应该长成什么样?
>
> 本文按 DESIGN.md 的体例写:先摆既有约束(读源码,不信转述),再逐条评估
> 三条路并给裁决,再把裁决摊开成帧调度 / 场景树 / 降级 / 无障碍四张具体的账,
> 最后诚实回答"要不要现在做"。
>
> 所有外部事实都在 §9 给了实查证据(GitHub API 查询日期 2026-07-22、
> crates.io、官方文档 URL);查不到的一律写"未核实",**没有一个版本号、
> API 名或性能数字是推测的**。

---

## 0. 结论速览

| 问题 | 裁决 |
|---|---|
| 三条路选哪条 | **(c) 离线转换**。(a) 纯 Rust 全量解析否;(b) 绑定 libpag **否,且是硬否** |
| (b) 为什么是硬否 | libpag→tgfx 的 C++ 依赖闭包含 **pathkit(仓库自述"extracted from the Skia library")与 skcms**,ADR-3 排除 skia-safe 的理由在这里原样复现且更重;且 **Windows/Linux 无预编译库** |
| 时间轴怎么合一 | PAG **不拥有时钟**:`sv_ui::anim` 增一条 Timeline 通道,每帧把帧号写进场景树(照常 bump 版本)。"有动画就排帧、没动画零功耗"由既有机制**自动**保住 |
| 场景树里是什么 | 新增**一个** `ElementKind::Animation`(**不是** `Pag`),载荷走 `ViewNode.anim: Option<Box<AnimData>>`(与既有 `input` 同款)。格式差异全部收在 `AnimSource` 枚举里,以后加 Lottie/SVG-animate **不再动 ElementKind** |
| 前端标签 | `<animation src="..." />`,**不叫 `<pag>` / `<lottie>`** —— 标签描述用途,不绑格式(textarea 先例:前端有 `Tag::TextArea`,运行时只有一个 kind) |
| 能力差异怎么办 | **构建期拒绝优先**:importer 在 build.rs 里就报错/告警。运行期只保留 `PainterCaps` 位查询。**不做**"运行期发现画不了就跳过" |
| 每帧成本 | 进 ADR-9 帧预算,并且要进 membench CI 场景。矢量档成本随素材复杂度走;序列帧档恒定 |
| **要不要现在做** | **不要**。它前面至少还压着三个更前置的缺口(`draw_image`、SVG 静态 importer、渐变/任意路径 stroke),而且都是它的前置。触发条件见 §8.3 |

---

## 1. 既有约束(读源码确认,不是转述)

### 1.1 Painter 的动词表就这么大

`crates/sv-shell/src/paint.rs` 里 `trait Painter` 的全部动词:

```
fill_rounded_rect / stroke_rounded_rect / glyph_run / push_clip / pop_clip / fill_path
caps() -> PainterCaps { external_texture: bool, blur: bool }
```

主进程刚加的 `fill_path(&[PathCmd], PathFill, Color)` 是矢量动画唯一的地基。
它的注释把裁决写得很死,直接决定了本文的可行域:

- `PathCmd` / `PathFill` 是**自有轻量类型,刻意不借 kurbo/peniko** ——
  理由是 vello 在本仓库是 optional dependency,让接口签名依赖只在某 feature
  下存在的类型 = 把 GPU 后端焊死进 CPU 路径;
- `fill_path` **没有默认实现是刻意的** —— 给 no-op 默认会让新后端静默不画;
- 填充只有**纯色** `Color`,**没有渐变、没有 paint 抽象**;
- `TinySkiaPainter::fill_path` 的**已知缺口**(源码注释原文):矩形裁剪没有
  接进去,"滚动容器内的路径图标不会被裁掉";
- **没有 `draw_image`**。整个 Painter 里画不出一张位图。

`PainterCaps::external_texture` 目前 CPU/vello 两个后端**都是 false**
(vello 只报 `blur: true`),注释写明它是 `<surface3d>` 的预留位。
**也就是说:今天没有任何一条把外部 GPU 纹理合成进我们画面的通道。**

### 1.2 动画的现状是一个 60 行的 thread_local 队列

`crates/sv-ui/src/anim.rs`:`ANIMS: RefCell<Vec<Anim>>`,两条通道
(`Channel::Opacity` / `Channel::ScrollY`),`pump(now_ms) -> bool` 里
**直接写场景树**(`update_style` / `set_scroll`),写入 bump doc 版本;
节点被销毁的动画自动出队;`active()` 供渲染壳决定是否继续排帧。

### 1.3 帧调度与静止帧短路

`crates/sv-shell/src/lib.rs::App::paint()`:

```
tasks::pump() → anim::pump(now_ms) → sv_reactive::tick() → 帧键短路 → 布局 → 绘制
let frame_key = (self.doc.version(), size.width, size.height, scale.to_bits());
if unchanged && !animating && !self.show_fps { return; }
...
if animating { ws.window.request_redraw(); }
```

**"零功耗静止"的全部机制就在这四行里**:版本号没变 + 没动画 = 直接 return。
`animating` 为真才继续排帧。任何新动画能力都必须走这条路,否则要么动不起来,
要么让整个窗口永远重绘。

### 1.4 ElementKind 的连带成本(实数)

`ElementKind::` 在 **8 个文件、约 61 处**出现:

| 文件 | 出现数 | 是什么 |
|---|---|---|
| `sv-ui/src/lib.rs` | 16 | 枚举定义 + Doc 构造函数 + `dump()` |
| `sv-ui/src/tmpl.rs` | 16 | 模板原语 |
| `sv-shell/src/render.rs` | 13 | `measure_leaf` 一处 match + `paint_tree` 一处 match |
| `sv-shell/src/a11y.rs` | 8 | role 映射 |
| `sv-macro/tests/view.rs` | 4 | 宏前端测试 |
| `sv-ui/src/focus.rs` / `input.rs` / `sv-shell/src/lib.rs` | 2/1/1 | 默认可获焦位等 |

再加**两个前端的标签表**:`sv-compiler/src/template.rs` 的 `Tag` 枚举 +
未知标签错误信息(它把内置标签名列在报错文案里)、`codegen.rs` 的 Tag→字符串、
`style.rs::ELEMENT_NAMES`、`sv-macro/src/parse.rs` 的 `LeafKind` 表。

**先例(必须对齐)**:多行 textarea **没有**新增 ElementKind ——
它是 `ElementKind::TextInput` + `input.multiline = true`;前端侧有独立的
`Tag::TextArea`(所以用户仍写 `<textarea>`、`rows` 只对它合法),
a11y 侧靠 multiline 位选 `Role::MultilineTextInput`。**前端标签可以多,
运行时 kind 要省。**

### 1.5 ADR 侧的红线

- **ADR-3**:排除 skia-safe,理由是"C++ 构建重、拖累鸿蒙交叉编译"。
  这条对**任何带 C++ 依赖的方案**都适用,是本次判决的核心约束。
- **ADR-3b**:tiny-skia 栈**能力冻结**,"CSS ⏳ 项一律不在 CPU 栈实现"。
- **ADR-5 / R5**:鸿蒙 = XComponent + wgpu(GLES)自绘 surface,
  交叉编译到 `aarch64-unknown-linux-ohos`。
- **ADR-6**:写入攒到帧边界,渲染壳统一冲刷。
- **ADR-9**:帧预算 6.94ms(144fps);虚拟化让帧成本与逻辑控件数解耦。

---

## 2. PAG 是什么(实查事实,不是印象)

| 事实 | 数字 / 出处 |
|---|---|
| 官方实现 | `Tencent/libpag`,**C++**,5737 star,仓库约 208 MB、main 分支 **3758 个文件** |
| 最新 release | **v4.5.81**,2026-07-22T08:11:52Z(同日还发了维护线 v4.4.57);34 个产物 |
| 许可 | README 写 Apache-2.0;GitHub 判定 `NOASSERTION`(仓库里另有企业版产物 `libpag_enterprise_*`) |
| 预编译产物覆盖 | iOS / Android / macOS / Web(wasm) / OpenHarmony / 微信小程序。**没有 Windows,没有 Linux** |
| README 原文 | "We currently only publish precompiled libraries for iOS, Android, macOS, Web, and OpenHarmony. You can build libraries of other platforms from the source code." |
| 鸿蒙产物 | `libpag_4.5.81_ohos_arm64-v8a.har`(1.65 MB);仓库 `ohos/` 是一个完整的 hvigor/DevEco 工程 |
| 二进制格式 | TLV(`TagHeader` + `Attributes` + `AttributeHelper`),**`TagCode` 枚举 83 项、值域到 94**;`src/codec/tags` 下 147 条路径;12 类 effect tag、13 类 shape tag、5 类 text tag |
| 非矢量成分 | `BitmapSequence` / `VideoSequence` / `VideoComposition` tag;`src/codec/utils/NALUReader`(H.264 NAL)、`WebpDecoder`;DEPS 里有 **libavc**;release 里专门有 `noffavc` 变体 |
| 压缩 | `src/rendering/utils/LZ4Decoder` + DEPS 里的 lz4 |
| 格式规范 | pag.io 有 "PAG File Format Spec" 页面(中英各一份,页面上有"下载 PDF"入口)。**PDF 直链未核实**。核心 codec 模块开源,可当规范的可执行版本 |
| **PAGX** | libpag main 分支带 `spec/pagx_spec.md`(**118 KB**)+ 中文版(108 KB)+ `pagx.xsd`(52 KB)。**XML 明文**格式,规范原文:"PAGX and binary PAG formats are bidirectionally convertible" |
| **libpag-lite** | 官方的**纯 JS + WebGL**(不用 wasm)Web SDK,57 KB / gzip 15 KB。README 特性原文:"**仅支持播放包含单独一个 BMP 视频序列帧的 PAG 动效文件**" |

### 2.1 两条最重要的事实,单独拎出来说

**其一:libpag-lite 是官方给出的"不背 C++ 就能吃 PAG"的答案,而这个答案放弃了矢量。**

腾讯自己写了一个纯 JavaScript 的 PAG 播放器,而它明确只支持"单一 BMP 视频
序列帧"的 PAG 文件 —— 也就是说,在**官方**的判断里,脱离 C++ 运行时之后
可行的 PAG 子集是**预合成的视频序列帧**,不是矢量图层。这不是我们的猜测,
是同一个 repo 里同一批人写下的取舍。

它同时暴露了序列帧路线的一个真实坑:BMP 序列 = H.264,而 libpag-lite
的 README 直说"因为 FireFox 对 H264 视频的支持不兼容带 Bframe 的视频,
所以简化版不支持 FireFox" —— **视频编解码的平台差异会原样变成你的平台差异**。
这条直接影响我们的降级设计(§6):**运行期不碰 H.264**。

**其二:PAGX 的存在把"离线转换"从将就变成了正解。**

`.pag` 的二进制解码有 83 个 tag、位打包属性、LZ4、H.264;而 `.pagx` 是有
118 KB 规范 + XSD 的 XML,并且与二进制**双向可转**。于是转换链可以是:

```
.pag  --[libpag 官方工具,只在设计侧/构建机跑]-->  .pagx(XML)
      --[我们的纯 Rust 转译器,读 XML]-->  自有中间格式(VectorClip / FrameAtlas)
      --[运行期]-->  Painter 动词
```

**C++ 只出现在构建期、只出现在我们自己的机器上,不进任何用户的依赖树、
不进任何交叉编译目标。** 这是唯一与 ADR-3 相容的形态。

---

## 3. 三条路逐条评估

### 3.1 路线 (a):纯 Rust 解析 PAG + 走我们的 Painter

**格式文档够不够写?** 够,但代价被低估了三层。

- 第一层:**规范可得**。pag.io 有格式规范 PDF 入口,核心 codec 完全开源
  (Apache-2.0,`src/codec` 可读),而且 `web/lite` 目录里有一份**纯 TypeScript
  的重新实现**(131 个文件)可当交叉参考。这是"能写"的证据。
- 第二层:**表达面**。83 个 tag / 12 类 effect / 13 类 shape / 5 类 text
  不是可选装饰,是设计师从 AE 导出时随手就会用到的东西(图层样式、轨道遮罩、
  3D 变换 + 摄像机、AE 文本动画器)。
- 第三层(**真正的杀手**):**PAG 不是矢量格式**。BitmapSequence /
  VideoSequence / VideoComposition 是一等公民,DEPS 里的 libavc 与
  release 里的 `noffavc` 变体就是证据。纯 Rust 路线要么放弃这一大类文件,
  要么背一个 H.264 解码器 —— 而 Rust 生态里没有一个能同时满足
  "纯 Rust / 覆盖 AVC profile / 有维护"的解码器(未核实是否有例外,
  但 openh264/ffmpeg 都是 C 依赖,等于绕回 §3.2 的问题)。

**即使解析出来了,我们的 Painter 也画不了。** 对照表:

| PAG 要求 | 我们有的 |
|---|---|
| 任意路径填充(纯色) | ✅ `fill_path` |
| 2D 变换(位置/缩放/旋转/锚点/倾斜) | ❌ Painter 无 transform 动词,只能在上层把点烘进坐标 |
| 渐变填充 / 渐变描边 | ❌ 只有 `Color` |
| 任意路径描边 + join/cap/dash/trim | ❌ 只有 `stroke_rounded_rect` |
| 遮罩 / 轨道遮罩 / blend mode | ❌ 只有矩形 `push_clip`(且圆角是矩形近似) |
| 12 类效果(模糊/发光/置换/边角定位…) | ❌ |
| 图层不透明度合成(离屏 group) | ❌ 只有节点级 `opacity` 靠父链连乘 |
| 位图/视频层 | ❌ 无 `draw_image` |

要把这张表填绿,等于把 Painter 从 6 个动词扩到接近完整的 2D imaging model,
**这是对 ADR-3b"CPU 栈能力冻结"的正面撞击** —— 而 ADR-3b 明确说 CPU 栈
"定位过渡与测试基准",能力不再扩。

**工作量的外部参照(重要)**:Linebender 官方的 Lottie→vello 渲染器
**velato**,`src/` 约 **213 KB Rust**,从 2024 年做到 2026-07,
自己的 README 仍列着不支持:文本、图片嵌入、时间重映射、虚线/锯齿等
高级形状、运动模糊/投影等高级效果、色标处理、拆分旋转、位置关键帧缓动。
**而 Lottie 是公开 JSON schema、素材面最大、社区最活跃的格式。**
PAG 的表达面比 Lottie 更大,文档面更小,还多一个视频复合。

| 维度 | 评估 |
|---|---|
| 工作量 | 全量:**数人年**,且要背 H.264。shape 子集:**6–10 人周**(见 §8.4) |
| 风险 | 高。真实素材命中不支持特性的概率极高,而"画错"比"画不出"更难定位 |
| 可逆性 | 好 —— 代码全是我们的,删掉即可 |
| 对 ADR-3 的冲击 | 无(不引入 C++);但**对 ADR-3b 的冲击大**(逼着扩 CPU 栈能力) |

**裁决:全量版否。子集版本身是合理的,但不应该以 PAG 为入口** ——
理由见 §8.2:同样的子集,SVG 与 Lottie 的性价比高一个数量级。

### 3.2 路线 (b):绑定 libpag,渲到独立 surface,我们做合成

先把"意味着什么"摊开。

**C++ 依赖闭包(实查 DEPS 文件)**:

- libpag `DEPS`(common,即所有平台都拉):`vendor_tools`、**tgfx**、
  `libavc`(H.264)、`rttr`(C++ 反射)、`harfbuzz`、`lz4`、`expat`、
  `SheenBidi`、`libxml2`、`woff2` —— **10 个**。
- tgfx `DEPS`(common):`vendor_tools`、**`pathkit`**、**`skcms`**、
  `zlib`、`libpng`、`libwebp`、`libjpeg-turbo`、`freetype`、`harfbuzz`、
  `googletest`、`nlohmann/json`、`expat`、`concurrentqueue`、`highway`、
  **`shaderc`**、**`SPIRV-Cross`**、`Vulkan-Headers`、`volk`、
  `VulkanMemoryAllocator`、**`tint`**、**`abseil-cpp`** —— **21 个**。
- 去重后 **≈ 28 个第三方 C/C++ 仓库**。

**其中两个是 Skia 的零件**(GitHub 仓库描述原文):

- `libpag/pathkit`:*"This library is extracted from the Skia library,
  which lets you use Skia's feature-rich PathOps API."*
- `libpag/skcms`:*"A library for converting pixels in variety of formats."*
  (Skia 的色彩管理组件,BSD-3-Clause)

**ADR-3 排除 skia-safe 的理由在这里 100% 复现,而且更重。**
skia-safe 至少是一个有 build 脚本、能 `cargo add` 的 crate;
libpag 的依赖同步走 **depsync(一个 Node 工具)**,构建前置是
CMake 3.13+ / Ninja 1.9+ / Node.js 14.14+ / NDK 28+ / Emscripten 3.1.58+ /
VS2019+,**没有一样是 cargo 能表达的**。而且它还顺手带进来 tint +
shaderc + SPIRV-Cross + abseil —— 一整套着色器编译栈,与我们的 wgpu
在功能上完全重叠。

**Windows/Linux 没有预编译库**(README 原文见 §2 表)。我们的主力开发与
首发平台是 Windows。这意味着**每一个使用者**(不只是我们)都要在自己机器上
过一遍这套 C++ 构建,CI 从分钟级推到十分钟级以上。这与"cargo add 就能用"
的分发承诺直接冲突,而 R4 的整章都在做发布工程。

**合成时机 / 纹理共享**:

- **vello 档**:我们是 vello 0.9 → wgpu 29。libpag 通过 `PAGSurface` 渲染,
  tgfx 的后端是 GL/Metal/Vulkan(自带 shaderc/SPIRV-Cross/tint 就是证据)。
  两套 API 要共享纹理,要么走外部内存扩展(`VK_KHR_external_memory` /
  D3D11 共享句柄),要么退化成**读回 + 上传**——1080p RGBA 每帧 ≈ 8 MB
  的 GPU→CPU→GPU 往返,在 6.94ms 预算里是灾难。而 wgpu 侧我们
  `PainterCaps::external_texture` **两个后端都还是 false**,连通道都没开。
  (libpag 在 Windows 上具体用哪个 GL/ANGLE 来源 —— **未核实**。)
- **CPU 档:没有出路。** libpag/tgfx 是 GPU 优先的;想在 tiny-skia 上合成
  只能让 libpag 渲到离屏 GPU surface 再读回,而"无 GPU 环境"这个前提下
  连这一步都不存在。tgfx 是否有软件光栅后端 —— **未核实**;
  即使有,也等于往 CPU 兜底路径里塞一个完整的 C++ 光栅器,
  与 ADR-3b"CPU 栈能力冻结"直接相斥。
- **鸿蒙**:libpag **确实有** OHOS 预编译产物,但 `.har` 是 DevEco/ArkTS
  的包格式,`ohos/` 目录是一个完整 hvigor 工程 —— 它的集成范式是
  "你用 ArkUI,我给你一个组件"。我们的 R5 路线是 XComponent + wgpu(GLES)
  **自绘 surface**,两个 GL/Vulkan 消费者要共存于同一个 EGLSurface,
  只能靠 EGLImage/FBO 交换手工缝合,而 wgpu-OHOS 目前只有 GLES 后端。
  这个风险等级**等同于 ADR-5 已经点名的两个最高风险工程点**
  (自绘 surface 上的 IME、OH_NativeVSync 帧循环)—— 为一个动画格式
  再开一个同量级的风险口子,不划算。

| 维度 | 评估 |
|---|---|
| 工作量 | **不估**。这不是工作量问题 |
| 风险 | 极高:构建、分发、许可(企业版另有授权)、三平台合成、鸿蒙 surface 共享 |
| 可逆性 | **极差**。一旦进构建,分发形态(包体积 + 平台预编译矩阵 + 许可证审查)全部改写,退出等于把能力整个撤掉 |
| 对 ADR-3 的冲击 | **正面否定**。ADR-3 排除 skia-safe 的每一条理由都成立,而且 libpag 把 Skia 的零件真的拉了进来 |

**裁决:否。而且理由不是"暂时不做",是"与 ADR-3 的核心裁决不相容"。**
如果哪天要推翻,推翻的应该是 ADR-3 本身,而不是在 ADR-3 存续期间开个后门。

### 3.3 路线 (c):离线转换

分三档,能力与代价各不相同。

#### c1 — 静态首帧 / 关键姿势

构建期解一帧,产出 `Vec<PathCmd>` 或一张位图。

- **够用**:品牌 logo、空状态插画、"看起来会动但其实不需要动"的占位。
- **不够**:任何真动画。基本只是一个诚实的 fallback,不是产品能力。

#### c2 — 序列帧图集(**今天就能做的那一档**)

构建期离线出帧序列(WebP/PNG 图集),运行期只做"贴图 + 换帧"。

- **运行期零解析、零矢量、零 C++、零 H.264**。
- CPU/GPU/鸿蒙三档**能力完全一致**(前提:有 `draw_image` 动词)。
- 帧调度天然可控:整帧、无插值、按素材帧率推进。
- **被官方路径验证过**:PAG 导出面板支持"全 BMP 预合成"导出,
  libpag-lite 消费的正是这一档;我们只是把它的 H.264 换成离线转码后的
  图集,从而把"平台视频解码差异"整个消掉。
- **代价(要说清楚)**:体积(N 帧 × 分辨率,失去矢量可缩放性)、
  多 DPI 要出多套、**运行期无法换字换图**(而"运行期替换文本/图片"
  恰恰是 PAG 最大的产品卖点之一)。
- **适用边界**:短(≤2s)、小尺寸(≤200×200 逻辑 px)、循环、
  不需要换字换图 —— 也就是 UI 里绝大多数 loading / success /
  点赞 / 开关动效。

#### c3 — PAG → PAGX → 自有中间格式(**目标形态**)

见 §2.1 的链条图。

- **可逆性极好**:中间格式是我们自己的,PAG 只是众多 importer 之一;
  换掉 PAG 不影响运行期一行代码。
- **能力边界从运行期移到构建期**:importer 遇到 track matte / 效果 /
  视频层,在 `build.rs` 里报错或告警 —— 用户在**编译**时知道,
  而不是在**用户机器上**看到画错的一帧。这是 (c) 相对 (b) 的额外红利。
- **诚实的两个缺口**:
  1. `.pag → .pagx` 转换工具的具体形态(是否有开源 CLI、能否进 CI)
     —— repo 里有 `cli/npm`、`pagx/wechat`、`src/pagx`,pag.io 有
     "PAG 转换工具"下载页,但**具体命令与开源状态未核实**。
     **这是开工前第一个必须 spike 的点**,c3 的唯一硬前置。
  2. 能转出来 ≠ 我们画得出来。PAGX 的表达面 = PAG 的表达面,
     转译器仍要面对 mask/matte/effect/blend 的取舍,
     只是不必再面对**二进制解码**这一层。

| 维度 | c1 | c2 | c3 |
|---|---|---|---|
| 工作量 | 并入 c2 | 2–3.5 人周(含 `draw_image`) | 增量 3–6 人周(须先有矢量 IR) |
| 风险 | 无 | 低(体积、DPI 矩阵) | 中(依赖 pagx 转换工具可自动化) |
| 可逆性 | 好 | 好 | **极好** |
| 对 ADR-3 冲击 | 无 | 无 | 无 |

**总裁决:走 (c)。c2 是最小闭环,c3 是目标形态,c1 是二者的自然副产品。**

---

## 4. 动画驱动与帧调度:两套时间怎么合一

### 4.1 核心裁决:**PAG 不拥有时钟**

PAG 有自己的时间轴(composition duration + frameRate + 图层入出点),
libpag 的运行期用 `PAGAnimator` 按 wall clock 播。**我们不采用这个模型。**

`sv_ui::anim` 增一条通道:

```rust
enum Channel {
    Opacity,
    ScrollY,
    /// 素材时间轴:把"当前应该显示第几帧"写进场景树
    Timeline,
}
```

`pump(now_ms)` 里这条通道做的事:算出帧号 → 写进 `ViewNode.anim.frame`
→ 照常 bump 版本。**和 Opacity/ScrollY 完全同构**,不引入第二套机制。

### 4.2 于是四条性质**自动**成立(不需要新写任何短路逻辑)

| 性质 | 为什么自动成立 |
|---|---|
| 有动画就排帧 | `pump` 返回 true → `animating` → `request_redraw()`(lib.rs 现有代码) |
| 没动画零功耗 | 播完 → 队列出队 → `active()` 假 → 帧键不变 → `paint()` 直接 return |
| 与 ADR-6 帧对齐 | `anim::pump` 已经排在 `tick()` 之前,动画写入与用户写入在同一轮 flush 落地 |
| 节点销毁不泄漏 | `pump` 里已有"节点没了就丢弃动画"的 retain 逻辑 |

### 4.3 必须写进文档的**反模式**

> **不要让动画节点在 `paint_tree` 里读系统时间自己算帧号。**

后果是二选一的灾难:
- 帧键(`doc.version()`)不含时间 → 静止短路把动画**卡死**;
- 或者为了让它动而让帧键恒变 → **整个窗口**永远重绘,
  ADR-9 辛苦挣来的"静止功耗归零"当场失效。

**时间必须先落进场景树(变成版本号的一部分),再被绘制读到。**
这正是既有 `anim.rs` 那样写的原因 —— 不是风格问题,是架构约束。

### 4.4 帧率解耦:按**素材帧率**取整帧

素材有自己的 frameRate(常见 24/30/60),显示器是 60/144/165Hz。

裁决:`frame = floor((now_ms - start_ms) * fps / 1000.0)`,
**帧号没变就不写场景树、不 bump**。

收益:24fps 素材在 144Hz 屏上每 6 个 vsync 才真重绘一次 —— 白拿的省电,
而且序列帧档**只能**这么做(它只有整帧)。矢量档理论上可以插值出素材里
不存在的中间帧,但那是在给自己找工作:AE 的关键帧曲线已经在素材里烘好了,
超采样只增加成本不增加信息。

### 4.5 循环、暂停与"危险品清单"

`AnimState { playing: bool, repeat: Repeat, speed: f32 }`,
`Repeat = Once | Count(u32) | Forever`。

**`Forever` 是危险品,必须在文档里标红**:它让 `active()` 恒真 →
每帧 `request_redraw` → **整个窗口**的静止功耗归零性质失效
(不只是那个节点)。缓解措施三条,缺一不可:

1. **视口外不 pump**。滚出可视区的动画应该暂停 —— 虚拟化让节点数与帧成本
   解耦,但动画是"每帧真工作",不会被虚拟化省掉。
   **现状 `anim.rs` 没有视口概念,这是新增项。**
2. **窗口不可见/失焦时暂停**。winit 有 `Occluded` / `Focused` 事件;
   **现状渲染壳是否已接 —— 未核实,需要新接线。**
3. **`prefers-reduced-motion` 一票否决**(见 §6)。

---

## 5. 场景树里是什么

### 5.1 裁决:新增**一个** `ElementKind::Animation`,载荷走 side 字段

```rust
pub enum ElementKind {
    View, Text, Button, Checkbox, TextInput,
    /// 动画叶子:内在尺寸 = 素材尺寸;载荷在 ViewNode::anim
    Animation,
}

pub struct ViewNode {
    // ...既有字段...
    pub input: Option<Box<InputState>>,   // 既有形态
    pub anim:  Option<Box<AnimData>>,     // 新增,同款
}

pub struct AnimData {
    pub source: AnimSource,
    /// 当前帧号(由 anim::pump 写入;整数语义,f32 只为省一次转换)
    pub frame: f32,
    pub state: AnimState,
    /// 无障碍名称(必填,见 §7)
    pub label: String,
}

pub enum AnimSource {
    /// 构建期转译出的矢量剪辑(PathCmd 序列 + 变换 + 关键帧)
    Vector(Rc<VectorClip>),
    /// 构建期转出的序列帧图集
    Frames(Rc<FrameAtlas>),
}
```

### 5.2 为什么是"新增一个 kind"而不是复用

对照 textarea 的先例:textarea 能复用 `TextInput`,是因为它**共用编辑内核**
—— 同样的 measure 路径(shape 文本)、同样的绘制路径(glyph_run + 光标 +
选区)、同样的 a11y 家族(TextInput/MultilineTextInput)、同样的输入事件。
差异小到一个 bool 就能表达。

动画没有这样的宿主:

| 关注点 | 动画要什么 | 现有 kind 谁能给 |
|---|---|---|
| measure_leaf | 内在尺寸 = 素材 composition 宽高 | 没有(Text 走 shape、Button 走 measure+居中、Checkbox 是方块) |
| paint_tree | 一串 PathCmd / 一张位图 | 没有 |
| a11y | `Role::Image` + label | 没有 |
| focus | 默认**不可获焦** | View 可以给 |

硬塞进 `View` + 一张 side table,会让 `paint_tree` 里那句
`ElementKind::View => {}` 变成"其实还要看 side table" —— **比多一个枚举
更难读**,而可读性正是 textarea 那次裁决真正在保护的东西。

### 5.3 但载荷**不进** ElementKind —— 这是本节的关键

`AnimSource` 是一个枚举,不是一个 kind。于是:

- 将来加 **Lottie**:`AnimSource` 里什么都不用加(Lottie 与 PAG 转译到
  同一个 `VectorClip`);
- 加 **SVG SMIL / CSS keyframes 驱动的矢量**:同上;
- 加 **APNG/WebP 动图**:`AnimSource::Frames` 直接复用;
- 加一种全新的东西:加一个 `AnimSource` 变体,**8 个文件、61 处 match
  一处都不用动**。

**连带成本一次性付清,之后归零。** 这正是 textarea 那次裁决的精神在
"一定要新增 kind"场景下的正确落法。

### 5.4 前端标签:`<animation>`,不叫 `<pag>`

```html
<animation src="assets/loading.pag" label="加载中" loop autoplay />
```

- **标签名描述用途,不绑格式**。`<pag>` / `<lottie>` 是最贵的错误:
  格式会换(我们已经打算把 PAG、Lottie、序列帧都收成一种),标签不会。
  textarea 的经验反过来用一次:**前端标签可以多、可以贴合直觉,
  运行时 kind 要省、要描述行为**。
- `src` 指向的是**源素材**,由 build.rs 里的 importer 在构建期转译;
  运行期加载的是转译产物,不是 `.pag`。
- 两个前端各加一个标签(`sv-compiler/template.rs::Tag` +
  `sv-macro/parse.rs::LeafKind`)+ 属性校验(`loop`/`autoplay`/`label`
  只对它合法,和 `rows` 只对 textarea 合法同款)+ `style.rs::ELEMENT_NAMES`。

### 5.5 代价的诚实账单

新增一个 kind ≈ 改 **8 个文件、约 10 处 match**:
`measure_leaf`、`paint_tree`、`dump()`、a11y role 映射、focus 默认位、
两个前端标签表 + 属性校验、`ELEMENT_NAMES`、宏前端测试。

**这笔钱只有在决定要做动画能力时才付。** 而如果只想做 c2 的最小版,
有没有更便宜的形态?——**没有**。理论上可以让 `<animation>` 编译成
"一个 View + 一个 effect 每帧改背景图",但我们**连 `Painter::draw_image`
都没有**。

> **顺带得到本文最重要的一个副产品结论:位图绘制动词(`draw_image`)是比
> PAG 更前置的缺口。** 它挡住的不只是动画,还有 `<img>` —— 而 `<img>` 的
> 使用频率比动画高一个数量级。见 §8。

---

## 6. 能力降级:不要让能力差异变成运行时惊喜

原则落成三条,按优先级:

### 6.1 编译期拒绝优先

因为 importer 在**构建期**跑(路线 c 的直接红利),素材用了什么特性在
`build.rs` 里就完全已知。裁决:

- 素材用了当前目标画不了的特性(track matte / 效果 / 视频层 /
  CPU 档下的渐变)→ **importer 报错或告警,附素材路径 + 图层名 + 特性名**;
- 提供 `--allow-degrade` 式的显式开关把 error 降成 warning,
  **但默认是 error**。理由与 `fill_path` "没有默认实现是刻意的"同源:
  **漏画/画错在自绘 UI 里极难定位,宁可在构建期逼着人面对它**。

### 6.2 运行期只保留可查询,不做静默跳过

`PainterCaps` 已经是这个机制的位置,增两位:

```rust
pub struct PainterCaps {
    pub external_texture: bool,
    pub blur: bool,
    /// 能否画位图(序列帧动画 / <img> 的前置)
    pub image: bool,
    /// 能否用渐变 paint 填充路径(矢量动画的常见需求)
    pub gradient: bool,
}
```

**不新增"animation: bool"这种复合能力位** —— 能力位应该对应**动词**,
不对应**特性**,否则每加一个 UI 特性就要加一个位,caps 会变成第二张需求表。

### 6.3 降级链条(写死、可查、无惊喜)

| 环境 | 矢量档 | 序列帧档 | 说明 |
|---|---|---|---|
| vello + GPU | 全能力(路径 + 渐变 + 未来 blur) | ✅ | 目标形态 |
| CPU(tiny-skia) | **shape 子集**:纯色填充/描边 + 变换 + 关键帧 | ✅ | 渐变**不破例**开(见下) |
| 无 GPU | 同 CPU 档 | ✅ | |
| 鸿蒙早期(vello_hybrid 未定) | ❌ 暂不支持 | ✅ **只支持这一档** | 矢量档跟 vello_hybrid 一起到位 |

**"CPU 档要不要为动画破例开渐变"**是这里唯一有争议的裁决。
tiny-skia 本身**有**渐变能力,但 ADR-3b 已经把 CPU 栈能力冻结,
理由是"定位过渡与测试基准"。我的裁决是**不破例**:
importer 在 CPU 档把渐变降级为中点纯色,并在构建期打 warning。
理由有二:(1) 一旦为动画破例,box-shadow / backdrop-filter 会立刻拿这个
先例来敲门,冻结线就守不住了;(2) 序列帧档已经提供了"CPU 上也要精确
还原"的答案,不需要矢量档去兼职。

### 6.4 `prefers-reduced-motion` 是一等公民

CSS-SUPPORT.md 已把 `prefers-reduced-motion` 列在 C2(`@media` 通道,
接系统无障碍设置)。动画必须接上它:

- 开启时:**停在首帧**(或素材标注的 poster 帧),`active()` 不再返回 true;
- 这既是无障碍要求,也是"零功耗"的另一半 ——
  **一个系统级开关能把所有 `Forever` 动画一次关掉**。

### 6.5 运行期**不碰** H.264

libpag-lite 的 FireFox/Bframe 坑(§2.1)说明视频编解码的平台差异会原样
变成产品的平台差异。裁决:**任何视频复合的 PAG,由构建期转码成图集**,
运行期永远不出现视频解码器。这条同时也是"不引入 C++"的守门条款 ——
Rust 生态没有一个能覆盖 AVC profile 面的纯 Rust 解码器
(**未核实是否有例外**,但 openh264/ffmpeg 路线都会绕回 §3.2)。

---

## 7. 无障碍与性能

### 7.1 播放中的动画对 AT 是什么

accesskit 0.24 的 `Role` 里相关变体(实查 docs.rs):
`Image`(4)、`Canvas`(51)、`Video`(131)、`SvgRoot`(119)、
`GraphicsDocument`(136)、`GraphicsObject`(137)、`GraphicsSymbol`(138)、
`ProgressIndicator`(101)。

**裁决:`Role::Image` + 必填 `label`。**

- `Image` 是屏幕阅读器处理最成熟、跨平台行为最一致的角色 —— 读 name 就够;
- `Canvas` / `GraphicsDocument` 会让 AT 期待里面有**可探索的子结构**,
  而我们的动画对 AT 是不透明的一整块,报这些角色是撒谎;
- `Video` 会带出播放控制的期待(播放/暂停/进度),我们没有;
- **label 必填**(在 `<animation>` 上没写 label = 编译错误)。
  一个无名的动画对屏幕阅读器用户就是一段静默的空白。

**不做**:把播放状态/进度报给 AT。除非动画确实是一个 loading 指示器,
那时正确做法是让**外层**报 `ProgressIndicator`,而不是让动画自己报。

### 7.2 每帧推 a11y 的坑(P6 白拿的红利,以及会把它作废的写法)

动画每帧 bump doc 版本 → `push_access_tree()` 每帧进 `build_tree_update`。
但 R3-P6 的 `A11yCache` 会把"内容真没变的节点"过滤掉
(`accesskit::Node: PartialEq` 直接比对),所以:

> **只要动画节点的语义内容(role / label / bounds)每帧不变,
> 增量推送就是空的** —— 代价只剩纯函数映射本身(全量算,但便宜)。

这是 P6 白拿的红利。但它有一个**会把红利整个作废的写法**,必须点名:

> **不要把帧号/进度写进 a11y 的 name / value / description。**

那样每帧都是一个真 update,屏幕阅读器会被刷屏 —— 这是一个在 Web 上
反复出现的真实踩坑模式(把进度百分比写进 `aria-label` 导致 NVDA 连续朗读)。

**待查**:accesskit 是否有 `busy` 一类属性可用于"内容正在变化" ——
**未核实**。

### 7.3 性能:进不进 ADR-9 的帧预算

**进,而且要有基准。**

- 预算 6.94ms(144fps)。虚拟化把帧成本与**逻辑控件数**解耦,
  但动画是"每帧真工作",虚拟化省不掉它。
  → 直接推出 §4.5 的第 1 条设计要求:**视口外不 pump**。
- **矢量档**每帧成本 = 关键帧求值(便宜)+ 路径重建(每帧一个新的
  `Vec<PathCmd>`,注意这是**每帧分配**,应该用一个可复用的 buffer)
  + 光栅(随路径复杂度与覆盖面积走)。
  **具体数字未实测,本文不给数字。**
- **序列帧档**每帧成本 = 一次位图 blit,**与素材复杂度无关** ——
  这是它在预算上的最大优势,也是它值得先做的第二个理由。
- **基准要进 CI**:membench 已经 CI 化(路线图 M4 落地,门槛故意宽,
  拦的是数量级回归)。动画应加一个场景:
  N 个动画节点 × 素材复杂度,卡 p99。

### 7.4 验收工具:`RecordingPainter` 是天然的金样载体

`RecordingPainter` 记 `PaintCmd::Path { cmds, fill, color, bbox }`
(逐点不记,只记条数 + 规则 + 颜色 + 取整包围盒 —— 源码注释说得很清楚:
逐点会让金样长到没人看得懂,而包围盒足以抓住"画错位置/画歪了")。

于是动画的正确性可以这样测,**零像素、零 GPU、零窗口**:

```
第 0 帧命令流金样 / 第 12 帧 / 第 24 帧 / 循环回绕后的第 0 帧
→ 命令条数 + 包围盒 + 颜色
```

这条要写进去,因为它把"动画对不对"从"人眼看离屏图"变成了**可回归的测试**。
既有的双后端对拍(GPU/CPU 非白像素比)可以作为第二道。

---

## 8. 要不要现在做

### 8.1 直接回答:**不要**

本仓库当前的欠账(DESIGN.md 实录):

| 欠账 | 状态 | 是否阻塞商用 |
|---|---|---|
| ADR-10 改名 | **待裁决** | **是**(阻塞 crates.io 首发) |
| 双前端内核合并 | 未做 | **是**(API 冻结前置) |
| `.sv` 的 LSP | 未 spike | **是**(风险清单第 1 位) |
| 鸿蒙 R5 | 未开工 | 是(档 C) |
| 增量布局 / 增量场景编码 | 未做 | 否(ADR-9 后续阶梯) |
| NVDA/VoiceOver/Orca 真机冒烟、IME 三平台手测 | 未做 | 是(R3 剩余项) |
| **PAG** | — | **否** |

PAG **不在任何一条商用阻塞链上**。档 B("单桌面平台可商用")的定义里
没有"能播 AE 动效";调研 26(sv-arco)点名的最大风险是**图标管线**,
而图标管线的地基是 `fill_path`(已落地)+ **SVG 静态转译**,不是动画。

### 8.2 它前面还压着三个更前置的缺口,而且都是它的前置

1. **`Painter::draw_image`(位图绘制动词)**。
   没有它,序列帧路线走不通、`<img>` 也走不通 —— 而 `<img>` 的使用频率
   比动画高一个数量级。**这是整个渲染栈今天最便宜、性价比最高的一个动词。**
2. **SVG 静态 importer(构建期,SVG → PathCmd)**。
   sv-arco 图标管线的正主,而且它是"矢量素材 importer"这条流水线的
   **最小版本** —— PAG/Lottie importer 是它的严格超集
   (多了时间轴与关键帧,几何/填充/描边那一层完全共用)。
3. **渐变 paint 与任意路径 stroke(join/cap/dash)**。
   任何真实矢量素材第一屏就会撞上;现在 Painter 只有纯色 `fill_path` +
   圆角矩形 stroke。

**做完这三样,PAG 的增量成本会从"数人月"掉到"数人周"** ——
这就是为什么"现在做 PAG"不只是排序问题,而是**顺序错了会多花好几倍钱**。

### 8.3 而且 PAG 应该排在 Lottie 后面

不是因为 Lottie 更好,是因为:

| 维度 | Lottie | PAG |
|---|---|---|
| 格式 | 公开 JSON schema | 二进制(83 tag + 位打包 + LZ4)+ XML 的 PAGX |
| Rust 生态 | **velato 0.11.0(2026-07-21)**,并且 2026-07-01 刚跟到 **vello 0.9** —— 与我们的 vello 版本**完全对齐** | **crates.io 上没有任何 PAG 相关 crate**(实查) |
| 可当参考实现 | 可以。甚至可以直接把 velato 当 GPU 档实现,我们只补 CPU 档 | 只能读 C++ / TypeScript 源码 |
| 素材供给面 | 大 | 小(且集中在国内移动端) |
| 非矢量成分 | 无(图片是引用) | **视频序列帧是一等公民** |

PAG 的真实优势(体积小、解码快 10 倍、运行期换字换图、视频复合)
**对桌面 UI 库的价值排在很后面** —— 那些优势是给移动端短视频/模板类 App 的。

### 8.4 什么条件下才值得开工(触发条件,四条**全部**满足)

1. 有一个**真实的、付费的**下游场景点名要 **PAG**(而不是泛指"动效"):
   即客户素材库已经是 `.pag`,重做成 Lottie/序列帧的成本高于我们做
   importer 的成本。
2. `draw_image` + SVG importer 已落地,且**矢量剪辑 IR 已经稳定** ——
   此时 PAG importer ≈ 再写一个前端。
3. **`.pag → .pagx` 的官方转换路径经 spike 确认可自动化**
   (命令行、可进 CI、许可允许)。这是 c3 的唯一硬前置。
4. R4 改名 + crates.io 首发已完成,R5 鸿蒙至少跑通三角形 ——
   否则会在还没有"平台"的时候先做"素材"。

### 8.5 粗估(相对量级,不是承诺)

| 项 | 估算 | 备注 |
|---|---|---|
| `draw_image` 动词(CPU + vello 双后端 + RecordingPainter) | **0.5–1 人周** | 顺带解锁 `<img>` |
| c2 最小闭环:`<animation>` 标签 + `ElementKind::Animation` + `AnimSource::Frames` + Timeline 通道 + 构建期图集转换 + 金样 | **2–3.5 人周** | 含 §5.5 的 8 文件改动 |
| SVG 静态 importer(shape 子集 + 变换,构建期) | **3–5 人周** | sv-arco 本来就要付这笔钱 |
| 矢量剪辑 IR + Lottie importer(shape layer 子集、关键帧,不含文本/图片/效果) | **6–10 人周** | 参照 velato:213 KB Rust / 两年半 / 仍不完整 |
| PAG importer(经 PAGX,复用同一 IR) | 增量 **3–6 人周** | 前提是 §8.4 第 3 条成立 |
| PAG importer(直接吃二进制) | 上一行的 **2 倍以上** | 且 BMP/Video 复合素材直接不支持 |
| 绑定 libpag | **不估** | 见 §3.2:不是工作量问题 |

### 8.6 如果非做不可,最小可交付是什么

> 构建期把 `.pag`(或 `.pagx`)转成 WebP/PNG 序列帧图集 +
> 一个 `<animation>` 标签 + `Painter::draw_image`。

它今天就能做,不引入任何 C++ 到用户依赖树,CPU/GPU/鸿蒙三档能力一致,
并且**可逆**:哪天有了矢量 IR,把 importer 的输出换掉即可,
标签、场景树节点、Timeline 通道、a11y 映射、金样测试**一行都不用改**。

---

## 9. 待核实清单(开工前必查)

| # | 待核实项 | 为什么重要 |
|---|---|---|
| 1 | `.pag → .pagx` 是否有**开源/可自动化**的转换工具(repo 有 `cli/npm`、`src/pagx`;pag.io 有"PAG 转换工具"下载页) | c3 的唯一硬前置 |
| 2 | PAG 二进制格式规范 PDF 的直链与版本(pag.io 页面有"下载 PDF"入口,直链本次未抓到) | 决定 (a) 子集路线的可行性判断精度 |
| 3 | libpag 在 Windows 上的 GL 来源(是否 ANGLE / D3D) | 只在万一重启 (b) 时才需要 |
| 4 | tgfx 是否有软件(CPU)光栅后端 | 同上 |
| 5 | accesskit 是否有 `busy` 一类"内容正在变化"的属性 | §7.2 的动画状态上报 |
| 6 | winit 的 `Occluded` / `Focused` 事件在本仓库渲染壳是否已接线 | §4.5 的 `Forever` 缓解措施之二 |
| 7 | `vello::Scene` 能否被 `vello_cpu`(0.0.9,2026-05-30)消费 | 决定"velato 当 GPU 档实现、CPU 档怎么办"的答案 |
| 8 | 纯 Rust 的 H.264 解码器是否存在可用者 | 只影响"直接吃二进制 PAG"这条已被否掉的路 |
| 9 | libpag 企业版 / `NOASSERTION` 许可判定的法务口径 | 只在万一重启 (b) 时才需要 |

---

## 10. 证据(实查,2026-07-22)

**libpag / tgfx(GitHub API 查询)**
- `Tencent/libpag` — C++,5737 star,仓库 ~208 MB,main 分支 3758 个文件;
  `pushed_at` 2026-07-22T10:07:13Z;许可字段 `NOASSERTION`(README 写 Apache-2.0)。
  https://github.com/Tencent/libpag
- 最新 release **v4.5.81**(2026-07-22T08:11:52Z,34 个产物);
  同日维护线 **v4.4.57**。产物含 `libpag_4.5.81_ohos_arm64-v8a.har`(1,654,079 B)、
  Android `.aar`、iOS/macOS `.zip`、`web.zip`、`miniprogram.zip`;
  **无 Windows / Linux 产物**。
  https://github.com/Tencent/libpag/releases
- README(原文):平台 iOS 9.0+ / Android 5.0+ / **HarmonyOS Next 5.0.0(12)+** /
  macOS 10.15+ / Windows 7.0+ / Chrome 69+ / Safari 15+;
  "We currently only publish precompiled libraries for iOS, Android, macOS, Web,
  and OpenHarmony.";构建前置 CMake 3.13+ / Ninja 1.9+ / Node.js 14.14+ /
  NDK 28+ / Emscripten 3.1.58+ / VS2019+ / depsync。
  https://github.com/Tencent/libpag/blob/main/README.md
- `DEPS`(libpag,common 段 10 个仓库):vendor_tools、**tgfx**、libavc、rttr、
  harfbuzz、lz4、expat、SheenBidi、libxml2、woff2。
  https://github.com/Tencent/libpag/blob/main/DEPS
- `DEPS`(tgfx,common 段 21 个仓库):vendor_tools、**pathkit**、**skcms**、zlib、
  libpng、libwebp、libjpeg-turbo、freetype、harfbuzz、googletest、nlohmann/json、
  expat、concurrentqueue、highway、**shaderc**、**SPIRV-Cross**、Vulkan-Headers、
  volk、VulkanMemoryAllocator、**tint**、**abseil-cpp**。
  https://github.com/Tencent/tgfx/blob/main/DEPS
- `libpag/pathkit` 仓库描述(原文):"This library is extracted from the Skia
  library, which lets you use Skia's feature-rich PathOps API."(BSD-3-Clause)
  https://github.com/libpag/pathkit
- `libpag/skcms` 仓库描述:"A library for converting pixels in variety of
  formats."(BSD-3-Clause)https://github.com/libpag/skcms
- `include/pag/file.h::TagCode` — **83 个显式枚举项**,`End = 0` … `ImageScaleModes = 94`,
  末尾 `Count`。https://github.com/Tencent/libpag/blob/main/include/pag/file.h
- `src/codec/tags/` — **147 条路径**;`effects/` 下 12 个 `.h`
  (BrightnessContrast / Bulge / CornerPin / DisplacementMap / EffectCompositingOption /
  FastBlur / Glow / HueSaturation / LevelsIndividual / Mosaic / MotionTile / RadialBlur);
  `shapes/` 下 13 个 `.h`;`text/` 下 5 个族。
- `src/codec/utils/` — `DecodeStream` / `EncodeStream` / **`NALUReader`**(H.264)/
  `WebpDecoder`;`src/rendering/utils/LZ4Decoder`。
- `src/c/` — 存在 C API 层(19 个文件);`include/pag/c/`。
  (即使有 C API,§3.2 的构建/分发问题一条都不减少。)

**PAGX 与格式规范**
- `spec/pagx_spec.md`(118,024 B)、`spec/pagx_spec.zh_CN.md`(108,418 B)、
  `spec/pagx.xsd`(52,082 B)、`spec/html_subset.md`(29,268 B)。
  规范 §1.2 原文:"PAGX and binary PAG formats are bidirectionally convertible"。
  https://github.com/Tencent/libpag/tree/main/spec
- PAG 二进制格式规范页(有"下载 PDF"入口,**直链未核实**):
  https://pag.io/docs/en/pag-spec.html / https://pag.art/docs/pag-spec.html

**libpag-lite(官方非 C++ 实现)**
- `web/lite/README.md` 特性(原文):"基于 Javascript + WebGL,不使用 WebAssembly";
  "**仅支持播放包含单独一个 BMP 视频序列帧的 PAG 动效文件**";
  "包体只有 57KB,GZip 之后只有 15KB";
  "因为 FireFox 对 H264 视频的支持不兼容带 Bframe 的视频,所以简化版不支持 FireFox"。
  https://github.com/Tencent/libpag/blob/main/web/lite/README.md

**Rust 生态**
- **velato**(Linebender,Lottie→vello):**0.11.0**,发布 2026-07-21,
  累计 17,685 次下载;2026-07-01 提交 "Update to vello 0.9 (#110)";
  `Cargo.toml` `[features] default = ["vello"]`、`vello = ["dep:vello"]`
  —— **vello 是可选依赖,模型层可脱离 vello 使用**;
  `src/` 合计 212,933 B;lib.rs 文档明列不支持:位置关键帧缓动、时间重映射、
  文本、图片嵌入、高级形状(虚线/锯齿)、高级效果(运动模糊/投影)、
  色标处理、拆分旋转、拆分位置。
  https://crates.io/crates/velato / https://github.com/linebender/velato
- **vello_cpu**:0.0.9,2026-05-30。https://crates.io/crates/vello_cpu
- **crates.io 上没有 PAG / libpag 相关 crate**(搜索 "pag" 返回的是
  pag-lexer / pag-parser / pag-compiler,属 Paguroidea 解析器项目)。
- **accesskit 0.24** `Role` 相关变体:`Image`(4)、`Canvas`(51)、
  `ProgressIndicator`(101)、`SvgRoot`(119)、`Video`(131)、
  `GraphicsDocument`(136)、`GraphicsObject`(137)、`GraphicsSymbol`(138)。
  https://docs.rs/accesskit/0.24.0/accesskit/enum.Role.html
- ThorVG(Samsung,C++,Lottie 一等公民)是"如果非要用 C++ 就别用 libpag"
  的备选;**其与 OpenHarmony 的集成关系本次未核实**,不作为依据。
  https://github.com/samsung/thorvg

**本仓库(源码实查)**
- `crates/sv-shell/src/paint.rs` — `Painter` 六动词 + `PainterCaps`;
  `PathCmd`/`PathFill` 自有类型的理由;`fill_path` 无默认实现的理由;
  `TinySkiaPainter::fill_path` 的裁剪已知缺口。
- `crates/sv-shell/src/render.rs` — `paint_tree`(667–930 行)、
  `measure_leaf`(319–357 行)两处 `ElementKind` match。
- `crates/sv-shell/src/lib.rs` — `App::paint()` 帧前流水线(211–316 行)、
  `last_frame_key` 静止短路(227–232 行)、`animating → request_redraw`(311–313 行)。
- `crates/sv-ui/src/anim.rs` — `Channel`、`pump`、`active`。
- `crates/sv-shell/src/a11y.rs` — role 映射(105–115、219–221 行)。
- `crates/sv-ui/src/lib.rs:313` — `ElementKind` 定义(5 个变体)。
- `crates/sv-compiler/src/template.rs:310/320`、`codegen.rs:534`、
  `style.rs:41`、`crates/sv-macro/src/parse.rs:114` — 两个前端的标签表。
- `docs/CSS-SUPPORT.md:176` — `prefers-reduced-motion` 已列 C2。

---

## 附:如果这份文档被采纳,DESIGN.md 该加什么

建议记为 **ADR-11(动画素材:构建期 importer,运行期只认自有中间格式)**,
要点三句:

1. **格式解析永不进运行期。** PAG / Lottie / SVG-animate 一律在构建期
   转译成自有中间格式(`VectorClip` / `FrameAtlas`);运行期只认中间格式。
   直接后果:不引入任何 C++ 依赖、能力边界从运行期移到构建期、
   换格式不动运行期一行代码。
2. **动画不拥有时钟。** 时间轴归 `sv_ui::anim`,每帧把帧号写进场景树 →
   bump 版本 → 走既有帧调度;"有动画就排帧、没动画零功耗"因此自动成立。
   按素材帧率取整帧,帧号没变不 bump。
3. **场景树只加一个 `ElementKind::Animation`,格式差异收在 `AnimSource` 枚举里。**
   前端标签叫 `<animation>` 而不是 `<pag>` —— 标签描述用途,不绑格式。

以及在 §6 风险清单里加一条:
**`Repeat::Forever` 会让整个窗口失去静止零功耗性质**,
必须由"视口外不 pump + 窗口不可见暂停 + `prefers-reduced-motion`"三条共同兜住。
