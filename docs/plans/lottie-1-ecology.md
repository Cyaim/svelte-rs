# Lottie 在 Rust 生态里的现状核实

> 生成:2026-07-22。方法:crates.io / docs.rs / GitHub API 逐项实查(版本号、
> 发布时间、依赖约束、许可证、最近 push 日期一律取自接口原始返回,不取转述);
> velato 源码经 GitHub Contents API 全文拉取逐行读;**并在 scratchpad 里真的把
> velato 0.11 接到 tiny-skia 0.11 上跑了一遍**(§6,含帧时间、命令覆盖率与
> 一个可复现的 panic)。查不到的写"未核实",见 §8。
>
> 问题:**如果 svelte-rs 现在要支持 lottie,最省力的路是什么;这条路的死穴是什么。**

## 0. TL;DR 判决

**最省力的路:`velato 0.11`(关掉 vello feature)+ 一个约 200 行的 `RenderSink →
Painter` 适配器 + `catch_unwind` 兜底。今天就能跑通,不需要 GPU,不需要新动词。**
实测:Tiger.json(419 KB / 14 图层 / 60fps)在纯 CPU 路径上 **256px 0.99 ms/帧**,
现有 `Painter` 能原样接住 **96.2%** 的渲染命令。

**死穴有两条,都不是"画得好不好看"的问题,是"能不能上生产"的问题:**

1. **velato 在合法 Lottie 输入上 panic,而不是返回 `Err`。** 实测可复现:把每个
   图层 transform 的 `r`(旋转)键删掉——这正是所有 Lottie 优化器的常规操作——
   `Composition::from_str` 直接 `panicked: not yet implemented: split rotation`
   (`converters.rs:213`)。它的 `Error` 枚举**只有** `Json(serde_json::Error)`
   一个变体,所有"不支持的特性"走的都是 `todo!()`/`unimplemented!()`(§1.4)。
   资产是设计师给的、可能被工具链改过,这等于把第三方数据接到进程存活性上。
2. **我们没有脏矩形。** 一个 lottie 在跑 = 每帧整窗重绘。上面的 0.99 ms 只是
   lottie 自己,窗口其余部分的重绘还没算。这条不该记在 lottie 账上(是全仓
   欠账),但 lottie 会是第一个把它逼出来的特性。

次级死穴:**文本层永远不会来**(velato 不支持,Lottie 规范 1.0.1 里根本没有文本
层类型,§3.2);**track matte 是最后那 4%**,而它恰好是"看起来不对"的那 4%(§6.3)。

分档结论(详见 §7):

| 档 | 内容 | 估算 |
| --- | --- | --- |
| **(a) 现在就能做** | `sv-lottie` crate:velato + PainterSink + panic 兜底;纯色填充/矩形裁剪/平凡图层;spinner、空状态插画、成功勾选 | 3–5 人日(高置信) |
| **(b) 先补基建** | `stroke_path` → `push_layer(alpha/blend)` → 路径裁剪 → 渐变笔刷 → 脏矩形 | 见 §7.2,逐项排序 |
| **(c) 不该做** | rlottie/dotlottie-rs/ThorVG 绑定;自研解析器;文本层与表达式;构建期转译 | —— |

---

## 1. velato 逐项核实

### 1.1 版本、活跃度、许可证

crates.io API(`/api/v1/crates/velato`)原始返回:

| 项 | 值 |
| --- | --- |
| 最新版本 | **0.11.0** |
| 发布时间 | **2026-07-21T12:29:44Z**(即昨天) |
| 首次发布 | 2024-03-26 |
| 许可证 | **Apache-2.0 OR MIT**(0.1.0 起每个版本都是) |
| 已发布版本数 | 14(0.1.1 被 yank) |
| 近期下载 | 2915 |

GitHub API(`/repos/linebender/velato`):`pushed_at` **2026-07-21**、152 star、
6 个 open issue、**未归档**、默认分支 main。最近 15 次提交横跨 2026-01 至
2026-07,2026-04-22 一天连落 5 个 PR(PolyStar 形状、`begin/end_layer_group`
钩子、隐藏图层 transform 修复、precomp `start_time` 修复、资产反序列化错误信息)。

**判读:活的,但节奏是"攒一批然后一天推完",不是持续投入。** 主力贡献者
`@RobertBrewitz` 与 `@nicoburns`(后者是 Blitz/Taffy 的人),Linebender 官方仓库
但显然不是 vello 团队的一线优先级。

### 1.2 vello 版本对应 —— **完美吻合,这是本次调研最重要的一条**

velato README 自带对应表(原文):

| velato | vello |
| --- | --- |
| **main, 0.11** | **0.9** |
| 0.10 | 0.7 |
| 0.9 | 0.7 |
| 0.7, 0.8 | 0.6 |
| 0.6 | 0.5 |

本仓库 `crates/sv-shell/Cargo.toml:27` 锁的是 `vello = { version = "0.9", optional = true }`。
**velato 0.11 就是为 vello 0.9 发的那一版,发布日期 2026-07-21,比本调研早一天。**

更关键的是依赖形态。crates.io API(`/crates/velato/0.11.0/dependencies`)原始返回:

```
kurbo       ^0.13.0   normal, required
peniko      ^0.6.0    normal, required
serde       ^1.0.228  normal, required (derive)
serde_json  ^1.0.149  normal, required
serde_repr  ^0.1.20   normal, required
vello       ^0.9.0    normal, OPTIONAL, default-features = false
vello       ^0.9.0    dev
```

feature 表(仓库 `Cargo.toml` 原文):

```toml
[features]
default = ["vello"]
vello = ["dep:vello"]
wgpu = ["vello", "vello/wgpu"]
```

**`vello` 是可选依赖。`default-features = false` 之后 velato 是一个纯解析 +
求值库,依赖只剩 kurbo/peniko/serde 三支。** 实测依赖树(§6.1)一共 15 个 crate,
零 wgpu、零 vello、零 GPU。

顺带一致性检查:我们的 vello 0.9 依赖 `peniko ^0.6.1`,velato 0.11 依赖
`peniko ^0.6.0` —— 同一 semver 兼容区间,`cargo tree` 实测解析出唯一的
`kurbo 0.13.1` / `peniko 0.6.1`(§6.1)。**没有版本撕裂。**

许可证 Apache-2.0 OR MIT,在本仓库 `deny.toml` 的 allowlist 内(该表已含
`MIT` 与 `Apache-2.0`);来源是 crates.io,满足 `[sources] allow-registry`。
**cargo-deny 门禁零阻力。**

MSRV:README 写 "verified to compile with **Rust 1.88** and later";
`Cargo.toml` 的 `rust-version = "1.88"`,edition 2024。

### 1.3 `RenderSink` —— 不必绑 vello 的那扇门

velato 0.9.0(2026-01-18)的 CHANGELOG 原文:

> The `RenderSink` trait has been reintroduced, making it possible to use `velato`
> with any rendering backend. The `vello` dependency is now optional. (#95 by @nicoburns)

`src/runtime/render.rs` 里的 trait 定义(逐字):

```rust
pub trait RenderSink {
    fn push_layer(&mut self, blend: impl Into<peniko::BlendMode>, alpha: f32,
                  transform: Affine, shape: &impl kurbo::Shape);
    fn push_clip_layer(&mut self, transform: Affine, shape: &impl kurbo::Shape);
    fn pop_layer(&mut self);
    fn draw(&mut self, stroke: Option<&fixed::Stroke>, transform: Affine,
            brush: &fixed::Brush, shape: &impl kurbo::Shape);

    /// Called before rendering a Lottie layer.
    fn begin_layer_group(&mut self, _name: &str, _index: usize) {}
    /// Called after rendering a Lottie layer.
    fn end_layer_group(&mut self) {}
}
```

其中(`src/runtime/model/fixed.rs`):`fixed::Brush = peniko::Brush`、
`fixed::Stroke = kurbo::Stroke`、`fixed::Transform = kurbo::Affine`。

驱动入口是 `Renderer::append(&comp, frame, transform, alpha, &mut sink)`;
`render_to_vello_scene` 只是它外面套了个 `Scene::new()`。

**trait 不是 dyn-compatible**(`impl Into<..>` / `&impl Shape` 参数)。这不构成
障碍:适配器本身是具体类型,内部持 `&mut dyn Painter` 即可——`velato_imaging`
就是这么写的(`ImagingSink<'a, S: PaintSink + ?Sized = dyn PaintSink + 'a>`),
泛型只在适配器边界内单态化,不上浮。与本仓库"`dyn` 只收在 sv-shell 边界内"的
纪律(paint.rs 头注释)完全相容。

**已有第二个 `RenderSink` 实现可抄**:`velato_imaging` 0.0.1(2026-05-21,
Apache-2.0 OR MIT,作者 waywardmonkeys = Bruce Mitchener,Linebender 的人,
仓库 `forest-rs/imaging`)。全文 ~200 行,把 velato 的单层栈翻译成 imaging 的
"隔离组 vs 非隔离裁剪"双栈,并且——值得学的一点——**把栈失衡收成
`Error::UnbalancedLayerStack` 而不是 panic**,`draw` 里一旦出错就整体转 no-op。
这份文件就是我们要写的东西的模板。

### 1.4 特性子集与缺口 —— README 说的 vs 源码里的实情

README 的 "Missing features" 原文:

- Position keyframe (`ti`, `to`) easing
- Time remapping (`tm`)
- **Text**
- **Image embedding**
- Advanced shapes (stroke dash, zig-zag, etc.)
- Advanced effects (motion blur, drop shadows, etc.)
- Correct color stop handling
- Split rotations
- Split positions

源码逐行核对之后,要给这张表打三处补丁:

**(1) 遮罩(mask)是实现了的,蒙版(matte)是半实现。** 别把两者混为一谈:

- `masksProperties` → `render.rs` 里 `for mask in &layer.masks { ... scene.push_clip_layer(transform, &self.mask_elements.as_slice()) }`,**走路径裁剪,能用**。
- track matte(`tt`/`tp`)→ `layer.mask_layer` 分支,先 `push_layer(Mix::Normal, 1.0, ..)` 把蒙版源图层画进去,再 `push_layer(matte_mode, 1.0, ..)`。源码里紧挨着的注释:

  > `// todo: re-enable masking when it is more understood (and/or if it's currently supported in vello?) Extra layer to isolate blending for the mask`

  也就是说 matte 依赖 blend 模式的正确合成,作者自己都标着不确定。

**(2) "Split positions" 已经实现了,README 过期。** `converters.rs` 的
`conv_transform` 里 `AnyTransformP::SplitPosition(SplitVector{x,y,..}) =>
Position::SplitValues((conv_scalar(x), conv_scalar(y)))`——实测(§6.4)split
position 的文件解析正常。但 `conv_shape_transform`(形状组的 transform)里仍是
`todo!("split position")`。**同一个概念两处实现,一处好一处炸。**

**(3) 所有"不支持"都不是错误,是 panic。** `src/error.rs` 全文只有:

```rust
pub enum Error { Json(serde_json::Error) }
```

而 `import/converters.rs` 里散着:

| 位置 | 内容 | 触发条件 |
| --- | --- | --- |
| `:103` | `unimplemented!("asset {:?} is not yet implemented", asset)` | 非 precomp 资产(**图片资产**) |
| `:211` | `todo!()` | `AnyTransformR::SplitRotation` |
| `:213` | `todo!("split rotation")` | **图层 transform 的 `r` 字段缺失(`None`)** |
| `:243` | `todo!("split rotation")` | 形状组的 split rotation |
| `:252` | `todo!("split position")` | 形状组的 split position |
| `:805/806` | `unimplemented!()` | blend 模式 `Add` / `HardMix` |

`:213` 那条是纯 bug:schema 把 rotation 声明成 `Option`,却在 `None` 分支
`todo!()`。**§6.4 实测复现:删掉 `r` 键 → 进程 panic。**

顺带一提代码成熟度信号:`converters.rs:109` 附近留着被注释掉的合并冲突标记
`// <<<<<<< HEAD`。能编译,但说明这份代码的打磨程度。

### 1.5 输入格式

`Composition::from_slice(impl AsRef<[u8]>)` / `from_json(serde_json::Value)` /
`FromStr`。**只吃 JSON,不吃 `.lottie`(dotLottie 是个 zip)。** 要支持 `.lottie`
得自己解 zip(`rasterlottie` 就是加了个 `zip` 可选 feature 做的)。

`Composition` 的公开字段:`frames: Range<f64>`、`frame_rate: f64`、
`width/height: usize`、`assets: HashMap<String, Vec<Layer>>`、`layers: Vec<Layer>`。
**帧率与帧区间是数据自带的**,不需要我们猜。

### 1.6 下游用户

crates.io 反向依赖只有三个:`bevy_vello` 0.13.1(velato `^0.9`)、
`velato_imaging` 0.0.1(`^0.10`)、`open-weather-wizard` 0.3.0(`^0.10`)。

**判读:velato 没有被大规模压测过。** 这与 vello 本体被 Blitz/Masonry/Bevy 共同
压测(ADR-3 的立论依据)是两回事——不要把对 vello 的信心平移到 velato 上。

---

## 2. 其它候选

逐个实查,按"能不能进本仓库"排序。

### 2.1 dotlottie-rs(LottieFiles 官方)—— GitHub 活跃,crates.io 死了

- GitHub `LottieFiles/dotlottie-rs`:`pushed_at` **2026-07-22**(今天)、273 star、
  MIT、19 个 open issue。描述:"High-performance Lottie & dotLottie player in Rust,
  with bindings for Android, iOS, Web (WASM), and C/C++"。
- crates.io:**只有 `0.1.0-alpha.1`,发布于 2024-09-18,此后两年未更新**,
  近期下载 183。

**渲染靠 ThorVG(C++)。** ThorVG(`thorvg/thorvg`,`pushed_at` 2026-07-22、
1704 star、MIT)自我描述是 "A production-ready **C++** vector graphics engine"。

判决:**出局**。它的定位是"给 Kotlin/Swift/WASM 发 FFI 二进制的播放器",不是
"给 Rust 程序做依赖的库"——crates.io 上两年不发版就是证据。加上 C++ 构建,
ADR-3 排除 skia-safe 的理由(构建重、拖累鸿蒙交叉编译)逐条命中。

### 2.2 rlottie / rlottie-sys(Samsung rlottie 的 Rust 绑定)

- crates.io:`rlottie` **0.5.4**、`rlottie-sys` **0.2.12**,都是 2026-03-07 发布;
  绑定层 MIT;仓库在 codeberg(`msrd0/rlottie-rs`);`rlottie` 总下载 33002
  (主要来自 Telegram 生态的 tgs 贴纸处理)。
- 上游 `Samsung/rlottie`:`pushed_at` 2026-07-22、1428 star、C++、
  GitHub license 字段 **NOASSERTION**。

三条硬伤:

1. **构建期联网下载并编译 C++。** `rlottie-sys` README 原文:默认 feature
   `vendor-samsung` —— "if rlottie cannot be found on the system, download
   Samsung's version of rlottie and compile it"。本仓库 `deny.toml` 里
   `[sources] unknown-git = "deny"`、`allow-registry` 只放 crates.io;
   sys crate 在 build.rs 里拉源码,cargo-deny **既看不见也管不住**。
2. **许可证是拼盘。** `Samsung/rlottie` 的 `COPYING` 原文:"rlottie basically
   comes with MIT license ... but some parts of shared code are covered by
   different licenses",随后列出 `src/vector/` → COPYING.SKIA、
   `src/vector/freetype` → **COPYING.FTL**、`pixman`、`stb`、`rapidjson`。
   FTL 不在我们的 allowlist 里,而且因为是 C++ 源码,cargo-deny 根本扫不到。
3. **API 形态不对。** docs.rs 的例子是"渲染到 `Surface`,然后遍历像素"——
   它给的是**一整块 RGBA 位图**,不是绘制命令流。我们的 `Painter` 连
   `draw_image` 动词都没有(§0 的既有事实核对),接它等于先补一个图片子系统,
   而调研 26 明确写着"图片子系统无排期"。

判决:**不该做**(§7.3)。

### 2.3 lottie-rs(zimond)—— 停更

GitHub `zimond/lottie-rs`:`pushed_at` **2024-05-29**、78 star、MIT。
crates.io 上对应的 `lottie` crate 停在 0.1.0(2024-05-05),近期下载 97。
默认播放器用 Bevy 渲染。**事实停更两年,出局。**

**踩坑提醒**:crates.io 上还有一个叫 `lottie-rs` 的包(0.2.17,2026-03-30,
仓库 `coignard/lottie`),描述是 "A simple yet powerful **Fountain screenplay
editor**" —— 同名不同物,搜索时别踩。

### 2.4 inlottie(mhfan)—— 后端已被 ADR-3b 判出局

GitHub `mhfan/inlottie`:`pushed_at` 2025-11-29、11 star、**GitHub license 字段为
null**(未声明许可证)。自述 "a simple and straightforward viewer/renderer for
Lottie animation implemented based on **femtovg**"。

femtovg 在 ADR-3b 的后端排序里因"能力天花板"已经出局;加上无许可证声明,
**不可用**。

### 2.5 rasterlottie(neodyland)—— 纯 Rust,但太早

crates.io:0.2.1(2026-04-24),MIT OR Apache-2.0,`tiny-skia 0.12` 后端,
纯 Rust 零 C++。GitHub `neodyland/rasterlottie`:`pushed_at` 2026-07-03、
**0 star**、创建于 2026-04-22。

README 自述现状(原文):

> - Parses a small but useful Lottie subset
> - Exposes a rendering API backed by `tiny-skia`
> - **Can render static rectangles, rounded rectangles, and ellipses into RGBA frames**

另外它的定位是 "headless ... for deterministic **server-side** rendering",
产物是 RGBA 帧 + GIF,不是命令流;tiny-skia 版本(0.12)也与本仓库(0.11)错位。

判决:**方向对(纯 Rust、明确报告不支持的特性而不是炸),但成熟度差两个数量级。
记入观察名单,现在不用。** 它有一点值得抄:内置一个 "deterministic support
analyzer",加载时先告诉你这个文件有哪些特性不支持——正是 velato 缺的那一层。

### 2.6 skia-rs-skottie

crates.io 0.3.0(2026-07-13),仓库 `pegasusheavy/skia-rs`。走 Skia/Skottie,
ADR-3 已排除 skia-safe,**同理出局**,不再展开。

### 2.7 汇总表

| 方案 | 最新版/日期 | 维护 | 后端 | 许可 | C/C++ | 产物形态 | 判决 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **velato** | 0.11.0 / 2026-07-21 | 活跃(阵发) | vello **或自带** | Apache-2.0 OR MIT | 无 | **绘制命令流** | **采纳** |
| dotlottie-rs | crates.io 0.1.0-alpha.1 / 2024-09 | GitHub 活跃、crate 死 | ThorVG | MIT(+ThorVG) | **是** | 位图 | 出局 |
| rlottie(-sys) | 0.5.4 / 2026-03-07 | 活跃 | Samsung rlottie | 拼盘(MIT+FTL+…) | **是,构建期下载** | 位图 | 出局 |
| lottie-rs | 0.1.0 / 2024-05 | **停更** | Bevy | MIT | 无 | — | 出局 |
| inlottie | — / 2025-11 | 低 | femtovg | **未声明** | 无 | — | 出局 |
| rasterlottie | 0.2.1 / 2026-04-24 | 新(0 star) | tiny-skia 0.12 | MIT OR Apache-2.0 | 无 | RGBA 帧 | 观察 |
| skia-rs-skottie | 0.3.0 / 2026-07-13 | 新 | Skia | — | **是** | 位图 | 出局 |

---

## 3. 格式本身

### 3.1 规范现状

Lottie 有正式规范,由 **LAC(Lottie Animation Community)** 维护——
lottie.github.io 原文:"a non-profit open source project hosted by **The Linux
Foundation**",在 Joint Development Foundation 之下。

版本(GitHub `lottie/lottie-spec` 的 releases/tags API 原始返回,**只有两个 tag**):

| tag | release 发布时间 | release 标题 |
| --- | --- | --- |
| **1.0.1** | 2025-06-26 | "v1.0.1 (April 2025)" |
| 1.0 | 2024-09-23 | "1.0" |

仓库 `pushed_at` 2026-06-23——**在动,但一年多没发新版**。

规范首页的自我定位(原文):

> The Lottie specification is still a work in progress, this document contains
> a **subset of features** that have been approved by the Lottie Animation Community.

**这句话是本节的重点:规范本身就是子集。** 因此"某实现不合规范"和"某文件用了
规范外特性"是两个独立的失败面,而现实中后者更常见——因为导出工具(After Effects
+ Bodymovin)导出的东西远超规范。

### 3.2 规范覆盖了什么

1.0.1 的 Layers 章节只定义 **5 种图层类型**:

| ty | 类型 |
| --- | --- |
| 0 | Precomposition |
| 1 | Solid |
| 2 | Image |
| 3 | Null |
| 4 | Shape |

**没有文本层类型。** effects 不在图层规范内。matte 通过图层上的 `tt`/`tp` 属性
表达,mask 通过 `masksProperties` 数组表达(两者都在规范内)。

**表达式(expressions)在规范之外。** LottieFiles 官方博客对 v1.0 的说法:
"A commonly used extension to the format outside of the spec is 'expressions'
which allow **code execution** ... Support for expressions and security
considerations for the same are **dependent on the renderer used**."

→ 表达式意味着在 UI 库里嵌一个 JS 求值器,并把任意代码执行面暴露给美术资产。
**永不做**(§7.3)。Qt 给 Lottie 后端加 `assumeTrustedSource` 属性就是在处理
这个问题面。

### 3.3 导出工具与典型体量

主流导出链是 **Adobe After Effects + Bodymovin**(velato 的示例 Tiger.json 里
`"v": "5.8.1"` 就是 Bodymovin 版本号);此外 LottieFiles、Lottielab 等在线
编辑器直接产出 JSON。

体量量级(LottieFiles 生态的经验建议,**非规范数据**,当量级参考):
图标 < 30 KB、UI 动画 < 100 KB、插画 < 300 KB、全屏动画 < 500 KB;
`.lottie`(JSON + 资产打成 zip)可再压 ~80%。

**实测的一个真实样本**(velato 官方示例,§6):

| 项 | 值 |
| --- | --- |
| Tiger.json 文件大小 | **428 543 B(419 KB)** |
| 画布 | 1024 × 1024 |
| 帧率 / 帧数 | 60 fps / 0–306 帧(5.1 秒) |
| 顶层图层 | 14(全是 ty=4 shape 层) |
| 其中 track matte 对 | **4 组**(`tt:1` + `td:1`) |
| 每帧发出的填充 | **≈ 49 次** |
| 每帧展平后的路径段 | **≈ 444 段** |

**判读:JSON 文件大 ≠ 每帧绘制量大。** 419 KB 里绝大部分是 306 帧的关键帧数据,
而任意单帧只有 49 个填充、444 段路径——比一个中等复杂度的 UI 页面还轻。
这直接决定了 §6.2 的性能结论。

---

## 4. 业界先例:非 Web 的原生 UI 框架怎么做 lottie

结论先行:**渲染后端是 Skia 的,白嫖 Skottie;渲染后端自研的,一律自绘。
没有第三条路。** 而且 Flutter 这个反例证明,即便后端是 Skia,也未必愿意用 Skottie。

### 4.1 Flutter —— 有 Skia 也不用 Skottie,纯 Dart 自绘

官方渠道推的是 pub.dev 的 `lottie` 包(`xvrh/lottie-flutter`),自述是
"a **pure Dart** implementation of a Lottie player",lottie-android 的移植,
画进 Flutter 自己的 Canvas。

关于 Skottie,flutter/flutter#118093(2023-01-06 开,已关闭)里 Flutter engine
的 jonahwilliams 原话:

> **We still have no plans to add Skottie to Flutter builds.**

**这是最贴近我们处境的先例:框架有自己的绘制层,于是 lottie 也进自己的绘制层,
不引入第二套渲染栈。** 我们要做的正是同一件事(velato → Painter)。

### 4.2 Qt 6 —— 双轨,且已把"构建期转译"当成上策

Qt Lottie Animation 模块在 Qt 6 **未弃用**,提供 QML 类型 `LottieAnimation`
(运行时解释)。但 Qt 6.10 起加了第二条路:`lottietoqml` 工具 +
CMake 的 `qt_target_qml_from_lottie`,**构建期把 Lottie 编译成 QML**。
Qt 官方博客("Animated Vector Graphics in Qt 6.10")称这条路
"scalable and hardware-accelerated",且"**more performant than using the
alternative `LottieAnimation` type**"。

同一篇博客也坦承覆盖度问题:

> there are animatable properties in both SVG and Lottie which we still do not
> support. For instance, animating the individual control points of curves ...

**对我们的启示见 §7.3 最后一条:构建期转译诱人,但 Qt 能这么干是因为 QML 已经有
一整套通用属性动画系统在下面接着;我们的 `anim.rs` 只有 opacity 和 scrollY
两个通道。** 先有通用属性动画机,才谈得上转译。

### 4.3 Avalonia(.NET)—— 后端是 Skia,所以是白捡

`Avalonia.Labs.Lottie` 的依赖是 `Avalonia.Skia` + `SkiaSharp` +
`SkiaSharp.Skottie`。Avalonia 的渲染后端本来就是 Skia,Skottie 是 Skia 自带的
Lottie 模块,**加个控件包装即可**。这条路对我们不存在——我们已经在 ADR-3 里
排除了 Skia。

### 4.4 Bevy —— 与我们同构

`bevy_vello` 0.13.1(2026-01-29)通过可选依赖 `velato ^0.9` 支持 Lottie。
Bevy 自绘 + vello 场景命令,和我们是同一套架构。**velato 目前唯一有规模的
下游就是它。**

### 4.5 Slint —— 没有

`slint-ui/slint#5549` "Support for rendering lottie animations",开于
**2024-07-04**,状态 **open**,最后更新 2025-06-23,4 条评论,标签
`a:runtime`/`api`。**两年了还是个 feature request。**

判读:Slint 有四套渲染器(ADR-3b 引用过),多后端恰恰让 lottie 更难落地——
每套渲染器都要接一遍。**这正是我们 `Painter` 抽象要面对的同一个问题**,
也是 §7.2 里"CPU 端明确降级、caps 报 false"这条裁决的由来。

### 4.6 Telegram —— rlottie 那 3.3 万下载量的出处

Telegram 的动画贴纸(`.tgs`)用 Samsung rlottie。这解释了 `rlottie` crate 的
下载量为什么远高于 velato:它服务的是"服务端/工具链批量转码",不是"UI 框架
里放个动画"。**下载量高不代表适合我们的场景。**

---

## 5. 鸿蒙(OHOS)

### 5.1 ArkUI 没有内置 lottie

官方路子是 OpenHarmony TPC 的第三方库:`@ohos/lottie`(仓库
gitee `openharmony-tpc/lottieArkTS`,也叫 lottie-ohos-ets)。核实到的事实:

- 定位:解析 Bodymovin 导出的 JSON 并在设备上本地渲染;
- **渲染载体是 ArkUI 的 `Canvas` / `OffscreenCanvas` 组件的
  `CanvasRenderingContext2D`** —— 本质是 lottie-web 的 ArkTS 移植;
- 许可 MIT(承自 Bodymovin);
- OpenHarmony 官方 docs 里有 `ts-components-canvas-lottie.md`,即它被文档化在
  **Canvas 章节之下**,不是一等 UI 组件;
- 版本 2.0.x(gitee 上见到 2.0.17-rc.1),声称支持 API 9+。

### 5.2 这对我们意味着什么

ADR-5 定的鸿蒙渲染路径是 **ArkTS 薄壳 → XComponent → OHNativeWindow →
EGL/GLES3**,渲染热路径零 NAPI 调用。**走这条路就拿不到 ArkUI 的 Canvas 组件**
——`@ohos/lottie` 对我们价值为零。

**这反而是好消息**:不存在"平台原生 vs 自绘"的分裂,鸿蒙上的 lottie 就是桌面
那份代码原样跑。不用维护第二条路径,不用做 parity 测试,不用为 ArkUI 版本
差异买单。**唯一的代价是它继承了 §7 里所有的自绘代价,一分不少。**

GPU 档现状(crates.io 实查):`vello_hybrid` **0.0.9**(2026-05-30)、
`vello_cpu` **0.0.9**(2026-05-30),均 Apache-2.0 OR MIT。**仍是 0.0.x**,
与 ADR-3b "OHOS 首选 vello_hybrid(仍 early,spike 当周复核)"的判断一致。
在它稳定之前,鸿蒙上的 lottie 只能走 CPU 光栅——而 §6.2 的实测说明
**这在图标尺寸上完全够用**。

---

## 6. 实弹验证(scratchpad,不入库)

光读文档不算数。在
`.../scratchpad/velato-probe/` 建了个独立 cargo 项目(**不在仓库内、不动
本仓库任何 Cargo.toml**),把 velato 真接了一遍。

### 6.1 依赖形态

`velato = { version = "0.11", default-features = false }` 的实际依赖树
(`cargo tree -e normal`,已剔除 proc-macro 分支):

```
velato v0.11.0
├── kurbo v0.13.1  (arrayvec, polycool, smallvec)
├── peniko v0.6.1  (color 0.3.3, kurbo, linebender_resource_handle, smallvec)
├── serde v1.0.229
├── serde_json v1.0.151
└── serde_repr v0.1.21
```

**零 wgpu、零 vello、零 GPU。** 全量编译(冷启动、release)15 个 crate,秒级。
kurbo/peniko 各解析出唯一版本,与我们 vello 0.9 用的是同一支。

### 6.2 性能:场景构建 + CPU 光栅

自己写了个 `RenderSink` 实现,直接打到 **tiny-skia 0.11**(与本仓库 CPU 后端
同版本)。约 80 行,处理 fill/stroke/路径裁剪(`Mask`),`push_layer` 的
blend/alpha 降级为忽略。样本是 velato 官方的 Tiger.json(§3.3)。

**场景构建**(velato 侧,不含光栅,120 帧扫描平均,release):

| 样本 | 解析(一次性) | 场景构建 |
| --- | --- | --- |
| Tiger.json(419 KB) | **19.3 ms** | **0.007 ms/帧** |
| PolyStarTest.json(2.6 KB) | 0.7 ms | 0.001 ms/帧 |

**CPU 光栅**(tiny-skia 0.11,60 帧扫描平均,含场景构建,release):

| 输出尺寸 | 帧时间 | 占 60fps 预算 |
| --- | --- | --- |
| 128 × 128 | **0.55 ms** | 3% |
| 256 × 256 | **0.99 ms** | 6% |
| 512 × 512 | **2.24 ms** | 13% |
| 1024 × 1024 | **6.18 ms** | 37% |

输出经肉眼验证正确(渲染出完整的老虎,`Tiger.json.256.png`)。

**判读:**

- **场景构建可以忽略不计**(7 µs)。velato 的成本几乎全在光栅,而光栅是我们
  自己的后端在付。源码里 `compute_transform` 每帧重走整条父链、作者注明
  "If it becomes a bottleneck, we can implement caching"——**实测远没到瓶颈**。
- **解析 19 ms 是一次性的,但不该在帧内做**。419 KB 的 JSON 解析要走
  `tasks` 或首帧之前。
- **图标尺寸的 lottie(≤256px)在纯 CPU 上就是免费的**,连 GPU 都不需要。
  这直接决定了 §7.1:(a) 档不依赖 `backend-vello`。
- 512px 以上开始吃预算,1024px 全屏动画在 CPU 上是 37% 预算——**能跑,但那
  是"启动画面"级别的场景,不是常驻 UI**。
- 一个可优化点:`Renderer::append` 每帧都会先 `push_clip_layer` 一个合成画布
  矩形,我的实现给它分配了一张 w×h 的 `Mask`(1024px 时 1 MB/帧)。
  **这个根裁剪是轴对齐矩形,应当特判走现有的 `push_clip`**,别落进路径裁剪。

### 6.3 命令覆盖率:现有 `Painter` 能接住多少

统计 60 帧里 velato 发出的每一条 `RenderSink` 调用,按"现有 `Painter`
(`fill_rounded_rect` / `stroke_rounded_rect` / `fill_path`(纯色)/
`push_clip`(矩形)/ `pop_clip` / `glyph_run`)能否原样接住"分类:

| 样本 | 可接 / 总数 | 覆盖率 | 缺口构成 |
| --- | --- | --- | --- |
| **Tiger.json** | 3102 / 3223 | **96.2%** | 全部 121 条是 `push_layer`(非平凡 blend/alpha) |
| PolyStarTest.json | 180 / 180 | **100%** | —— |

Tiger 的细分:纯色填充 2921、矩形裁剪 60、平凡图层 121(可忽略)、
**非平凡图层 121**;渐变填充 0、描边 0、路径裁剪 0。

那 121 条从哪来?查 JSON 原文:Tiger 有 **4 组 track matte**
(图层 1/5/8/12 带 `tt:1`,对应的蒙版源图层 0/4/7/11 带 `td:1`),
每帧 2 组生效 × 60 帧 ≈ 121。

**诚实的话要说清楚:我把 `push_layer` 忽略掉之后,渲染出来的老虎目视没有明显
缺陷(见 §6.2 的 PNG,那是接近动画末尾的一帧)。但 track matte 的语义确实被
丢弃了——蒙版源图层的形状被当作普通内容画了出去。这不是正确性证明,只能说明
"在这个样本上退化得不难看"。** 换一个把 matte 用在关键位置的文件,退化就会
是可见的错误而不是可接受的近似。

### 6.4 健壮性:一个可复现的 panic

拿 Tiger.json 做了三组变异(python 改 JSON,`ks` 字段):

| 变异 | 模拟的真实场景 | 结果 |
| --- | --- | --- |
| 删除每个图层 transform 的 `r` 键 | **Lottie 优化器剥掉默认字段**(TinyLottie 之类工具的标准操作) | ❌ **panic**:`not yet implemented: split rotation`,`converters.rs:213` |
| 改成 split rotation(`rx`/`ry`/`rz`) | AE 勾选 "Separate Dimensions" 导出旋转 | ❌ **panic**,同一行 |
| 改成 split position(`p: {s:true, x, y}`) | AE 勾选 "Separate Dimensions" 导出位置 | ✅ 正常解析并渲染 |

**这就是 §0 说的第一条死穴,已从"读源码推测"升级为"实测复现"。**
`Composition::from_str` 返回 `Result` 会让调用方以为错误是可恢复的——它不是。

缓解手段(已验证前提):本仓库根 `Cargo.toml` 的 `[profile.*]` **没有设
`panic = "abort"`**,所以 `std::panic::catch_unwind` 可用。velato 的解析是纯
函数式的(`Composition` 是纯数据,`Renderer` 只持一个可清空的 batch),
unwind 后没有跨边界的残留状态,catch 是安全的。

**但 catch_unwind 是创可贴,不是治疗**:panic hook 仍会打印栈、污染日志;
而且上表那六个 `todo!()`/`unimplemented!()` 全在**导入期**(`converters.rs`),
罩住 `from_str` 就能挡住它们——**渲染期(`Renderer::append`)也要罩,但理由
不同**:0.8.1 的 CHANGELOG 记着修过"a panic on WASM when finding roots"
(样条求根),说明求值路径同样有算术 panic 的历史。两处都罩,别只罩一处。

真正的治疗是给 velato 提 PR 把这些 `todo!()` 改成 `Error` 变体 ——
考虑到 `:213` 是个纯 bug(`Option` 的 `None` 分支写了 `todo!`),
这大概率是一个 20 行的 PR。**这件事的性价比高于本文档里其它任何一项。**

---

## 7. 裁决

### 7.0 先更正一条既有事实

任务书里写的"Painter 现在只有 fill_rounded_rect / stroke_rounded_rect /
glyph_run / push_clip / pop_clip / caps,**没有 fill_path**"——**已经过期了**。
本次调研期间读源码发现,commit `7966785`(`feat(adr-2 ②): 模板数据面落地
(Template/stamp)+ Painter::fill_path`)已经把它加进去了:

```rust
// crates/sv-shell/src/paint.rs:146
fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color);
```

配套的 `PathCmd`(MoveTo/LineTo/QuadTo/CubicTo/Close)与 `PathFill`
(NonZero/EvenOdd)是**自有轻量类型,刻意不借 kurbo**,理由写在 paint.rs 的
注释里:vello 是 optional dependency,让接口签名依赖只在某 feature 下存在的
类型 = 把 GPU 后端焊死进 CPU 路径。**这条裁决对 lottie 同样成立,后面
(b) 档的每一个新动词都必须遵守它**(尤其是渐变笔刷:别把 `peniko::Brush`
放进 `Painter` 签名)。

也就是说:调研 26 点名的"图标管线最大风险 = 缺 fill_path"这个前置**已经解除**,
lottie 与图标共享的地基已经打好。

### 7.1 (a) 现在就能做

**范围**:新建 `crates/sv-lottie`(独立 crate,不进 sv-ui —— 与 sv-arco 同一条
理由:sv-ui 是编译目标,不该绑定任何资产格式)。

```toml
velato = { version = "0.11", default-features = false }
```

内容三件:

1. **`PainterSink` 适配器**(~200 行,模板见 `velato_imaging`):
   具体类型持 `&mut dyn Painter`,实现 `velato::RenderSink`。
   - `draw(None, transform, Brush::Solid(c), shape)` → 把 `kurbo::PathEl`
     按 `transform` 变换后转成 `&[PathCmd]`,调 `Painter::fill_path`。
     (`transform` 我们自己烘进顶点,因为 `fill_path` 不吃 affine——反正
     路径本来就要展平一遍,这一步是免费的。)
   - `push_clip_layer` 若 shape 是轴对齐矩形且 transform 无旋转/斜切 →
     `Painter::push_clip`;否则退化为**其包围盒**的矩形裁剪(与 CPU 端
     圆角裁剪的矩形近似同一套降级哲学,调研 22 §2.3)。
   - `push_layer` 若 `alpha == 1.0 && blend == Normal` → 无操作;否则**忽略并
     计数**(caps 未来报 true 时改走真 layer)。
   - `draw(Some(stroke), ..)` 与 `Brush::Gradient` → 跳过并计数。
   - 学 `velato_imaging`:栈失衡收成 `Error` 不 panic;一旦出错整体转 no-op。
2. **panic 兜底**:`catch_unwind` 包住 `Composition::from_str` **和**
   每帧的 `Renderer::append`,失败降级为不画 + 一次性 `log::warn`(带文件名)。
   **这一条不是可选项**,§6.4 已证明。
3. **集成点**(三处小改,均在既有机制上):
   - `sv-ui`:`ElementKind` 加 `Lottie`(节点持 composition 句柄 + 当前帧);
   - `sv-ui/anim.rs`:`Channel` 加一档帧推进(现成的 `pump(now_ms)` + "有动画
     就继续排帧" 正是 lottie 需要的循环,`shell/lib.rs:220` 已经在每帧调);
   - `sv-shell/render.rs`:`paint_tree` 在该 kind 上调适配器。

**覆盖面**:纯色填充 + 轴对齐矩形裁剪 + 平凡图层。实测 Tiger 96.2%、
PolyStar 100%。**不需要 GPU**(§6.2:256px 0.99 ms/帧)。

**适用场景**——恰好命中调研 26 §3.1 里最缺动效的那几件:
Spin(加载转圈)、Skeleton、Result(成功/失败插画)、Empty(空状态)。

**代价**:3–5 人日(高置信,适配器有现成模板,集成点都是既有机制)。
外加一份运行时报告能力:加载时把"跳过的命令数按类型分桶"打出来,让使用者
知道这个文件退化了什么——抄 rasterlottie 的 support analyzer 思路(§2.5)。

### 7.2 (b) 需要先补基建(按性价比排序)

| # | 动词/能力 | 为什么 | CPU 端怎么办 | 顺带收益 |
| --- | --- | --- | --- | --- |
| 1 | **`stroke_path`** | Tiger 恰好零描边,但线条动画/图标 morph 是 lottie 的主流一半 | tiny-skia `stroke_path` 原生支持,**不违反 CPU 冻结政策**(与 fill_path 同理,是两栈共有能力) | **直接省掉调研 26 §4 的"usvg 描边转填充"**——arco 图标以 stroke 为主,有了这个动词就不用在构建期做轮廓化 |
| 2 | **`push_layer(alpha, blend)` / `pop_layer`** | track matte 的唯一出路,也是 §6.3 那 3.8% 缺口的全部 | vello 端一行 `Scene::push_layer`;**CPU 端建议明确降级为忽略,caps 报 `layers: false`** —— 要做就得离屏 Pixmap + `draw_pixmap(PixmapPaint{opacity, blend_mode})`,与 ADR-3b "CPU 栈能力冻结" 正面冲突 | 同一个动词能承接 CSS `opacity` 组合与未来的混合模式 |
| 3 | **路径裁剪 `push_clip_path`** | Lottie 的 `masksProperties`(velato 已实现,会真的发出来) | tiny-skia 要 `Mask`(w×h 字节/层,嵌套逐像素相乘)—— **正是调研 22 §2.3 拒绝过的东西**。CPU 端退化为包围盒矩形裁剪,vello 端精确 | 圆角裁剪的精确化(现在是矩形近似)可以搭这趟车 |
| 4 | **渐变笔刷** | `fill_path` 现在只吃 `Color`。Tiger 零渐变,但渐变在 lottie 里很常见 | tiny-skia `Shader::LinearGradient/RadialGradient` 原生支持 | CSS 渐变背景(CSS-SUPPORT 里的 ⏳ 项)。**务必自有轻量枚举,不要把 `peniko::Brush` 放进 `Painter` 签名**(§7.0) |
| 5 | **脏矩形 / 局部重绘** | 一个角落里 64px 的 spinner 逼着整窗每帧重画 | 全仓性改造,不该记在 lottie 账上 | 所有动画、光标闪烁、hover 反馈全都受益 |

**顺序建议**:1 → 5 → 2 → 4 → 3。把脏矩形提到第二位,是因为 (a) 档一落地,
"整窗重绘"就会立刻成为最大的实际开销,而它的收益远超 lottie 本身。

### 7.3 (c) 不该做

1. **rlottie / dotlottie-rs / ThorVG 绑定。** ADR-3 排除 skia-safe 的每一条理由
   在这里全部成立(C++ 构建重、拖累鸿蒙交叉编译),**还多两条**:
   rlottie-sys 默认 feature 在构建期**联网下载源码**(与 `deny.toml` 的
   `unknown-git = "deny"` 精神相悖,且 cargo-deny 扫不到 C++ 侧的
   FTL/SKIA/pixman/stb 许可拼盘);以及它们的产物是**像素面而非命令流**,
   接入需要先造一个我们完全没有的图片子系统。
2. **自研 lottie 解析/求值器。** velato 的 schema 目录有 27 个 shape 文件,
   外加动画属性/样条/缓动求值——几个人月起步,而规范还在动(LAC 的 roadmap
   明写 "Feature Expansion")。**给 velato 提 PR 修那几个 `todo!()`,
   比自研便宜两个数量级**(§6.4)。
3. **文本层与表达式。** 文本层:velato 不支持,规范 1.0.1 里根本没有这个图层
   类型(§3.2)——两头都没有,不存在"等上游"这条路。资产侧要求"文字转轮廓
   后再导出"是唯一理智的做法,写进文档。表达式:规范之外、需要嵌 JS 引擎、
   且是任意代码执行面(Qt 为此加了 `assumeTrustedSource`)——**永不做**。
4. **构建期把 lottie 转译成 `.sv` / `view!` 代码**(Qt 的 lottietoqml 路线)。
   看着最诱人(编译期展开、零解析开销、零 panic 面),但 **Qt 能这么干是因为
   QML 底下有一整套通用属性动画系统**;我们的 `anim.rs` 只有 opacity 和
   scrollY 两个通道,连缓动函数都只有一个 ease-out quad。要走这条路得先把
   动画系统做成通用属性动画机——那是另一个立项,不是"省力路"。
   **列为 R5 之后的可选题。**

### 7.4 一句话答复原问题

> **最省力的路** = velato 0.11(`default-features = false`,不带 vello)
> + 约 200 行 `PainterSink` 适配器 + `catch_unwind` 兜底。今天就能跑,
> CPU 后端够用,实测 96.2% 命令覆盖、256px 1 ms/帧、依赖只多 15 个纯 Rust crate。
>
> **死穴** = ① velato 在合法输入上 panic 而非报错(实测:删掉 `r` 键即炸),
> 资产来自设计师和优化器,这是进程存活性问题;② 我们没有脏矩形,一个 lottie
> 在跑就等于整窗每帧重绘。两者都与 lottie 的"画质"无关,都是能不能上生产的问题。
> ③ 长期看,文本层是永久缺口(velato 与规范都没有),track matte 是那 4% 的
> "看起来不对"。

---

## 8. 未核实清单

写清楚哪些没查到,别让读者以为都核过了。

1. **velato 在 GPU 端(本仓库 `backend-vello`)的实机帧时间**——未测。需要
   adapter 且本次不开窗。§6.2 的数字**只覆盖 CPU 路径**。
2. **`rlottie-sys` 在 Windows / OHOS 交叉编译下是否真能构建通过**——未试
   (判定其出局的理由是构建形态与许可,不依赖这一项)。
3. **`@ohos/lottie` 的确切版本、许可与发布时间**——ohpm.openharmony.cn 是 SPA,
   静态抓取只拿到页头;§5.1 的信息来自 gitee `openharmony-tpc/lottieArkTS`
   仓库页与检索结果,**未在包管理器上逐字核对**。
4. **Lottie 规范是否有 1.1 在途**——`lottie-spec` 仓库有 `/dev/` 文档路径
   (说明有开发中版本),但 releases/tags API 只有 `1.0` 与 `1.0.1` 两个 tag。
   **"1.1 何时发"未核实。**
5. **§3.3 的文件体量建议**(图标 <30KB 等)来自 LottieFiles 的博客与工具站,
   **不是规范数据**,只当量级参考。唯一实测样本是 Tiger.json 一个。
6. **track matte 被忽略后的真实视觉损失程度**——只在 Tiger 一个样本上目视
   检查过(§6.3 已声明这不是正确性证明)。要下结论需要一组带 matte 的样本
   做像素对拍。
7. **velato 上游对 panic 问题的态度**——未开 issue、未查历史 issue 里是否已有
   人提过。6 个 open issue 的内容未逐条读。

---

## 附:证据索引

**velato**
- crates.io API:<https://crates.io/api/v1/crates/velato>、`/0.11.0/dependencies`、`/reverse_dependencies`
- GitHub API:`/repos/linebender/velato`(pushed_at / license / stars)、`/commits`、`/contents/{README.md,CHANGELOG.md,Cargo.toml,src/lib.rs,src/error.rs,src/runtime/mod.rs,src/runtime/render.rs,src/runtime/vello.rs,src/runtime/model/mod.rs,src/runtime/model/fixed.rs,src/import/converters.rs}`
- docs.rs:<https://docs.rs/velato/0.11.0/velato/trait.RenderSink.html>、`struct.Composition.html`
- 仓库主页:<https://github.com/linebender/velato>

**其它候选**
- <https://crates.io/api/v1/crates/dotlottie-rs>、`/rlottie`、`/rlottie-sys`、`/rasterlottie`、`/velato_imaging`、`?q=lottie`
- GitHub API:`/repos/LottieFiles/dotlottie-rs`、`/repos/Samsung/rlottie`(含 `contents/COPYING`)、`/repos/thorvg/thorvg`、`/repos/zimond/lottie-rs`、`/repos/mhfan/inlottie`、`/repos/neodyland/rasterlottie`、`/repos/forest-rs/imaging`(含 `velato_imaging/src/lib.rs`、`imaging/src/paint.rs`)
- codeberg:<https://codeberg.org/msrd0/rlottie-rs>(`rlottie/README.md`、`rlottie-sys/README.md`)

**规范**
- <https://lottie.github.io/>、`/changelog/`、`/implementations/`
- <https://lottie.github.io/lottie-spec/1.0.1/>、`/specs/layers/`
- GitHub API:`/repos/lottie/lottie-spec`、`/releases`、`/tags`
- LottieFiles 博客:Lottie Specification v1.0 里程碑一文(表达式在规范外的表述)

**业界先例**
- Flutter:GitHub API `/repos/flutter/flutter/issues/118093`(+ comments,jonahwilliams 2023-01-06 原话);pub.dev `lottie` 包 / `xvrh/lottie-flutter`
- Qt:<https://doc.qt.io/qt-6/qtlottieanimation-index.html>、<https://www.qt.io/blog/animated-vector-graphics-in-qt-6.10>
- Avalonia:nuget.org `Avalonia.Labs.Lottie` 的依赖清单
- Bevy:crates.io `bevy_vello` 0.13.1 反向依赖记录
- Slint:GitHub API `/repos/slint-ui/slint/issues/5549`(state / created_at / updated_at)

**鸿蒙**
- gitee `openharmony-tpc/lottieArkTS`、`openharmony-tpc/lottie-ohos`
- gitee `openharmony/docs` 的 `.../arkui-ts/ts-components-canvas-lottie.md`
- crates.io API:`/vello_hybrid`、`/vello_cpu`

**本仓库**(逐行读,非转述)
- `crates/sv-shell/src/paint.rs`(`Painter` trait 全部动词、`PathCmd`/`PathFill` 的裁决注释、`PainterCaps`)
- `crates/sv-shell/src/vello_backend.rs`(`caps()`、`fill_path` 的 kurbo 转译)
- `crates/sv-shell/src/render.rs:664`(`paint_tree`)、`crates/sv-shell/src/lib.rs:220`(`anim::pump` 调用点)
- `crates/sv-ui/src/anim.rs`(`pump` / `Channel` / `active`)、`crates/sv-ui/src/lib.rs:313`(`ElementKind`)
- `Cargo.toml`(`[profile.*]` 无 `panic = "abort"`)、`deny.toml`(许可 allowlist / sources 策略)
- `docs/DESIGN.md` ADR-3 / ADR-3b / ADR-5 / ADR-6、`docs/research/26-arco-design-ui-kit.md` §3.2 §4

**实弹**:`<scratchpad>/velato-probe/`(3 个 bin:`velato-probe` 场景构建计时、
`raster` CPU 光栅计时 + PNG 输出、`gap` 命令覆盖率统计;`Tiger_no_r.json` /
`Tiger_split_r.json` / `Tiger_split_p.json` 三份变异样本)。**不入库。**
