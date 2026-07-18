# 14 · 可切换渲染后端的可行性与 Painter 抽象设计

> 调研日期:2026-07-18。主题:sv-shell 从"tiny-skia 硬编码"走向"可切换后端(CPU / vello
> 家族)"的抽象设计。关键版本号与各框架现状均已联网核实(来源见文末);少数标注
> "⚠ 仅基于训练数据"的点未逐条核实。本文与 [DESIGN.md ADR-3](../DESIGN.md)(vello 家族
> 归宿 + 自有 Painter trait)、[调研 05](05-rendering-stack.md)(渲染栈选型)衔接,
> 落点是 M2"Painter trait"里程碑的具体设计与最小重构清单。

---

## 0. TL;DR(结论先行)

1. **可行性判决:高度可行,且是业界标准做法。** Slint(4 个渲染器)、Iced(wgpu/tiny-skia
   双渲染器 + 运行时回退)、Flutter(DisplayList 之下 Skia/Impeller 双 dispatcher 共存四年
   平滑迁移)全部验证了"共享遍历 + 窄绘制接口 + 编译期 feature × 运行时选择"的组合。
   Rust 侧的失败案例(tachys 泛型渲染器)失败在**泛型参数污染整棵用户视图树**,而不是
   抽象本身——用 `dyn` 收在一个 crate 边界内即可避开。
2. **形态裁决:trait 即时调用(方案 a)作为接口;显示列表(方案 b)不是另一种架构,
   而是该 trait 的一个实现**(`RecordingPainter`),免费获得金样测试与未来的局部重绘/
   场景片段缓存;场景树直读(方案 c)拒绝——它把 Doc 的全部语义(继承、组透明、
   元素合成)复制进每个后端,正是 Slint 靠共享遍历避开的坑。
3. **词汇表对齐 vello/anyrender,不自造第三套**:trait 方法集 = fill / stroke / glyph_run /
   push_layer / pop_layer(+ 预留 box_shadow / image / caps),几何与画刷直接用
   kurbo / peniko 类型。这样 `VelloPainter` 是 1:1 转发,`anyrender`(0.11.1,已有
   vello / vello_cpu / vello_hybrid / skia 四个后端)可以做免费桥接。
4. **文本裁决:painter 拿"定位好的 glyph run"(字形 id + 位置 + 字号 + 画刷),
   不拿字符串、不拿位图。** shaping/布局在 painter 之上(现 fontdue,M2 换 Parley),
   光栅化在 painter 之下(CPU 后端用 fontdue `rasterize_indexed`,vello 后端走
   `draw_glyphs` builder → Glifo/swash)。这是 GPU/CPU 双端唯一都不吃亏的切分点;
   Slint 把文本测量下放到每个渲染器,导致其软件渲染器"仅支持西文"——反面教材。
5. **切换机制:cargo feature 决定编进哪些后端;运行时"env 覆盖 → GPU 探测 → CPU 回退"
   决定用哪个**(Slint `SLINT_BACKEND` / iced `ICED_BACKEND` 同款)。顶层用 enum 静态
   分发(iced `fallback::Renderer` 同款),绘制热路径 `&mut dyn Painter`。
6. **抽象税判决:值得付,且很便宜。** 每帧几百到几千次虚调用(个位数 µs)对上毫秒级
   光栅化,运行时开销可忽略;真正的税是**多后端功能齐平**(Slint 软件渲染器无旋转/
   阴影即为此税),缓解办法是后端家族收敛到 vello 系(三档共享同一 imaging model,
   齐平是上游的工作),tiny-skia 后端定位为过渡与测试基准,不做长期功能追平。

---

## 1. 先例深挖(联网核实,2026-07)

### 1.1 Slint:共享遍历 + ItemRenderer trait,四渲染器,编译期 feature + 运行时环境变量

**抽象层形态**:两层 trait——`Renderer`/`RendererSealed` 管渲染器级能力(文本测量、
资源管理、窗口绑定),`ItemRenderer` 管逐帧逐项绘制(draw_rectangle / draw_text /
draw_image…)。关键结构:**场景树遍历在 core 里共享,渲染器只实现"如何画一个图元"**;
`ItemRendererFeatures` 允许 item 查询渲染器能力做降级(如 GPU 端硬件图片平铺 vs
软件端手工裁剪)。(ItemRenderer 细节来自 DeepWiki 二手梳理,与官方仓库结构一致。)

**切换粒度**:三层——编译期 cargo feature(renderer-software / renderer-femtovg /
renderer-femtovg-wgpu / renderer-skia)决定编入哪些;运行时 `SLINT_BACKEND=winit-software`
这类"backend-renderer"环境变量覆盖;程序化 `BackendSelector` API(要求特定 renderer /
图形 API)。backend 默认按 qt → winit → linuxkms 顺序尝试初始化。官方文档**没有**
"GPU 初始化失败自动换 renderer"的承诺——选择在启动时定死,这点不如 iced。

**代价与社区信号**:四渲染器并存的真实成本清晰可见——软件渲染器**不支持旋转、
缩放、drop-shadow、带裁剪的圆角、文本描边,文本"目前仅限西文脚本"**(官方文档原话)。
即:切换机制本身不贵,贵的是每个后端把功能面追平。Slint 用公司化投入扛这笔税
(femtovg 还在加 wgpu 变体),单人/小团队不应模仿"多套异质渲染器",而应选
**同一 imaging model 的一族**(见 §1.5)。

### 1.2 Iced:wgpu/tiny-skia 双渲染器 + 运行时回退,enum 静态分发

**抽象层形态**:`iced_renderer` 把两个后端组合成 `fallback::Renderer<A, B>` 枚举
(`Primary`=wgpu,`Secondary`=tiny-skia),配套 `fallback::Compositor` 管 surface
创建与 present——**painter(画什么)与 compositor(怎么上屏)是两个正交抽象**,
这个切分我们要抄。widget 绘制面向统一的 Renderer API(fill_quad / 文本 / 图片…),
两个后端各自实现。

**切换粒度**:feature `wgpu` + `tiny-skia`(0.14.0 默认两个都开;至少必须开一个);
运行时先试 wgpu(adapter/device 创建),**失败自动落 tiny-skia**;
`ICED_BACKEND=wgpu,tiny-skia` 环境变量可指定尝试顺序,`WGPU_BACKEND` 进一步控制
图形 API。这是 Rust 生态里"GPU 探测失败 → CPU 兜底"最成熟的实现,虚拟机/远程
桌面/坏驱动场景的口碑来源。

**代价**:enum 静态分发(match 两臂)避免 dyn,但代价是双后端都要编译进二进制
(默认体积税);社区偶有"两后端渲染结果有细微差异"类 issue,靠 wgpu 端与
tiny-skia 端共享 tessellation/文本栈压差异。iced 0.14 保留双渲染器,没有收敛迹象
——证明这套结构长期可维护。

### 1.3 egui:wgpu 默认 / glow 可选,抽象位置更低(网格)

egui 的切换点与前两家不同:egui/epaint 把 UI **tessellate 成带纹理三角网格
(ClippedPrimitive)**,后端(egui-wgpu / egui_glow)只负责"画网格"——抽象位置
压到最低,后端极薄,所以双后端维护成本低。eframe 用 feature 选择(wgpu 为默认,
glow 需显式开启),两者都开时用 `NativeOptions::renderer` 运行时指定;web 上
WebGPU→WebGL 的回退由 wgpu 自己完成。社区痛点是两条集成路径的行为分叉
(issue #3079"Different logic for glow and wgpu integration")。
**对我们的启示**:抽象位置越低,后端越薄、越容易齐平——但"网格"这一层丢掉了
矢量语义(渐变/模糊/文本 hinting 都要在 tessellation 前解决),不适合我们
"vello 场景编码"的归宿;不过它证明了"共享上层 + 薄后端"方向正确。

### 1.4 Flutter:DisplayList 层让 Skia→Impeller 四年渐进迁移成为可能

工程经验最有分量的先例。Flutter 在 framework 与渲染器之间有 **DisplayList**
(SkPicture 的自研替代,录制 Canvas 操作的紧凑命令格式):官方 Impeller FAQ 原话
——"Impeller sits behind the Display List interface","display lists 提供通用接口
与可指定的 **dispatcher**;今天引擎同时有 Skia 与 Impeller 两个 dispatcher"。
录制格式"基本后端无关,raster 阶段动态转成 Skia/Impeller 对象"。

**迁移节奏**(渐进切换教科书):Impeller 先 iOS opt-in → iOS 默认(Flutter 3.10)
→ Android API 29+ 默认 → 逐步移除 Skia opt-out。动机(消除 shader 编译 jank:
Skia 运行时 JIT 编译 shader 超帧预算,Impeller AOT 预编译)与收益(第三方实测
jank 帧 12%→1.5% 一类数据)社区反复验证;早期 Android 上 Impeller 也经历过
回归期靠 opt-out flag 兜底(⚠ 回归细节仅基于训练数据)。
**结论**:一个稳定的"命令层"(display list)是双引擎共存、灰度切换、出问题回滚的
前提——没有它,切换就是硬分叉。我们的 `RecordingPainter`/`PaintCmd` 就是微缩版。

### 1.5 Linebender/vello 家族:统一 API 受挫 → AnyRender 与 imaging 两层抽象

对我们最直接的上游信号(2026 Q1 官方博客):

- 官方原计划做统一"Vello API"同时服务 vello_cpu 与 vello_hybrid,**因复杂度受挫,
  转向拥抱两个既有抽象**:**AnyRender**(Blitz/Dioxus 阵营,"贴近传统 canvas API,
  易用优先")与 **imaging**(forest-rs,"性能优先、操作集更全")。两者都支持
  vello 系与 Skia 多后端。
- **Masonry 已从硬编码 vello classic 迁移到 imaging**,由此获得"GPU-optional"
  (无 GPU 环境跑 vello_cpu)——与我们要做的事同构,方向被正主验证。
- **anyrender 0.11.1**(已核实 docs.rs)的 `PaintScene` trait 方法集:
  `fill(style, transform, brush, brush_transform, shape)` /
  `stroke(...)` / `push_layer(blend, alpha, transform, clip, filter, backdrop_filter)` /
  `push_clip_layer` / `pop_layer` / `draw_glyphs(font, size, hint, normalized_coords,
  transform, glyph_transform, brush, …)` / `draw_box_shadow` / `append_scene` /
  `draw_image`。后端已有 anyrender_vello / anyrender_vello_hybrid /
  anyrender_vello_cpu / anyrender_skia,README 欢迎 tiny-skia/femtovg 后端贡献。
- **vello 0.9.0**(2026-05-15,crates.io 核实)`Scene` API:
  `push_layer(clip_style, blend, alpha, transform, clip)` / `pop_layer` /
  `fill(style, transform, brush, brush_transform, shape)` / `stroke(...)` /
  `draw_image` / `draw_glyphs(&FontData) -> DrawGlyphs`(builder:字号、hint、
  变量字体坐标、逐字形 transform)/ **`draw_blurred_rounded_rect`**(box-shadow
  直接对应物)。vello_cpu 0.0.9(2026-05-30)。
- 文本:vello_api(sparse strips 新线)自己不管文本,字形绘制正在收敛到 **Glifo**
  (前身 parley_draw,已迁入 vello 仓库,做 outline 提取 + 彩色 emoji + glyph 缓存)。

**启示**:① 我们的 Painter 词汇表应当与 PaintScene/Scene 同构,让 vello 后端成为
1:1 转发;② 连 Linebender 自己都放弃"一个大一统 API",选择"薄抽象 + 多后端",
我们更没必要造大而全;③ anyrender 是现成的"别人替我们维护的后端集",留桥接口。

### 1.6 反例与旁证:Zed GPUI、Chromium viz、Leptos tachys

- **GPUI(Zed)**:每平台一个手写渲染后端(Metal/blade/DX),不做可切换抽象,
  深度绑定 Zed 自身需求;2026 年官方明确"community-facing GPUI work paused"
  (调研 05 已核实)。单渲染器绑死 = 换后端等于重写,是我们要避免的形态。
- **Chromium viz/cc**:paint op 显示列表(cc::PaintOpBuffer)使光栅化得以跨进程、
  跨 GPU/软件路径调度——浏览器级工程再次证明"命令列表是渲染架构的解耦单位"
  (⚠ 细节仅基于训练数据,作方向性旁证)。
- **Leptos tachys(关键对照,已核实)**:0.7 曾把 `Renderer` 做成**泛型参数
  贯穿所有 view 类型**以支持多渲染后端,结果"所需泛型数量导致灾难性编译时间与
  大型应用上的链接器错误",**0.7.0 发布前整个移除**。教训不是"别抽象",而是
  **别让后端类型参数化用户代码的类型系统**——用运行时多态(dyn/enum)收在
  边界内,代价只是每图元一次间接调用。

### 1.7 先例对比总表

| 框架 | 抽象形态 | 切换粒度 | 文本归属 | 主要代价 | 现状口碑 |
|---|---|---|---|---|---|
| Slint | 共享遍历 + ItemRenderer trait(dyn) | feature + env(SLINT_BACKEND)+ BackendSelector;启动时定死 | 各渲染器自管测量/光栅 | 软件渲染器功能滞后(无旋转/阴影/仅西文) | 生产可用,公司投入扛齐平税 |
| Iced | Renderer + Compositor,fallback enum(静态分发) | feature + env(ICED_BACKEND)+ **GPU 失败自动回退** | 共享文本栈 | 双后端都编入(体积);细微渲染差异 | 回退机制口碑好,0.14 仍双轨 |
| egui | tessellate 成网格,后端只画网格 | feature + NativeOptions::renderer | 上层(epaint 光栅进 atlas) | 丢矢量语义;双集成路径分叉 | 后端极薄,维护省力 |
| Flutter | DisplayList + 双 dispatcher | 引擎 flag,按平台灰度,最终移除 opt-out | 上层(text 库),引擎拿 DL | 四年双引擎并存的验证矩阵 | 迁移被普遍认为成功 |
| Masonry/Blitz | imaging / AnyRender trait | 后端 crate 选择 | glyph run 进 trait,Glifo 光栅 | 0.x API 波动 | 官方主推方向 |
| tachys | 泛型 Renderer 参数 | 编译期(泛型) | — | 编译时间/链接器爆炸 | **已移除**,反面教材 |

---

## 2. 设计提案:sv-shell 的 Painter 抽象

### 2.1 三种形态对比与裁决

结合本仓库现实:`render.rs` 已经是"**共享遍历 + 内联 tiny-skia 调用**"——layout
产出平铺 `Vec<Placed>`(父先子后),paint 循环逐节点解析样式(继承、组透明近似)
后直接调 `fill_rounded/stroke_rounded/draw_text`。要抽象的只是最后一步。

| | (a) trait 即时调用 | (b) 显示列表/命令缓冲 | (c) 场景树直读 |
|---|---|---|---|
| 形态 | paint 遍历调 `painter.fill(...)` | paint 产出 `Vec<PaintCmd>`,后端消费 | 每个后端自己遍历 Doc |
| 遍历/样式解析逻辑 | **一份**(现 render.rs 的循环) | 一份 | **N 份**(每后端复制继承/组透明/元素合成) |
| 对 vello Scene 的适配 | 1:1 转发(Scene 本身就是"被编码的显示列表",即时调用就是编码驱动) | 多一次中转:Cmd→Scene 再编码 | 直接编码,但遍历逻辑重复 |
| 金样测试 | 需要记录型实现 | 天然(列表即金样) | 只能像素级 |
| 局部重绘/缓存前景 | 需升级到 (b) | 天然(diff/缓存列表) | 每后端自造 |
| 额外拷贝/分配 | 零(参数借用) | 每帧一次命令 Vec(小) | 零 |
| 对应先例 | Slint ItemRenderer、anyrender PaintScene | Flutter DisplayList、Chromium cc | (无成功先例:等于放弃抽象) |

**裁决:(a) 为接口,(b) 作为 (a) 的一个实现免费获得,(c) 拒绝。**

- 我们的重绘模型是"版本号变更 → 整帧重画",没有(也暂不需要)保留式命令缓存;
  即时 trait 是最小惊讶、零迁移撕裂的选择——现 render.rs 的 paint 循环几乎原样
  保留,只把"画"的动词换成 trait 调用。
- 但 Flutter 的经验说明命令层价值巨大,所以**trait 的方法集设计成"可无损录制"**
  (所有参数可克隆成 owned 形式),提供 `RecordingPainter` 输出 `Vec<PaintCmd>`:
  金样测试(§3.3)、未来的脏区 diff、vello `append_scene` 子树缓存,全部从这里长出,
  不需要第二次架构变更。
- (c) 的诱惑是"vello 后端想要整棵树的信息做优化"——实际上 vello 的输入就是
  Scene 编码流,拿不到树也不需要树;而它的成本(每后端一份继承解析 + 组透明 +
  checkbox 合成)正是 Slint 用共享遍历避开、我们没理由再踩的。

### 2.2 trait 草案(对象安全,词汇对齐 vello/anyrender)

```rust
// v0:crates/sv-shell/src/paint.rs(M2 拆独立 crate sv-paint)
// 几何/颜色词汇直接用 kurbo + peniko(vello 的语言),不自造第三套。
pub use kurbo::{Affine, BezPath, Rect, RoundedRect, Stroke};
pub use peniko::{BlendMode, Brush, Color};

/// 形状小枚举:保持对象安全 + 给后端留矩形快速路径(vello 0.8 起有 rect fast path)
pub enum PaintShape<'a> {
    Rect(Rect),
    RRect(RoundedRect),        // kurbo 原生支持四角独立半径,C1 四角值免费
    Path(&'a BezPath),         // 未来矢量裁剪/图标
}

/// 定位好的字形:shaping 与行布局在上游已完成
#[derive(Clone, Copy)]
pub struct PlacedGlyph { pub id: u32, pub x: f32, pub y: f32 }

/// 字体身份 = 同一份字节,两端各自解析(fontdue::Font / vello FontData 都从 bytes 构造)
#[derive(Clone)]
pub struct FontHandle { pub data: std::sync::Arc<dyn AsRef<[u8]> + Send + Sync>, pub index: u32 }

pub struct GlyphRun<'a> {
    pub font: &'a FontHandle,
    pub size: f32,                 // 逻辑 px;物理缩放走 transform
    pub glyphs: &'a [PlacedGlyph],
}

#[derive(Clone, Copy, Default)]
pub struct PainterCaps {
    pub blur: bool,                // box-shadow/backdrop 模糊
    pub gradients: bool,
    pub arbitrary_transform: bool, // 旋转/斜切(CPU 端 v0 只保证平移缩放)
}

pub trait Painter {
    fn fill(&mut self, transform: Affine, brush: &Brush, shape: PaintShape<'_>);
    fn stroke(&mut self, transform: Affine, style: &Stroke, brush: &Brush, shape: PaintShape<'_>);
    fn glyph_run(&mut self, transform: Affine, brush: &Brush, run: &GlyphRun<'_>);
    /// 组不透明度/裁剪/混合的正确载体(替换现在的"alpha 沿祖先链相乘"近似)
    fn push_layer(&mut self, transform: Affine, alpha: f32, blend: BlendMode,
                  clip: Option<PaintShape<'_>>);
    fn pop_layer(&mut self);

    // —— M2+ 扩展面(带默认降级实现,配合 caps 协商;对应 CSS-SUPPORT ⏳ 项)——
    fn box_shadow(&mut self, _t: Affine, _rect: Rect, _color: Color,
                  _radius: f64, _std_dev: f64) {}
    fn image(&mut self, _t: Affine, _img: &ImageHandle) {}
    fn caps(&self) -> PainterCaps { PainterCaps::default() }
}
```

要点:

- **transform 作为逐调用参数**(vello/anyrender 同款),而非 push/pop 状态:
  scale 因子变成根 `Affine::scale(dpi)`,render.rs 里散落的 `* scale` 全部消失;
  未来 2D transform(CSS-SUPPORT ⏳)就是给子树换个 Affine,接口零改动。
- **Brush 用 peniko::Brush**:纯色今天就有,渐变(⏳)到位时接口不动,tiny-skia
  后端把 LinearGradient 转 tiny_skia::LinearGradient,vello 端直通。
- **对象安全**:全部方法可通过 `&mut dyn Painter` 调用;`PaintShape` 枚举替代
  `impl Shape` 泛型,既保住 dyn 又保住矩形快速路径。
- **kurbo/peniko 依赖税**:两者是纯 Rust 小 crate(无 GPU/系统依赖),tiny-skia
  后端反正要做类型转换,不因此变重;调研 05 已定调"对齐 Linebender 词汇而非自造
  第三套",此处落实。

**与 vello 0.9 Scene 的映射表**(证明 VelloPainter 是薄转发):

| Painter | vello Scene |
|---|---|
| `fill(t, brush, shape)` | `scene.fill(Fill::NonZero, t, brush, None, &shape)` |
| `stroke(t, style, brush, shape)` | `scene.stroke(style, t, brush, None, &shape)` |
| `glyph_run(t, brush, run)` | `scene.draw_glyphs(&font).font_size(run.size).transform(t).brush(brush).hint(true).draw(Fill::NonZero, glyphs)` |
| `push_layer(t, alpha, blend, clip)` | `scene.push_layer(Fill::NonZero, blend, alpha, t, &clip)` |
| `pop_layer()` | `scene.pop_layer()` |
| `box_shadow(...)` | `scene.draw_blurred_rounded_rect(t, rect, color, radius, std_dev)` |
| `image(...)` | `scene.draw_image(image, t)` |

anyrender `PaintScene` 与上表同构(多了 filter/backdrop_filter 与 glyph 变量字体
参数),因此可选做一个泛型桥 `impl<T: PaintScene> Painter for AnyRenderBridge<T>`
放在独立 feature crate 里——一次桥接白得 vello / vello_cpu / vello_hybrid / skia
四个后端,同时保留"上游 0.x 波动只打在桥 crate"的隔离。

### 2.3 文本字形路径(关键裁决)

三个候选切分点:

| painter 收到什么 | 谁做 shaping | 谁做光栅 | 判决 |
|---|---|---|---|
| **字符串**(draw_text(str)) | 每个后端 | 每个后端 | ❌ 文本栈 × N 份;Slint 走此路,其软件渲染器"仅西文"即是代价;与 M2 Parley 计划冲突 |
| **字形位图**(上游光栅好) | 上游 | 上游 | ❌ GPU 端灾难:位图上传丢 hinting/亚像素/缩放自由度,带宽浪费;vello 的 Glifo/glyph cache 全部用不上 |
| **glyph run**(id+位置+字号) | 上游(fontdue→Parley) | **后端**(fontdue 光栅 / vello draw_glyphs) | ✅ 与 vello `DrawGlyphs`、anyrender `draw_glyphs`、Glifo 的行业切分完全一致 |

**裁决:glyph run。**具体接线:

- 上游:`font.rs` 升级为 `text.rs` 文本引擎门面——
  `shape(&FontHandle, &str, px) -> ShapedRun { glyphs: Vec<PlacedGlyph>, w, h }`,
  现用 fontdue::layout 实现(`GlyphPosition.key.glyph_index: u16` → `id: u32`),
  M2 换 Parley 时**只有这个门面改**,painter 接口与后端全部不动。measure_text
  与绘制共用同一 shaping 结果(measure 缓存键 = 文本+字号,与今天一致)。
- CPU 后端:`fontdue::Font::rasterize_indexed(id as u16, px)` 得 coverage,
  复用现有 `blend_pixel`;按 (id, 量化后 px) 加一层小 LRU coverage 缓存
  (今天每帧重复光栅同字形,顺手的性能净赚)。
- vello 后端:`FontHandle.data` 构造 `FontData`(一次,缓存),`draw_glyphs`
  builder 直通,hinting/glyph cache 交给上游 Glifo 演进。
- 注意:**字号进 run、缩放进 transform**。CPU 端对 scale-only transform 可以
  "把 scale 乘进光栅字号"保证清晰度;遇到旋转(caps.arbitrary_transform=false)
  按能力协商降级(v0 直接不支持,与 Slint 软件渲染器同款约束,但我们在 caps
  里显式声明而非静默画错)。

### 2.4 层与组不透明度纠偏

现状 `effective_opacity`(alpha 沿祖先链相乘)在子元素重叠时是错的(重叠处双倍
上色)。Painter 落地时:遍历器对 `opacity < 1 且有子节点` 的节点发
`push_layer(alpha)`……子树……`pop_layer()`。tiny-skia 后端 v0 可以继续用
乘 alpha 近似实现 push_layer(行为不回退),vello 后端天然正确;两个后端共用
同一遍历器,差异被 caps/实现质量吸收,视觉一致性用 §3.3 的跨后端对比测试守住。
这同时是 border-radius overflow 裁剪与未来 backdrop 模糊的载体。

---

## 3. 切换机制

### 3.1 编译期 feature × 运行时探测的组合(三层,抄 Slint/iced 作业)

```toml
# crates/sv-shell/Cargo.toml
[features]
default = ["backend-cpu"]
backend-cpu   = ["dep:tiny-skia", "dep:softbuffer", "dep:fontdue"]
backend-vello = ["dep:vello", "dep:wgpu"]          # M2
# backend-vello-hybrid = [...]                     # M3(OHOS,GLES)
```

1. **feature 决定编进哪些后端**(体积/构建时间自己选;鸿蒙包只编 hybrid+cpu);
2. **运行时选择**:`SV_RENDERER=cpu|vello`(env 覆盖,调试/CI 用,对齐
   SLINT_BACKEND / ICED_BACKEND 习惯)→ 未设则按序探测:vello 后端做 wgpu
   `request_adapter` + device 创建,**任一步 Err 即静默落 CPU 后端**(iced
   fallback 同款,失败原因打 log);
3. **顶层 enum 静态分发**(iced `fallback::Renderer` 形态),绘制面 `&mut dyn Painter`:

```rust
enum ShellBackend {
    Cpu(CpuBackend),       // softbuffer surface + Pixmap + TinySkiaPainter
    #[cfg(feature = "backend-vello")]
    Vello(VelloBackend),   // wgpu surface + Scene + VelloPainter
}
impl ShellBackend {
    fn frame(&mut self, doc: &Doc, w: u32, h: u32, scale: f32) -> Vec<Placed> {
        // 各臂:layout_tree(共享)→ paint_tree(doc, &placed, &mut painter, root_affine)
        //       → 各自 present(softbuffer buffer / wgpu surface texture)
    }
}
```

关键切分(学 iced):**Painter(画)与 Present(上屏)是同一后端的两半,但对
shell 是一个 `ShellBackend` 单元**——因为 surface 类型(softbuffer vs wgpu)与
painter 类型强耦合,拆开反而漏抽象。回退只发生在窗口创建时(启动探测);运行中
GPU 死亡(device lost)v1 先按"重建同后端,再失败则下次启动走 CPU"处理,不做
热切换(Flutter/iced 也不做)。

### 3.2 测试策略:记录型 painter 金样 + 三档验证

1. **`RecordingPainter` 金样(主力,零像素、零平台差异)**:`PaintCmd` 是 trait
   调用的 owned 镜像(Glyph id 来自打包字体,跨平台确定)。现有
   `offscreen_click_roundtrip` 一类测试补一条:构建 counter 场景 → 录制 →
   与 checked-in 快照文本(RON/debug 格式)对比。样式/遍历/继承回归一网打尽,
   毫秒级,CI 免 GPU。
2. **CPU 后端像素金样(次级)**:现 `render_to_png` 保留;tiny-skia 确定性光栅,
   PNG 哈希即可。GPU 后端不做逐像素金样(驱动差异),改用——
3. **跨后端一致性(M2 起)**:同一 Doc 分别过 tiny-skia 与 vello_cpu(确定性),
   感知 diff(dssim 阈值)断言"两个世界画得足够像";vello GPU 与 vello_cpu 的
   一致性是上游职责(同一 imaging model),不重复测。

### 3.3 增量迁移路径(现有代码 → 第一个 backend,不推倒重写)

现 `render.rs` 400 行里,布局(measure/place/layout_tree,~190 行)完全不动;
绘制段重构为"遍历器 + 后端"两半。**每一步全绿可交付:**

| 步骤 | 动作 | 涉及文件 |
|---|---|---|
| 1 | 新建 `paint.rs`:Painter trait + PaintShape/GlyphRun/FontHandle + `RecordingPainter`/`PaintCmd`(+ kurbo/peniko 依赖) | `crates/sv-shell/src/paint.rs`(新) |
| 2 | `render.rs` 拆出 `paint_tree(inner, &placed, &mut dyn Painter, root: Affine)`:现 paint 循环原样搬入,`fill_rounded(..)` 改写为 `p.fill(t, &brush, RRect(..))`,checkbox 仍是两次 fill,文本改为"shape → glyph_run";`* scale` 全部收进 root Affine | `crates/sv-shell/src/render.rs` |
| 3 | 现 tiny-skia 私有函数(skia_color/fill_rounded/stroke_rounded/blend_pixel/draw_text 光栅内核)平移进 `TinySkiaPainter`(持 `&mut Pixmap`),文本部分改按 glyph id 光栅(`rasterize_indexed`) | `crates/sv-shell/src/tiny_skia_painter.rs`(新,≈纯搬运) |
| 4 | `render_frame` 签名不变:内部构造 TinySkiaPainter 调 paint_tree——**lib.rs、两个 example、全部现有测试零改动** | `render.rs` |
| 5 | `font.rs` → `text.rs` 门面:`shape()` 返回 PlacedGlyph 集;measure 与 paint 共用 | `crates/sv-shell/src/font.rs` |
| 6 | RecordingPainter 金样测试 1–2 条 | `crates/sv-shell/src/lib.rs` tests |
| 7 | (M2)`VelloPainter` + `ShellBackend` enum + 启动探测回退 + `SV_RENDERER`;组不透明度换 push_layer | 新 `vello_painter.rs`、`lib.rs` |
| 8 | (M2 末)评估 anyrender 桥,决定 vello_cpu 是否接替 tiny-skia 当兜底后端 | 独立小 crate |

步骤 1–6 估计 **2–4 人日**(核心是搬运而非新写),不阻塞任何 M1 工作,且立即
产出金样测试红利——这正是"第一个 backend = 现有代码改个壳"的证明。

---

## 4. 结论

### 4.1 可行性判决

**可行,低风险,建议按 §3.3 清单在 M1 尾声/M2 开头执行。**证据链:业界四个主流
框架(Slint/Iced/egui/Flutter)全部维持 2–4 个可切换后端多年;Linebender 官方把
"抽象层 + 多后端"定为 Masonry 主路线(imaging);我们的差异化(编译期定点更新)
发生在 sv-ui 场景树层,**编译器产物完全不感知 Painter**——渲染后端切换对上层
是纯实现细节,这是比以上任何先例都干净的边界。

### 4.2 "抽象税"评估(tachys 对照)

- **运行时税**:每帧图元数 ≈ 节点数 × 2–4(bg/border/text/…),几百节点 →
  低千级 `dyn` 虚调用,每次 1–2ns,合计个位数 µs;对照 CPU 光栅毫秒级、vello
  Scene 编码百 µs 级,**占比 <1%,不可测**。vello 后端里 trait 转发与手写编码
  生成完全相同的 Scene 字节,无二次代价。
- **编译期税**:零。tachys 的灾难来自 `Renderer` 作为泛型参数传染**每一个用户
  视图类型**,单体应用被迫 monomorphize 整棵视图树(官方结论:"灾难性编译时间
  与链接器错误",0.7 前移除)。我们的 dyn 收在 sv-shell 一个 crate 边界内,
  用户代码、sv-macro/sv-compiler 生成代码、sv-ui 均不含 Painter 类型参数——
  两者是"泛型污染用户类型系统" vs "边界内运行时多态"的本质区别,不可比。
  Slint 的 dyn ItemRenderer 在生产上跑了四个渲染器,是同形态的正面样本。
- **真实的税在功能齐平**(Slint 软件渲染器无旋转/阴影/仅西文为证):缓解 =
  后端家族收敛到 vello 系(classic/hybrid/cpu 同一 imaging model,齐平是上游
  职责)+ caps 显式能力协商(不静默画错)+ tiny-skia 后端定位为"过渡 + 金样
  基准",CSS ⏳ 特性(渐变/阴影/模糊)只承诺在 vello 系后端点亮,不给 tiny-skia
  补课。
- **净收益**:金样测试(立即)、GPU/CPU 双路(M2)、鸿蒙 GLES(M3,vello_hybrid
  换后端不动上层)、Flutter 式灰度切换与回滚能力(长期)。**判决:税极低,
  收益覆盖数倍,值得。**

### 4.3 开放问题

见结构化返回;最大悬置是 imaging vs anyrender vs 自有 trait 的长期站队(M2 复评,
观察 Masonry/imaging 与 anyrender 的收敛方向)与 vello_cpu 何时接替 tiny-skia。

---

## 5. 来源

- Slint 后端与渲染器(选择机制、软件渲染器限制):https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backends_and_renderers/ ;BackendSelector:https://docs.rs/slint/latest/x86_64-pc-windows-msvc/slint/struct.BackendSelector.html ;ItemRenderer 结构(二手):https://deepwiki.com/slint-ui/slint/3-rendering-systems
- iced fallback 渲染器与 Compositor:https://docs.iced.rs/iced_renderer/fallback/enum.Renderer.html 、https://docs.iced.rs/src/iced_renderer/lib.rs.html 、https://jl710.github.io/iced-guide/render_backend.html ;0.14.0:https://github.com/iced-rs/iced/releases/tag/0.14.0
- eframe 渲染器 feature 与选择:https://docs.rs/eframe/latest/eframe/ ;wgpu 默认化:https://github.com/emilk/egui/issues/5889 ;双路径分叉痛点:https://github.com/emilk/egui/issues/3079
- Flutter Impeller(DisplayList dispatcher、迁移):https://docs.flutter.dev/perf/impeller 、https://github.com/flutter/engine/blob/main/impeller/docs/faq.md 、DisplayList 起源 PR:https://github.com/flutter/engine/pull/26928 、现状综述:https://dev.to/eira-wexford/how-impeller-is-transforming-flutter-ui-rendering-in-2026-3dpd
- Linebender 2026 Q1(统一 API 受挫、AnyRender/imaging、Masonry GPU-optional、Glifo):https://linebender.org/blog/tmil-25/
- anyrender(PaintScene 签名、后端矩阵):https://github.com/dioxuslabs/anyrender 、https://docs.rs/anyrender/latest/anyrender/trait.PaintScene.html (0.11.1)
- vello 0.9.0 Scene API(draw_glyphs/draw_blurred_rounded_rect):https://docs.rs/vello/latest/vello/struct.Scene.html ;版本:https://crates.io/api/v1/crates/vello (0.9.0 = 2026-05-15)、https://crates.io/api/v1/crates/vello_cpu (0.0.9 = 2026-05-30)
- vello 字形/Glifo 方向:https://docs.rs/vello_api/latest/vello_api/ 、https://github.com/linebender/vello/issues/204
- Leptos tachys 泛型渲染器移除(编译时间教训):https://github.com/leptos-rs/leptos/issues/1830 、https://docs.rs/leptos/latest/leptos/tachys/index.html
- GPUI 社区维护暂停(调研 05 已核实):https://github.com/zed-industries/zed/tree/main/crates/gpui
