# Lottie 接入 svelte-rs:架构方案

> 状态:**设计裁决,未实现**。日期 2026-07-22。
> 范围:回答"lottie 要不要做、做成什么形状、第一刀切在哪、真实成本多少"。
> 前置阅读:[DESIGN.md](../DESIGN.md) ADR-3 / ADR-3b(渲染双后端)、ADR-6(帧调度)、
> ADR-9(规模策略)、[调研 26](../research/26-arco-design-ui-kit.md) §3.2(图标管线,
> 与本方案共享同一个前置动词)。
> 本文所有上游版本/API 均于 2026-07-22 联网实查,证据见 §1;未核实项集中在 §9,
> **没有一处是凭记忆写的版本号**。

> ## ⚠️ 复核纠偏(2026-07-22,对抗性复核)
>
> 本文初稿有**一条前提整体过期**、**一处关键论据被同批入库的兄弟文档实测推翻**、
> **两处与源码不符的断言**。全文已就地打补丁(每处以 `> ⚠️ 复核` 开头),
> 清单与无法验证项见文末 §10「复核记录」。**先读这五条再往下看**:
>
> 1. **§2 与「步骤 0」已经落地了,不是待办。** `Painter::fill_path` 在
>    `7966785` 入库、`stroke_path` 在 `3ebe81c` 入库,commit 标题原文就是
>    「路径动词齐活(lottie/SVG 图标的"步骤 0")」,**比本文入库的 `419d447`
>    早 6 分钟**。`cargo test -p sv-shell --lib paint::` 现在 7 绿。
>    于是 §0 表格第 6/7 行、§7「步骤 0」、§8.1 第一行、§8.3「前一半:该做」
>    **全部失效** —— 本文最强的那个结论已经被实现兑现掉了。
> 2. **§4.2 的"不 bump 就便宜"是错的量级。** 同批入库的
>    `docs/plans/incremental-layout.md` §8.2 实测:30k 树的**静止帧
>    (布局缓存全命中)仍要 117ms**,而 `layout_full_cached` 命中时仍
>    `clone` 一份 1.4MB。不 bump 只砍掉布局那一刀,绘制那堵墙原封不动。
>    本文引的「130–160ms」还是 DESIGN.md 当天自己标记为**测错了**的读数。
> 3. **§5.3 的"加载期扫描"在 velato 上做不到。** `lottie-1-ecology.md` §6.4
>    实测:velato 在**合法** Lottie 上 `panic` 而非返回 `Err`
>    (删掉图层 transform 的 `r` 键 → `todo!("split rotation")`,
>    `converters.rs:213`)。拿不到 `Composition` 就无从扫描,而这直接撞
>    R4「去 panic」红线 —— 本文只把上游不成熟记成**画质**风险,记轻了一档。
> 4. **§6.5 的 a11y 断言与 `lib.rs` 相反。** `push_access_tree()`
>    (`lib.rs:318`)在**每一个非短路帧**都跑,而 `animating` 恰恰是绕开短路的
>    那个条件。媒体时间不 bump **不能**让语义树免跑 —— 只会让它每帧白跑一次
>    全树遍历 + HashMap 重建,diff 出零个变更。
> 5. **§4.4 抓到的 vello 短路只是一半。** 同一个"不 bump"还会让
>    **非循环动画的最后一帧在两个后端上都被丢掉**(推导见 §4.4 补丁),
>    而这正好砸在 lottie 最该用的场景(成功勾选的定格姿势)上。

---

## 0. 裁决速查

| # | 问题 | 裁决 | 主要代价 |
|---|---|---|---|
| 1 | Painter 要不要长 `fill_path` | **要**,并同时长 `stroke_path`;路径用**自有 `PathCmd` 序列**(f32、设备像素、变换已烘焙),不借 kurbo/peniko 类型 | 多一次 PathEl→PathCmd 的转换与一次坐标变换;各向异性缩放下描边宽度是近似 |
| 2 | lottie 节点是什么 | **不新增 ElementKind**。`View` 上加一个 `paint_source` 槽(不透明 id)+ 壳侧资源注册表;lottie/SVG 图标/未来图片共用这一个扩展点 | measure 的 `View` 分支不再 `unreachable!`;多一层 id 命名空间纪律 |
| 3 | 动画驱动 | **接 `anim::pump`**,新增一条 `Channel::Media`;关键在于这条通道**每帧写节点时间但不 bump 版本** | 引出一处必须同步修的 vello 短路(§4.4);"不 bump 的写"是新概念,要写进注释与测试 |
| 4 | 双后端一致性 | **CPU 后端也能做**(velato 的 `vello` 依赖是 optional,渲染动词经 `RenderSink` 抽象)。v1 两个后端**降级到完全一致**:渐变取平均色、遮罩降为外接矩形、轨道遮罩不支持 | 放弃 vello 本可白拿的渐变保真;换来"换后端画面不变"这条硬保证 |
| 5 | 无障碍 | 有 `aria-label` → `Role::Image` + 名称;无 label → **装饰性,不进语义树**;自动播放且循环的必须可暂停(WCAG 2.2.2 A 级),接 `prefers-reduced-motion`(CSS-SUPPORT 已排期 C2) | 需要一个全局 reduced-motion 开关,以及"默认 autoplay+loop 是否合规"的文档口径 |
| 6 | 第一步切多小 | ~~步骤 0 = 只加路径动词 + 三后端实现 + 金样~~ **已落地(`7966785` + `3ebe81c`)**;新的第一刀是**脏矩形**(见下) | 无 |
| 7 | 现在该不该做 | ~~步骤 0 该做~~ **已做完**;lottie 本体仍排在改名 ADR-10、`.sv` LSP spike、增量布局/脏矩形之后 | 见 §8.3 |

> ⚠️ **复核:第 6/7 行的裁决已被实现兑现,本表原样保留只为存档。**
> 路径动词落地后,本文推荐的"第一刀"落空,而**真正的第一刀由复核补上:脏矩形**。
> 理由不是本文的,是 `lottie-1-ecology.md` §0 死穴二与 §7.2 排序的:
> lottie 一跑 = **整窗每帧重绘**,而 30k 树的纯绘制帧实测 117ms
> (`incremental-layout.md` §8.2)。本文 §4 花整节论证"怎么让布局别失效",
> 却从没问过"这一帧的绘制本身多少钱" —— 而那才是大头。
> **在有脏矩形之前,任何常驻动画(不止 lottie)在中大型界面上都不该开。**

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

> ⚠️ **复核:整节已经是既成事实,不是提案。** 本节的裁决(方案 A、自有 `PathCmd`、
> 不带 `transform`、`stroke_path` 不省)与 `paint.rs` 里**已经跑着的代码逐条一致**,
> 连"为什么不借 kurbo"的说辞都几乎逐字相同。差异只有命名与三处缩水,列在 §2.8。
> 复核实跑:`cargo test -p sv-shell --lib paint::` → **7 passed; 0 failed**。
> 保留本节的唯一价值是"裁决理由的存档";把它当待办排期就是重复劳动。

### 2.1 路径怎么表达

~~现状(`crates/sv-shell/src/paint.rs:84`)`Painter` 只有 `fill_rounded_rect` /
`stroke_rounded_rect` / `glyph_run` / `push_clip` / `pop_clip` / `caps`,**没有任意路径**。~~

> ⚠️ **复核:这句已经过期。** `paint.rs:189/198` 现在有
> `fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color)` 与
> `fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color)`,
> 三个后端(tiny-skia / vello / Recording)全部实现。
> `lottie-1-ecology.md` §7.0 已经点名更正过同一条事实 —— 两份同批入库的文档
> 一份说"已解除"、一份说"待办",**读者会以为是两个时间点的仓库**。

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

> ⚠️ **复核:两条 API 签名核对无误(docs.rs/tiny-skia/0.11.4 逐字比对,
> `Mask::new(u32,u32)->Option<Self>`、`fill_path(&Path, FillRule, bool, Transform)`、
> `intersect_path(&Path, FillRule, bool, Transform)`;1080p ≈ 2.07MB 的算术也对)。
> **但这个方案没有落地。** 已入库的 `fill_path`/`stroke_path` 给 tiny-skia 的
> `clip_mask` 参数传的是 `None`,并在注释里挂了缺口:
> 「**已知缺口**:滚动容器内的路径图标不会被裁掉;等真有这个场景再补」。
> 连带后果:§7「步骤 0」验收清单里的 `path_clip_confines_fill`(自称是
> "Mask 通道的守门员")与 `vello_path_parity` **两条都不存在**,
> 实际入库的 7 条测试没有一条碰裁剪。
> **所以"步骤 0 已完成"要打个折:动词齐了,裁剪语义还是欠的**,
> 而 lottie 的 `push_clip_layer`(§5)一定会踩到它。
>
> 另有一条本节没算的账:缓存键取 `(pixmap 尺寸, 裁剪矩形)`,而**平滑滚动期间
> 裁剪矩形每帧都在变** —— 那正是"路径图标 + 裁剪"最可能同时出现的场景。
> 键每帧失效 = 每帧 `Mask::new` + 清零 + 重光栅一张全画布掩码。
> `lottie-1-ecology.md` §6.2 实测了这条路的量级:1024px 时 **1MB/帧**。
> 真要做,缓存键得是"裁剪矩形取整后的 rect",并且要特判**轴对齐矩形直接走
> 现有的手动求交**,只有非矩形裁剪才落进 Mask。

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

> ⚠️ **复核:入库形态与本节提案有出入,以代码为准。** 实际的 `PaintCmd` 是
> `Path { cmds, fill, color, bbox: (i32,i32,i32,i32) }` 与
> `StrokePath { cmds, width, cap, join, color, bbox }` —— 变体名叫 `Path` 不叫
> `FillPath`,字段叫 `cmds` 不叫 `verbs`,`bbox` 是四元组不是数组,
> 而且**入库版比本节提案更强**:它连 `cap`/`join` 也记了(本节的 `StrokePath`
> 只记 `width`,会漏掉"圆端帽退化成平端帽"这类回归)。引用本节写测试会写错字段名。

### 2.7 顺带兑现:SVG 图标

调研 26 把"`fill_path` + SVG 编译期转译"列为 sv-arco 的头号卡点,并断言
"没有这项,arco 视觉完成度上限约六成"。步骤 0 交付的两个动词**就是那一半**:
剩下的是 build 期用 usvg 把 SVG 解析成 `PathCmd` 数据表(`生成数据而非类型`,ADR-2 哲学),
与 lottie 完全解耦。**这也是本方案建议把步骤 0 单独提前的根本原因**(§8.3)。

> ⚠️ **复核:引文核对无误**(调研 26 §5「没有这项,arco 视觉完成度上限约六成
> ——箭头/勾选/关闭/加载/类型提示图标无处不在」;§3.2 整体估 2–3 人周)。
> **但"那一半"已经交付了,而剩下的一半才是本文该指向的下一步**:
> build 期 usvg → `PathCmd` 数据表。它与 lottie 完全解耦、不新增运行时依赖、
> 不碰 MSRV、不引 serde,并且直接解锁调研 26 点名的环形 Progress /
> Tooltip 箭头 / Rate 星形。**复核认为这才是当前真正的"20% 力气 80% 收益",
> 排序上应当在 lottie 本体之前**(展开见 §8.4)。
> 另注:调研 26 §3.2 原文写的是 `fill_path(&[PathEl], color)`(kurbo 类型),
> 入库实现改成了自有 `PathCmd` —— 本文 §2.1 的方案 A 论证等于**追认**了
> 一个已经做出的选择,不是在做选择。

### 2.8 复核:入库形态 vs 本节提案的逐项差异

| 本节写的 | 实际入库的 | 影响 |
|---|---|---|
| `enum FillRule { NonZero, EvenOdd }` | **`enum PathFill`** | 名字冲突规避:`tiny_skia::FillRule` 已在 `paint.rs` 顶部 `use`,同名会撞 |
| `PaintCmd::FillPath { verbs, bbox: [i32;4], rule, color }` | `PaintCmd::Path { cmds, fill, color, bbox: (i32,i32,i32,i32) }` | 字段名/类型全不同 |
| `PaintCmd::StrokePath { verbs, bbox, width, color }` | 同名但**多 `cap` / `join`** | 入库版更严,提案版会漏回归 |
| tiny-skia 侧惰性 `Mask` 裁剪 | **未实现**,传 `None` + 注释挂缺口 | §2.4 补丁 |
| `clipped_bbox()` 提前剔除 | **未实现**,路径不做视口剔除 | 全屏外的路径仍会走完整光栅 |
| 验收 `path_clip_confines_fill` / `vello_path_parity` | **不存在** | 裁剪与双后端 parity 都无守门员 |

### 2.9 复核补充:两处本节没提、适配器一定会踩的有损映射

1. **`kurbo::Stroke` 的端帽是两个字段。** kurbo 0.13 的 `Stroke` 带
   `start_cap` 与 `end_cap`(以及 `dash_pattern`/`dash_offset`),而我们的
   `StrokeStyle` 只有一个 `cap`。适配器必须裁决"两端不同帽时取谁",
   并把 dash 明确记进不支持清单(velato README 本来就把 dash 列在不支持里,
   但那是 velato 不求值,不等于 `Stroke` 里不会带)。
2. **`Shape::path_elements(tolerance)` 的 tolerance 是我们选的,不是给定的。**
   §2.3 说"圆弧/椭圆由调用方在 kurbo 侧展开成三次贝塞尔"——展开精度是一个
   自由参数,选大了图标边缘出多边形,选小了每帧点数暴涨。它应当**随设备像素
   缩放**(经验值 0.1 物理像素),而不是写死。本文未给这个数。

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
`sv-shell/src/lib.rs:214` 的 `paint()`(`anim::pump` 在 `:220`)用
`let animating = sv_ui::anim::pump(now_ms);`
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

> ⚠️ **复核:相位纪律核对无误**(`lib.rs:218–221` 就是这个顺序),
> **但这段伪代码有一处会静默吃掉最后一帧的坑,见 §4.4 的复核补丁。**
> 另外 `pump` 现有的实现是 `retain_mut`,`Anim` 结构体里只有
> `from/to/start_ms/dur_ms` 四个数值槽 —— Media 通道要的是
> `last_ms`/`speed`/`looped`,**没有一个能复用**。所以"接 `anim::pump`"
> 实际是给 `Anim` 加一个枚举载荷或第二个数组,不是加一个 `Channel` 变体就完事。
> 这项工作量本文的 §8.1 没有单列。

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

> ⚠️ ### 复核:本节的机制对,量级错了一个数量级,而且遗漏了两个消费方
>
> **(1)引的数字是 DESIGN.md 当天自己划掉的读数。** 本节两处写「30k 全量档
> 一次布局 ~130–160ms(taffy 裸 ~45ms + 叶子 measure ~70ms)」。`DESIGN.md`
> R2 条目在 **2026-07-22 当天**(即本文入库同日)加了纠偏原文:
> 「当时那个 ~130–160ms 的读数**本身就偏了**:探针让 30000 个叶子共享两种文本串,
> 而 measure 缓存的键含文本,于是**永远命中**,量到的是'没有 measure 成本的布局'。
> 改成逐行唯一文本(真实界面的口径)后是 **2525ms**。」
> 且 `caea14c`(比本文入库早 1 分钟)已把它压到 **111ms**。
> **本文引了一个被作废的数,又没引作废它的那条修正。**
>
> **(2)致命的一条:"不 bump 就便宜"不成立。**
> `incremental-layout.md` §8.2(同一 commit `419d447` 入库)实测:
> **30k 静止帧(布局缓存全命中)仍要 117ms**,原文「把布局从 311ms 压到 2ms,
> 整帧从 ~430ms 变成 ~120ms —— **还是 8fps**」。
> 也就是说本节精心设计的"不 bump"最多把 lottie 帧从 430ms 救到 120ms,
> **离 16.7ms 的 60fps 预算还差 7 倍,离 ADR-9 的 6.94ms 差 17 倍。**
> 本节把布局失效当成唯一的墙,而实际上**绘制才是更高的那堵墙** ——
> 兄弟文档的原话是「**如果只能排一件事,那件事是给 `shape` 加缓存,不是增量布局**」。
>
> **(3)"缓存命中 = 免费"也不成立。** `layout_full_cached`
> (`render.rs:458`)命中时执行的是 `return layout.clone()` ——
> **一份完整的 `Vec<Placed>` 深拷贝**。更糟的是 CPU 路径**每帧调它两次**:
> `render_frame`(`render.rs:959`)内部一次、`lib.rs:239` 再一次。
> `incremental-layout.md` 步骤 2 已经把这条记成待修项:
> 「`lib.rs:239/282` 在 `render_frame` 内部已经算过布局之后又调一次
> `layout_full_cached`,拿到的是缓存命中但仍 clone 一份 **1.4MB**(30k 档)」。
> 60fps × 2 次 × 1.4MB ≈ **168MB/s 的纯 memcpy**,只为了让一个转圈图标动起来。
>
> **(4)漏了两个 `version()` 消费方,而本文自己的 §8.1 说"审计比写代码贵"。**
> 本节表格里列的免责理由是"不影响布局/命中/语义树",但:
> - **语义树不免责**,见 §6 的复核补丁(`push_access_tree` 每帧都跑);
> - **IME 上报不免责**:`lib.rs:295–302` 的 `set_ime_cursor_area` 也在
>   短路之后无条件执行,`animating` 同样绕开短路。lottie 播放期间每帧都会
>   向平台重报一次候选窗矩形。值没变,但系统调用照发。
>
> **复核结论:"不 bump"这个技巧本身是对的、该做的**(它确实消灭了一次
> 每帧 311ms 的全量重算),**但它不足以让 lottie 可用**。
> 真正的前置是**脏矩形 / 局部重绘**,而那是全仓性改造,不该记在 lottie 账上
> —— 却必须记在 lottie 的**前置条件**里。本文 §8.2 的隐藏成本清单里没有它。

### 4.3 帧对齐(ADR-6)下的排帧

`paint()` 尾部已经有 `if animating { ws.window.request_redraw(); }` —— 有动画就续帧,
没有就停。Media 通道天然接上:`playing=false` 后 `pump` 不再保留该 anim,
`active()` 归 false,窗口回到零功耗静止。用户点"播放"→ 写 signal → effect 调
`doc.play(node)` → 注册 anim + `bump()` → `on_mutate` → `request_redraw` → 循环起来。
**没有一行新的调度代码。**

### 4.4 静止帧短路:CPU 已经对,vello 有一处必须改

```rust
// lib.rs:230(复核更正:本文原写 :227,当前 HEAD 是 :230)
let frame_key = (self.doc.version(), size.width, size.height, scale.to_bits());
let unchanged = self.last_frame_key == Some(frame_key);
if unchanged && !animating && !self.show_fps { return; }   // ← 正确:animating 时不短路
```

CPU 路径没问题。但下面这行有问题:

```rust
// lib.rs:281(复核更正:本文原写 :278)
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

> ⚠️ **复核:这一处的分析成立,源码逐行核过。**
> `vello_backend.rs:299` 确实是 `if !scene_unchanged { self.painter.scene.reset();
> paint_tree(..); paint_scrollbars(..); }`,`scene_unchanged=true` 时整段跳过、
> 只重呈现上一份 `Scene`。修法与测试建议(抽成
> `should_skip_scene_encode(unchanged, animating)` 纯函数)都合理。**本条保留。**

#### 4.4b 复核补充:同一个"不 bump"还会丢掉最后一帧(两个后端都丢)

本文找到了 vello 那一半,漏了对称的另一半。**推导链全部出自已入库的源码**:

1. `anim::pump` 是 `anims.retain_mut(|an| { ...写值...; t < 1.0 })` ——
   **到点那一帧仍然会写最终值,然后返回 false 把自己摘掉**
   (`anim.rs:111–133`;`ScrollY` 分支还特意在 `t >= 1.0` 时写精确目标,
   注释写着"收尾一帧写精确目标……浮点上差一点点会留下半像素错位")。
2. 现有两条通道靠 `update_style` / `set_scroll` **在这一帧 bump 了版本**,
   于是 `unchanged` 为 false,收尾帧照常绘制 —— 这正是
   `scroll_snaps_exactly_to_target_on_final_frame` 那条测试在守的东西。
3. Media 通道**按设计不 bump**。于是收尾帧上:
   `animating = pump(..) = false`(anim 已被摘掉)、
   `unchanged = true`(版本没动)、`show_fps = false`
   → `lib.rs:232` **提前 return,这一帧根本没画**。

**后果**:非循环 lottie 的**最后一帧永远不显示**,画面定格在倒数第二帧。
`on_end` 照常触发,所以从代码上看一切正常 —— 这是一个只有肉眼能发现的 bug,
而本文全篇的验收都是无窗测试。而且它砸中的正是 lottie 最该用的场景:
`lottie-1-ecology.md` §7.1 列的适用场景第一条就是"Result(成功/失败插画)"
—— 成功勾选的**定格姿势**恰恰是最后一帧。

**修法(择一,建议前者)**:

- **收尾帧破例 bump 一次。** Media 通道在 `t >= duration` 且 `!looped` 的
  那一次写调 `bump()`。代价是一整帧的全量重布局 —— 但**一次**,不是每帧,
  与 §4.2 反对的"每帧 bump"不是一回事。语义上也更对:动画结束
  = 状态跃迁,本来就该让 a11y/命中/IME 各消费方看见一次。
- 或者让 `paint()` 记住 `was_animating`,`animating || was_animating` 时不短路。
  更省一次布局,但多一个跨帧状态,且要同步改 §4.4 的 vello 条件。

**验收测试(无窗)**:`media_last_frame_is_painted` ——
把 §7 步骤 2 的 `should_skip_scene_encode` 扩成
`fn should_skip_frame(unchanged, animating, was_animating, show_fps) -> bool`,
钉死 `(true, false, true, false) => false`。

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

> ✅ **复核:本节全部核实通过,是本文最扎实的一节。** 逐项证据(2026-07-22 重查):
> - `crates.io/api/v1/crates/velato/0.11.0/dependencies`:`vello ^0.9.0` **optional=true**
>   (另有一条同版本的 **dev** 依赖,不影响下游);`kurbo ^0.13.0` / `peniko ^0.6.0` /
>   `serde ^1.0.228`(derive)/ `serde_json ^1.0.149` / `serde_repr ^0.1.20` 均 required。
> - velato `Cargo.toml` 的 `[features]` 原文:`default = ["vello"]` /
>   `vello = ["dep:vello"]` / `wgpu = ["vello", "vello/wgpu"]` ——
>   **`default-features = false` 确实是必需且充分的**,本文的 Cargo 行写对了。
> - velato `src/lib.rs`:只有 `pub use vello;` 带 `#[cfg(feature = "vello")]`;
>   **`pub use runtime::{Composition, RenderSink, Renderer, model};` 无条件导出**。
>   §1.1 那句"lottie 不是 vello 专属能力"因此成立。
> - 本仓库 `Cargo.lock` 里 `kurbo 0.13.1` / `peniko 0.6.1` / `vello 0.9.0`,
>   与 §1.1 的"零新解析风险"一致。
>
> 兄弟文档 `lottie-1-ecology.md` §6 更进一步:**已经在 scratchpad 里真接过一遍**
> —— `velato 0.11 (default-features=false)` + 自写 `RenderSink` → tiny-skia 0.11,
> 依赖树 15 crate、零 wgpu、Tiger.json 在 **256px 0.99ms/帧**渲染正确。
> **本节的可行性从"推理"升级成"实测",可以更硬地写。**

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

> ⚠️ **复核:这张表有一条搞反、一条把"画错"写成了"不画"、一条 README 已过期。**
>
> **(a)"图层遮罩 → 降级为外接矩形"把 mask 和 matte 搞混了,而且把 mask 说轻了。**
> `lottie-1-ecology.md` §1.4(1) 逐行读过源码:`masksProperties` 走的是
> `scene.push_clip_layer(transform, &self.mask_elements.as_slice())` ——
> **velato 会真的发出任意路径裁剪命令**。降级成外接矩形不是"我们只有矩形 push_clip"
> 这么中性的一句:一个用 mask 做"擦除揭示"的动画,退化成矩形后**整块内容直接露出来**。
> 这条要么写清楚"退化是可见错误、不是精度损失",要么就把
> `Painter::push_clip_path` 列进前置(tiny-skia 侧就是 §2.4 那张 `Mask`)。
>
> **(b)轨道遮罩"不支持 + 配平"没说清楚不支持之后画面上发生了什么。**
> velato 对 track matte 的发法是:先 `push_layer(Mix::Normal, 1.0, ..)`
> 把**蒙版源图层当普通内容画进去**,再 `push_layer(matte_mode, ..)` 合成。
> 我们忽略 `push_layer` 只保证栈配平,结果不是"这层没画",而是
> **蒙版源图层被当作可见内容画了出来**。`lottie-1-ecology.md` §6.3 的实测原话:
> 「我把 `push_layer` 忽略掉之后……**蒙版源图层的形状被当作普通内容画了出去**。
> 这不是正确性证明,只能说明'在这个样本上退化得不难看'。」
> 而且这不是边角料:官方 Tiger.json **60 帧里有 121 条**这种命令(占 3.8%),
> 是它唯一的缺口。**正确的 v1 行为应该是"带 `tt`/`td` 的图层整层跳过",
> 而不是"忽略图层动词"** —— 少画一层远好过多画一层蒙版源。
>
> **(c)"Split positions 不支持"过期。** velato README 仍列着,但
> `converters.rs` 的 `conv_transform` 已实现,`lottie-1-ecology.md` §6.4 实测
> split position 文件解析渲染正常(而 split **rotation** 会 panic)。
> **别把 README 当支持子集的唯一来源** —— 本文 §1.1 恰恰这么做了。
>
> **(d)"渐变取色标平均色"仍未核实,而且平均色是个可疑的降级。**
> 本文 §9 自己承认 `Gradient` 的 `stops` 结构未实查(复核也未查到逐字签名,
> 只确认 `peniko::ColorStops` 经 `fixed::ColorStops` 别名暴露)。
> 更值得说的是:线性渐变按色标**算术平均**在视觉上通常偏暗/偏灰
> (未按 stop 的 offset 加权、也未做 premultiplied 混合)。
> 若真要降级,至少按 offset 区间加权;更省事的是**取中点色**。
> 顺带:`lottie-1-ecology.md` §6.3 实测 Tiger.json 的渐变填充命令数是 **0**,
> 描边也是 0 —— 这条降级在最常见的样本上根本不触发,优先级可以调低。

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

> ⚠️ ### 复核:纪律 2 在 velato 上物理上做不到,而且漏掉了第一纪律
>
> **本文最严重的一处遗漏**:整篇把上游不成熟当成**画质**风险
> (§8.2 第 1 条:"会随机踩到不支持特性,而我们没有能力修"),
> 但真实的失败模式是**进程死亡**。
>
> `lottie-1-ecology.md` §1.4(3) + §6.4 的实测(不是读源码推测,是改了 JSON 跑出来的):
>
> - velato 的 `src/error.rs` **全文只有** `pub enum Error { Json(serde_json::Error) }`;
> - 所有"不支持"走的是 `todo!()` / `unimplemented!()`,散在 `import/converters.rs`:
>   `:103` 图片资产、`:211`/`:213` split rotation、`:243`/`:252` 形状组 split、
>   `:805`/`:806` blend 模式 `Add`/`HardMix`;
> - **`:213` 是个纯 bug**:schema 把 rotation 声明成 `Option`,`None` 分支写了
>   `todo!("split rotation")`。**删掉图层 transform 的 `r` 键就 panic** ——
>   而"剥掉默认字段"正是所有 Lottie 优化器的常规操作。
>
> **连带的三条后果:**
>
> 1. **纪律 2("加载期扫一遍图层")做不到。** 扫描的前提是拿到
>    `Composition`;而失败发生在 `from_slice` **内部**、以 panic 形式。
>    这条纪律要么改成"`catch_unwind` 之后才谈扫描",要么删掉。
> 2. **撞 R4「去 panic」红线,而本文自己在 §3.3 引用了这条红线**
>    (「加载失败的 lottie 会**崩进程**,正好撞在 R4 的红线上」)——
>    本文只想到我们自己的 `unreachable!`,没想到**依赖里的**。
>    资产是设计师给的第三方数据,这等于把外部输入接到进程存活性上。
> 3. **§7 步骤 1 的验收测试挡不住它。** `lottie_missing_source_draws_placeholder`
>    测的是"坏 JSON → 返回 `Err`",那条路径是通的(serde 报错);
>    真正会炸的是**合法 JSON + 不支持特性**。验收要改成:
>    夹具里放一个**删掉 `r` 键**的图层,断言"不 panic、退化成占位"。
>
> **补一条"第 0 纪律(运行期):panic 兜底不是可选项。"**
> `catch_unwind` 必须同时罩住 `Composition::from_slice`(导入期六个 `todo!()`)
> **和**每帧的 `Renderer::append`(velato 0.8.1 CHANGELOG 记着修过求根算术 panic)。
> 前提已核:本仓库根 `Cargo.toml` 只有 `[profile.dev]` / `[profile.dev.package."*"]`,
> **没有设 `panic = "abort"`**,unwind 可用。
> 但要写清楚这是创可贴:panic hook 仍会打栈污染日志。
> **真正的治疗是给 velato 提 PR 把 `:213` 的 `todo!()` 改成 `Error` 变体
> —— 那大概是 20 行,性价比高于本文里任何一项工程。**

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
   只动一个节点"),lottie 每帧推树是对它的直接背叛。~~§4.2 的"不 bump 版本"顺带
   保证了这一点 —— a11y 推送是版本节拍驱动的,媒体时间不 bump,自然不推。~~

> ⚠️ ### 复核:第 5 条的后半句与源码相反 —— 恰恰是每帧都跑
>
> **1–4 条复核通过**,逐项证据:
> - `accesskit 0.24.1` 的 `Role` 里 `Image`(`:68`)、`Canvas`(`:125`)、
>   `SvgRoot`(`:193`)、`GraphicsObject`(`:215`)、`ProgressIndicator`(`:175`)、
>   `Marquee`(`:165`)、`Timer`(`:199`)全部存在(本地 registry 源码 grep);
> - WCAG 2.2.2 的引文与 **Level A** 逐字核对无误(w3.org/WAI/WCAG22 原页)。
>   **唯一要补的是本文省掉的例外从句**:原文结尾是
>   "…unless the movement, blinking, or scrolling is **part of an activity where
>   it is essential**"。一个"正在加载"的 spinner 是不是 essential 存在解释空间;
>   本文取保守解读(一律要求可暂停)没问题,但**文档口径里要写明这是我们的选择,
>   不是规范的强制**,否则将来会被当成规范原文引用。
>
> **第 5 条的后半句是错的。** `lib.rs` 的实际控制流:
>
> ```rust
> // lib.rs:232
> if unchanged && !animating && !self.show_fps { return; }
> ...
> // lib.rs:318 —— 函数尾部,无条件
> self.push_access_tree();
> ```
>
> a11y 推送**不是**"版本节拍驱动"的:它由"这一帧有没有被短路"驱动,
> 而 `animating` 正是绕开短路的那个条件。lottie 一开播,
> `push_access_tree()` → `a11y::incremental_tree_update()` **每帧都跑**。
> 而 `incremental_tree_update`(`a11y.rs:54`)是"**增量推送、全量计算**":
> 内部先 `collect(doc, placed, scale)` **走一遍完整场景树** +
> 建一个全节点的 `HashMap<ViewId, Rect>` + 再建一个全节点的 `next: HashMap`,
> 逐节点 `PartialEq` 比对。
>
> **净效果:媒体时间不 bump,换来的是每帧一次"遍历全树、diff 出零个变更"的空转。**
> 60fps × 全树遍历 × 两张 HashMap 分配,而且**只在屏幕阅读器已激活时**发生
> —— 也就是说,这个开销只砸在最需要性能预算的那批用户身上。
>
> **这恰恰是本文 §4.2 对布局做对了、对 a11y 没做的同一件事。** 修法二选一:
> - `push_access_tree` 前加一道 `if self.doc.version() != self.a11y_version { .. }`
>   的版本闸(与 §4.2 的"不 bump"配合后自动为零成本);**推荐**,一行,
>   且顺带修掉"平滑滚动 140ms 内每帧全树 diff"这个既有开销;
> - 或者把"这一帧只有媒体在动"这个事实传给 `push_access_tree`。
>
> 无论选哪条,**§4.4b 的"收尾帧破例 bump 一次"都必须保留** ——
> 动画结束是语义树该看见的一次状态跃迁。
>
> 另注:本条的验收测试应当是 `media_frame_does_not_touch_a11y`
> (推进媒体时间 N 帧,断言 `incremental_tree_update` 的调用次数为 0
> 或返回的 `changed` 恒空),§7 步骤 3 的 "a11y 金样(有/无 label 两例)"
> 抓不到这个 —— 金样只看内容,不看**跑了几次**。

---

## 7. 分步落地

每一步都要求:**能单独合入、单独有价值、有不开窗的验收测试**。

### 步骤 0:路径动词(不含任何 lottie)——**✅ 已落地,本节转存档**

> ⚠️ **复核:`7966785`(`fill_path`)+ `3ebe81c`(`stroke_path`)已入库,
> `cargo test -p sv-shell --lib paint::` 7 绿。** 但**范围缩了两块**,不能当"完成"划掉:
> - **惰性 `Mask` 裁剪未实现**(传 `None`,代码注释里挂着缺口);
> - 验收清单里的 **3(`path_clip_confines_fill`)与 4(`vello_path_parity`)不存在**。
>   实际入库的 7 条是:`fill_rule_nonzero_vs_evenodd` / `cubic_curve_is_rasterized` /
>   `degenerate_paths_do_not_panic` / `stroke_paints_the_line_not_the_area` /
>   `stroke_width_reaches_the_backend` / `line_cap_shape_reaches_the_backend` /
>   `recording_painter_records_path_shape` —— **没有一条碰裁剪或双后端 parity**。
>
> **所以"步骤 0"应当拆成 0a(✅ 已完成)与 0b(⬜ 未完成)**:
> 0b = 路径裁剪(CPU 侧 `Mask` / vello 侧原生)+ 那两条守门员测试。
> 0b 不是 lottie 的可选项:velato 的 `push_clip_layer` 会真的发路径裁剪命令
> (§5.2 复核 (a))。

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

> ⚠️ **复核:步骤 1 的验收挡不住真正会炸的那类输入,而且范围少了一件必做项。**
> - `lottie_missing_source_draws_placeholder` 测的是"**坏 JSON**",那条路 serde
>   会正常返回 `Err`。真正会 panic 的是"**合法 JSON + 不支持特性**"。
>   夹具必须再加一个:**手写 lottie 里删掉图层 transform 的 `r` 键**,
>   断言不 panic(见 §5.3 复核)。这条测试写出来会**立刻红**,除非先做下一条。
> - 范围里必须加上 **`catch_unwind` 兜底**(`from_slice` 与 `append` 两处)。
>   本文把它完全漏了 —— 而没有它,步骤 1 合入的那天就是把第三方数据接到
>   进程存活性上的那天。
> - `lottie_first_frame_commands` 断言"总包围盒落在节点矩形内"是好设计,
>   但要注意 velato 的 `Renderer::append` **一上来就无条件发一条
>   `push_clip_layer(transform, Rect(0,0,w,h))`**(composition 边界)。
>   适配器要**特判这条轴对齐矩形走现有的 `push_clip`**,否则它会落进
>   路径裁剪通道(§2.4 那张全画布 `Mask`,1024px 实测 1MB/帧)。

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

> ⚠️ **复核:步骤 2 的验收有一条会给出假绿灯,还少两条。**
> - **假绿灯**:第 4 条 `media_time_does_not_invalidate_layout` 断言"命中缓存"
>   就收工 —— 但命中缓存**仍然 clone 一份 1.4MB**(§4.2 复核 (3)),
>   而 CPU 路径每帧 clone 两次。这条测试会绿,而用户会卡。
>   要么把断言升级成"整帧耗时预算",要么在注释里写死"本测试只证明没触发
>   taffy,**不证明这一帧便宜**"。
> - **缺 `media_last_frame_is_painted`**(§4.4b:非循环动画丢最后一帧);
> - **缺 `media_frame_does_not_touch_a11y`**(§6 复核:每帧全树 diff)。

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

> ⚠️ ### 复核:这张表同时**高估**和**低估**,方向相反,不能简单加减
>
> **高估的部分(本文最该改的一处):步骤 1 的 1–1.5 人周站不住。**
> 本文自己说"适配器本身约 150 行",而 `lottie-1-ecology.md` §6 **已经把这 200 行
> 写出来跑过了**(scratchpad,velato 0.11 → tiny-skia 0.11,Tiger.json 渲染正确)。
> 它给同类范围的估算是 **3–5 人日(高置信)**。两份文档差 **5–7 倍**。
> 范围不完全重叠(本文含 `PaintSource` 槽 + 布局接线 + 双前端 + 双语文档,
> 兄弟文档是独立 `sv-lottie` crate),但**"适配器 150 行 = 1–1.5 人周"这一项
> 本身就自相矛盾**,而且兄弟文档有可运行的证据、本文没有。
>
> **低估的部分(三项,都不在表里):**
>
> | 漏项 | 出处 | 量级 |
> |---|---|---|
> | 步骤 0b:路径裁剪 + 两条守门员测试 | §2.4 / §7 复核 | 0.5–1 人周,**且是 velato 的硬需求** |
> | `catch_unwind` 兜底 + 不支持特性的**运行期**分桶报告 | §5.3 复核 | 0.3–0.5 人周(小,但漏了就没有步骤 1) |
> | `Anim` 结构体扩容(现有四个数值槽没有一个能给 Media 复用) | §4.1 复核 | 小,但表里"代码量小"的判断建立在"加个枚举变体"上 |
>
> **以及一项无法定价的**:§4.2 复核 (2) 表明,**在脏矩形落地之前,
> 中大型界面上的 lottie 是不可交付的**(30k 树纯绘制帧 117ms)。
> 把脏矩形算进来,这就不是 4–6.5 人周的事 —— 它是另一个立项。
> **正确的写法是把脏矩形列为前置条件,而不是列进成本。**
>
> 复核给的修正区间:**在脏矩形已就位的前提下,步骤 0b+1+2 约 2–3 人周(中);
> 步骤 3(a11y + 双前端 + 双语文档)1.5–2.5 人周不变。合计 3.5–5.5 人周。**
> 比本文低,是因为兄弟文档已经把最不确定的那块(适配器可行性)实测掉了。

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

> ⚠️ ### 复核:第 1 条记轻了一档,第 3、4 条数字要改,并补两条
>
> **第 1 条**:见 §5.3 复核 —— 上游的问题不是"画不对",是 **panic**。
> 这一条要从"画质风险"升级为"**进程存活性风险 + R4 红线**"。
> 「缓解是 §5.3 的加载期扫描」这句要删掉:那个扫描做不到。
>
> **第 3 条(MSRV)**:本文写"CI 的 MSRV 构建道**若**开 `lottie` feature" ——
> 这个"若"是可以现在就消掉的,`.github/workflows/ci.yml:219–242` 的 msrv job 跑的是
> `cargo build --workspace --all-features`,**任何新 feature 都无条件进这条道**。
> 但同一段 CI 注释也已经把这件事当成**设计意图**而非风险:
> 「现状核过:`--all-features` 依赖图里声明的最高 rust-version 恰好就是 1.88
> (parley/fontique/vello 一档),没有富余 —— **上游一抬,这条道会先红,
> 那正是我们要的早期信号**」。所以准确的写法是:**velato 不会新增 MSRV 风险类型,
> 只是多一个会触发这个既有信号的上游**。README 的原文复核逐字无误:
> 「Future versions of Velato might increase the Rust version requirement.
> It will not be treated as a breaking change and as such can even happen with
> small patch releases.」
>
> **第 4 条(依赖面):"≈ 8 个 crate" 少算了一半。** 实测(`lottie-1-ecology.md` §6.1,
> `cargo tree -e normal`)是 **15 个**。本文漏掉的是传递依赖:
> `kurbo` → `arrayvec` / `euclid` / `polycool` / `smallvec`(本仓库 `Cargo.lock` 可查),
> `peniko` → `color` / `linebender_resource_handle` / `smallvec`,
> `serde_json` → `itoa` / `ryu` / `memchr`。
> 好消息是同一份实测给了结论:「全量编译(冷启动、release)15 个 crate,**秒级**」
> —— 所以本文列为"未实测"的编译时间影响,**方向上已经可以判定为可忽略**。
> `cargo-deny` 一项复核确认:velato 是 `Apache-2.0 OR MIT`,来源 crates.io,
> 门禁零阻力。
>
> **补第 6 条:整窗重绘。** 见 §4.2 复核。这是本文隐藏成本清单里**最贵的一项**,
> 却完全没出现 —— 而它恰好是 `lottie-1-ecology.md` §0 列的两条死穴之一。
>
> **补第 7 条:上游没有被压测过。** velato 在 crates.io 上的反向依赖只有三个
> (`bevy_vello` / `velato_imaging` / `open-weather-wizard`)。
> **不要把对 vello 的信心平移到 velato 上** —— ADR-3 选 vello 的立论依据是
> "被 Blitz/Masonry/Bevy 共同压测",velato 不在那个句子的范围内。

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
