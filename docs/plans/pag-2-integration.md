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
>
> **2026-07-22 对抗性复核后修订**:外部事实全部复查通过(逐条见 §11),
> 但**三处结论已被推翻或改写**,正文已就地改正,不再保留原文:
> §4 的"零功耗自动成立"是错的(`animating` 是 OR 短路,一开动画就每帧
> 全量重绘);§3.3 c3 的硬前置从"未核实"降级为**有反证**(官方 CLI 与
> 公开头文件里没有 `.pag → .pagx` 这条边);§1.1 的动词表已过期
> (`stroke_path` 已落地)。修订依据与全部复核记录见 **§11**。

---

## 0.0 ⚠️ 2026-07-22 落地后的更新:**两条裁决被实测改写**

本文写作时把 PAG 支持整体判为"现在不做",依据是两条前置:
`draw_image` 缺失、以及 c2 序列帧转换需要官方工具链。**两条都已经变了。**

1. **`Painter::draw_image` 已落地**(CPU/vello/Recording 三后端 + 后端无关的
   `PixelImage` 载体)。§1.1 那句"**没有 `draw_image`**。整个 Painter 里
   画不出一张位图"已过期。§6.2 要求的 `PainterCaps::image` 位**有意没加**
   —— 该位对三后端恒 true(`draw_image` 无默认实现,谁都得实现),
   恒真的位是噪音;理由与替代信号写在 `paint.rs` 的 `PainterCaps` 文档里。
2. 🔴 **c2 不需要 libpag —— 这条是本轮最大的改写。** `crates/sv-pag` 已落地:
   **零依赖纯 Rust** 读 PAG 容器。PAG 的位图序列帧数据完整存在于文件里
   (AE 插件导出时把每帧编成 WebP 直接写进去),格式全部公开在 Apache-2.0 的
   `src/codec`。所以 §3.3 c2′ 那张表把"构建期的 PAG 渲染器从哪来"列为
   首选 npm wasm + 未核实的 headless spike —— **对位图序列帧档已经不需要了**,
   c2′ 的适用范围应收窄成"**矢量档 / 视频档**才需要官方渲染器"。

场景树座位与时间轴通道也已落地(`ElementKind::Animation` + `AnimSource` +
`sv_ui::anim` 的时间轴表 + `sv_shell::animation` 注册表),端到端出图有测试。

**仍然成立的裁决**:运行期绑 libpag 是硬否(理由未变);
接入形态(一个 `ElementKind::Animation`、`<animation>` 标签、
构建期拒绝优先)全部照本文落地。

**仍然缺的**:`.pag` 里那些 WebP 帧的**解码器**(要引第三方 crate,
是独立的一次裁决)、差分帧重放、`<animation src>` 前端标签,
以及最要紧的 —— **sv-pag 从未在真实 `.pag` 上验证过**。

---

## 0. 结论速览

| 问题 | 裁决 |
|---|---|
| 三条路选哪条 | **(c) 离线转换**,且落点是 **c2 序列帧**;(a) 纯 Rust 全量解析否;(b) 在**运行期**绑定 libpag **否,且是硬否** |
| (b) 为什么是硬否 | libpag→tgfx 的 C++ 依赖闭包含 **pathkit(仓库自述"extracted from the Skia library")与 skcms**,ADR-3 排除 skia-safe 的理由在这里原样复现且更重;且 **Windows/Linux 无预编译原生库**。**注意这条只管运行期**:构建期另有 wasm 通道,见 §3.3 c2′ |
| c3(经 PAGX)算不算裁决 | **不算,是待验证假设**。官方 `pagx` CLI(npm `@libpag/pagx` 0.4.33)与公开 C++ 头文件里**都没有 `.pag → .pagx` 这条边**(实查,§10);规范里"双向可转"是格式设计声明,不是已交付工具。**不得写进 ADR** |
| 时间轴怎么合一 | PAG **不拥有时钟**:`sv_ui::anim` 增一条 Timeline 通道,每帧把帧号写进场景树。但"没动画零功耗"**不是自动的** —— 现有短路是 `unchanged && !animating`,`animating` 一真就每帧全量重绘,必须新增一条"帧号没变只重呈现"的短路。见 §4.2 |
| 场景树里是什么 | 新增**一个** `ElementKind::Animation`(**不是** `Pag`),载荷走 `ViewNode.anim: Option<Box<AnimData>>`(与既有 `input` 同款)。格式差异全部收在 `AnimSource` 枚举里,以后加 Lottie/SVG-animate **不再动 ElementKind** |
| 前端标签 | `<animation src="..." />`,**不叫 `<pag>` / `<lottie>`** —— 标签描述用途,不绑格式(textarea 先例:前端有 `Tag::TextArea`,运行时只有一个 kind) |
| 能力差异怎么办 | **构建期拒绝优先**:importer 在 build.rs 里就报错/告警。运行期只保留 `PainterCaps` 位查询。**不做**"运行期发现画不了就跳过" |
| 每帧成本 | 进 ADR-9 帧预算,并且要进 membench CI 场景。矢量档成本随素材复杂度走;序列帧档恒定 |
| **要不要现在做** | ~~**不要**~~ —— **已部分做了**,见 §0.0:`draw_image`、容器解析、场景树座位、时间轴均已落地;剩下的是 WebP 解码与前端标签 |
| 真要交付的最小件 | **不是 `<animation>`,是 `<img>`**。本文自己论证了 `draw_image` 是最前置缺口且 `<img>` 频率高一个数量级 —— 那最小可交付就该是 `<img>`。见 §8.6 |

---

## 1. 既有约束(读源码确认,不是转述)

### 1.1 Painter 的动词表就这么大

`crates/sv-shell/src/paint.rs` 里 `trait Painter` 的全部动词:

```
fill_rounded_rect / stroke_rounded_rect / glyph_run / push_clip / pop_clip
fill_path(&[PathCmd], PathFill, Color)
stroke_path(&[PathCmd], &StrokeStyle, Color)
caps() -> PainterCaps { external_texture: bool, blur: bool }
```

> **复核修正**:本文初稿写的是"六动词、任意路径描边 ❌"。主进程在本文成文
> 期间已把 `stroke_path` 连同 `StrokeStyle { width, cap: LineCap,
> join: LineJoin, miter_limit }` 一起落地(CPU/vello/Recording 三个后端全实现,
> 带 `stroke_paints_the_line_not_the_area` / `stroke_width_reaches_the_backend` /
> `line_cap_shape_reaches_the_backend` 三条测试)。**任意路径描边不再是缺口,
> 只剩 dash/trim 与渐变。** 下文 §3.1 对照表与 §8.2 前置清单已按此改写。

`fill_path` / `stroke_path` 是矢量动画唯一的地基。它们的注释把裁决写得很死,
直接决定了本文的可行域:

- `PathCmd` / `PathFill` / `StrokeStyle` 是**自有轻量类型,刻意不借
  kurbo/peniko** —— 理由是 vello 在本仓库是 optional dependency,让接口签名
  依赖只在某 feature 下存在的类型 = 把 GPU 后端焊死进 CPU 路径;
- `fill_path` **没有默认实现是刻意的** —— 给 no-op 默认会让新后端静默不画;
- 填充/描边只有**纯色** `Color`,**没有渐变、没有 paint 抽象**;
  描边**没有 dash、没有 trim path**(Lottie/PAG 的 trim 是常用件);
- `TinySkiaPainter::fill_path` 的**已知缺口**(源码注释原文):矩形裁剪没有
  接进去,"滚动容器内的路径图标不会被裁掉";
- **没有 `draw_image`**。整个 Painter 里画不出一张位图。
- **没有 transform 动词**。变换只能在上层把点烘进坐标 —— 这对每帧变的
  动画意味着**每帧重建整条路径**(§7.3 的分配成本由此而来)。

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

**但要把这条短路的真实语义读准(复核订正,这是本文最初读错的一处)**:
`animating` 在短路条件里是 **`&&` 的取反项**,也就是说它**否决**版本号短路。
`animating == true` 时 `paint()` **永不提前 return**,后面两个后端都照跑:

- CPU 档:`render_frame(...)` 无条件全量重画整张 pixmap,再全量拷进 softbuffer;
- vello 档:`render_cached(&doc, scale, unchanged)` 只在 `unchanged` 时跳过
  **场景重编码**,`layout_full_cached` + `render_to_texture` + `present`
  **每帧照跑**。

**结论:今天"有一个动画在跑"= 整窗口按 vsync 满速重绘,与那个动画本帧有没有
变化无关。** 这不是 `Repeat::Forever` 独有的问题,任何在途动画都如此
(现有 fade/scroll 动画时长都在 140–400ms,所以从来没暴露过)。
§4 的设计必须正面处理这一条。

### 1.4 ElementKind 的连带成本(实数)

`ElementKind::` 在 **9 个文件、79 处**出现
(`grep -ro ElementKind crates/ --include=*.rs`,2026-07-22 复核重数):

| 文件 | 出现数 | 是什么 |
|---|---|---|
| `sv-ui/src/lib.rs` | 22 | 枚举定义 + Doc 构造函数 + `dump()` |
| `sv-ui/src/tmpl.rs` | 18 | 模板原语 |
| `sv-shell/src/render.rs` | 15 | `measure_leaf` 一处 match + `paint_tree` 一处 match |
| `sv-shell/src/a11y.rs` | 10 | role 映射 |
| `sv-macro/tests/view.rs` | 6 | 宏前端测试 |
| `sv-ui/src/focus.rs` | 4 | 默认可获焦位 |
| `sv-ui/src/input.rs` | 2 | 输入路由 |
| `sv-shell/src/lib.rs` | 1 | — |
| `sv-compiler/src/emit.rs` | 1 | **初稿漏计** |

> 初稿写的是"8 个文件、约 61 处",每一格都偏低且漏掉 `sv-compiler/src/emit.rs`。
> 标题写了"(实数)"就得是实数 —— 这条已按实测重写。

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
| **PAGX** | libpag main 分支带 `spec/pagx_spec.md`(**118,024 B**)+ 中文版(108,418 B)+ `pagx.xsd`(52,082 B)。**XML 明文**格式,规范 §1.2 原文:"PAGX and binary PAG formats are bidirectionally convertible"(**但这是格式设计声明,不是已交付工具 —— 见下两行**) |
| **PAGX 是另一套栈** | `src/pagx` **181 个文件**,自带 `PAGXDocument` / `PAGStateMachine` / `PAGViewModel*` / `DataBind`,渲染经 `src/renderer/LayerBuilder` 直接落 **tgfx layers**;与经典 TLV 的 `src/codec`(175 条路径)是**并行的两套**。公开头文件里的转换边只有:`PAGXImporter`(PAGX XML→Doc)、`PAGXExporter`(Doc→XML)、`SVGImporter`/`HTMLImporter`(入)、`SVGExporter`/`HTMLExporter`/`PPTExporter`(出)。**没有 `PAGFile`(二进制 .pag)→ `PAGXDocument` 的公开入口** |
| **`pagx` CLI** | npm **`@libpag/pagx` 0.4.33**(2026-07-14 发布,**Apache-2.0**,`os: darwin/linux/win32`,`cpu: x64/arm64`,预编译原生二进制)。已发布命令:validate / render / optimize / format / bounds / font / embed。仓内更新的 `cli.md` 另有 import(**SVG/HTML→PAGX**)/ resolve / export(**PAGX→SVG/HTML/PPTX**)/ layout / verify。**两份命令表都没有 `.pag` 的任何一个方向**;`render` 也**没有 `--time`/`--frame`**,只出单张静态图 |
| **wasm 版 libpag** | npm **`libpag` 4.5.81**,**Apache-2.0** —— 预编译 wasm,**吃真 `.pag`、走完整 83-tag codec、不需要任何 C++ 工具链**。这是"Windows/Linux 无预编译库"这条结论**在构建期不成立**的原因,初稿漏了。见 §3.3 c2′ |
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

**其二(已被复核推翻,保留原判 + 反证):PAGX 不是"把离线转换从将就变成正解"的那把钥匙。**

初稿的推理是:`.pag` 的二进制解码有 83 个 tag、位打包属性、LZ4、H.264;
而 `.pagx` 是有 118 KB 规范 + XSD 的 XML,并且与二进制**双向可转**;
于是转换链是 `.pag →(官方工具)→ .pagx →(纯 Rust)→ 自有 IR`。

**这条链的第一段没有证据支持它存在。** 复核实查(2026-07-22):

- 官方 `pagx` CLI 的两份命令表(npm 已发布的 `@libpag/pagx` 0.4.33 README,
  与仓内更新的 `.codebuddy/skills/pagx/references/cli.md`)**都没有 `.pag`
  的任何一个方向**。`import` 收 **SVG/HTML**,`export` 出 **SVG/HTML/PPTX**;
- 公开 C++ 头文件里 `PAGXImporter::FromFile/FromXML` 只吃 **XML**,
  `PAGXExporter::ToXML` 只吐 XML;**没有 `PAGFile → PAGXDocument`**;
- `src/pagx`(181 文件,带 StateMachine/ViewModel/DataBind)与 `src/codec`
  (TLV,175 条路径)是**两套并行的栈**,`PAGX` 更像下一代格式而不是
  `.pag` 的 XML 序列化;
- 规范 §1.2 原文的语境是 **PAGX → PAG 方向**:"convert to PAG for optimized
  loading performance **during publishing**; use PAGX format for reading and
  editing **during development**"。它描述的是**创作侧 PAGX、发布侧 PAG**
  的单向生产流水线。

**所以 c3 的地位从"目标形态"降级为"待验证假设"**,而 §9 里"唯一硬前置"
这条也从"未核实"升级为**有反证**:不是"没查到",是"查了,公开面上没有"。
它仍可能存在于 PAGViewer / AE 导出插件 / 企业版工具里 —— 那要真拿到工具
才能写进计划,不能靠规范里一句话推。

**下面这个判断反而更硬了,并且不依赖 PAGX**:

```
.pag  --[构建期跑一个 libpag 实例,只在我们/CI 机器上]-->  帧序列 / 静态几何
      --[纯 Rust 打包]-->  自有中间格式(FrameAtlas / VectorClip)
      --[运行期]-->  Painter 动词
```

**C++(或 wasm)只出现在构建期,不进任何用户的依赖树、不进任何交叉编译
目标。** 这才是与 ADR-3 相容的那条形态 —— 而"构建期那个 libpag 实例从哪来"
的答案不是 PAGX,是 §3.3 c2′ 的 wasm 包。

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
| 2D 变换(位置/缩放/旋转/锚点/倾斜) | ❌ Painter 无 transform 动词,只能在上层把点烘进坐标(**动画 = 每帧重建整条路径**) |
| 渐变填充 / 渐变描边 | ❌ 只有 `Color` |
| 任意路径描边 + join/cap | ✅ `stroke_path` + `StrokeStyle`(**复核订正:已落地**) |
| 描边 dash / trim path | ❌ `StrokeStyle` 只有 width/cap/join/miter_limit |
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

**Windows/Linux 没有预编译原生库**(README 原文见 §2 表)。我们的主力开发与
首发平台是 Windows。这意味着**每一个使用者**(不只是我们)都要在自己机器上
过一遍这套 C++ 构建,CI 从分钟级推到十分钟级以上。这与"cargo add 就能用"
的分发承诺直接冲突,而 R4 的整章都在做发布工程。

> **复核加注(重要,初稿漏了):这条只对"运行期绑定"成立。**
> 官方另有 npm `libpag` **4.5.81 / Apache-2.0** 的 **wasm** 包 —— 预编译、
> 平台无关、Node 里跑、吃真 `.pag`、走完整 83-tag codec、**不需要任何
> C++ 工具链**。它进不了运行期(我们不背 wasm 运行时,也不把 GL 上下文
> 塞进渲染壳),但它**完全可以进构建期**。也就是说 §3.2 这一整节否掉的是
> **(b) 运行期绑定**,不能顺手把"构建期用官方实现"也一起否掉 ——
> 后者恰恰是 §3.3 c2 落地的最短路径(c2′)。

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

**裁决:运行期绑定 = 否。理由不是"暂时不做",是"与 ADR-3 的核心裁决不相容"。**
如果哪天要推翻,推翻的应该是 ADR-3 本身,而不是在 ADR-3 存续期间开个后门。
**但"构建期用官方实现出素材"不在本裁决范围内**,它与 ADR-3 无冲突
(ADR-3 管的是运行期依赖树与交叉编译目标),见 c2′。

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

#### c2′ — 那台"构建期的 PAG 渲染器"从哪来(复核补:初稿整节缺失)

c2 说"构建期离线出帧序列",但初稿从头到尾没回答**谁来解这个 `.pag`**。
把选项摆全,按代价排:

| 方案 | 拿得到吗 | 代价 | 结论 |
|---|---|---|---|
| **npm `libpag` 4.5.81(wasm)** | ✅ 预编译、Apache-2.0、平台无关 | Node + 一个离屏 GL 上下文;`PAGSurface`+`PAGPlayer` 逐帧 seek → 取像素 → 编 WebP | **首选**。吃真 `.pag`、完整 codec、零 C++ 工具链 |
| **`pagx render`(`@libpag/pagx` 0.4.33)** | ✅ 预编译、Apache-2.0、含 win32 x64/arm64 | 但**只吃 `.pagx`**,而 `.pag→.pagx` 无公开工具;且 render **无 `--time`/`--frame`**,只出单张 | 出不了序列,**当不了 c2 的工具** |
| **PAGViewer / AE 导出插件导出视频** | 需 GUI,macOS/Windows | 人工步骤,进不了 CI | 一次性素材可用,流水线不行 |
| **本地编译 libpag 原生库** | 需 CMake/Ninja/Node/VS2019+ | §3.2 那套构建 | **只在我们的机器上**也仍然贵,除非前两条都塌 |

**未核实(开工前第一个 spike,不许跳过)**:npm `libpag` 的 wasm 构建在
**纯 Node(无浏览器)**下能否离屏渲染 —— 它依赖 WebGL,Node 侧要么
headless-gl 要么 puppeteer 起个无头 Chromium(libpag 仓自己的
`tools/html-snapshot` 走的就是 puppeteer 路线)。**这条通了 c2 就通了;
不通就退回 PAGViewer 出视频 + ffmpeg 拆帧。**

#### c2″ — 别自造图集容器(复核补)

初稿默认 `FrameAtlas` 是自研格式。更省的落法是**直接用动态 WebP 或 APNG
单文件**:一个文件、自带帧时序、编解码器成熟、`AnimSource::Frames` 照样
复用。省掉的是"自研容器 + 自研索引 + 自研构建期打包器 + 它们的金样"。
只有当"多 DPI 多套 + 逐帧裁剪打包"真的成为瓶颈时,才值得自造图集。

#### c3 — PAG → PAGX → 自有中间格式(~~目标形态~~ **待验证假设,不得写进 ADR**)

见 §2.1 的链条图 —— **以及 §2.1"其二"里对它第一段的反证**。

- **可逆性极好**:中间格式是我们自己的,PAG 只是众多 importer 之一;
  换掉 PAG 不影响运行期一行代码。
- **能力边界从运行期移到构建期**:importer 遇到 track matte / 效果 /
  视频层,在 `build.rs` 里报错或告警 —— 用户在**编译**时知道,
  而不是在**用户机器上**看到画错的一帧。这是 (c) 相对 (b) 的额外红利。
- **三个缺口(第 1 条已由复核升级为反证)**:
  1. **`.pag → .pagx` 无公开工具**(复核实查,§10):`pagx` CLI 两份命令表
     都没有这条边;`PAGXImporter` 只吃 XML;`PAGFile → PAGXDocument`
     无公开入口。**这不是"未核实",是"查了、公开面上没有"。**
     在拿到可自动化的工具实物之前,c3 只能是假设。
  2. 能转出来 ≠ 我们画得出来。PAGX 的表达面 ≥ PAG 的表达面
     (它还多了 StateMachine / ViewModel / DataBind 这套交互层),
     转译器仍要面对 mask/matte/effect/blend 的取舍,
     只是不必再面对**二进制解码**这一层。
  3. **更省的一条边初稿完全没看见**:PAGX 侧官方**有** `SVGExporter` /
     `pagx export --format svg`。如果素材真能进 PAGX,那么
     `PAGX →(官方)→ SVG →(§8.2 第 2 条本来就要写的 SVG importer)→ IR`
     **一行新运行期代码都不用加**。这条比自研 PAGX 转译器省一整个 importer。
     代价:SVG 是静态的,时间轴另说(SMIL/逐帧导出,**未核实**官方
     export 是否带动画)。

| 维度 | c1 | c2(经 c2′) | c3 |
|---|---|---|---|
| 工作量 | 并入 c2 | 见 §8.5 修订行(**不含** `draw_image`) | **不估**(前置无证据) |
| 风险 | 无 | 中:体积、DPI 矩阵、**c2′ 的 wasm 离屏 spike** | 高:**转换工具是否存在本身就是未知数** |
| 可逆性 | 好 | 好 | **极好** |
| 对 ADR-3 冲击 | 无 | 无(C++/wasm 只在构建期) | 无 |

**总裁决:走 (c),落点是 c2(经 c2′ 的 wasm 构建期渲染 + c2″ 的
动态 WebP 容器)。c1 是它的自然副产品。c3 是一条**假设**,
在拿到 `.pag→.pagx` 工具实物之前不进任何计划、不进 ADR。**

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
→ bump 版本。

> **复核订正 1:它和 Opacity/ScrollY 并不"完全同构"。** 现有
> `struct Anim { doc, node, channel, from: f32, to: f32, start_ms, dur_ms }`
> 是一个**有限补间**:`pump` 对**所有** channel 统一算
> `t = clamp((now-start)/dur, 0, 1)` → ease-out-quad → 用 `t < 1.0` 决定 retain。
> Timeline 通道:**没有 `from/to`**;**不该过缓动曲线**(§4.4 自己说了
> AE 的曲线已经烘在素材里);**`Repeat::Forever` 在 `t < 1.0` 的 retain
> 语义下根本表达不出来**。所以 `Anim` 要么改成 enum(两种载荷两条推进
> 路径),要么 Timeline 单开一条 —— **这是结构改动,不是加一个枚举变体**,
> §5.5 的账单里必须补上这一笔。
>
> **复核订正 2:写 `anim.frame` 没有现成的 bump 入口。** Opacity 走
> `update_style`、ScrollY 走 `set_scroll`,两者都是既有的写入原语。
> 写 side 载荷需要新增一个 `Doc::update_anim` 之类的原语(参照 `input`
> 的写法)。同样不在账单里。
>
> **复核订正 3:播放状态放哪儿要想清楚。** §5.1 把
> `AnimState { playing, repeat, speed }` 放进 `ViewNode.anim`(场景树),
> 而驱动在 `anim.rs` 的 thread_local 队列 —— 于是 `pump` 每帧要
> `doc.read` 回查每个动画的 repeat/speed。既有两条通道**都没有这种回读**
> (参数全在 `Anim` 里)。要么状态只放队列(树上只留只读的 `frame`),
> 要么接受这次回读并写清为什么。**别两头都放。**

### 4.2 三条性质自动成立,**第四条不成立,必须新写短路**

| 性质 | 结论 |
|---|---|
| 有动画就排帧 | ✅ 自动:`pump` 返回 true → `animating` → `request_redraw()` |
| 与 ADR-6 帧对齐 | ✅ 自动:`anim::pump` 已排在 `tick()` 之前,动画写入与用户写入同轮 flush |
| 节点销毁不泄漏 | ✅ 自动:`pump` 里已有 `retain_mut` 的"节点没了就丢弃" |
| **动画期间的静止帧不重绘** | ❌ **不成立**。见下 |

**这一条是初稿写错的地方,而它是整个 §4 的承重墙。**

短路条件是 `if unchanged && !animating && !self.show_fps { return; }` ——
`animating` 是 `&&` 里的**取反项**,它**否决**版本号短路。于是:

- 动画在跑 → `animating == true` → **`paint()` 永不提前 return**;
- CPU 档每 vsync 全量 `render_frame` + 全量像素拷贝;
- vello 档每 vsync `layout_full_cached` + `render_to_texture` + `present`
  (只有场景**重编码**被 `unchanged` 跳过)。

也就是说,**"播完 → 出队 → 零功耗"是对的,但"播放中的静止帧不花钱"是错的**。
`Repeat::Forever` 只是把"播完"这一步删掉,它不是问题的来源 ——
**问题在于任何在途动画都是满 vsync 全量重绘**。现有 fade(≤400ms)/
smooth scroll(140ms)时间太短,从来没暴露过这条;一个 loading 动画会当场暴露。

**修法(必须写进实施项,不是可选优化)**:把 `animating` 拆成两个语义 ——
"**还要继续排帧**"(决定 `request_redraw`)与"**本帧内容变了**"
(决定要不要真绘制)。伪代码:

```
let still_running = anim::pump(now_ms);        // 决定是否继续 request_redraw
...
if unchanged && !self.show_fps {               // 帧键不变 = 本帧无事可做
    if still_running { ws.window.request_redraw(); }
    return;                                    // ← 新增:动画期间也能短路
}
```

配合 §4.4 的"帧号没变不 bump",24fps 素材在 144Hz 屏上才真的是
6 帧里只画 1 帧。**没有这条改动,§4.4 那句"白拿的省电"一分钱都拿不到。**

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

收益:24fps 素材在 144Hz 屏上每 6 个 vsync 才真重绘一次。
**但这份收益不是白拿的 —— 它以 §4.2 那条新短路为前提。**
只做"帧号没变不 bump"、不改 `paint()` 的短路条件,结果是版本号确实不变、
而 `animating` 仍然为真、于是照样每 vsync 全量重绘 —— 一分钱省不到,
还多写了一段取整逻辑。**两处必须成对落地。**

序列帧档**只能**按整帧走(它只有整帧)。矢量档理论上可以插值出素材里
不存在的中间帧,但那是在给自己找工作:AE 的关键帧曲线已经在素材里烘好了,
超采样只增加成本不增加信息。

### 4.5 循环、暂停与"危险品清单"

`AnimState { playing: bool, repeat: Repeat, speed: f32 }`,
`Repeat = Once | Count(u32) | Forever`。

**`Forever` 是危险品,必须在文档里标红**:它让 `active()` 恒真 →
每帧 `request_redraw`。**在 §4.2 那条短路落地之前**,这等于
**整个窗口**永久满 vsync 全量重绘;落地之后,退化为"永久排帧但多数帧
只重呈现" —— 仍然不是零功耗(GPU present / 事件循环不睡),
所以下面三条缓解措施一条都不能省:

1. **视口外不 pump**。滚出可视区的动画应该暂停 —— 虚拟化让节点数与帧成本
   解耦,但动画是"每帧真工作",不会被虚拟化省掉。
   **现状 `anim.rs` 没有视口概念,这是新增项。**
2. **窗口不可见/失焦时暂停**。**复核已查实(不再是"未核实"):本仓库
   渲染壳没有接。** `crates/sv-shell/src/lib.rs` 的 `WindowEvent` match
   只有 CloseRequested / RedrawRequested / Resized / ScaleFactorChanged /
   CursorMoved / MouseWheel / ModifiersChanged / KeyboardInput / Ime /
   MouseInput。`Occluded` 与 `Focused` 两个分支都要新增接线。
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

新增一个 kind ≈ 改 **9 个文件**(§1.4 修订表:`ElementKind` 共 79 处,
其中真需要动的 match 臂约 10–14 处):`measure_leaf`、`paint_tree`、
`dump()`、a11y role 映射、focus 默认位、`input.rs` 路由、`emit.rs`、
两个前端标签表 + 属性校验、`ELEMENT_NAMES`、宏前端测试。

**复核补:账单里初稿漏掉的三笔**(全在 §4.1 订正里论证过):

- `Anim` 结构改造(补间 vs 时间轴两种语义,`Forever` 的 retain 条件);
- 新增 `Doc::update_anim` 写入原语(写 side 载荷 + bump);
- `paint()` 短路条件改造(§4.2)—— 这一笔改的是**帧调度核心**,
  必须配回归测试:"动画在跑 + 帧号没变 → 不重绘"。
  它同时惠及既有的 fade/scroll,但也**最容易把静止短路改坏**,
  是这批改动里风险最高的一处。

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

**已核实(复核补,原为"未核实")**:accesskit **0.24.1 有**
`Node::is_busy() / set_busy() / clear_busy()`,另有 live region 一套
`live() / set_live(Live) / is_live_atomic() / set_live_atomic()`
(docs.rs 实查)。

于是 §7.2 的裁决可以更硬一点:**播放中用 `set_busy()` 标记,
name/value/description 一律不动**。这不是"没办法只好不报",
是"有正确的位置可报,所以更没有理由往 name 里塞帧号"。
`Live` 则**不要**用在动画上 —— live region 的语义是"内容变化要主动播报",
正是我们要避免的刷屏。

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
| ADR-10 改名 | ~~待裁决~~ ✅ 2026-07-23 落地(svelte-rs + .svelte + 标识符批) | ~~是~~ 已出清 |
| 双前端内核合并 | ~~未做~~ ✅ 2026-07-23 落地(公共 IR + 共享 codegen) | ~~是~~ 已出清 |
| `.svelte` 的 LSP | ~~未 spike~~ ✅ sv-lsp MVP 已落地(诊断;补全/跳转未做) | 部分(风险清单第 1 位仍在:rust-analyzer 转发未 spike) |
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
   **复核补**:官方 `pagx` 侧有 `SVGExporter` / `pagx export --format svg`,
   所以 SVG importer 顺带也是**未来吃 PAGX 素材的那条边**(§3.3 c3 缺口 3)。
3. **渐变 paint**,以及描边的 **dash / trim path**。
   **复核订正:任意路径 stroke(width/cap/join/miter)已落地**(§1.1),
   这一条从"三个缺口"缩到"渐变 + dash/trim"。渐变仍是硬缺口:
   任何真实矢量素材第一屏就会撞上,而 ADR-3b 冻结了 CPU 栈能力(见 §6.3)。
4. **(初稿漏)`paint()` 的动画期短路**(§4.2)。它不是性能优化,
   是"动画能不能在不烧 CPU 的前提下存在"的前提。

**做完这几样,PAG 的增量成本会从"数人月"掉到"数人周"** ——
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
3. **一条可自动化的"`.pag` → 帧序列/几何"构建期路径经 spike 确认**
   (可进 CI、许可允许)。**复核改写**:初稿把这条写成
   "`.pag → .pagx` 转换工具",而复核实查表明这条边在公开面上不存在
   (§2.1、§10)。真正要 spike 的是 **c2′ 表里的第一行**:
   npm `libpag`(wasm)能否在 Node 里离屏逐帧渲染。
   `.pag→.pagx` 若哪天真出现了,是 c3 的前置,不是 c2 的。
4. R4 改名 + crates.io 首发已完成,R5 鸿蒙至少跑通三角形 ——
   否则会在还没有"平台"的时候先做"素材"。

### 8.5 粗估(相对量级,不是承诺)

**复核已上调。初稿这张表乐观得离谱的地方在于:把"带资源生命周期的动词"
按"纯几何动词"计价,把互相依赖的行并列计数,并且给了一个前置不成立的估算。**

| 项 | 初稿 | **修订** | 备注 |
|---|---|---|---|
| `draw_image` 动词(CPU + vello + Recording) | 0.5–1 人周 | **1–2.5 人周** | 初稿低估。它不是 `fill_path` 那种纯几何动词:要含**纹理/位图的生命周期**(vello 端上传与缓存键、CPU 端采样与缩放)、多 DPI 取整、`PaintCmd::Image` 金样口径(记 id+bbox+采样,不能记像素)、`PainterCaps::image` 位、双后端对拍。仓内先例:`fill_path`+`stroke_path` 两个**无资源**动词已经是主进程一整个 commit 的量 |
| `paint()` 动画期短路(§4.2) | **漏计** | **0.3–0.5 人周** | 改的是帧调度核心,需回归测试兜住静止短路不被改坏 |
| c2 最小闭环:`<animation>` + `ElementKind::Animation` + `AnimSource::Frames` + Timeline 通道 + 金样 | 2–3.5 人周(含 `draw_image`) | **2.5–4 人周**(**不含** `draw_image`,**不含**构建期工具) | 初稿把 `draw_image` 折进这一行又在上一行单列 = 重复计价的反面(依赖被吞掉)。另加 §5.5 复核补的三笔 |
| 构建期图集转换器(c2′ + c2″) | 折在上行 | **1–3 人周 + 一个前置 spike** | 上限取决于 c2′ 那个 spike:wasm 离屏通了取下限,退回 PAGViewer+ffmpeg 取上限。**spike 不通则整行不可估** |
| SVG 静态 importer(shape 子集 + 变换,构建期) | 3–5 人周 | **3–5 人周**(维持) | sv-arco 本来就要付这笔钱 |
| 矢量剪辑 IR + Lottie importer(shape 子集、关键帧) | 6–10 人周 | **6–10 人周**(维持) | 参照 velato:212,933 B Rust / 2024-03 起算 / 仍不完整 |
| PAG importer(经 PAGX) | 增量 3–6 人周 | **不估** | **前置(§8.4 #3)已被反证**,见 §2.1。前提不成立的估算不该给数字 |
| PAG importer(直接吃二进制) | 上行 2 倍以上 | **数人月起,且 BMP/Video 素材直接不支持** | 与其估这个不如别做 |
| 运行期绑定 libpag | 不估 | **不估** | 见 §3.2:不是工作量问题 |

**这张表整体还漏了一类成本:素材验收。** 动效的"对不对"最终是人眼判定,
不是测试判定 —— §7.4 的命令流金样能抓住"画错位置",抓不住"缓动不对味"。
velato 的 issue 史里这一项是主要成本。做 PAG/Lottie 必须给设计侧留一条
"出图对拍 + 人工签收"的流程,不要假装它是零。

### 8.6 如果非做不可,最小可交付是什么

**复核订正:初稿给的"最小"不是最小。** 它把三件事捆成一件交付
(位图动词 + 动画节点/时间轴 + 构建期转换工具),其中第三件今天还是个
未验证的 spike。真正的最小可交付要拆成两级,**第一级今天就该做,
而且和 PAG 没关系**:

**第 0 级(本文最该带走的一条结论)—— 只做 `<img>`:**

> `Painter::draw_image` + `PainterCaps::image` + 一个 `<img src>` 标签 +
> 金样口径。

本文自己论证了 `draw_image` 是最前置的缺口、`<img>` 的使用频率比动画
高一个数量级、而且它是序列帧路线的硬前置。那么排序上它就该单独先交,
**不要挂在动画的裙带上一起排**。它落地后,第 1 级只剩"加时间"。

**第 1 级 —— 序列帧动画:**

> 构建期把 `.pag` 转成**动态 WebP / APNG 单文件**(c2″,别自造图集容器)+
> 一个 `<animation>` 标签 + `ElementKind::Animation` + Timeline 通道 +
> §4.2 的动画期短路。

不引入任何 C++/wasm 到用户依赖树,CPU/GPU/鸿蒙三档能力一致,
并且**可逆**:哪天有了矢量 IR,把 importer 的输出换掉即可,
标签、场景树节点、Timeline 通道、a11y 映射、金样测试**一行都不用改**。

**比第 1 级还省的两条(初稿漏了,遇到具体素材先问这两句)**:

- **只要静态图标?** 走 §8.2 第 2 条的 SVG importer,一帧都不用动。
  设计侧多数"PAG 动效"需求其实是"我要个会动的 loading",
  而 loading 用 CSS 式旋转 + 一个 SVG 路径就够,**根本不需要素材管线**。
- **只要一段短动效,且只此一处?** 构建期用 PAGViewer 导出视频 →
  ffmpeg 拆帧 → 动态 WebP,手工一次性完成,**零代码、零流水线**。
  只有当"素材会持续新增"时,构建期工具链才值得存在。

---

## 9. 待核实清单(开工前必查)

**复核已结掉 3 条(1 / 5 / 6),新增 3 条(10 / 11 / 12)。**

| # | 待核实项 | 状态 | 为什么重要 |
|---|---|---|---|
| 1 | `.pag → .pagx` 是否有开源/可自动化的转换工具 | ❌ **已查,公开面上不存在**(`pagx` CLI 两份命令表 + 公开头文件均无此边,§10)。**从"未核实"改判为"有反证"** | c3 的硬前置,现已否掉 c3 的"目标形态"地位 |
| 2 | PAG 二进制格式规范 PDF 的直链与版本 | 未核实 | 决定 (a) 子集路线的可行性判断精度 |
| 3 | libpag 在 Windows 上的 GL 来源(是否 ANGLE / D3D) | 未核实 | 只在万一重启 (b) 时才需要 |
| 4 | tgfx 是否有软件(CPU)光栅后端 | 未核实 | 同上 |
| 5 | accesskit 是否有 `busy` 一类属性 | ✅ **有**:0.24.1 `Node::is_busy/set_busy/clear_busy` + `Live` 一套(docs.rs 实查) | §7.2 已按此改写 |
| 6 | winit `Occluded`/`Focused` 在本仓库是否已接线 | ✅ **没接**(`sv-shell/src/lib.rs` 的 `WindowEvent` match 无此两臂) | §4.5 已按此改写。**这是仓库内事实,当初就不该写"未核实"** |
| 7 | `vello::Scene` 能否被 `vello_cpu`(0.0.9,2026-05-30)消费 | 未核实 | 决定"velato 当 GPU 档实现、CPU 档怎么办"的答案 |
| 8 | 纯 Rust 的 H.264 解码器是否存在可用者 | 未核实 | 只影响"直接吃二进制 PAG"这条已被否掉的路 |
| 9 | libpag 企业版 / `NOASSERTION` 许可判定的法务口径 | 未核实 | 只在万一重启 (b) 时才需要 |
| **10** | **npm `libpag`(wasm 4.5.81)能否在纯 Node 下离屏逐帧渲染 `.pag`**(需 WebGL:headless-gl 还是无头 Chromium?) | 未核实 | **c2 的唯一硬前置,取代了原来的第 1 条**。通了 c2 就能开工 |
| **11** | `pagx export --format svg` 导出的是否带动画(SMIL / 逐帧) | 未核实 | 决定 §3.3 c3 缺口 3 那条"复用 SVG importer"的省力路能省多少 |
| **12** | 动态 WebP / APNG 的纯 Rust 解码在本仓库可用性(`image` crate 的 animation feature?体积?) | 未核实 | c2″ 的前置:决定"不自造图集容器"能不能成立 |

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
  规范 §1.2 原文(复核逐字核对):"PAGX is a plain XML file (`.pagx`) that can
  reference external resource files … **PAGX and binary PAG formats are
  bidirectionally convertible: convert to PAG for optimized loading performance
  during publishing; use PAGX format for reading and editing during development
  and review.**" —— 注意后半句把方向说成了 **PAGX →(发布)→ PAG**。
  https://github.com/Tencent/libpag/tree/main/spec
- PAG 二进制格式规范页(有"下载 PDF"入口,**直链未核实**):
  https://pag.io/docs/en/pag-spec.html / https://pag.art/docs/pag-spec.html

**PAGX 工具链(复核新增 —— 这一组是推翻 c3"目标形态"地位的证据)**
- npm **`@libpag/pagx` 0.4.33**,发布 **2026-07-14T13:18:47Z**,`license: Apache-2.0`,
  `os: [darwin, linux, win32]`、`cpu: [x64, arm64]`、`engines.node >= 16.7.0`,
  `bin/<platform>-<arch>/pagx[.exe]` 为**预编译原生二进制**(`scripts/check-binaries.js`
  原文:"The native `pagx` binaries live in `bin/<platform>[-<arch>]/` and are gitignored")。
  已发布 README 命令表:**validate / render / optimize / format / bounds / font / embed**。
  https://registry.npmjs.org/@libpag/pagx
  https://github.com/Tencent/libpag/blob/main/cli/npm/README.md
- 仓内更新版 CLI 参考 `.codebuddy/skills/pagx/references/cli.md` 命令表:
  **verify / render / format / layout / bounds / font / import / resolve / export**。
  其中 `pagx import` 原文:"Convert a file from another format to a standalone PAGX file.
  **Two input formats are supported: SVG and HTML**";`pagx export` 原文:
  "export PAGX to **SVG/HTML/PPTX**"。
  `pagx render` 选项表:`-o / --format png|webp|jpg / --scale / --crop / --id /
  --xpath / --quality / --background / --font / --fallback` —— **没有 `--time`
  或 `--frame`**,只能出单张静态图。
  **两份命令表都不含 `.pag` 的任何一个方向。**
- 公开 C++ 头文件(实读):
  `include/pagx/PAGXImporter.h` — `FromFile(filePath)` / `FromXML(xmlContent)` /
  `FromXML(data, length)`,注释原文 "**PAGXImporter parses PAGX XML format into
  PAGXDocument**";
  `include/pagx/PAGXExporter.h` — `ToXML(const PAGXDocument&, Options)`,注释原文
  "**PAGXExporter exports PAGXDocument to PAGX XML format**";
  另有 `SVGImporter.h` / `HTMLImporter.h`(入)与 `SVGExporter.h` / `HTMLExporter.h` /
  `PPTExporter.h`(出);`src/renderer/LayerBuilder.h` 把 `PAGXDocument` 映到
  **tgfx layers**。**全组头文件中没有 `PAGFile`(二进制 `.pag`)→ `PAGXDocument`
  的入口。**
- `src/pagx/` **181 个文件**(含 `DataBindRuntime` / `LayoutContext` / `PAGStateMachine` /
  `PAGViewModel*`);`src/codec/` **175 条路径**(其中 `src/codec/tags/` 146 条)。
  两者是并行的两套栈。
- npm **`libpag` 4.5.81**,`license: Apache-2.0`,description "Portable Animated Graphics"
  —— 官方 **wasm** Web SDK 的预编译包(**平台无关,不需要 C++ 工具链**;
  能否在纯 Node 下离屏渲染 = §9 第 10 条,**未核实**)。
  https://registry.npmjs.org/libpag

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

**本仓库(源码实查;标 ★ 的为复核重查/订正)**
- ★ `crates/sv-shell/src/paint.rs` — `Painter` **七动词**(`fill_rounded_rect` /
  `stroke_rounded_rect` / `glyph_run` / `push_clip` / `pop_clip` / `fill_path` /
  **`stroke_path`**)+ `PainterCaps { external_texture, blur }`;
  `PathCmd`/`PathFill`/`StrokeStyle` 自有类型的理由;`fill_path` 无默认实现的理由;
  `TinySkiaPainter::fill_path` 的裁剪已知缺口;
  `PaintCmd::Path { cmds, fill, color, bbox }` / `PaintCmd::StrokePath { cmds,
  width, cap, join, color, bbox }`。
  `StrokeStyle { width, cap: LineCap, join: LineJoin, miter_limit }`,
  `LineCap = Butt|Round|Square`,`LineJoin = Miter|Round|Bevel`。
  **`stroke_path` 尚在工作区未提交**(`git diff` 显示 paint.rs +202 行),
  初稿成文时确实还没有 —— 但结论受影响,已在 §1.1/§3.1/§8.2 就地改正。
- ★ `crates/sv-shell/src/vello_backend.rs` — `VelloPainter::caps()` 硬编码
  `external_texture: false, blur: true`;`render_cached(doc, scale, scene_unchanged)`
  仅在 `scene_unchanged` 时跳过 `scene.reset() + paint_tree`,
  **`layout_full_cached` / `render_to_texture` / present 每帧照跑**。
- ★ `crates/sv-shell/src/lib.rs::App::paint()` — 短路条件原文
  `if unchanged && !animating && !self.show_fps { return; }`(`animating`
  **否决**版本键短路);CPU 档 `render_frame` 无条件全量重绘 + 全量像素拷贝。
- ★ `crates/sv-ui/src/anim.rs` — `struct Anim { doc, node, channel, from, to,
  start_ms, dur_ms }`;`pump` 对所有 channel 统一 `t=clamp((now-start)/dur,0,1)`
  → ease-out-quad → `retain` 条件 `t < 1.0`;`Channel = Opacity | ScrollY`;
  写入走既有的 `update_style` / `set_scroll`(**没有写 side 载荷的原语**)。
- ★ `crates/sv-shell/src/lib.rs` 的 `WindowEvent` match — **无 `Occluded`、
  无 `Focused`** 分支。
- ★ `ElementKind` 出现数(`grep -ro`):9 文件 / 79 处,见 §1.4 修订表。
- `crates/sv-ui/src/lib.rs:352` — `pub input: Option<Box<input::InputState>>`
  (§5.1 "与既有 input 同款"的先例,属实)。
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
   *(复核:这一条是全文唯一经得起写进 ADR 的裁决 —— 它不依赖任何
   尚未核实的外部工具。)*
2. **动画不拥有时钟。** 时间轴归 `sv_ui::anim`,每帧算出帧号写进场景树 →
   bump 版本 → 走既有帧调度。按素材帧率取整帧,**帧号没变不 bump**,
   并且 **`paint()` 的短路条件必须同步改成"帧键没变就不绘制,
   哪怕还在动画中"**(见 §4.2)—— 两者缺一,"没动画零功耗"就只是口号。
   *(复核订正:初稿写的是"因此自动成立",错。)*
3. **场景树只加一个 `ElementKind::Animation`,格式差异收在 `AnimSource` 枚举里。**
   前端标签叫 `<animation>` 而不是 `<pag>` —— 标签描述用途,不绑格式。

**不进 ADR 的**(复核加):`.pag → .pagx → 自有 IR`(c3)。它的第一段
在公开工具面上不存在(§2.1、§10),是假设不是裁决。

以及在 §6 风险清单里加两条:
- **任何在途动画都会让整窗口按 vsync 满速全量重绘**(不只是 `Repeat::Forever`
  —— 现有短路条件里 `animating` 否决版本键短路)。必须先改短路条件,
  再谈动画能力。
- **`Repeat::Forever` 会让窗口永久排帧**,必须由"视口外不 pump +
  窗口不可见暂停(`Occluded`/`Focused` 目前**未接线**)+
  `prefers-reduced-motion`"三条共同兜住。

---

## 11. 复核记录(对抗性复核,2026-07-22)

复核立场:**默认这份产物有问题**。范围 = 逐条重查外部事实、重读本仓库源码、
找编造与虚假确定性、找与 ADR-3/Painter/鸿蒙/optional vello/单线程的正面冲突、
压工作量估算、找被漏掉的省力替代。正文已就地改正,本节只记**判决与依据**。

### 11.1 先说结论:事实基座是干净的,问题出在推论层

初稿自称"没有一个版本号、API 名或性能数字是推测的"。**复核逐条重查,这句话成立。**
下面每一条都是本次独立复查、与初稿逐字/逐位吻合的(GitHub API + npm registry +
crates.io API + docs.rs,查询日 2026-07-22):

| 初稿断言 | 复核结果 |
|---|---|
| libpag:C++、5737 star、~208 MB、main 3758 文件、`pushed_at` 2026-07-22T10:07:13Z、许可 `NOASSERTION` | ✅ 全部吻合(API 返回 `size: 212945` KB、`stargazers_count: 5737`、tree 3758 项) |
| release v4.5.81 @ 2026-07-22T08:11:52Z,34 个产物,含 `libpag_4.5.81_ohos_arm64-v8a.har`,**无 Windows/Linux** | ✅ 全部吻合(资产清单逐个核对) |
| libpag `DEPS` common **10 个** | ✅ 逐个吻合:vendor_tools / tgfx / libavc / rttr / harfbuzz / lz4 / libexpat / SheenBidi / libxml2 / woff2 |
| tgfx `DEPS` common **21 个**,含 pathkit、skcms、shaderc、SPIRV-Cross、tint、abseil | ✅ 逐个吻合(解 base64 后按 JSON 数组核对) |
| `libpag/pathkit` 描述"extracted from the Skia library…PathOps API"、BSD-3-Clause | ✅ **逐字吻合** |
| `libpag/skcms` 描述"A library for converting pixels in variety of formats."、BSD-3-Clause | ✅ 逐字吻合 |
| `TagCode` **83 项、值域到 94** | ✅ 实解 `include/pag/file.h`:84 个成员 = 83 个 tag + 末尾 `Count`,`ImageScaleModes = 94` |
| `spec/` 三个文件 118 KB / 108 KB / 52 KB | ✅ 精确到字节:118,024 / 108,418 / 52,082 |
| 规范 §1.2 "PAGX and binary PAG formats are bidirectionally convertible" | ✅ 逐字吻合(**但见 11.2 第 ② 条:上下文被截断了**) |
| libpag-lite:纯 JS+WebGL、57 KB/gzip 15 KB、"仅支持播放包含单独一个 BMP 视频序列帧的 PAG 动效文件"、FireFox/Bframe 那段 | ✅ **三处逐字吻合** |
| README "We currently only publish precompiled libraries for iOS, Android, macOS, Web, and OpenHarmony." | ✅ 逐字吻合(README 第 79 行) |
| velato 0.11.0 / 2026-07-21 / 17,685 下载 / "Update to vello 0.9 (#110)" @ 2026-07-01 / src 212,933 B | ✅ 全部吻合(crates.io API + GitHub commits) |
| accesskit 0.24 `Role`:Image=4 / Canvas=51 / ProgressIndicator=101 / SvgRoot=119 / Video=131 / GraphicsDocument=136 / GraphicsObject=137 / GraphicsSymbol=138 | ✅ **八个判别值全对**(docs.rs,`#[repr(u8)]`) |
| crates.io 上无任何 PAG 相关 crate | ✅ 吻合(搜 "pag" 返回 pag-lexer/pag-parser/pag-compiler,属 Paguroidea) |

**"作者说官方支持鸿蒙"——查证属实,且证据比初稿更具体**:release 资产里有
`libpag_4.5.81_ohos_arm64-v8a.har`(1,654,079 B)与 `.symbol.zip`;README 平台表
列 **HarmonyOS Next 5.0.0(12)+**;仓库根有完整 `ohos/` hvigor 工程。
初稿关于"`.har` 是 DevEco/ArkTS 包格式、集成范式是'你用 ArkUI 我给你组件'、
与我们 XComponent 自绘 surface 冲突"的推论,**成立**。

**没有找到一处编造的版本号、API 名或性能数字。** 这一条要明说,
因为它决定了后面的批评该往哪儿使劲:问题不在"查得不实",在"从实里推得太满"。

### 11.2 推翻的结论(3 条,均已就地改正)

**① §4.2「四条性质自动成立、不需要新写任何短路逻辑」—— 错,且是承重墙。**

`App::paint()` 的短路是 `if unchanged && !animating && !self.show_fps { return; }`。
`animating` 在 `&&` 里取反,**它否决版本键短路**:动画在跑 → 永不提前 return →
CPU 档每 vsync 全量 `render_frame` + 全量像素拷贝;vello 档每 vsync
`layout_full_cached` + `render_to_texture` + present(只有场景重编码被
`scene_unchanged` 跳过)。

连带塌掉的是 §4.4 那句"24fps 素材在 144Hz 屏上每 6 个 vsync 才真重绘一次
—— 白拿的省电":**这份省电一分钱都拿不到**,除非新增一条"帧键没变就不绘制、
哪怕还在动画中"的短路。初稿把它算成白拿,还据此把 `Forever` 说成唯一危险品
—— 实际上**任何在途动画**都是满 vsync 全量重绘,只是既有 fade(≤400ms)/
smooth scroll(140ms)太短没暴露过。

已改:§1.3 加短路语义实录、§4.2 整节重写并给出修法伪代码、§4.4 加前提、
§4.5 重新界定 `Forever` 的危害边界、§5.5 与 §8.5 补上这笔工作量、
附录 ADR-11 第 2 条改写。

**② §2.1「PAGX 的存在把离线转换从将就变成了正解」—— 前置不存在,c3 降级为假设。**

初稿把规范 §1.2 一句"bidirectionally convertible"当成 `.pag → .pagx` 链路
的存在证明。复核去查了**工具实物**:

- npm `@libpag/pagx` **0.4.33**(2026-07-14,Apache-2.0,预编译含 win32
  x64/arm64)已发布命令表:validate / render / optimize / format / bounds /
  font / embed;
- 仓内更新版 `cli.md` 命令表:verify / render / format / layout / bounds /
  font / **import(SVG,HTML → PAGX)** / resolve / **export(PAGX → SVG/HTML/PPTX)**;
- 公开头文件:`PAGXImporter` 只吃 XML,`PAGXExporter` 只吐 XML,
  另有 SVG/HTML 的进出口,`LayerBuilder` 把 `PAGXDocument` 映到 tgfx layers。

**三处都没有 `.pag` 的任何一个方向。** 且 `src/pagx`(181 文件,带
StateMachine/ViewModel/DataBind)与 `src/codec`(TLV,175 条路径)是并行两套栈,
PAGX 更像下一代格式;规范原文的完整上下文是 "convert to PAG for optimized
loading performance **during publishing**; use PAGX format for reading and
editing **during development**" —— 描述的是 **PAGX → PAG 的单向生产流水线**,
初稿引用时把这半句略掉了,而恰恰是这半句决定了方向。

**这已经不是"未核实",是"查了、公开面上没有"。** 一条被自己标为"唯一硬前置
未核实"的边,不该同时被写成"关键发现""唯一相容路""目标形态"并提议进 ADR。
这是本文最典型的**虚假确定性**:证据强度是"规范里有一句话",
结论强度是"总裁决"。

已改:§0 速览加一行、§2 表拆成三行事实、§2.1 其二整节重写并给反证、
§3.3 c3 标题降级 + 缺口 1 改判、§8.4 触发条件第 3 条重写、§8.5 该行改"不估"、
§9 第 1 条改判、§10 新增一整组证据、附录加"不进 ADR 的"。

**③ §1.1「Painter 动词表就这么大」—— 已过期(工作区)。**

`crates/sv-shell/src/paint.rs` 现有 `stroke_path(&[PathCmd], &StrokeStyle, Color)`,
`StrokeStyle { width, cap: LineCap, join: LineJoin, miter_limit }`,
CPU/vello/Recording 三后端全实现,带 3 条测试,`PaintCmd::StrokePath` 金样口径齐。
初稿的 §3.1 对照表把"任意路径描边"标 ❌、§8.2 把它列为三大前置之一 —— 都失效。
现在只剩 **渐变** 与 **dash/trim**。

这一条对作者不算苛责(`stroke_path` 在工作区未提交,初稿成文时确实没有),
**但结论受影响就必须改**,而且"读源码确认,不是转述"这个标题意味着
它要对着**当前工作树**负责,不是对着 HEAD。

已改:§1.1 动词表 + 新增 transform 缺口条、§3.1 对照表两行、
§8.2 第 3 条改写并新增第 4 条、§10 本仓库证据组重写。

### 11.3 改判 / 上调(5 条)

**④ §1.4「8 个文件、约 61 处(实数)」—— 不是实数。**
`grep -ro ElementKind crates/ --include=*.rs` 实测 **9 文件 / 79 处**,
每一格都比初稿高,且漏掉 `sv-compiler/src/emit.rs`。既然标题写了"(实数)",
数就得能复算。已按实测重写。

**⑤ §4.1「Timeline 与 Opacity/ScrollY 完全同构」—— 不同构。**
现有 `Anim` 是**有限补间**(`from`/`to`/`dur_ms`,统一过 ease-out-quad,
`retain` 条件 `t < 1.0`)。Timeline 没有 `from/to`、不该过缓动(§4.4 自己说了)、
`Forever` 在 `t < 1.0` 的 retain 语义下表达不出来。这是**结构改造**。
另外两笔初稿没算:写 side 载荷需要新增 `Doc::update_anim` 原语(既有两条通道
走的是现成的 `update_style`/`set_scroll`);`AnimState` 放场景树而驱动在
thread_local 队列 = 状态两头放,`pump` 每帧要 `doc.read` 回查。已在 §4.1
加三条订正、§5.5 补账单。

**⑥ §9 里有两条是"自己仓库里 30 秒能查完"却写了"未核实"。**
第 5 条 accesskit `busy`:**有** —— 0.24.1 `Node::is_busy/set_busy/clear_busy`,
外加 `Live` 一套。第 6 条 winit `Occluded`/`Focused`:**本仓库没接** ——
`sv-shell/src/lib.rs` 的 `WindowEvent` match 只有十个分支,两个都不在。
外部事实查到了字节数,自己仓库里的接线反而挂"未核实",顺序反了。已结掉并改写。

**⑦ §8.5 估算乐观,三行不成立。**
- `draw_image` **0.5–1 人周 → 1–2.5 人周**。它不是 `fill_path` 那种纯几何动词,
  带**资源生命周期**(vello 端纹理上传/缓存键、CPU 端采样与缩放)、多 DPI 取整、
  `PaintCmd::Image` 金样口径、caps 位、双后端对拍。仓内先例摆着:
  `fill_path` + `stroke_path` 两个**无资源**动词已经是主进程一整个 commit 的量。
- c2 最小闭环那行把 `draw_image` **折进去**又在上一行**单列**,依赖被吞掉;
  且把"构建期图集转换"当成一个小格子,而那正是 ⑧ 里说的"工具今天不存在"。
  已拆成三行并上调。
- PAG importer 经 PAGX **3–6 人周 → 不估**:前置已被反证,前提不成立的估算
  不该给数字(初稿自己对 (b) 就是这么处理的,标准要一致)。
- 全表还漏了一类:**素材验收成本**。动效的"对不对"是人眼判定,
  §7.4 的命令流金样抓得住"画错位置",抓不住"缓动不对味"。velato 的 issue 史
  里这是主要成本。已加。

**⑧「构建期那台 PAG 渲染器从哪来」—— 初稿整节缺失,而这是 c2 的命门。**
初稿反复说"构建期离线出帧序列",却从没回答**谁来解这个 `.pag`**。
复核补了 c2′ 一整节和一张四行对照表。关键遗漏是:
**npm `libpag` 4.5.81 / Apache-2.0 的 wasm 包** —— 预编译、平台无关、
吃真 `.pag`、走完整 83-tag codec、**零 C++ 工具链**。
初稿"Windows/Linux 无预编译库"这条结论对**运行期绑定**成立,
但被顺手扩用到了构建期,而构建期它不成立。
(注意这**不动摇** (b) 的硬否 —— (b) 否的是运行期依赖树与交叉编译目标,
那部分论证复核后完全站得住,而且证据比初稿说得还硬。)
同时查到一条反向事实:`pagx render` **没有 `--time`/`--frame`**,只出单张静态图
—— 所以即使素材真进了 PAGX,官方 CLI 也**出不了序列帧**。§8.6 那个
"最小可交付 = 构建期出 WebP 图集"在初稿里是没有工具支撑的。

### 11.4 被漏掉的更省力替代(4 条,已补进 §3.3 / §8.6)

1. **先只做 `<img>`。** 初稿自己论证了 `draw_image` 是最前置缺口、
   `<img>` 频率高一个数量级、且是序列帧的硬前置 —— 那最小可交付就是 `<img>`,
   不该挂在动画的裙带上一起排。这是全文推论链的自然终点,初稿走到门口没进去。
2. **动态 WebP / APNG 单文件,别自造 `FrameAtlas` 容器。** 自带帧时序、
   编解码成熟、`AnimSource::Frames` 照样复用,省掉自研容器+索引+打包器+金样。
3. **`PAGX → SVG`(官方 `SVGExporter` / `pagx export --format svg`)复用
   SVG importer。** 初稿把 SVG importer 列为 PAG 的前置,却没发现官方工具链里
   PAGX→SVG 是一条**现成的边** —— 走这条,矢量档一行新运行期代码都不用加。
   (导出是否带动画 = 新增待核实第 11 条。)
4. **一次性素材:PAGViewer 导出视频 → ffmpeg 拆帧,零代码零流水线。**
   只有"素材会持续新增"时构建期工具链才值得存在。初稿默认要建流水线,
   没先问这一句。

另外,§8.2 提到的场景里很大一部分其实是 loading —— 这类需求用 SVG 路径 +
既有 `anim` 通道就够,**根本不进素材管线**。这是最省的一条,已写进 §8.6。

### 11.5 与既有约束的正面对照(逐条,复核确认)

| 约束 | 初稿是否正面回答 | 复核判定 |
|---|---|---|
| **ADR-3 排除 C++ 重依赖** | 是,且是全文最强的一段 | ✅ 论证成立,证据(28 个仓库、pathkit=Skia 零件、depsync/CMake/NDK 构建前置)全部复查属实。**唯一瑕疵**:把"运行期不许"扩用到了构建期(见 ⑧) |
| **Painter 抽象 / optional vello** | 是 | ✅ 正确把握了"`PathCmd` 不借 kurbo 是因为 vello 是 optional dep";§6.2"caps 位对应动词不对应特性"的裁决是对的。**扣分**:动词表过期(③),且没点出**无 transform 动词 ⇒ 动画每帧重建整条路径**这条真实成本(已补进 §1.1/§3.1) |
| **鸿蒙交叉编译** | 是 | ✅ `.har`/hvigor 与 XComponent 自绘 surface 的范式冲突论证成立;"再开一个与 ADR-5 两大风险点同量级的口子"判断合理。序列帧档三平台一致的论证也成立 |
| **单线程响应式模型** | **否 —— 没有正面回答** | ⚠️ 全文没有一处讨论"构建期 importer 产出的 `Rc<FrameAtlas>` / `Rc<VectorClip>` 在 `!Send` 世界里怎么加载"。`AnimSource` 用 `Rc` 是对的(与单线程模型一致),但**素材加载是 IO**:同步阻塞第一帧,还是走 `sv_ui::tasks`(帧前 `tasks::pump` 已经在流水线里)?后者才对,而它意味着 `AnimData` 要有"未就绪"态、`measure_leaf` 要在素材到位前给出尺寸。**这是一个真实的设计缺口,本次复核只指出,不代拟。** |
| **ADR-6 帧边界** | 是 | ✅ "`anim::pump` 已排在 `tick()` 之前"复查属实 |
| **ADR-9 帧预算** | 是 | ✅ 进 membench、卡 p99、不给未实测数字 —— 这几条处理得很干净 |
| **ADR-3b CPU 栈冻结** | 是 | ✅ §6.3"不为动画破例开渐变"的裁决与理由(先例一旦开,box-shadow 立刻敲门)是本文写得最好的一段 |

### 11.6 复核后的总裁决

- **(b) 运行期绑定 libpag = 硬否** —— 维持,且证据比初稿更硬。
- **(a) 纯 Rust 全量解析 = 否** —— 维持。
- **(c) 离线转换 = 唯一相容路** —— 维持,但**落点从 c3 移到 c2**,
  且 c2 的工具链从"PAGX 转译"换成"构建期 wasm 渲染 + 动态 WebP"。
- **c3(经 PAGX)= 假设,不进 ADR、不进估算表、不进路线图。**
- **要不要现在做:不要** —— 维持,理由更强了(前置缺口清单里多了一条
  `paint()` 短路,而它是所有动画能力的共同前提)。
- **本文真正该带走的一条**:`Painter::draw_image` + `<img>`,今天就排。
  它是全文推论链的终点,却被写在了副产品位置。

**复核对本文的总体评价**:事实调查过关(逐位复查无一处编造),
坑位识别(a11y 刷屏、`Forever`、CPU 档渐变破例、标签不绑格式)有真功夫;
问题集中在**从实查事实到工程结论的最后一跳** —— 把规范里的一句话当成工具、
把 OR 短路读成 AND 短路、把带资源的动词按纯几何动词计价。
这三处都不是知识问题,是**没有在下结论前再回头核一次前提**。
