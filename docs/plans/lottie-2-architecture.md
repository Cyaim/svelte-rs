# Lottie 接入 svelte-rs:架构方案

> 状态:**设计裁决,未实现**。日期 2026-07-22。
> 范围:回答"lottie 要不要做、做成什么形状、第一刀切在哪、真实成本多少"。
> 前置阅读:[DESIGN.md](../DESIGN.md) ADR-3 / ADR-3b(渲染双后端)、ADR-6(帧调度)、
> ADR-9(规模策略)、[调研 26](../research/26-arco-design-ui-kit.md) §3.2(图标管线,
> 与本方案共享同一个前置动词)。
> 本文所有上游版本/API 均于 2026-07-22 联网实查,证据见 §1;未核实项集中在 §9,
> **没有一处是凭记忆写的版本号**。

---

## 0. 裁决速查

| # | 问题 | 裁决 | 主要代价 |
|---|---|---|---|
| 1 | Painter 要不要长 `fill_path` | **要**,并同时长 `stroke_path`;路径用**自有 `PathCmd` 序列**(f32、设备像素、变换已烘焙),不借 kurbo/peniko 类型 | 多一次 PathEl→PathCmd 的转换与一次坐标变换;各向异性缩放下描边宽度是近似 |
| 2 | lottie 节点是什么 | **不新增 ElementKind**。`View` 上加一个 `paint_source` 槽(不透明 id)+ 壳侧资源注册表;lottie/SVG 图标/未来图片共用这一个扩展点 | measure 的 `View` 分支不再 `unreachable!`;多一层 id 命名空间纪律 |
| 3 | 动画驱动 | **接 `anim::pump`**,新增一条 `Channel::Media`;关键在于这条通道**每帧写节点时间但不 bump 版本** | 引出一处必须同步修的 vello 短路(§4.4);"不 bump 的写"是新概念,要写进注释与测试 |
| 4 | 双后端一致性 | **CPU 后端也能做**(velato 的 `vello` 依赖是 optional,渲染动词经 `RenderSink` 抽象)。v1 两个后端**降级到完全一致**:渐变取平均色、遮罩降为外接矩形、轨道遮罩不支持 | 放弃 vello 本可白拿的渐变保真;换来"换后端画面不变"这条硬保证 |
| 5 | 无障碍 | 有 `aria-label` → `Role::Image` + 名称;无 label → **装饰性,不进语义树**;自动播放且循环的必须可暂停(WCAG 2.2.2 A 级),接 `prefers-reduced-motion`(CSS-SUPPORT 已排期 C2) | 需要一个全局 reduced-motion 开关,以及"默认 autoplay+loop 是否合规"的文档口径 |
| 6 | 第一步切多小 | 步骤 0 = **只加路径动词 + 三后端实现 + 金样**,一行 lottie 代码都不写 | 无。这一步对 SVG 图标同样是前置,单独就有价值 |
| 7 | 现在该不该做 | **步骤 0 该做(优先级高于 lottie 本体)**;lottie 本体排在改名 ADR-10、`.sv` LSP spike、增量布局之后 | 见 §8.3 |

---

## 1. 上游事实(2026-07-22 实查)

### 1.1 选定:velato(Linebender)

| 项 | 值 | 证据 |
|---|---|---|
| 最新版 | **0.11.0**,发布 **2026-07-21** | <https://crates.io/api/v1/crates/velato>(JSON 版本表) |
| 仓库活跃度 | `pushed_at` 2026-07-21,152 star,6 open issue,未 archive | <https://api.github.com/repos/linebender/velato> |
| 许可 | 源码 SPDX 头 `Apache-2.0 OR MIT`(GitHub 侧检测显示 Apache-2.0) | 抓取的 `src/runtime/render.rs` 首行 |
| MSRV | **1.88** | README 版本表 |
| 依赖 | `kurbo ^0.13`、`peniko ^0.6`、`serde ^1.0.228`、`serde_json ^1.0.149`、`serde_repr ^0.1.20`;**`vello ^0.9` 是 optional** | <https://crates.io/api/v1/crates/velato/0.11.0/dependencies> |
| 版本对应 | velato 0.11 ↔ **vello 0.9** | README 兼容表 |

**三个和本仓库严丝合缝的巧合**(这是选它的核心理由,不是感情因素):

1. **velato 0.11 对应 vello 0.9**,正是 `sv-shell/Cargo.toml` 里锁的版本;
2. **velato 的 MSRV 1.88** 正是本仓库 workspace 的 `rust-version`;
3. `kurbo 0.13.1` / `peniko 0.6.1` **已经在 `Cargo.lock` 里**(经 vello → peniko → kurbo
   进来),velato 的 `^0.13` / `^0.6` 直接命中同一份解析结果 —— 几何/样式层零新解析风险。

**决定性的一条**:velato 的渲染出口是一个**后端无关 trait**,`vello` 只是它的一个
可选实现。抓自 `src/runtime/render.rs`(main 分支)原文:

```rust
pub trait RenderSink {
    fn push_layer(&mut self, blend: impl Into<peniko::BlendMode>, alpha: f32,
                  transform: Affine, shape: &impl kurbo::Shape);
    fn push_clip_layer(&mut self, transform: Affine, shape: &impl kurbo::Shape);
    fn pop_layer(&mut self);
    fn draw(&mut self, stroke: Option<&fixed::Stroke>, transform: Affine,
            brush: &fixed::Brush, shape: &impl kurbo::Shape);
    fn begin_layer_group(&mut self, _name: &str, _index: usize) {}
    fn end_layer_group(&mut self) {}
}

pub fn append(&mut self, animation: &Composition, frame: f64, transform: Affine,
              alpha: f64, scene: &mut impl RenderSink)
```

其中 `fixed::Brush = peniko::Brush`、`fixed::Stroke = kurbo::Stroke`
(`src/runtime/model/fixed.rs`)。`Composition` 的公开面:

```rust
pub struct Composition {
    pub frames: Range<f64>,      // 动画活跃帧区间
    pub frame_rate: f64,         // fps
    pub width: usize, pub height: usize,
    pub assets: HashMap<String, Vec<model::Layer>>,
    pub layers: Vec<model::Layer>,
}
// Composition::from_slice(impl AsRef<[u8]>) / from_json(serde_json::Value) / FromStr
```

**这意味着 lottie 不是"vello 专属能力"**:`RenderSink` 要的四个动词,和我们
`Painter` 的动词是同一层抽象(填充/描边/裁剪/图层)。把 `RenderSink` 适配到
`&mut dyn Painter` 上,**两个后端一次性都拿到 lottie**。这正是 ADR-3b 那句
"后端只实现三个动词、共享一个 `paint_tree` 遍历器"的第二次兑现。

`RenderSink` 的方法带 `impl Trait` 参数(非对象安全),所以适配器必须是**具体类型**
(`struct PainterSink<'a> { p: &'a mut dyn Painter, .. }`),不能是 `dyn RenderSink`——
这不构成障碍,`dyn` 依旧只收在 sv-shell 边界内(ADR-3b 纪律)。

**上游明说的不支持清单**(README 原文):位置关键帧缓动、时间重映射、**文本渲染**、
**图片内嵌**、高级形状(dash / zig-zag)、高级特效(运动模糊、投影)、色标处理、
拆分旋转与位移。代码里还有一处自述:轨道遮罩分支带注释
`todo: re-enable masking when it is more understood`。**这些是我们的支持子集的上界**
(§5.2),不能靠我们这边补——补它等于接管半个 lottie 运行时。

### 1.2 被否的候选

| 候选 | 事实 | 否掉的理由 |
|---|---|---|
| `rlottie` 0.5.4(2026-03-07) | Samsung rlottie 的 Rust 绑定,`rlottie-sys` + `vendor-samsung`/`vendor-telegram` feature;仓库在 codeberg | **C++ 构建**。ADR-3 排除 skia-safe 的理由(构建重、拖累鸿蒙交叉编译)在这里逐字成立 |
| `dotlottie-rs`(LottieFiles,273 star,2026-07-22 仍在推) | README 原文:"powered by the ThorVG renderer";"you will need a C++ toolchain (clang, plus libclang for bindgen)";ThorVG 以 git submodule vendored 并由 build script 从源码编译;还要 GNU make | 同上,且更重(submodule + make + bindgen)。功能最全(它是 .lottie 容器的事实标准),但代价与本仓库的自绘路线冲突 |
| `lottie-rs`(zimond) | 默认播放器基于 Bevy,文档口径停在 Bevy 0.13 | 绑死游戏引擎;活跃度与 API 稳定性未核实 |
| `rasterlottie` 0.2.1(2026-04-24,累计下载三位数) | "Pure Rust, headless Lottie rasterizer for deterministic server-side rendering" | 定位是服务端出图(GIF/PNG),不是交互式逐帧;发布三个月、下载量三位数,**太新**。留作 §9 待观察项 |

**结论**:velato 是唯一一个"纯 Rust + 与我们已有的 vello/kurbo/peniko 同族 + 渲染
后端无关"的选项。它也确实是被 Linebender 自己定义为"working towards correctness,
there are missing features"的未完成品 —— 这是本方案最大的上游风险,写进 §8.2。

---

## 2. 裁决一:Painter 长出路径动词

### 2.1 路径怎么表达

现状(`crates/sv-shell/src/paint.rs:84`)`Painter` 只有 `fill_rounded_rect` /
`stroke_rounded_rect` / `glyph_run` / `push_clip` / `pop_clip` / `caps`,**没有任意路径**。
三个选项:

| 方案 | 好处 | 代价 | 裁决 |
|---|---|---|---|
| A. 自有 `PathCmd` 序列(f32) | sv-shell 的公开接口零新类型依赖;CPU 后端(tiny-skia,f32)零转换;`RecordingPainter` 好序列化 | vello 侧要把 f32 再抬成 f64 建 `BezPath`;velato 侧要把 `PathEl`(f64)降成 f32 | **采用** |
| B. 直接收 `&kurbo::BezPath` / `&dyn kurbo::Shape` | 与 velato/vello 零阻抗 | **kurbo 变成 `Painter` 接口的必需依赖**。今天 kurbo 只经由 optional 的 vello 进来;把它焊进 CPU 后端也要用的接口,等于给"纯 CPU 构建"加一个几何库 —— ADR-3b 的"vello 类型不得泄漏到 CPU 侧接口"精神上是同一条 | 否 |
| C. 收 `&[peniko/kurbo::PathEl]` | 比 B 轻(只要 kurbo 的一个 enum) | 同 B 的依赖问题,且 `PathEl` 是 f64 + `Point`,CPU 侧每点都要降精度 | 否 |

裁决 A 的关键理由:**`Painter` 是本仓库的后端切换支点,它的签名必须能在
"tiny-skia 独苗构建"下自洽**。今天 `cargo build -p sv-shell`(默认 feature)不含
vello、不含 kurbo;如果 `fill_path` 收 kurbo 类型,这条构建道就多一个第三方几何库,
而且未来换 CPU 后端(ADR-3b 规划的 vello_cpu)时接口跟着别人的类型走。
自有 `PathCmd` 是 12 行代码换一条边界。

**这不是"再造一个 kurbo"**:`PathCmd` 只是动词的参数编码,不带任何几何算法
(求交、偏移、弧长全部留在 kurbo 侧,由适配器在进 Painter 之前算完)。

### 2.2 签名

```rust
/// 路径指令(设备像素,f32;变换已由调用方烘焙进坐标——见 §2.3)。
/// 只有五个变体:圆弧/椭圆由调用方在 kurbo 侧展开成三次贝塞尔
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PathCmd {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// 二次贝塞尔(tiny_skia::PathBuilder::quad_to / kurbo::PathEl::QuadTo 直通)
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FillRule { #[default] NonZero, EvenOdd }

/// 描边风格。**这一个动词打包成结构体**,与既有"参数不打包"惯例的差异是
/// 有意的:vello `Scene::stroke` 本身就收 `&kurbo::Stroke`,tiny-skia
/// `stroke_path` 收 `&tiny_skia::Stroke`——打包才是"对齐后端词汇"
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct StrokeStyle {
    pub width: f32,
    pub cap: LineCap,        // Butt | Round | Square
    pub join: LineJoin,      // Miter | Round | Bevel
    pub miter_limit: f32,
}

pub trait Painter {
    // ... 既有六项不动 ...

    /// 任意路径填充(lottie 与 SVG 图标共享的前置;调研 26 §3.2)
    fn fill_path(&mut self, path: &[PathCmd], rule: FillRule, color: Color);
    /// 任意路径描边
    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color);
}
```

**为什么 `stroke_path` 不省掉**(把描边在上层用 `kurbo::stroke()` 展开成填充轮廓
再走 `fill_path`,确实可以只加一个动词):

- tiny-skia 与 vello **都有原生描边**,vello 侧还是 GPU 管线的一等公民;展开成填充
  等于在 CPU 上做两个后端本来都不用做的事,而且每帧都做;
- SVG 图标同样以描边为主(调研 26 §5 "arco 图标以 stroke 为主,usvg 需描边转填充"),
  有了原生 `stroke_path`,图标管线可以**跳过**描边转填充那一步;
- 代价是两个后端的描边几何不会逐像素一致(join/cap 的实现细节不同)——parity 口径
  本来就是"非白像素比"而不是逐像素(ADR-3b 实测 1.001/1.017),不构成新问题。

**不加的东西**(明确划界,避免这一刀砍成半个 Canvas API):渐变画刷、图像画刷、
虚线、路径裁剪、任意变换、混合模式。它们各自有独立的触发条件(CSS 渐变、图片子系统、
遮罩),不由 lottie 顺手带进来。渐变的降级见 §5.2。

### 2.3 为什么不带 `transform` 参数

velato 的 `draw(stroke, transform, brush, shape)` 是**把仿射变换单独给出来**的,
tiny-skia 的 `fill_path(path, paint, rule, transform, mask)` 和 vello 的
`Scene::fill(style, transform, brush, ..)` 也都收变换。看起来直通更省事。**仍然不加**,
三条理由:

1. `Painter` 现有的全部动词都建立在同一条不变量上:**"坐标 = 物理像素,调用方已乘
   scale"**(paint.rs 文件头注释)。裁剪栈、命中测试、金样命令流、`glyph_run` 的基线
   原点,全部长在这条不变量上。引入 per-draw 变换等于让"设备空间"这个概念在一个动词
   上失效,后续每个消费方都要问"这条命令的坐标在哪个空间"。
2. CPU 后端的裁剪是**手动矩形求交**(调研 22 §2.3 裁决,paint.rs:236 `clipped()`)。
   路径带变换后,剔除与裁剪判定都要先算变换后包围盒——那就等于在后端里重做一遍
   适配器已经能做的事。
3. 烘焙的成本可忽略:velato 每帧本来就重算全部几何(`Batch::clear()` + 重新求值),
   适配器多做的是"每个点 4 次乘加",与 tiny-skia 内部对 `Transform` 做的事同量级。

**已知偏差(必须写进代码注释并配卫兵测试)**:变换烘焙进坐标后,描边宽度不再随
变换缩放。适配器按 `width * (|det A|).sqrt()` 补偿 —— 各向同性缩放下**精确**,
各向异性缩放下是近似(椭圆笔画退化为圆笔画)。lottie 里各向异性缩放的描边罕见但存在
(拉伸的 loading 条)。若真机踩到,升级路径是给两个路径动词加一个
`transform: Option<&Affine2>` 参数(`Painter` 只在 sv-shell 内部 `dyn`,不是公开 API,
改签名不构成 semver 事件)。

### 2.4 tiny-skia 侧实现

直译,已有全部零件:

```rust
fn fill_path(&mut self, path: &[PathCmd], rule: FillRule, color: Color) {
    let Some(p) = build_ts_path(path) else { return };   // PathBuilder move/line/quad/cubic/close
    // 视口/裁剪剔除:包围盒完全在裁剪外 → 直接返回(热路径省一次光栅)
    if self.clipped_bbox(&p).is_none() { return; }
    let mut paint = Paint::default();
    paint.set_color(skia_color(color));
    paint.anti_alias = true;
    self.pixmap.fill_path(&p, &paint, rule.into(), Transform::identity(), self.clip_mask());
}
```

**裁剪的例外裁决(对调研 22 §2.3 的定点修订)**:现有 CPU 裁剪是矩形求交,对
`fill_rounded_rect` 够用,但**路径填不进矩形求交**(路径不是矩形)。方案:

- 裁剪栈为空 → `mask = None`,零成本(绝大多数图标/动画场景);
- 裁剪栈非空 → **惰性物化**当前裁剪矩形为一张 `tiny_skia::Mask`
  (`Mask::new(w,h)` + `fill_path(裁剪矩形)`),缓存在 `TinySkiaPainter` 内,
  按 (pixmap 尺寸, 裁剪矩形) 键复用,只在**路径动词**上创建。

这**不违反**调研 22 的裁决。当时否掉 Mask 的理由是"每层裁剪都要分配整画布 w×h 字节
且嵌套逐像素相乘";这里是**一张、惰性、只在有路径绘制且有裁剪时**,1080p 约 2MB、
一次分配跨帧复用。既有的矩形裁剪路径(圆角矩形/字形)一个字节都不多花。

`Mask::fill_path(&Path, FillRule, bool, Transform)` 与 `Mask::intersect_path(..)`
在 tiny-skia 0.11.4 均存在(docs.rs 实查),后者是未来做"任意路径裁剪"(§5.2 遮罩)
的现成升级点。

### 2.5 vello 侧实现

```rust
fn fill_path(&mut self, path: &[PathCmd], rule: FillRule, color: Color) {
    let bez = build_bez(path);            // f32 → kurbo::BezPath(f64)
    self.scene.fill(rule.into(), Affine::IDENTITY, pcolor(color), None, &bez);
}
```

`Affine::IDENTITY` 是 §2.3 烘焙裁决的直接后果 —— vello 侧不再需要变换栈。
`BezPath` 每帧新建的分配可以用一个复用缓冲消掉(`VelloPainter` 持一个
`BezPath`,`truncate(0)` + push),视基准结果决定要不要做。

### 2.6 `RecordingPainter` 怎么记录路径

金样要**稳定、可比对、失败时看得懂**。逐点记录做不到第三条(一条 lottie 帧几千个点,
diff 无法阅读),也做不到第一条(velato 小版本升级会动几何)。裁决:**记摘要**。

```rust
PaintCmd::FillPath {
    /// 指令条数(结构指纹:形状变了这个数多半会变)
    verbs: usize,
    /// 控制点凸包的整数包围盒 [x0,y0,x1,y1](不做展平,纯 min/max,确定性)
    bbox: [i32; 4],
    rule: FillRule,
    color: Color,
},
PaintCmd::StrokePath { verbs: usize, bbox: [i32; 4], width: i32, color: Color },
```

- **确定性**:同一输入两次绘制命令流逐字相等(既有测试
  `assert_eq!(rec.cmds, rec2.cmds, "命令流应确定性可重放")` 的口径不变);
- **可诊断**:失败信息是"期望 3 条填充、bbox 约 (10,10)-(34,34),实际 2 条"
  ——人能立刻判断是几何塌了还是图层丢了;
- **抗上游噪音**:velato 打磨一个缓动曲线不会翻绿变红,而"整层没画出来"必翻红。
  取整到整数像素与既有 `FillRect { x: x as i32, .. }` 同款。

需要逐点比对时(定位几何 bug),再加一个 `SV_PAINT_DUMP=1` 的调试输出即可,不进金样。

### 2.7 顺带兑现:SVG 图标

调研 26 把"`fill_path` + SVG 编译期转译"列为 sv-arco 的头号卡点,并断言
"没有这项,arco 视觉完成度上限约六成"。步骤 0 交付的两个动词**就是那一半**:
剩下的是 build 期用 usvg 把 SVG 解析成 `PathCmd` 数据表(`生成数据而非类型`,ADR-2 哲学),
与 lottie 完全解耦。**这也是本方案建议把步骤 0 单独提前的根本原因**(§8.3)。

---

## 3. 裁决二:lottie 节点在场景树里是什么

### 3.1 三条路的代价

| 方案 | 要改的地方 | 评价 |
|---|---|---|
| **A. 新增 `ElementKind::Lottie`** | `sv-ui/lib.rs` 枚举 + `create()` 的 focusable/accepts_text/input 三个表 + `dump`;`render.rs` 的 `build_taffy`(叶子判定)、`measure_leaf`、`paint_tree` 三处 match;`a11y.rs` 的 role 映射;`sv-compiler` 的 `ElemKind` + 属性名表;`sv-macro` 的标签表 —— **七处以上** | 与多行 textarea 那次的裁决冲突:那次明确"加一个变体要连带改 render 两处 match、dump、a11y 与两个前端的标签表,而多行本就是同一控件的模式"。**同一条理由在这里成立**:"内容由外部资源绘制"是盒子的属性,不是新控件 |
| **B. `View` + `paint_source` 槽** | `sv-ui` 加一个 `Option<PaintSource>` 字段 + setter;`render.rs` 改 `build_taffy` 的叶子判定条件(1 行)、`measure_leaf` 的 `View` 分支(把 `unreachable!` 换成查注册表)、`paint_tree` 的 `ElementKind::View => {}` 分支(现为空);`a11y.rs` 的 `View` 分支加一个前置判断 | **推荐**。且这一个扩展点**同时**服务 SVG 图标、静态图片、未来 `<surface3d>`(`PainterCaps.external_texture` 已为它预留通道)——不然每来一种媒体就多一个 ElementKind |
| **C. `Painter::draw_scene(handle, frame, rect)`** | Painter 加一个动词,每个后端各自实现播放 | **否**。paint.rs 文件头写着"painter 不拿字符串也不拿位图(Slint 软件渲染器与 GPU 灾难的双重教训)";把 lottie 塞进 Painter 意味着 tiny-skia/vello/Recording 三个后端各写一遍解析与求值,`RecordingPainter` 还得记录"一个不透明的动画"——金样直接失效。**Painter 必须保持哑指令集** |

### 3.2 推荐形态

```
sv-ui(零依赖)         sv-shell(feature = "lottie")
┌──────────────────┐    ┌────────────────────────────────┐
│ ViewNode         │    │ 资源注册表(thread_local)      │
│  paint_source:   │───▶│  id → Composition + 固有尺寸    │
│    Option<..>    │ id │                                │
│  media:          │    │ PainterSink(RenderSink 适配器)│
│    Option<Box<>> │    │  velato → Painter 动词          │
└──────────────────┘    └────────────────────────────────┘
```

sv-ui 侧**只存一个 u64 和一个语义标签**,不知道 velato、不知道 lottie、不新增依赖
(CLAUDE.md 约束:sv-ui 是编译目标,须保持零依赖 —— 与 P3 词边界"不引 UAX #29 表"
同一条纪律)。

```rust
// sv-ui/src/lib.rs
/// 外部绘制源句柄:id 由渲染壳的注册表分配,sv-ui 只搬运不解释
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PaintSource {
    pub id: u64,
    /// 语义角色(a11y 与 dump 用;sv-ui 唯一"看得懂"的部分)
    pub role: PaintRole,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaintRole { Image, Animation }

/// 播放态。与 `input: Option<Box<InputState>>` 同构——同一个先例:
/// 少数节点才有的重状态装箱放节点里,不进 Signal(R1 裁决)
pub struct MediaState {
    pub playing: bool,
    pub looped: bool,
    pub speed: f32,
    /// 当前时间(ms,composition 内部时间轴)。**真源在树上**,与 scroll_x/y、
    /// checked 同款;但它的每帧推进**不 bump 版本**(§4.2)
    pub time_ms: f32,
    pub on_end: Option<Rc<dyn Fn()>>,
}
```

`ViewNode` 加两个字段:`paint_source: Option<PaintSource>`、`media: Option<Box<MediaState>>`。
`Doc` 加 `set_paint_source` / `play` / `pause` / `seek`(都 bump —— 它们是用户事件,
一秒最多几次)。

**为什么 id 不透明而不是把 `Composition` 塞进树**:`Composition` 是几百 KB 到几 MB 的
解析产物,多个节点共用同一份是常态(列表里 20 个 spinner);树上放句柄、壳里放资源,
与**字体注册表**(text.rs,按 Blob id 建键)是同一套路,已有先例。

### 3.3 布局:固有尺寸

`build_taffy`(render.rs:279)现在按 `n.kind == ElementKind::View` 判定容器/叶子。
改为:

```rust
let is_leaf = n.kind != ElementKind::View || n.paint_source.is_some();
```

`MeasureCtx` 加 `intrinsic: Option<(f32, f32)>`(建树时查注册表填入),
`measure_leaf` 的 `ElementKind::View => unreachable!("View 不是叶子")` 改为返回
固有尺寸(注册表没查到 → `Size::ZERO`)。

**注意**:这条 `unreachable!` 属于 R4 去 panic 审计里"自证不变量"的白名单条目;
改动后它不再是不变量,必须换成 `unwrap_or(ZERO)` —— 否则加载失败的 lottie 会**崩进程**,
正好撞在 R4 的红线上("崩进程是最坏处理")。

缩放语义 v1 **硬编码 contain**:composition 的 `width × height` 等比缩放居中填进内容盒。
`object-fit` 是 CSS 属性面的事,与图片子系统一起做,不由 lottie 定义。

### 3.4 id 命名空间纪律

`PaintSource.id` 未来要同时容纳 lottie 与 SVG 图标(以及可能的图片)。DESIGN.md 里
记着一次同类事故:"字体注册按 Blob id 建键(**保留键 0 归内置字体,注册键高位恒 1**
——撞键 = Latin 全员错字,实测踩过并有回归卫兵)"。同样的账不吃第二次:

```
id = (kind << 56) | index      // kind: 1=lottie, 2=vector(SVG), 3=image, 0 保留
```

注册表按 kind 分表,取用时先校验 kind —— 拿错表返回 `None` 走降级,而不是画出别的资源。

---

## 4. 裁决三:动画驱动

### 4.1 单一驱动:接 `anim::pump`

**接 `anim::pump`,不自持时间轴。** 理由是排帧的正确性只能有一个权威:
`sv-shell/src/lib.rs:211` 的 `paint()` 用 `let animating = sv_ui::anim::pump(now_ms);`
的返回值决定"要不要继续 `request_redraw`"。任何第二个时间轴都必须把
"我还在动"这个事实**再告诉**排帧器一次,而两个来源迟早会不一致(经典症状:
动画停了但窗口继续 100% 重绘,或者反过来动画卡在某一帧直到你动鼠标)。

新增通道:

```rust
enum Channel {
    Opacity,
    ScrollY,
    /// 媒体时间轴(lottie):推进节点内 MediaState.time_ms。
    /// 与另两条的根本差别:**它不写 style、不 bump 版本**(§4.2),
    /// 且 looped 时永不自然结束
    Media,
}
```

`pump` 里的 Media 分支:

```
dt = now_ms - last_ms                      // 首帧 last_ms = NAN → dt = 0
t  = time_ms + dt * speed
if t >= duration_ms {
    if looped { t = t % duration_ms; }     // 取模而不是清零:掉帧不丢相位
    else { t = duration_ms; playing = false; on_end 触发; }
}
```

节点被销毁时随之丢弃 —— 复用 `pump` 已有的
`if an.doc.read(|inner| inner.nodes.get(an.node).is_none()) { return false }`。

`on_end` 在 `pump` 内触发,而 `paint()` 的顺序是
`tasks::pump → anim::pump → sv_reactive::tick()`(ADR-6),所以回调里写 signal
会在**同一帧**冲刷完 —— 与 `tasks` 的完成回调同一条相位纪律,不需要新机制。

### 4.2 关键技巧:每帧写,但不 bump 版本

这是整个方案里唯一一个"新概念",也是绕不开的一处:

**`Doc::version()` 是布局缓存的键。** `render.rs:458` 的 `layout_full_cached` 用
`(doc.identity(), doc.version(), 宽位, 高位)` 做键。**版本一变,整棵树重新走 taffy。**
DESIGN.md 记着实测:30k 全量档一次布局 ~130–160ms(taffy 裸 ~45ms + 叶子 measure ~70ms)。

一个 24×24 的 loading 图标,如果按 `Opacity`/`ScrollY` 通道的写法每帧 `bump()`,
就会让**整个应用每帧重新布局**。平滑滚动(S6)今天确实这么干,但它只持续 140ms;
lottie 的 spinner **一直转**。这不是"lottie 慢",是"lottie 把一个既有的架构缺陷
从瞬时暴露成了常态"。

两条出路:

| 出路 | 说明 | 裁决 |
|---|---|---|
| 拆版本号:`struct_version`(影响布局)/ `paint_version`(只影响像素) | 更彻底,顺带惠及"只改颜色"的更新;但要审计每一个 `bump()` 调用点归哪一类,且 a11y 缓存、命中测试、IME 上报全部键在 `version()` 上 | **不在本方案范围**。它是一件独立的、值得单独立项的事(和"增量布局"是同一族),不该被 lottie 顺手改掉 |
| 媒体时间的每帧写**不 bump** | `MediaState.time_ms` 是纯绘制输入:不影响布局(尺寸恒定)、不影响命中(矩形不变)、不影响语义树(§6:动画不该刷屏幕阅读器)。"脏"这一事实由 `anim::pump` 的返回值携带,`paint()` 已经在消费它 | **采用** |

于是 `Doc` 上多一个刻意不 bump 的写入口,注释要写死为什么:

```rust
/// 推进媒体时间。**刻意不 bump 版本**:time_ms 只进绘制,不进布局/命中/语义树;
/// bump 会让 layout_full_cached 每帧失效(30k 档 ≈130ms),把一个转圈图标
/// 变成全应用卡顿。"这一帧要重画"由 anim::pump 的返回值负责传达。
/// 新增任何**读取 media 且影响布局**的代码,必须回来改这条契约。
pub fn advance_media_time(&self, id: ViewId, t: f32) { /* 不调 self.bump() */ }
```

### 4.3 帧对齐(ADR-6)下的排帧

`paint()` 尾部已经有 `if animating { ws.window.request_redraw(); }` —— 有动画就续帧,
没有就停。Media 通道天然接上:`playing=false` 后 `pump` 不再保留该 anim,
`active()` 归 false,窗口回到零功耗静止。用户点"播放"→ 写 signal → effect 调
`doc.play(node)` → 注册 anim + `bump()` → `on_mutate` → `request_redraw` → 循环起来。
**没有一行新的调度代码。**

### 4.4 静止帧短路:CPU 已经对,vello 有一处必须改

```rust
// lib.rs:227
let frame_key = (self.doc.version(), size.width, size.height, scale.to_bits());
let unchanged = self.last_frame_key == Some(frame_key);
if unchanged && !animating && !self.show_fps { return; }   // ← 正确:animating 时不短路
```

CPU 路径没问题。但下面这行有问题:

```rust
// lib.rs:278
let (_, presented) = vw.render_cached(&self.doc, scale, unchanged);
//                                               ^^^^^^^^^ scene_unchanged
```

`render_cached(.., scene_unchanged = true)` 会**跳过 `paint_tree` 重编码**,只重呈现
上一份 `Scene`。今天这不是 bug —— 现有两条动画通道都写树,版本必变,`unchanged`
在动画期间恒 false。**引入"不 bump 的每帧写"之后它立刻变成 bug**:
GPU 后端上 lottie 永远停在第一帧,而 CPU 后端一切正常。

修法一行:

```rust
let (_, presented) = vw.render_cached(&self.doc, scale, unchanged && !animating);
```

**这类"只有一个后端不动"的现象是最贵的 bug**(会被误诊成 GPU 驱动问题),
所以它要在步骤 2 里配一个专门的测试(§7),而不是靠开窗肉眼发现。

### 4.5 时间源与掉帧

`pump(now_ms)` 收的是 shell 的单调时钟(`self.epoch.elapsed()`),离屏/测试传合成时间
—— 这条既有契约让"两帧之间动画该走多远"完全可测,零窗口零 GPU。

掉帧策略:**丢帧不丢相位**。`time_ms` 按真实 dt 推进(不是"每帧 +1 帧"),
所以卡顿后动画会跳到应到的位置而不是整体变慢。循环用取模而非清零,同理。
**不做补帧**(不在一帧里渲染多个 lottie 帧)——那是视频的做法,矢量动画没有运动模糊
可补,只会白烧 CPU。

`速度上限`:`dt` 一次超过一个循环周期时按取模处理;`dt` 异常大(窗口被拖住十几秒)
时相当于随机相位,可接受。

---

## 5. 裁决四:双后端一致性

### 5.1 CPU 后端能做

**能。** 依据是 §1.1 的三条事实:velato 的 `vello` 依赖是 optional;渲染出口是
`RenderSink` trait;`kurbo`/`peniko` 是纯 CPU 的几何/样式库(不含 wgpu)。
`sv-shell` 加 `velato = { version = "0.11", default-features = false, optional = true }`
(**不开它的 `vello` feature**),适配器把 `RenderSink` 翻成 `Painter` 动词 ——
CPU 与 GPU 走**同一份适配器**,后端差异被压回 `Painter` 这一层,和现有全部绘制一样。

这条结论值得强调:**它把"CPU 栈能力冻结"(ADR-3b)与 lottie 解耦了**。冻结令针对的是
"CSS ⏳ 项一律不在 CPU 栈实现"(模糊、复杂合成);路径填充/描边是两栈共有能力
(调研 26 §3.2 已有同样判断),不在冻结范围内。

### 5.2 支持子集(两个后端一致)

| 特性 | v1 行为 | 缺口来源 |
|---|---|---|
| 形状/路径/裁剪路径动画、trim path、repeater | **支持** | velato 已在自己内部求值成几何 |
| 纯色填充/描边、图层与形状不透明度 | **支持**(alpha 由 velato 乘进 brush) | — |
| 渐变填充 | **降级:取色标平均色**(两个后端同样降级) | 我们的 `Painter` 无 brush 概念(§2.2 划界) |
| 图层遮罩(`layer.masks`) | **降级:外接矩形裁剪**(两个后端同样降级) | 我们只有矩形 `push_clip` |
| 轨道遮罩 / 混合模式(`push_layer(blend)`) | **不支持**,记账保证 push/pop 配平 | velato 自述 `todo: re-enable masking` |
| 文本图层 / 内嵌图片 / 时间重映射 / dash / 运动模糊 / 投影 | **不支持** | velato README 明列不支持 |
| dotLottie(`.lottie` zip 容器) | **不支持**,只吃裸 JSON | 需要 zip 依赖,与本方案正交 |

**渐变与遮罩为什么"连 vello 也一起降级"**:vello 明明能画渐变和任意裁剪。故意不用,
是为了让"换后端画面不变"这条保证成立。ADR-3b 已经把 parity 测试作为多后端的验收
口径(非白像素比 1.001/1.017);如果 lottie 在 GPU 上有渐变、CPU 上没有,parity 测试
要么失效要么写满例外。**代价是明确的**:GPU 用户少拿了一点保真度。**收益也是明确的**:
`SV_RENDERER=cpu` 回退时(无 adapter 自动回退是既有行为!)用户不会突然看到另一个动画。
等 `Painter` 因为 CSS 渐变而真的长出 brush 时,两个后端一起升级。

### 5.3 三条"不做运行时惊喜"的纪律

1. **编译期**:没开 `lottie` feature 时,`register_bytes` 这类 API **不存在**
   (而不是返回 `Err`)——用不了要在 `cargo build` 时知道,不是运行时。
   `PaintSource` 槽本身在 sv-ui 里无条件存在(零依赖),画不出来时画**占位**:
   1px 虚线框 + 中心对角线(仅 debug 构建;release 画空)。
2. **加载期**:`register_bytes` 解析 composition 后**扫一遍图层**,把命中不支持特性
   的图层名收集进 `LottieReport { unsupported: Vec<(层名, 原因)> }`,由调用方决定
   报警还是忽略;并提供 `sv lottie check <file>` 形态的离线校验(可以先是一个
   example,不必是 CLI 子命令)。**"为什么我的动画不对"必须在导入时就有答案**,
   否则这就是一个无底的 issue 来源(§8.2)。
3. **文档**:§5.2 的表进 `docs/en/` + `docs/zh-CN/`(双语镜像纪律),
   并在 SVELTE-SUPPORT/CSS-SUPPORT 同款的"逐项裁决"风格下维护。

### 5.4 parity 测试口径

沿用 ADR-3b:同一个 lottie、同一帧,CPU 与 vello 各出一张离屏图,比**非白像素比**
落在 `[0.95, 1.05]`(比字形宽松,因为路径 AA 与 MSAA 的边缘差异大于字形);
无 adapter 自动 skip。外加一条更强的:**`RecordingPainter` 命令流在两个后端之间
不适用**(Recording 就是第三个后端),但**同一 lottie 同一帧的命令流跨运行必须逐字
相等** —— 这条抓的是"velato 求值里混进了 HashMap 迭代序"这类不确定性。

---

## 6. 裁决五:无障碍

一个播放中的动画对屏幕阅读器**默认应该是装饰性的,并且必须可以停下来**。

1. **有 `aria-label`** → 语义树里报 `Role::Image` + 该名称。accesskit 0.24 的
   `Role` 里确有 `Image`(docs.rs 实查;同族还有 `Canvas`/`GraphicsObject`/`SvgRoot`)。
   `Image` 是屏幕阅读器最熟的角色,不生造。
2. **无 `aria-label`** → **不生成语义节点**(与 CSS 背景图同待遇)。
   a11y.rs 的映射是纯函数,跳过一个节点是 `filter` 一行的事。
   理由:一个没有名字的图形节点,对屏幕阅读器就是一句"图形"的噪音。
3. **不报 `ProgressIndicator`**,除非调用方显式声明。loading 动画看起来像进度条,
   但我们没有进度值可报;报了会让 AT 朗读"进度 未知"并可能反复播报。
   `Marquee`/`Timer`/live region 同理:**只报树里确实存在的信息**
   —— 这是 R3 P4/P5 已经确立的纪律("全部出自树里确实存在的信息,不猜")。
4. **可暂停(硬性)**:WCAG 2.2.2 Pause, Stop, Hide(**Level A**)原文:
   "For any moving, blinking or scrolling information that (1) starts automatically,
   (2) lasts **more than five seconds**, and (3) is presented in parallel with other
   content, there is a mechanism for the user to pause, stop, or hide it..."
   (<https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html>)。
   ⇒ `loop` + `autoplay` 的组合**必然**落进这一条。落地形态:
   - `Doc::pause/play` 是公开 API(组件层可以做暂停按钮);
   - **全局开关** `sv_ui::anim::set_reduced_motion(bool)`:开启后新建的 Media 通道
     直接停在**首帧**(不是最后一帧——loading 的最后一帧往往是空的),
     且既有 transition:fade 也应尊重它(顺带收益);
   - 系统级 `prefers-reduced-motion` 的探测**不在本方案内**:CSS-SUPPORT.md 已把它
     排在 C2("随 @media 通道,接系统无障碍设置")。本方案只保证**接口就位**,
     C2 落地时把系统值灌进这个开关即可。**不发明第二个探测路径。**
5. **不做**:动画帧变化推送 TreeUpdate。R3 P6 刚把语义推送做成增量("一次键入本该
   只动一个节点"),lottie 每帧推树是对它的直接背叛。§4.2 的"不 bump 版本"顺带
   保证了这一点 —— a11y 推送是版本节拍驱动的,媒体时间不 bump,自然不推。

---

## 7. 分步落地

每一步都要求:**能单独合入、单独有价值、有不开窗的验收测试**。

### 步骤 0:路径动词(不含任何 lottie)

- 范围:`PathCmd`/`FillRule`/`StrokeStyle`/`LineCap`/`LineJoin` 类型;
  `Painter::fill_path`/`stroke_path`;三个后端实现(tiny-skia 含惰性 Mask 裁剪、
  vello、Recording);`PaintCmd::FillPath`/`StrokePath`。
- 改动文件:`sv-shell/src/paint.rs`、`sv-shell/src/vello_backend.rs`(+ 测试)。
  **不动** sv-ui、不动两个前端、不动 `render.rs`。
- 验收测试(全部无窗口):
  1. `path_verbs_recording_golden`:直接对 `RecordingPainter` 画一个五角星
     (`EvenOdd`)与一条描边折线,断言命令流的 `verbs`/`bbox`/`rule`/`color`,
     并二次重放 `assert_eq!` 确定性;
  2. `path_fill_rule_matters`:同一路径 `NonZero` vs `EvenOdd` 在 tiny-skia 上
     出图非白像素数不同(证明 rule 真的传到了后端);
  3. `path_clip_confines_fill`:`push_clip` 后画一个大于裁剪矩形的路径,
     断言裁剪矩形外像素为白(**这条是 Mask 通道的守门员**);
  4. `vello_path_parity`(有 adapter 时):同一路径 CPU/GPU 非白像素比在容差内。
- 独立价值:**SVG 图标管线的前置**(调研 26 A3),以及圆弧 Progress、Tooltip 箭头、
  Rate 星形这些"没有它就画不出来"的组件件。

### 步骤 1:静态首帧(接上 velato,不动)

- 范围:`sv-shell/src/lottie.rs`(feature `lottie`):资源注册表 + `PainterSink`
  适配器 + `register_bytes`;sv-ui 的 `PaintSource` 槽 + `set_paint_source`;
  `render.rs` 三处(叶子判定 / measure / paint 的 View 分支);占位绘制。
- 播放不做:恒渲染 `frames.start`。
- 验收:`lottie_first_frame_commands`(仓库内放一个**自己手写的**几十行 lottie JSON
  作为夹具,不引第三方资产):断言命令流条数 > 0、总包围盒落在节点矩形内、
  跨运行逐字相等;`lottie_intrinsic_size`:未给宽高的节点被布局成 composition 的
  `width×height`;`lottie_missing_source_draws_placeholder`:坏 JSON → `register_bytes`
  返回 `Err` 且节点退化成占位、**不 panic**(R4 去 panic 纪律)。

### 步骤 2:播放

- 范围:`MediaState` + `anim::Channel::Media` + `advance_media_time`(不 bump)+
  `play/pause/seek/on_end`;**§4.4 的 vello 一行修复**。
- 验收:
  1. `media_pump_advances_time`:合成时间推两次,`time_ms` 单调增,
     `looped=false` 到点后 `pump` 返回 false 且 `on_end` 触发一次;
  2. `media_loop_wraps_by_modulo`:一次超长 dt 后相位正确(不是清零);
  3. `media_frame_differs_across_time`:同一 Doc 在 t=0 与 t=T/2 走
     `paint_tree` 得到**不同**命令流(证明时间真的进了绘制);
  4. `media_time_does_not_invalidate_layout`:推进时间后
     `doc.version()` **不变**,`layout_full_cached` 命中缓存
     (可用一个计数器或对比返回的 `Placed` 是同一份克隆来断言);
  5. `vello_scene_reencodes_while_animating`:**不需要 GPU** ——
     把 `unchanged && !animating` 的判定抽成一个纯函数
     `fn should_skip_scene_encode(unchanged: bool, animating: bool) -> bool`
     并单测钉死。(把一行条件抽成函数只为可测,值得。)

### 步骤 3:降级、无障碍、前端语法

- 范围:`LottieReport` 不支持特性扫描;渐变/遮罩降级与 §5.2 文档表(双语);
  a11y 的 `Image`/装饰性分支;`set_reduced_motion`;
  `.sv` 的 `<lottie source={h} autoplay loop />` 与 `view!` 的对应形态
  (走 `sv_compiler::emit` 单一发射口 —— ADR-2 ① 已把"改两处"降为"改一处")。
- 验收:a11y 金样(有/无 label 两例)、reduced-motion 下停首帧、
  `.sv` 端到端行为测试(counter-sfc 同款)。

### 步骤 4(可选,按需)

CPU 侧 `Mask::intersect_path` 做真遮罩;`Painter` 长 brush 后恢复渐变保真;
dotLottie 容器;预渲染缓存(把 N 帧命令流缓存下来重放——`RecordingPainter`
文件头写着它"未来可升级为帧间 diff 载体",这里是它的第一个真实用武之地)。

---

## 8. 这件事的真实成本

### 8.1 人周估算

| 步骤 | 人周 | 置信度 | 估算依据 |
|---|---|---|---|
| 0 路径动词 + 三后端 + 金样 | **0.5–1** | **高** | 纯机械映射,零件全部现成(tiny-skia `PathBuilder`/`Mask`、vello `Scene::fill`);调研 26 对"fill_path + SVG 管线"整体估 2–3 人周,其中动词部分是小头 |
| 1 静态首帧(注册表 + 适配器 + 布局接线) | **1–1.5** | **中** | 适配器本身约 150 行;不确定性在 `RenderSink` 的图层配平与坐标系对齐(composition 空间 → 内容盒 → 设备像素,三次变换叠加,**第一次一定会错**) |
| 2 播放(anim 通道 + 不 bump 写 + 短路修) | **1–1.5** | **中** | 代码量小,但"不 bump 的写"要审计所有 `version()` 消费方(布局缓存、a11y 缓存、命中、IME 上报),审计比写代码贵 |
| 3 降级 + a11y + 双前端 + 双语文档 | **1.5–2.5** | **中低** | 双语文档与支持子集表是实打实的时间;`.sv` 元素属性表 + 编译期错误信息按 `<overlay>` 那次的经验约 0.5 人周 |
| **合计** | **4–6.5 人周** | **中** | ≈ 1–1.5 个月全职 |
| 悲观值 | **8–10 人周** | — | 触发条件见 8.2 |

### 8.2 隐藏成本(这些不在上面的表里)

1. **上游是未完成品**。velato README 自述"working towards correctness, there are
   missing features",代码里带着 `todo: re-enable masking`。设计师用 AE 导出的文件
   会随机踩到不支持特性,而我们**没有能力修**(修 velato = 接管半个 lottie 运行时)。
   缓解是 §5.3 的加载期扫描,但那只是把"画错"变成"提前告诉你画不了"。
2. **答疑面**。"我的 lottie 在浏览器好好的,在你这里颜色不对/少了一层"——
   这类 issue 的排查成本极高(要对 AE 图层、对 velato 求值、对我们的降级)。
   DESIGN.md 风险清单第 5 条正是"单人/小团队维护面过宽",自研面被刻意收敛到
   "编译器 + 响应式 + 组件运行时三样"。**lottie 会在这三样之外开第四个答疑面。**
3. **MSRV 耦合**。velato MSRV = 1.88 = 我们的 MSRV,**一格余量都没有**。
   velato 下次抬 MSRV(README 明说"未来版本可能提升且不视为 breaking")时,
   我们要么跟着抬(影响全仓)要么钉住旧版。CI 的 MSRV 构建道若开 `lottie` feature,
   这就成了一条会自己变红的道。
4. **依赖面**。默认(CPU)构建下开 `lottie` 会新引入 velato + kurbo + peniko +
   serde + serde_core + serde_derive(proc-macro) + serde_json + serde_repr
   ≈ 8 个 crate。`kurbo`/`peniko` 今天只经 optional 的 vello 进来,`serde` 系在
   Linux 端经 zbus 进来。**编译时间影响未实测**(§9),但 `serde_derive` 是
   proc-macro,对"坚持生成数据而非类型"的编译时间纪律(风险清单第 4 条)是逆风。
   `cargo-deny` 的 licenses/bans 门禁应该都能过(全部 MIT/Apache),但要真跑一次。
5. **资产体积**。lottie JSON 动辄几百 KB;`include_bytes!` 编进二进制会撑大产物。
   资产管线(编入 vs 运行时加载 vs build 期预处理)是**图标/图片/lottie 共同的
   未决问题**,本方案不解决,只保证不与之冲突。

### 8.3 这是不是现在该做的事

**把问题拆成两半再回答,因为两半的答案不一样。**

**前一半(步骤 0,路径动词):该做,而且优先级不低。**
理由:
- 它是 **sv-arco 的头号卡点**。调研 26 的原话:"没有这项,arco 视觉完成度上限约六成
  ——箭头/勾选/关闭/加载/类型提示图标无处不在";Tooltip 箭头、环形 Progress、
  Rate 星形也全卡在这。
- 它**只改 sv-shell 一个文件半**,不动内核、不动 API 冻结面(R4),
  因此与"双前端合并 / 改名 ADR-10"这些冻结前置**不冲突**,可以并行插空做。
- 0.5–1 人周,高置信度。这是本仓库当前少见的"高收益、低风险、无耦合"条目。

**后一半(lottie 本体):现在不该做。** 建议排在 **R6/生态期**,在下列各项之后:

| 排在它前面的 | 为什么更靠前 |
|---|---|
| 改名 ADR-10 | **阻塞 crates.io 首发**,而首发之后改名等于弃坑重来。这是唯一一件"越晚做越贵"的事 |
| `.sv` 的 LSP spike | 风险清单**第 1 条**,是 `.sv` 前端能否转正的悬置条件。悬置一天,双前端策略就多背一天的债 |
| 增量布局 | 30k 全量档实测 130–160ms,**已越过预案里 2ms 的触发线**(DESIGN R2 记录)。这是"能不能做真应用"的问题;而且 §4.2 揭示的"版本号即布局缓存键"正是同一件事的另一面 —— 先做它,lottie 的驱动设计会更简单 |
| 真机 IME / NVDA·VoiceOver·Orca 朗读冒烟 | R3 的收尾欠账。**没朗读过的无障碍等于没有无障碍**,而这是商用档 B 的及格线 |
| 鸿蒙 R5 | 差异化立足点(ADR-5:"鸿蒙一等公民"是我们对 Freya/Floem 的差异化);越晚开始越难 |

**判据**:上面每一条都在回答"这个库能不能用/能不能发布/能不能被接受";
lottie 回答的是"这个库好不好看"。在一个尚未发布、尚无 LSP、30k 控件要布局 130ms
的原型上,后者的机会成本是**一次 LSP spike**。

**会改变这个结论的触发条件**(写出来,免得半年后靠感觉重议):
1. 出现明确的产品需求(要交付的应用需要 loading/空状态/引导动效),且设计侧已经在用
   AE + lottie 出稿 —— 那 lottie 就从"锦上添花"变成"需求";
2. sv-arco 真的开工到 A4/A5 波次(Spin/Skeleton/Progress 的动效需求集中爆发);
3. velato 上游把遮罩/文本补齐并去掉"missing features"的自述 —— 上游成熟度是本方案
   风险的主要来源,它一降,估算的悲观值也跟着降。

**并且**:即便决定做,也**先只做步骤 0**,把步骤 1–3 挂在触发条件上。
步骤 0 独立合入、独立有价值,这正是把它切成第一刀的意义。

---

## 9. 未核实与待验证

诚实清单,实现前要各花十几分钟落实:

1. **kurbo 辅助 API 名**:`Affine::transform_rect_bbox`、`Shape::path_elements(tolerance)`
   的确切签名未逐个实查(`Shape`/`PathEl`/`Affine` 的存在与 velato 的用法已实查)。
2. **`peniko::Brush` → 我们的 `Color`**:`AlphaColor::to_rgba8() -> Rgba8` 的签名已实查
   (docs.rs `color` crate),但 `Rgba8` 的字段名(`r/g/b/a`)按惯例假定,未逐字核对;
   `Gradient` 的 `stops` 结构未实查(平均色算法依赖它)。
3. **编译时间与二进制体积**:开 `lottie` feature 后的增量未实测。
   建议在步骤 1 的 PR 里附一组 `cargo build --timings` 数字。
4. **velato 的每帧编码开销**:一个典型 loading 动画(几十图层)在 `append` 上
   花多少 µs,完全未测。若超过帧预算的显著比例(ADR-9 的 6.94ms),
   步骤 4 的"命令流缓存"就从可选变成必需。**这个数决定 lottie 能不能和
   ADR-9 的百万控件目标共存。**
5. **`rasterlottie`**(纯 Rust、2026-04 首发)是否会长成更省事的 CPU 侧方案 ——
   当前太新,列为观察项,不作为方案依据。
6. **`sv-shell` 的 MSRV 构建道是否会开 `lottie` feature** —— 取决于 CI 配置的现状,
   本文未核实该 workflow 的 feature 矩阵。
7. **资产管线**(编入 / 运行时加载 / build 期预处理)与图标、图片共用,
   属于另一件事的范围,本文只保证接口不与之冲突,未给方案。
