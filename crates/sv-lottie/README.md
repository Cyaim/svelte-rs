# sv-lottie

把 [Lottie](https://lottiefiles.github.io/lottie-docs/) 矢量动画求值成
**sv-shell `Painter` 那套路径动词**(`fill_path` / `stroke_path` /
`push_clip` / `pop_clip`)。求值器是 [velato](https://crates.io/crates/velato)
0.11(Linebender,`Apache-2.0 OR MIT`)。

```rust
use sv_lottie::{Lottie, Placement, RecordingSink};

// 任意 lottie JSON;这里放一个 100×100 @ 60fps 的最小合法资产当占位
let json = r#"{"v":"5.9.0","fr":60,"ip":0,"op":60,"w":100,"h":100,"ddd":0,"layers":[]}"#;

let mut anim = Lottie::from_json_str(json).expect("合法 lottie");
let mut play = anim.playback();          // 循环、1x、从 0 开始

// 帧循环里:play.advance(dt_ms) → play.frame() → anim.render(..)
play.advance(16.7);
let place = anim.fit_contain(0.0, 0.0, 200.0, 200.0);   // object-fit: contain
let mut sink = RecordingSink::default();                // 换成你的 Painter 桥即可
let stats = anim.render(play.frame(), place, 1.0, &mut sink);
assert!(!stats.degraded(), "这一帧有降级:{stats:?}");
```

> 这段是**真跑的 doctest**(`src/lib.rs` 里有一个 `#[cfg(doctest)]` 的
> `#[doc = include_str!("../README.md")]`),`cargo test -p sv-lottie` 会带上它 ——
> 所以它编不过就是红的,不会烂在文档里。

---

## 为什么它不需要 GPU(这条反直觉结论是整个 crate 的立论)

"lottie = GPU 特性"是个直觉陷阱,而且它一度是本仓库路线图上的隐含前提
(「lottie 要等 vello 成为默认后端」)。**这个前提是错的。**

velato 0.11 把渲染出口抽成了一个**后端无关的 trait** `RenderSink`
(`src/runtime/render.rs:14`,四个必需方法:`push_layer` / `push_clip_layer` /
`pop_layer` / `draw`)。`vello::Scene` 只是它自带的**一个**实现 ——
`src/runtime/vello.rs` 整个文件 56 行。velato README 的首句就写着:
*"Render with the (optional) built-in Vello integration, or implement the
`RenderSink` trait to bring your own renderer."*

于是把 `default-features = false` 一关(velato 的 `default = ["vello"]`,
但 `vello` 是 `optional`),依赖树里**没有 vello、没有 wgpu、没有 naga**。
本次接入后 `cargo tree -p sv-lottie -e normal | grep -iE "vello|wgpu|naga"`
**零输出**(实跑),整棵树只有:

```text
sv-lottie ├── peniko 0.6.1 → color / kurbo 0.13.1 / linebender_resource_handle / smallvec
          ├── sv-ui  (slotmap + sv-reactive)
          └── velato 0.11.0 → kurbo / peniko / serde / serde_json / serde_repr
```

对 `Cargo.lock` 的净增量是 **5 个包**:`velato` / `serde_json` / `itoa` /
`zmij` / `sv-lottie` 自己。`kurbo 0.13.1` 与 `peniko 0.6.1` **早就在锁里**
(经 vello / parley 进来),velato 的 `^0.13` / `^0.6` 直接命中同一份解析结果。

道理其实很朴素:lottie 的**求值**(解析 JSON、按帧号插值出一堆带画刷的
贝塞尔路径)是纯 CPU 的算术,和用什么**光栅化**后端毫无关系。主线的
`Painter::fill_path` / `stroke_path`(tiny-skia / vello / Recording 三个后端
都已实现)正好就是这些路径的出口。**所以 CPU 默认后端可以原生吃下 lottie。**

副作用:`velato + vello_cpu` 也不冲突(lottie-3 spike §2.4 实验),
ADR-3b 那条"兜底从 tiny-skia 迁 vello_cpu"的退路不会被 lottie 堵死。

> **MSRV 零余量,值得盯一眼。** velato 0.11.0 声明 `rust-version = "1.88"`,
> 与仓库 MSRV **严丝合缝**(CI `msrv` job 也是 1.88.0)。velato 任何一个抬 MSRV
> 的补丁版都会把那条 job 直接弄红 —— 到时要么跟着抬仓库 MSRV,要么把
> velato 钉在 `=0.11.0`。依赖树里其他包都有余量(peniko 1.85、serde_json 1.71)。

---

## 量级数字

> ⚠️ **下面第一组数字全部引自 `docs/plans/lottie-3-spike.md`**(仓库外真跑过的
> spike,机器:i5-12400 / 31.7GB / Intel UHD 730 / Win11,`--release`)。
> **不是本 crate 测的**,口径也不同(那份 spike 用的是它自己写的 395 行
> tiny-skia sink,做了图层合成;本 crate 的降级更狠,不开图层 Pixmap)。
> 引它是为了给"lottie 到底多贵"一个数量级,不是本 crate 的性能承诺。

| 项 | 数字 | 出处 |
|---|---|---|
| 典型 UI 图标动画(Noto `1f602`,57 KB,15 个填充)@ 64×64,纯 CPU | **0.345 ms/帧** | lottie-3 §5.1 |
| 同上 @ 256×256 | 2.48 ms/帧 | lottie-3 §5.1 |
| Tiger(418 KB,14 层,复杂度上限样本)@ 1024×1024,纯 CPU | 29.0 ms/帧 | lottie-3 §5.1 |
| 同上,GPU(vello,含每帧同步回读) | 7.67 ms/帧 | lottie-3 §5.1 |
| `velato → vello::Scene` 编码(**与尺寸无关**) | 13–23 µs/帧 | lottie-3 §5.1 |
| 一个 57 KB 的 lottie 解析后常驻 | **≈1 MB** | lottie-3 §5.3 |
| 解析后常驻与 JSON 大小的相关性 | 弱(8 KB 与 132 KB 的资产都落在 0.7–1.1 MB) | lottie-3 §5.3 |

**本 crate 自己实测的一个数**(同机器,`--release`,固件 = 测试里那份手写
lottie:200×100、2 填充 + 1 描边、每帧 5 条 sink 命令;20000 帧取平均,
出口是 `RecordingSink`,**不含任何光栅化**):

```text
parse:                  0.40 – 0.60 ms   (2.1 KB JSON)
velato 求值 + 本适配器:  1.50 – 1.79 µs/帧
```

**给的是区间不是点值**:同一台机器、同一个二进制,跑次之间与热漂移能差
15–20%(初次交付报的是区间快端 0.40 ms / 1.50 µs,复核在同机重跑得到
0.60 ms / 1.78 µs —— 两头都是真的)。这个量级的数只用来回答"适配器是不是
成本中心",不该被当成基准线去卡回归。

读法:**适配器本身不是成本中心**。lottie 的钱花在光栅上,不在求值和翻译上
——这也是为什么 lottie-2 复核把「脏矩形 / 局部重绘」列成 lottie 的**前置条件**
(一个 lottie 在跑 = 整窗每帧重绘;`incremental-layout.md` §8.2 实测 30k 树的
**静止**帧就要 117ms)。**在有脏矩形之前,任何常驻动画在中大型界面上都不该开。**

---

## 已知缺口

### 1. 桥到 `sv_shell::Painter` 的最后一跳还没接上(**阻塞项**)

`sv-shell` 的 `mod paint` 是私有模块,而 `lib.rs` 的 `pub use paint::{...}`
只导出了 `Painter` / `PaintCmd` / `RecordingPainter` / `TinySkiaPainter`,
**没有导出 `PathCmd` / `PathFill` / `StrokeStyle` / `LineCap` / `LineJoin`**。
这五个类型于是"公开但不可命名":

```text
error[E0603]: module `paint` is private
  --> crates/sv-lottie/src/lib.rs
   |   use sv_shell::paint::PathCmd;
   |                 ^^^^^  ------- enum `PathCmd` is not publicly re-exported
```

外部 crate 既写不出它们的类型名、也构造不出它们的值,连
`Painter::fill_path` 都调不动。所以本 crate 自带一套**同名同形**的词汇
(`src/path.rs`),`PathSink` 的方法签名与 `Painter` 的路径动词逐字对齐。

**解法是 sv-shell 那边一行 re-export**(把那五个名字加进 `pub use paint::{...}`)。
补上之后,把下面这段原样存成 `crates/sv-lottie/src/shell.rs`、
在 `Cargo.toml` 加一个 `shell-painter = ["dep:sv-shell"]` 的 feature 即可。
**这段代码已经在一份逐字复制 sv-shell 公开路径 API 的模拟 crate 上编译通过**
(不是在真的 sv-shell 上 —— 因为它还编译不了):

```rust,ignore
use sv_lottie::{Color, LineCap, LineJoin, PathCmd, PathFill, PathSink, StrokeStyle};
use sv_shell::Painter;

/// 复用一个 scratch 缓冲:lottie 一帧几十次 draw,每次新建 Vec 就是
/// 几十次分配 × 每秒 60 帧
pub struct PainterBridge<'a> {
    painter: &'a mut dyn Painter,
    scratch: Vec<sv_shell::PathCmd>,
}

impl<'a> PainterBridge<'a> {
    pub fn new(painter: &'a mut dyn Painter) -> Self {
        Self { painter, scratch: Vec::new() }
    }

    fn stage(&mut self, path: &[PathCmd]) {
        self.scratch.clear();
        self.scratch.extend(path.iter().map(|c| match *c {
            PathCmd::MoveTo(x, y) => sv_shell::PathCmd::MoveTo(x, y),
            PathCmd::LineTo(x, y) => sv_shell::PathCmd::LineTo(x, y),
            PathCmd::QuadTo(cx, cy, x, y) => sv_shell::PathCmd::QuadTo(cx, cy, x, y),
            PathCmd::CubicTo(a, b, c2, d, x, y) => sv_shell::PathCmd::CubicTo(a, b, c2, d, x, y),
            PathCmd::Close => sv_shell::PathCmd::Close,
        }));
    }
}

impl PathSink for PainterBridge<'_> {
    fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color) {
        self.stage(path);
        let fill = match fill {
            PathFill::NonZero => sv_shell::PathFill::NonZero,
            PathFill::EvenOdd => sv_shell::PathFill::EvenOdd,
        };
        self.painter.fill_path(&self.scratch, fill, color);
    }

    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color) {
        self.stage(path);
        let style = sv_shell::StrokeStyle {
            width: style.width,
            cap: match style.cap {
                LineCap::Butt => sv_shell::LineCap::Butt,
                LineCap::Round => sv_shell::LineCap::Round,
                LineCap::Square => sv_shell::LineCap::Square,
            },
            join: match style.join {
                LineJoin::Miter => sv_shell::LineJoin::Miter,
                LineJoin::Round => sv_shell::LineJoin::Round,
                LineJoin::Bevel => sv_shell::LineJoin::Bevel,
            },
            miter_limit: style.miter_limit,
        };
        self.painter.stroke_path(&self.scratch, &style, color);
    }

    /// lottie 的裁剪永远是矩形(非矩形的在 sv-lottie 侧就被降级掉了),圆角恒 0
    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.painter.push_clip(x, y, w, h, 0.0);
    }

    fn pop_clip(&mut self) {
        self.painter.pop_clip();
    }
}
```

在那之前,任何实现了 `PathSink` 的类型都能直接消费本 crate ——
测试里就有一个 tiny-skia sink 作为样板(`tests/lottie.rs` 的 `PixmapSink`,
四个动词全实现,裁剪走 `tiny_skia::Mask`)。

### 2. `Painter` 缺的东西,以及不做会怎样降级

下面每一条都有 `RenderStats` 上的计数器,`stats.degraded()` 一次问完。
**每一条都有一条真的驱动过它的测试** —— 只断言计数器等于 0 是守不住
"降级可观测"这条属性的(那样只能证明"没触发",证不了"触发时会报")。

| `Painter` 缺什么 | velato 的哪个调用需要它 | 不做怎么降级 | 计数器 |
|---|---|---|---|
| **渐变画刷** | `draw(.., brush = Brush::Gradient(..), ..)` | 退化成**一种纯色**:色标沿 offset 的分段线性均值。lottie-2 §0 裁决 4 就是这么定的("v1 两个后端降级到完全一致:渐变取平均色"),换来"换后端画面不变"这条硬保证 | `gradient_fallbacks` |
| **任意路径裁剪** | `push_clip_layer(transform, shape)`,图层 `masksProperties` 走这条 | **整条裁剪忽略**(fail-open),被遮住的部分照常画出来 | `clips_ignored` |
| **带模式的图层遮罩** | 同上,但形状是轴对齐矩形 | 落到 `Painter::push_clip` 上,**一律按 intersect** —— 见下面「遮罩模式」那一段,`mode: "s"` 会画反 | `mask_clips_intersected` |
| **组透明度 / 隔离层 / 混合模式** | `push_layer(blend, alpha, ..)`(velato 只在**轨道遮罩**分支发,且 alpha 恒 1.0) | blend 丢弃、隔离层拍平;alpha 乘进后续颜色。**后果:带轨道遮罩(track matte)的资产会把遮罩图层本身当成可见内容画出来** —— velato 那边也还挂着 `todo: re-enable masking when it is more understood` | `layers_flattened` |
| **图像画刷** | `draw(.., Brush::Image, ..)` | 整条 draw 丢弃(velato README 本来也把"图片内嵌"列在不支持里) | `image_brushes_skipped` |
| **各向异性描边** | `draw(Some(stroke), transform, ..)`,transform 横竖缩放不同 | 宽度取 `√\|det A\|`(几何均值)。**最坏情况是零修正**,见下 | `stroke_width_approximated` |
| **两端不同的端帽** | `kurbo::Stroke` 有 `start_cap` + `end_cap`,`StrokeStyle` 只有一个 `cap` | 取 `start_cap`(**前向卫兵**,见 §2.1) | `stroke_cap_mismatch` |
| **虚线描边** | `kurbo::Stroke.dash_pattern` | 忽略(**前向卫兵**,见 §2.1) | `dashes_ignored` |

栈健康度另有两个:`unbalanced_pops`(多 pop)与 `unclosed_layers`(少 pop)。
后者更要紧 —— `push_clip_rect` 压的是**宿主 `Painter` 的**裁剪栈,漏一次
`pop_clip` 会让这一帧之后画的**所有**控件都被裁在 lottie 那个矩形里,而且是
窗口生命周期内永久的。所以 `PainterSink::finish()` 与它的 `Drop` 都会把欠下的
`pop_clip` 补发出去(`Drop` 那条是为 `append` 中途 panic 展开准备的)。

**遮罩模式:一条方向可能反的降级。** velato 的 `runtime::model::Mask` 是带
`mode`(Add / Subtract / Intersect / …)和 `opacity` 的,但
`runtime/render.rs:129-133` 只发一个裸的 `push_clip_layer`,两者都没传下来
(velato 自带的 vello 后端有**同一个**洞,`runtime/vello.rs:20`)。到适配器
这一层已经无从分辨,只能一律按 intersect 落地:

- `mode: "a"`(lottie 缺省、绝大多数资产)→ intersect 就是**正解**;
- `mode: "s"` / `inv: true` → 画面**正好反了**(该藏的露出来、该露的被裁掉)。

这是 fail-closed 且方向相反,比 `clips_ignored` 那种 fail-open 难查得多,所以
它单开了 `mask_clips_intersected`:**非零 = 这一帧有图层遮罩,正确性取决于
资产用的是哪种 mode**。真正修好需要上游把 `mode` 透到 `RenderSink`。

另外两条不是 `Painter` 的锅,是烘焙变换的代价:

- **各向异性缩放下描边宽度是近似**。`Painter` 的路径动词不收 transform
  (坐标 = 物理像素是它的全局不变量),所以变换烘焙进坐标后描边宽度要手工补
  `width × √|det A|` —— 各向同性缩放下**精确**。各向异性下它是两个奇异值的
  几何均值,**最坏情况不是"近似"是"零修正"**:squash-and-stretch(lottie
  最常见的手法之一,例如图层 `s: [400%, 25%]`)满足 `det = 1`,补偿因子恰好
  1.0,而正解是横向 4× / 纵向 0.25×。判据是两个奇异值之比 > 5%,超了就
  bump `stroke_width_approximated`。升级路径是给两个路径动词加 `transform` 参数。
- **曲线展平容差是我们选的自由参数**,不是给定的。默认 `0.1` 物理像素
  (lottie-3 spike 用的值),`render_with_tolerance` 可调(非法值会被消毒回默认)。
  实际影响比想象小:velato 交给 sink 的绝大多数形状已经是 `BezPath`,
  `path_elements` 原样吐出。

### 2.1 计数器**盖不住**的降级(诚实清单)

"每条降级都有计数器"这句话只对**本适配器这一层**成立。velato 的导入器会在
`RenderSink` 之前就丢掉一些东西,那些丢失在这个口径上不可观测,
`stats.degraded()` 返回 `false` 也可能已经画得不对了:

- **虚线描边**。`schema/shapes/base_stroke.rs:36` 是解析 `d` 的,但
  `runtime/model/animated.rs:414` 构造描边时只写
  `kurbo::Stroke::new(w).with_caps(cap).with_join(join)` —— `dash_pattern` 恒为空
  (`grep -rn dash_pattern velato-0.11.0/src` **零命中**)。也就是说虚线在
  import 层就没了,适配器只会看到实线,`dashes_ignored` **在 velato 0.11 上
  恒为 0**。实跑确认过:给 `"st"` 挂 `"d": [{"n":"d","v":{"a":0,"k":8}}]`,
  画出来是实线 4px,计数器为 0。
- **两端不同的端帽**:同上,`with_caps` 把 start/end 设成同一个值,
  `stroke_cap_mismatch` 同样恒为 0。
- **遮罩的 mode 与 opacity**:见上面「遮罩模式」。`mask_clips_intersected`
  能告诉你"这一帧有遮罩、方向不可信",但**告诉不了你它是不是真的反了**。

前两个计数器保留是当**前向卫兵**用的:上游哪天开始发 dash / 不同端帽,降级会
立刻变得可观测而不是静默画错。这条承诺本身有测试钉着
(`dash_guard_would_fire_if_upstream_ever_emitted_one` /
`cap_mismatch_guard_would_fire_if_upstream_ever_emitted_one`,直接手搓一个带
dash 的 `kurbo::Stroke` 喂进去)。

### 3. 上游(velato)本身的缺口

velato 自述的不支持清单(README 原文):位置关键帧缓动(`ti`/`to`)、
时间重映射(`tm`)、**文本**、**图片内嵌**、高级形状(dash / zig-zag)、
高级特效(运动模糊 / 投影)、**色标处理**、拆分旋转、拆分位置。
**这是我们支持子集的上界** —— 补它等于接管半个 lottie 运行时。

更要紧的是健壮性:**velato 在合法 Lottie 上 panic 而不是返回 `Err`**。
逐个数过(velato 0.11.0 源码,`#[cfg(test)]` 里的不算):

| 位置 | 宏 | 触发条件 |
|---|---|---|
| `import/converters.rs:211` `:213` `:243` `:252` | `todo!()` | 拆分旋转 / 拆分位置 |
| `import/converters.rs:103` | `unimplemented!()` | 未支持的资产类型 |
| `import/converters.rs:805` `:806` | `unimplemented!()` | `Add` / `HardMix` 混合模式 |

**共 7 处,全在导入器**(即解析期,正好是 `catch_unwind` 覆盖的窗口)。
其中 `:213` 是纯 bug:schema 把图层 transform 的旋转 `r` 声明成 `Option`
(Lottie 规范里它本来就可省),转换器却在 `None` 分支写
`todo!("split rotation")` —— 一个没有 `r` 键的合法图层会**崩进程**。

本 crate 在 `Lottie::from_slice` 里用 `catch_unwind` 把它接住,转成
`Error::Unsupported`(仓库没有开 `panic = "abort"`,这条路可用;有回归测试
`unsupported_lottie_reports_error_instead_of_panicking` 守着)。
**这是创可贴不是治疗**:根治是给 velato 提 PR 把那 7 处换成 `Error` 变体
(lottie-1 §6.4 估过,20 行的 PR)。

**渲染期(`Renderer::append`)没有 `catch_unwind`** —— 那是每帧都要跑的热路径,
不该为它付 unwind 屏障的代价。读过 velato 0.11 `runtime/` 里全部两处
`unwrap` 之后:**两处都被紧邻的上一行守住**,读下来不可达 ——
`model/animated.rs:257` 的 `vertices.last().unwrap()` 在 `:236` 的
`if vertices.is_empty() { return; }` 之后 22 行(同一函数、同一绑定),
`render.rs:306` 的 `self.geometries.last_mut().unwrap()` 由同一个 `if` 的
`self.drawn_geometry < self.geometries.len()` 条件保证。也就是说 0.11 上没找到
可达的渲染期 panic,但这不是上游的 API 承诺,真炸了只能靠调用方兜。
渲染期真正被覆盖的是**裁剪栈污染**:`PainterSink` 的 `Drop` 会在展开时
补发欠下的 `pop_clip`(见 §2 末尾)。

> 顺带澄清一条容易数错的:`grep -rn 'panic!' velato-0.11.0/src` 会在
> `schema/` 下命中 6 个 `panic!("{e}")`,但它们**全部位于 `#[cfg(test)]` 模块内**
> (velato 自己的单测),不进我们的编译产物,也就不会把畸形 JSON 从
> `Error::Parse` 抖到 `Error::Unsupported`。

### 4. 没做、且明确不在本 crate 范围内的事

- **不接帧循环。** `Timeline` / `Playback` 只做算术(总时长 / 帧率 /
  wall-clock → 帧号 / 循环)。真正接进 sv-shell 的 `anim::pump` 要新增一条
  `Channel::Media`、要实现"每帧写但**不 bump 版本**"、要同步修 vello 那条
  `render_cached(.., unchanged)` 短路(否则 GPU 后端上 lottie 会停在第一帧)
  —— 全是 sv-shell 的改动,见 lottie-2 §4。
- **不进场景树。** lottie 节点长什么样(`View` + `paint_source` 槽 + 壳侧资源
  注册表)是 lottie-2 §3 的裁决,要动 sv-ui + render.rs。
- **不做 a11y / reduced-motion。** lottie-2 §0 裁决 5:有 `aria-label` →
  `Role::Image`,无 label → 装饰性不进语义树;自动播放且循环的必须可暂停
  (WCAG 2.2.2 A 级)。这些都长在场景树上。
- **没有双后端逐像素 parity 测试。** lottie-3 复核实测:ADR-3b 那条"非白像素比"
  指标对渐变整体偏色**完全不敏感**(一次改了 4140 个像素的优化,该指标一个数
  都没动)。真要在 CI 守 lottie 的双后端一致性,验收项得加逐像素的
  (平均 |Δ| 与 Δ>N 的像素占比)。本 crate 只有单后端(tiny-skia)的像素测试。

---

## 测试

```sh
cargo test -p sv-lottie          # 22 单测 + 34 集成 + 1 doctest = 57 绿
                                 # (另有 1 条 doctest 标了 ignore:§1 那段桥接
                                 #  代码依赖 sv-shell 尚未 re-export 的类型)
```

固件全部是**代码内嵌的手写 lottie**,不下载、不往仓库塞第三方资产
——lottie-3 §1 复核提醒过,那批 Noto 动画 emoji 是 **CC BY 4.0(要求署名)**,
vendor 进来要在 `assets/LICENSE` 写明出处。基线固件(`FIXTURE`,2.1 KB):
一个圆角矩形从左移到右 + 一个从红变绿的圆;另外五个小固件专门驱动降级路径
(矩形 subtract 遮罩 / 三角形遮罩 / 渐变填充 / 轨道遮罩 / 各向异性描边)。

**主路径**(`tests/lottie.rs`):解析事实 / 时间轴环绕与钳制 / **半开区间上界**
(返回正好 `end_frame` 会让整帧一个图层都画不出来,velato 用 `Range::contains`
判图层活跃;沿"时间"和"时间轴量级"两个轴各扫一遍)/ 播放态掉帧不丢相位 /
倒放与 `speed` / t=0 与 t=末尾的绘制命令必须不同 / 命令流确定性 /
根裁剪转矩形裁剪且栈平衡 / 描边样式往返 / `fit_contain` 的平移与缩放真的落到
命令流上 / `render` 的 `alpha` 透传与钳制 / 容差消毒 / sink 经 `&mut` 与
`dyn` 转发后命令流不变 / 合法但 velato 不支持的输入报错而不是 panic /
`Error` 的 `Display` 与 `Lottie` 的 `Debug`。

**降级路径**(这是 §2 那张表的证据,每一条都真的跑过一次):矩形遮罩被记成
`mask_clips_intersected` / 三角形遮罩 fail-open 且不发多余的 `pop_clip` /
渐变拍成 `(128,0,128)` / 轨道遮罩把遮罩图层本身画了出来 / 各向异性描边宽度
零修正被报出来 / `alpha=0` 全跳过 / 图像画刷丢弃整条 draw / 空路径被计数 /
多 pop 只记账不转发、少 pop 在 `finish` 与 `Drop` 里都会补发 `pop_clip`。

**私有函数单测**(`src/sink.rs` 的 `mod tests`):渐变分段线性均值(含退化色标
表)/ `axis_aligned_rect` 的接受与拒绝(含**两条不相连的竖线**不能被当成矩形)/
仿射奇异值 / 两个前向卫兵计数器。

**像素**:落到 200×100 Pixmap 上,首末帧**矩形填充色**的最左列 20 → 140
(按颜色挑,不是"最左非背景列"——后者在末帧挑到的是**圆**的左边缘 85,
结论碰巧成立但机制不对);外加一条对照测试证明 `PathSink` 的裁剪真的削掉了
像素(实现裁剪 ≈10000 px vs 不实现 20000 px)。
