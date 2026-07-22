# Lottie 在 Rust 生态里的现状核实

> ⚠ **读之前先读文末的「## 9. 复核记录」**。对抗性复核(2026-07-22,基线 `50a700c`)
> **独立复现了 §6 的全部关键数字**(覆盖率 3102/3223 逐位相同),但推翻/修正了
> **一条既有事实**(§7.2 第 1 项 `stroke_path` 已落地,原文重犯了它自己在 §7.0
> 纠正过的错)、**一条架构硬冲突**(§7.1 的 `sv-lottie` 独立 crate **构不出来**,
> 循环依赖)、**一条被漏掉的静默错渲**(velato 丢弃 fill-rule),并补了
> **一条被漏掉的 20/80 替代**(构建期烘焙,实测 51 KB)。正文里已用【复核修正】标出。

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

> 【复核修正】覆盖率 **96.2%(3102/3223)复核逐位复现**,可信。但两处要改口:
> ① 帧时间偏悲观(原文自己的 sink 每帧给根裁剪分配了整张 `Mask`),
> 独立实现实测 **256px 0.64 ms、1024px 2.34 ms**(原文 0.99 / 6.18),见 §6.2;
> ② "不需要新动词"要加限定——**能接住 ≠ 画得对**,velato 把 Lottie 的 fill-rule
> 直接丢了(§1.4 补丁 4),`r:2`(even-odd)的图形会被静默填成实心。

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

> 【复核修正】还有**第三条死穴,原文完全没写**:**velato 静默丢弃 fill-rule**。
> `schema/shapes/fill.rs:25` 老老实实把 `"r"` 解析进 `FillRule`,而
> `src/import/` 里**一次都没读过它**(`grep -c rule src/import/` = 0),
> 运行时模型没有这个字段,`RenderSink::draw` 也没有这个参数。
> 后果:任何用 even-odd 的图形(圆环、字母 O、回字形——`paint.rs` 自己的测试注释
> 管这叫"图标渲染最经典的一处坑")会被填成**实心块**。
> 这比 matte 更糟:matte 丢了是"少一层遮罩",fill-rule 丢了是**静默画错且无从察觉**,
> 而且它**被算进了那 96.2% 的"可接住"里**——命令确实接住了,只是画错了。

分档结论(详见 §7):

| 档 | 内容 | 估算 |
| --- | --- | --- |
| **(a) 现在就能做** | `sv-lottie` crate:velato + PainterSink + panic 兜底;纯色填充/矩形裁剪/平凡图层;spinner、空状态插画、成功勾选 | ~~3–5 人日(高置信)~~ 【复核修正】**6–9 人日**,且 crate 形态要改(§7.1 复核框) |
| **(a′) 更省力的一档** | 【复核补充】**构建期烘焙**:build.rs 里跑 velato,把逐帧路径命令编成静态表,运行时零 velato / 零 JSON / 零 panic。实测 PolyStar 级资产 **51 KB**、Tiger 级 2.72 MB(§7.1b) | 2–3 人日 |
| **(b) 先补基建** | ~~`stroke_path` →~~ 【复核修正:已落地,commit `3ebe81c`】`push_layer(mix+compose)` → 路径裁剪 → 渐变笔刷 → 脏矩形 | 见 §7.2,逐项排序 |
| **(c) 不该做** | rlottie/dotlottie-rs/ThorVG 绑定;自研解析器;文本层与表达式;构建期**转译成 `.sv`**(≠ 上面的烘焙) | —— |

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

> 【复核修正】三条更正,都会影响 §7.1 的工期置信度:
> 1. **它依赖 `velato = "0.10.0"`**(`Cargo.toml.orig` 原文,`default-features = false`),
>    不是 0.11。0.x 的 minor 是 breaking,**它现在编不过 0.11**——可以照抄,
>    但不能当作"已被 0.11 验证过"的现成件。
> 2. **不是 ~200 行,是 331 行**(`src/lib.rs`,其中 248 行起是测试;
>    非空非注释 262 行)。实现体 ~247 行。
> 3. **最关键的一条:它从不降级。** `imaging` 有完整的组/合成/裁剪模型,
>    所以 `push_layer` 一对一映射成 `push_group(GroupRef::with_composite(...))`。
>    我们的 `Painter` 没有层——**"接不住时怎么办"恰恰是这份模板一个字都没演示的部分**,
>    而那正是我们全部的工作量所在。
>
> 但它有一条**必须照抄**、原文 §7.1 却漏掉的机制:`layer_stack: Vec<LayerKind>`。
> velato 的 `pop_layer` 是**按数量配平**的
> (`render.rs:169`:`for _ in 0..layer.masks.len() + (layer.mask_layer.is_some() as usize * 2)`),
> 一个 `push_*` 必配一个 `pop_layer`。**§7.1 写的"push_layer 忽略并计数"会直接把栈搞乱**——
> 忽略了 push 却照单执行 pop,弹掉的是别人压进去的裁剪。
> 正确做法是 velato_imaging 那样压一枚标记(`Clip` / `Group` / 我们还要多一个 `Elided`),
> `pop_layer` 按标记决定弹不弹。这是"200 行"里最容易写错的 5 行。

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

> 【复核确认 + 补三条】上表的六个行号**逐条核对通过**,但要注意口径:
> 它们对的是 **crates.io 上 `velato-0.11.0.crate` 的 `converters.rs`(1105 行)**;
> GitHub `main` 已经漂到 108/110/147/148/1045/1046(1084 行)。
> 引用行号时必须钉版本,否则半年后没人对得上。另,`:247`
> (`conv_shape_transform` 的 `None => &FLOAT_VALUE_ZERO`)**优雅地处理了缺失旋转**,
> 而 `:213` 同一个概念直接 `todo!()`——**这是 `:213` 是纯 bug 的铁证**,值得写进给上游的 PR 描述。
>
> **补丁 4:fill-rule 被静默丢弃(原文完全没提,后果比 matte 严重)。**
> `schema/shapes/fill.rs:25` 有 `pub fill_rule: Option<FillRule>`(`#[serde(rename = "r")]`),
> `schema/shapes/gradient_fill.rs:20` 同款。但 `grep -rn "rule" src/import/` = **零命中**,
> 运行时模型不带它,`RenderSink::draw` 没有这个参数,
> `runtime/vello.rs:38` 写死 `Fill::NonZero`。→ **even-odd 图形静默填成实心。**
>
> **补丁 5:"六处全走 panic"不全,渲染期还有裸 `unwrap`。** 原文 §6.4 说
> "那六个 `todo!()`/`unimplemented!()` 全在导入期",但至少还有两处在**求值/渲染期**:
> `runtime/model/animated.rs:257` `vertices.last().unwrap()`、
> `runtime/render.rs:306` `self.geometries.last_mut().unwrap()`。
> 这坐实了原文"渲染期也要罩"的结论,但理由要换成"有具体的裸 unwrap",不是"历史上修过一个"。
>
> **补丁 6:图片资产 = 必炸,且原文没测。** `:103` 的
> `unimplemented!("asset {:?} is not yet implemented")` 吃掉一切非 precomp 资产。
> 复核实测(§6.4 复核表):往 Tiger 里塞一条标准的 `assets: [{id, w, h, u, p, e}]`
> 图片资产 → **panic**。这不是边角料:AE 导出带位图的动画是常规操作,
> 而 README 的 "Missing features" 只写了 "Image embedding",读起来像"不画而已"。

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

> 【复核补充】比"同名"更该写下来的一条:这个冒名包的许可证是
> **GPL-3.0-or-later**(crates.io API 原始返回),**不在 `deny.toml` 的 allowlist 里**。
> 真手滑写进 `Cargo.toml`,拦下它的会是 `cargo deny check licenses` 而不是编译器——
> 但那要等到 CI,而且报错文本是许可证不合规,不会告诉你"你依赖错包了"。

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

> 【复核修正】树复现一致(kurbo 0.13.1 / peniko 0.6.1 / serde 1.0.229 /
> serde_json 1.0.151 / serde_repr 0.1.21,零 wgpu),但 **"15 个 crate"是对本仓库的
> 误导性口径**。逐个对 `Cargo.lock` 查过:**color 0.3.3、kurbo 0.13.1、
> linebender_resource_handle 0.1.1、peniko 0.6.1、polycool 0.4.0、serde、serde_repr、
> memchr、smallvec、arrayvec 全部已经在锁里**。
> **真正新增的只有 4 个:`velato`、`serde_json`、`zmij`(serde_json 1.0.151 的浮点格式化器)、`itoa`**
> (外加 serde 1.0.228→1.0.229、serde_repr 0.1.20→0.1.21 两个补丁位)。
> 供应链增量比原文说的小得多——这条是**利好**,但要说对。
>
> 反过来有一条**原文没看见的代价**:kurbo/peniko/color/polycool/linebender_resource_handle
> 目前**只经 `vello` 进树,而 `vello` 是 `optional` 且默认关**。加了 velato,
> 这几支第一次进入**默认(纯 CPU)构建图**。这不违反 `paint.rs:20-27` 的裁决
> (那条管的是 **`Painter` 签名**不许出现 feature-gated 类型,velato 藏在 sv-lottie 里,
> 签名依旧干净),但默认构建时间/体积会涨。
> **对策:`sv-lottie` 自己也做成 feature,默认关**——与 `backend-vello` 同一条纪律。
>
> 还有一条 **MSRV 咬合风险**:velato 0.11 的 `rust-version = "1.88"`,
> **和本仓库的 MSRV 一模一样**,零余量。而 velato README 白纸黑字:
> "Future versions ... might increase the Rust version requirement.
> It will not be treated as a breaking change and as such **can even happen with small patch releases**."
> 本仓库 CI 有一条 MSRV 1.88 构建道(workspace `Cargo.toml:23-25` 注释),
> 一次 `cargo update` 就可能把它打红。**要么锁 `=0.11.0`,要么接受 MSRV 跟着 velato 走。**

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

| 输出尺寸 | 帧时间 | 占 60fps 预算 | 【复核】独立实现实测 |
| --- | --- | --- | --- |
| 128 × 128 | **0.55 ms** | 3% | **0.35 ms**(2%) |
| 256 × 256 | **0.99 ms** | 6% | **0.64 ms**(4%) |
| 512 × 512 | **2.24 ms** | 13% | **1.20 ms**(7%) |
| 1024 × 1024 | **6.18 ms** | 37% | **2.34 ms**(14%) |

输出经肉眼验证正确(渲染出完整的老虎,`Tiger.json.256.png`)。

> 【复核修正】复核独立写了一份 sink(**根裁剪走矩形交集,不分配 `Mask`**,
> 即原文本节最后一条自己提的优化),四档全部更快,**且差距随面积单调放大
> (1.6× → 1.5× → 1.9× → 2.6×)**——这个形状正是"每帧一张 w×h `Mask` 的
> 分配 + 逐像素相乘"的指纹,不是机器差异。
> **结论方向不变但更强:1024px 全屏动画在纯 CPU 上是 14% 预算,不是 37%。**
> 原文用 37% 去论证"1024 只配当启动画面",这个论据站不住;真正限制全屏 lottie 的
> 是**没有脏矩形**(§0 死穴 2),不是光栅本身。
> 其余数字复核**逐位或近似复现**:解析 18.1–19.9 ms(原文 19.3)、
> 场景构建 0.0055–0.0059 ms/帧(原文 0.007)、单帧填充 48.7 次(原文 ≈49)、
> 单帧路径段 445.2(原文 ≈444)。**§6 不是纸上数据。**

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

> 【复核确认】本节全部数字**逐位复现**:2921 / 60 / 121 / 121 / 3102 / 3223 / 96.2%,
> PolyStar 180/180 = 100%;Tiger 的 4 组 matte 也对上了
> (`tt:1` 在图层 1/5/8/12,`td:1` 在 0/4/7/11,`nm` 分别是 body_crouched / body / body 2 / Tail)。
> 独立渲出的 256px 老虎目视同样正常。**这一节可以放心引用。**
>
> 【复核修正 · 两条口径纠正】
>
> 1. **"96.2%"量的是 API 可接性,不是视觉正确性。** 那 2921 条纯色填充里,
>    **凡是原文件写了 `"r": 2`(even-odd)的,全部会被画错**(§1.4 补丁 4:
>    velato 根本没把 fill-rule 传出来)。Tiger 恰好不用 even-odd,所以这个缺陷
>    在本样本上是隐形的——**和 matte 一样是"样本运气",但比 matte 更难发现**:
>    matte 丢了会看出来,fill-rule 丢了只是图标"变胖了"。
>    要给覆盖率加一列"能接住且画得对",Tiger 上仍是 96.2%,但这个数**不可外推**。
> 2. **那 121 条 `push_layer` 不是"blend 模式",是 Porter-Duff 合成算子。**
>    `import/builders.rs:45-49` 原文:`MatteMode::Alpha | Luma => Compose::SrcIn.into()`、
>    `InvertedAlpha | InvertedLuma => Compose::SrcOut.into()`,只有 `Normal` 才是
>    `Mix::Normal.into()`。也就是说 `peniko::BlendMode { mix: Normal, compose: SrcIn }`——
>    **`mix` 是 Normal**。复核第一版分类器只比 `mix` 就把 121 条全判成"平凡"了,
>    改成整个 `BlendMode` 相等才对上原文。**这条直接影响 §7.2 第 2 项的动词设计**
>    (那里写的是 `push_layer(alpha/blend)`,少了 compose 这一维)。

### 6.4 健壮性:一个可复现的 panic

拿 Tiger.json 做了三组变异(python 改 JSON,`ks` 字段):

| 变异 | 模拟的真实场景 | 结果 |
| --- | --- | --- |
| 删除每个图层 transform 的 `r` 键 | **Lottie 优化器剥掉默认字段**(TinyLottie 之类工具的标准操作) | ❌ **panic**:`not yet implemented: split rotation`,`converters.rs:213` |
| 改成 split rotation(`rx`/`ry`/`rz`) | AE 勾选 "Separate Dimensions" 导出旋转 | ❌ **panic**,同一行 |
| 改成 split position(`p: {s:true, x, y}`) | AE 勾选 "Separate Dimensions" 导出位置 | ✅ 正常解析并渲染 |

**这就是 §0 说的第一条死穴,已从"读源码推测"升级为"实测复现"。**
`Composition::from_str` 返回 `Result` 会让调用方以为错误是可恢复的——它不是。

> 【复核确认 + 扩表】三行**全部独立复现**(独立 cargo 项目,velato 0.11.0,
> `serde_json` 改 JSON,`catch_unwind` 判定)。补三行:
>
> | 变异 | 模拟的真实场景 | 结果 |
> | --- | --- | --- |
> | **`assets` 里放一条图片资产** | AE 导出带位图的动画(极常见) | ❌ **panic**:`converters.rs:103` |
> | 截断 JSON(取前一半) | 文件损坏 / 下载不全 | ✅ **返回 `Err`**(serde 错误确实可恢复) |
> | 帧号 `-1` / `-1e6` / `1e9` / `NaN` / 越界 | 时间轴算错、除零 | ✅ 全部 ok(画 0 次,不炸) |
>
> 前两行说明一件事:**velato 的 `Error` 只覆盖"JSON 不合法",不覆盖"Lottie 不合法"。**
> 语法层面它给你 `Result`,语义层面它给你 SIGABRT。
> 第三行是好消息——**帧号不用做防御性钳制**,这是 §7.1 少写的几行。

缓解手段(已验证前提):本仓库根 `Cargo.toml` 的 `[profile.*]` **没有设
`panic = "abort"`**,所以 `std::panic::catch_unwind` 可用。velato 的解析是纯
函数式的(`Composition` 是纯数据,`Renderer` 只持一个可清空的 batch),
unwind 后没有跨边界的残留状态,catch 是安全的。

> 【复核修正】`[profile.*]` 无 `panic = "abort"` 属实(全仓只有 `opt-level`)。
> 但这段的 unwind-safety 分析**看错了边界**:
>
> 1. **我们是库,不是应用。** `panic = "abort"` 是**下游 workspace 的 profile 说了算**,
>    不是我们说了算。下游一开 abort,`catch_unwind` 直接失效、进程照炸。
>    "兜底"必须写成"**在 unwind 构建下兜底**",别写成无条件保证。
> 2. **真正会脏的不是 velato,是 `Painter`。** velato 侧确实干净
>    (`append` 开头就 `self.batch.clear()`,`Composition` 是纯数据)。
>    但 `catch_unwind` 里包着的是 `&mut dyn Painter`——它要 `AssertUnwindSafe`,
>    而 `TinySkiaPainter` 有一条**内部裁剪栈**(`paint.rs:384 clips: Vec<[f32;4]>`),
>    `paint_tree` 外面还有一条**并行的 `clip_stack`**(`render.rs:717`),
>    两条栈靠"push/pop 严格配对"同步。**适配器画到一半 panic,两条栈就永久错位,
>    这一帧剩下的所有节点都会带着错误裁剪画出去,而且不会自愈。**
>    → 兜底必须是:进适配器前记下 `clips.len()`,catch 到之后**把栈弹回该深度**。
>    这需要给 `Painter` 加一个 `clip_depth()`(或让适配器自己数)——
>    **一个原文没有列进"新动词"清单的新动词。**

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

> 【复核修正 —— 原文在这里犯了它自己刚纠正过的那个错】
> **`stroke_path` 也已经落地了**,而且比 fill_path 还新:commit
> `3ebe81c feat(painter): 补 stroke_path——路径动词齐活(lottie/SVG 图标的"步骤 0")`,
> **2026-07-22,就在本文入库(`419d447`,同日)之前两个 commit**。
> 现在 `paint.rs:198` 是:
>
> ```rust
> fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color);
> ```
>
> 配套 `StrokeStyle { width, cap, join, miter_limit }` + `LineCap` + `LineJoin`
> (`paint.rs:48-89`),CPU 端走 tiny-skia 原生 `stroke_path`,金样端有
> `PaintCmd::StrokePath`,`paint.rs` 里已有 3 条针对性单测
> (线宽真的传下去、Round 端帽真的多出半圆、描边不当成填充画)。
>
> **连带作废两处**:
> - §0 分档表 (b) 档的第一项、§7.2 表格的第 1 行——`stroke_path` **不是"先补基建"**,
>   它是**已完成事实**。(b) 档实际只剩 4 项。
> - §6.3 那句"现有 `Painter`(fill_rounded_rect / stroke_rounded_rect / fill_path(纯色)/
>   push_clip(矩形)/ pop_clip / glyph_run)"——**漏了 `stroke_path`**。
>   Tiger 恰好零描边,所以 96.2% 这个数**不受影响**;但换一个有描边的样本,
>   原文会低估自己的覆盖率。
>
> 教训值得写进文档纪律:**"任务书里的既有事实"和"仓库 HEAD"是两份东西,
> 而且这个仓库一天能落三四个 commit。** 原文只对 `fill_path` 做了核对,
> 没有把同一次核对推到相邻动词上。写方案前先 `git log --oneline -20 -- crates/sv-shell/src/paint.rs`。

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

> ### 【复核修正】§7.1 的四条硬伤 —— 按写的做,第一天就卡住
>
> **1. `sv-lottie` 独立 crate **构不出来**:循环依赖。**
> 适配器要 `impl velato::RenderSink`,内部持 `&mut dyn Painter`——
> 而 `Painter` 住在 `sv-shell`(`crates/sv-shell/src/paint.rs:155`,由
> `sv-shell/src/lib.rs:20` 导出)。于是 `sv-lottie → sv-shell`。
> 同时本节第 3 条又要"`sv-shell/render.rs` 的 `paint_tree` 在该 kind 上调适配器",
> 于是 `sv-shell → sv-lottie`。**Cargo 不接受循环。**
> 三条出路,得选一条并写进方案:
> - (A) **适配器就放 `sv-shell` 里**,feature `lottie` 门控。最省事,代价是
>   sv-shell 又胖一圈,且"资产格式不进渲染壳"的洁癖破了一次。
> - (B) **把 `Painter`/`PathCmd`/`StrokeStyle` 下沉**到一个 `sv-paint` 基座 crate,
>   sv-shell 与 sv-lottie 都依赖它。最干净,但**动的是 ADR-3b 的物理分层**,
>   要单独立项、要改一堆 `use`,不属于"3–5 人日"。
> - (C) **反转控制**:sv-lottie 只吐 `Vec<PaintCmd>`(已有的金样命令类型),
>   sv-shell 负责回放。多一次拷贝,但**顺带白拿金样测试**——
>   lottie 的渲染正确性可以完全靠 `RecordingPainter` 对拍,不用像素比对。
>   复核倾向 (C):它同时解决了"怎么给 lottie 写测试"这个原文没提的问题。
>
> **2. "`ElementKind` 加 `Lottie`"不是"小改",是全仓穿透。**
> 这个枚举**没有 `#[non_exhaustive]`,也没有一处 `_` 兜底**,新增变体会点亮
> **每一个穷尽 `match`**:
> `a11y.rs:103-124`(要给一个 AccessKit `Role`,树里还没有"图形"语义)、
> `a11y.rs:127`(第二个 match)、
> `render.rs:319-358` `measure_leaf`(lottie 是叶子,要给固有尺寸——
> 好在 `Composition` 自带 `width/height`)、
> `render.rs:718-924` `paint_tree`、
> `sv-ui/lib.rs:1161-1169` `dump_tree`(**这条会改动金样文本 → 一批快照测试要重录**)、
> `sv-ui/lib.rs:477-501` `create`(focusable/accepts_text/input 三个默认位)、
> `sv-compiler/emit.rs:28-49` 的 `ElemKind` 镜像 + `create()`(否则 `.sv` 用不了),
> 以及 CLAUDE.md 点名的"改绑定原语要同步改 sv-macro codegen 与其测试"。
> **这是 6+ 处穷尽 match + 一批金样重录,不是"三处小改"。**
>
> 更麻烦的是**分层**:原文说"节点持 composition 句柄",可 `ViewNode` 在 `sv-ui`,
> 而 `sv-lottie` 明确"不进 sv-ui"。**要么 sv-ui 认识 velato(违反本节自己的裁决),
> 要么句柄是个不透明 `u64` + sv-shell 侧的旁表。** 必须选一个,原文两头都占。
>
> **3. `anim.rs` 的形状不对,而且会打穿"零 diff 定点更新"。**
> 现在的 `Anim` 是**有限时长**动画:`pump` 里 `t = ((now - start)/dur).clamp(0,1)`,
> `retain_mut` 在 `t >= 1.0` 时丢弃(`anim.rs:119-132`)。lottie 要的是
> **循环 / 可暂停 / 可 seek**,不是"跑完就出队"——这不是"`Channel` 加一档",
> 是 `Anim` 结构体加一个生命周期语义。
> **真正的地雷在下面**:两条现有通道推进的方式都是
> `doc.update_style(...)` / `doc.set_scroll(...)`,而 `update_style` 结尾无条件
> `self.bump()`(`sv-ui/lib.rs:658`)。lottie 每帧 bump → `doc.version()` 变 →
> **`layout_full_cached` 的缓存键失效(`render.rs:465-470`)→ 每帧重建整棵 taffy 树**。
> 一个角落里 64px 的 spinner 会把全窗布局拖成每帧全量。
> 好消息是**有干净解法,而且免费**:`App::paint` 的静止帧短路是
> `if unchanged && !animating && ...`(`sv-shell/lib.rs:230`)——
> **`animating` 为真本身就能强制重绘,根本不需要 bump 版本号。**
> → 裁决:**lottie 的帧推进必须走一条不 bump 版本的通道**(帧号存在 sv-shell 侧旁表,
> 或给 `Doc` 加一个 `update_style_silent`)。这条不写进方案,(a) 档一落地就是性能事故。
>
> **4. 工期。** 原文 3–5 人日的置信度建立在"适配器有现成模板 + 集成点都是既有机制"上,
> 而复核证明**模板是 velato 0.10 的、且从不演示降级**(§1.3 复核框),
> **集成点是 6+ 处穷尽 match + 金样重录 + 一个待定的 crate 分层裁决**。
> 再加上本文没算的:panic 兜底后的**裁剪栈恢复**(§6.4 复核框)、
> `push_layer` 的**标记栈配平**(§1.3 复核框)、以及这个仓库的测试密度
> (最近一个 commit 一次补 52 项单测)。
> **修正估算:6–9 人日**,其中 crate 分层裁决(上面 A/B/C)如果选 (B) 要另计。

### 7.1b 【复核补充】(a′) 被原文漏掉的"20% 力气拿 80% 收益"

原文的分档是 (a) 完整运行时 / (b) 补基建 / (c) 不做,**中间少了一整档**:
既然 (a) 档的目标场景是 Spin / Skeleton / Result / Empty 这类**小、循环、少量图层**的
资产,那就有两条比"塞一个运行时解析器"便宜得多的路。

**(a′-1) 构建期烘焙(bake),不是构建期转译。**
和 §7.3 第 4 条否掉的 "lottie → `.sv` 代码" **完全不是一回事**,别混:
转译要求把 Lottie 的动画语义映射到我们的属性动画系统(我们没有),
烘焙只要求把**每一帧的绘制命令**提前算出来存成数组——运行时是纯回放。

```
build.rs:  velato(dev-dependency) 解析 .json
        → 逐整数帧跑 Renderer::append
        → 收成 &[&[PathCmd]] + 颜色表,emit 成一个 .rs
运行时:   sv-lottie 不存在;帧推进 = 数组下标;绘制 = 现成的 fill_path/stroke_path
```

**这一档同时干掉了本文的两条死穴**:panic 只可能发生在**构建期**(构建挂了你立刻知道,
不是用户那里崩);velato **不进运行时依赖图**(MSRV 咬合、供应链、
默认构建变胖三个问题一起消失,velato 退化成 `[build-dependencies]`)。
fill-rule 缺陷仍在(那是 velato 的信息丢失),但可以在构建期**报警**。

**代价是体积,而且可量化。** 复核实测(每条路径命令按 1B tag + 至多 6×f32 计,
每条 draw 加 9B 头):

| 样本 | 帧数 | draw 条数 | 烘焙后体积 |
| --- | --- | --- | --- |
| PolyStarTest.json(512×256,2 图层) | 120 | 240 | **51 KB** |
| Tiger.json(1024×1024,14 图层) | 306 | 14863 | **2.72 MB** |

→ **判决:spinner / 空状态勾选 / 小图标动效这一类,烘焙是明显更优解**
(51 KB 静态数据 vs 一个会 panic 的解析器 + 4 个新依赖)。
**插画级(Tiger 级)不适用**,2.72 MB 进二进制不可接受——那一档才需要 (a)。
分界线大约在"帧数 × 单帧路径段数",可以在 build.rs 里设阈值,超了就报错让人改用 (a)。

**(a′-2) 最缺的那个组件根本不需要 lottie。**
原文说 (a) 档"恰好命中调研 26 §3.1 里最缺动效的那几件"。但去看调研 26 §3.2 原文:
"Progress / Skeleton / Spin | 直条 ✅;**环形 Progress 需圆弧路径(fill_path)**;
Spin/Skeleton 动画需帧循环"。
**arco 的 Spin 就是一段旋转的圆弧。** 有了 `fill_path` + `stroke_path`(两个都已落地),
它需要的全部东西是 **`anim.rs` 加一条 rotation 通道**——十几行,
而且那条通道是 CSS `transform: rotate` / 过渡系统的公共前置,**收益远超 lottie**。
Skeleton 是一条平移的线性渐变(等 §7.2 第 4 项),Empty/Result 是静态 SVG 图标
(fill_path + stroke_path 已就位,走调研 26 的 usvg 路线,连描边转填充都不用做了)。

**→ 所以"最省力的路"这个问题的答案要分两层:**
**要"播放设计师给的任意 Lottie 文件"** → velato,本文 (a) 档。
**要"让 arco 那几个组件动起来"** → 一条 rotation 通道 + 已有的路径动词,
**根本不用引入 lottie**。原文把这两件事合并论证了,这是它最大的框架性问题——
它证明了"velato 是最省力的 lottie 方案",但没有先问"我们现在需要的是 lottie 吗"。

**(a′-3) 预渲染成序列帧/雪碧图 —— 明确排除,理由要写下来。**
原文一个字没提,但这是业界最常见的"偷懒"路子,不写会有人反复提。
排除理由是**硬的**:`Painter` **没有 `draw_image`**(§0 既有事实核对过一遍,
仍然成立——`3ebe81c` 补的是 `stroke_path`,不是图片),
接序列帧要先造整个图片子系统(解码 + 纹理/位图缓存 + 缩放采样 + HiDPI 多倍图),
而调研 26 明写"图片子系统无排期"。**代价比 (a) 档高,收益还更差**
(矢量的分辨率无关性没了,HiDPI 上糊)。**归入 (c)。**

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

> ### 【复核修正】本表三条要改
>
> **第 1 行整行删掉:`stroke_path` 已落地**(commit `3ebe81c`,§7.0 复核框)。
> 连同它的"顺带收益"栏——"直接省掉调研 26 §4 的 usvg 描边转填充"——**已经是已实现的收益**,
> 不该再记在 lottie 的账上当立项理由。**(b) 档实际只有 4 项。**
>
> **第 2 行的动词签名不对。** 写的是 `push_layer(alpha, blend)`,
> 但 track matte 需要的是 **Porter-Duff 合成算子**,不是 mix:
> `import/builders.rs:45-49` 把 `MatteMode::Alpha|Luma` 映射成 `Compose::SrcIn`、
> `InvertedAlpha|InvertedLuma` 映射成 `Compose::SrcOut`,
> **`mix` 全程是 `Normal`**(§6.3 复核框)。
> 正确形状是 `push_layer(mix, compose, alpha)` 三维,
> 且按 `paint.rs:20-27` 的裁决**必须是自有轻量枚举**(`LayerMix` / `LayerCompose`),
> **不能把 `peniko::BlendMode` 放进 `Painter` 签名**——peniko 只在 velato/vello 下存在。
> CPU 端好消息:`tiny_skia::BlendMode` 原生有 `SourceIn` / `SourceOut`,
> 配 `draw_pixmap(PixmapPaint { blend_mode, opacity })` 语义是对得上的;
> 坏消息不变——要离屏 `Pixmap`,与 ADR-3b 的 CPU 能力冻结正面冲突,
> 所以"CPU 端降级 + `caps` 报 false"这条裁决**成立且应保留**。
> 但注意 `PainterCaps` 现在只有 `external_texture` / `blur` 两位
> (`paint.rs:147-152`),**要加位**,这也是一处 API 变更。
>
> **第 5 行(脏矩形)的排序理由要换。** 原文的论据是"1024px 占 37% 预算";
> 复核实测是 14%(§6.2 复核框),这个论据弱了。
> 但**结论反而更该保留**,理由换成:§7.1 复核框第 3 条证明,
> 按原文写法 lottie 每帧会 `bump()` → **每帧全量重建 taffy 树**,
> 那个开销和窗口复杂度成正比、和 lottie 大小无关。
> **脏矩形之前,先做"帧推进不 bump 版本号"——那是零成本的,而且立刻见效。**

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

> ### 【复核修正】一句话答复要改成两句
>
> **先问需求再选方案。**
> - **"让 arco 那几个组件动起来"** → **不用 lottie**。`anim.rs` 加一条 rotation 通道
>   + 已落地的 `fill_path`/`stroke_path`,Spin/Progress 就齐了(§7.1b-2)。
> - **"播放设计师给的固定几个 lottie 资产"** → **构建期烘焙**,
>   实测 spinner 级 51 KB,velato 退成 `[build-dependencies]`,
>   死穴 ① 从"用户那里崩"降级成"CI 挂"(§7.1b-1)。
> - **"播放用户/运行时任意 lottie"** → 才轮到本文的 (a) 档 velato 运行时,
>   **6–9 人日**(非 3–5),且必须先裁掉 `sv-lottie` 的循环依赖(§7.1 复核框第 1 条)。
>
> **死穴补第 ④ 条:velato 静默丢弃 fill-rule**,even-odd 图形被填成实心,
> **且这条被算在那 96.2% 的"可接住"里**——它是唯一一条"看起来跑通了、
> 其实画错了"的缺陷,比 ①②③ 都更难在验收时发现。

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

**【复核补充】还应入册的未核实项:**

8. **带 even-odd(`"r": 2`)的真实 Lottie 样本上的视觉损失**——§1.4 补丁 4 的缺陷
   是**读源码 + grep 证实的**(`src/import/` 零命中 "rule"),但**没有做像素对拍**。
   要量化需要一个用 even-odd 的资产,手头两个样本都不用。
9. **velato 0.10 → 0.11 之间 `RenderSink` trait 是否变过**——CHANGELOG 只写了
   "Vello upgraded to v0.9",推断未变,但**没有逐字 diff 两个版本的 `render.rs`**。
   这影响 `velato_imaging`(锁 0.10)能否直接照抄。
10. **烘焙体积表的编码假设**(§7.1b:1B tag + 至多 6×f32 + 每 draw 9B 头)是
    **按 `PathCmd` 的自然布局估的**,不是真跑过 codegen。真实体积会因
    去重 / 量化 / 增量编码而**更小**,因 Rust 静态数组的对齐而略大。量级可信,精确值不可引。
11. **(a′-1) 烘焙路线下的"逐整数帧"是否够**——Lottie 的 `fr` 与显示器刷新率不一致时
    (60fps 资产 @ 144Hz 屏)需要插值或重复帧,**未验证观感**。运行时路线没这个问题。
12. **`catch_unwind` + 裁剪栈恢复方案的可行性**——§6.4 复核框提出的
    "记 `clips.len()` / 加 `clip_depth()`" **只是设计,未实现验证**。
13. **本仓库开 `sv-lottie` 后 `cargo deny check` 的实跑结果**——只逐个查了新增 4 个
    crate 的许可(velato / serde_json / zmij / itoa),**没有在本仓库真跑一次门禁**
    (会改 Cargo.toml,越界)。

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

---

## 9. 复核记录(对抗性复核,2026-07-22,基线 `50a700c`)

复核立场是**尽力证伪**。方法:不改仓库任何 crate,在 `%TEMP%` 下建独立 cargo 项目
(`velato = { version = "0.11", default-features = false }` + `kurbo 0.13` + `peniko 0.6`
+ `tiny-skia 0.11`,独立 `CARGO_TARGET_DIR`),**自己重写一遍 `RenderSink`**
(一个纯计数 sink 做覆盖率、一个 tiny-skia sink 做光栅、一个烘焙 sink 量体积),
用 `serde_json` 现场改 JSON 做变异。velato 源码读的是
**crates.io 上 `velato-0.11.0.crate` 解包后的原文**(不是 GitHub main——两者已漂移),
`velato_imaging-0.0.1.crate` 同样解包逐行读。
外部事实一律取 crates.io / GitHub API 原始返回。

### 9.1 复现结果:§6 的数**站得住,而且精度高得像真跑过**

| 原文断言 | 复核实测 | 判定 |
| --- | --- | --- |
| Tiger.json **428 543 B** | 428 543 B(逐字节相同) | ✅ |
| 1024×1024 / 60fps / 帧 0–306 / 14 层全 ty=4 / Bodymovin 5.8.1 | 全部一致 | ✅ |
| 4 组 track matte:`tt:1` 在图层 1/5/8/12、`td:1` 在 0/4/7/11 | 一致(`nm` 为 body_crouched / body / body 2 / Tail) | ✅ |
| 解析 **19.3 ms** | 18.1–19.9 ms | ✅ |
| 场景构建 **0.007 ms/帧** | 0.0055–0.0059 | ✅ |
| 单帧填充 ≈**49**、路径段 ≈**444** | 48.7 / 445.2 | ✅ |
| 60 帧命令:纯色填充 **2921**、矩形裁剪 **60**、平凡层 **121**、非平凡层 **121** | **逐位相同** | ✅ |
| 覆盖率 **3102 / 3223 = 96.2%**;PolyStar **180/180 = 100%** | **逐位相同** | ✅ |
| 忽略 `push_layer` 后老虎目视无明显缺陷 | 独立渲出 256px PNG,一致 | ✅ |
| 删 `ks.r` → panic;split rotation → panic;split position → 正常 | 三条全复现 | ✅ |
| velato 0.11.0 / 2026-07-21 / Apache-2.0 OR MIT / MSRV 1.88 / vello 可选 | crates.io API 逐项一致 | ✅ |
| `converters.rs` 的 `:103/:211/:213/:243/:252/:805/:806` 六处 | **0.11.0 解包后逐行核对,行号全中** | ✅ |
| `RenderSink` trait 逐字引用、README 版本表、CHANGELOG 0.9.0/0.8.1 引文 | 逐字一致 | ✅ |
| dotlottie-rs 只有 `0.1.0-alpha.1`/2024-09-18/183 下载;rlottie 0.5.4/2026-03-07/总 33002;rasterlottie 0.2.1/2026-04-24;velato_imaging 0.0.1/2026-05-21;vello_hybrid & vello_cpu 均 0.0.9/2026-05-30;Slint #5549 open/2024-07-04/4 评论;`lottie-rs` 是 Fountain 编辑器 | 全部一致 | ✅ |

**结论:这不是纸上调研,作者真的把 velato 接起来跑过。下面推翻的都是解释、
遗漏与既有事实过期,不是数据。** 唯一对不上的是光栅帧时间(见 9.2 第 4 条)。

### 9.2 推翻/修正的六条(按严重度)

1. **既有事实过期:`stroke_path` 已落地。**(§7.0 复核框)
   commit `3ebe81c`(2026-07-22),就在本文入库 `419d447` **之前两个 commit**。
   原文用一整节 §7.0 去纠正"没有 fill_path"这条过期事实,却把**相邻动词**漏了,
   于是 §0 分档表与 §7.2 表第 1 行把一件已完成的事列成了"先补基建",
   §6.3 的 Painter 动词清单也漏了它。**同一类错误、同一天、同一个文件。**
2. **架构硬冲突:`sv-lottie` 独立 crate 构不出来(循环依赖)。**(§7.1 复核框第 1 条)
   `Painter` 在 sv-shell(`paint.rs:155`,`lib.rs:20` 导出),适配器要它;
   而 `paint_tree`(`render.rs:715`)要调适配器。**Cargo 拒绝。**
   给了 A/B/C 三条出路,推荐 (C)(sv-lottie 只吐 `Vec<PaintCmd>`),
   因为它顺带解决了原文没提的"lottie 怎么写测试"。
3. **静默错渲:velato 丢弃 fill-rule。**(§1.4 补丁 4)
   `schema/shapes/fill.rs:25` 解析了 `"r"`,`grep -rn "rule" src/import/` **零命中**,
   `runtime/vello.rs:38` 写死 `Fill::NonZero`。
   **even-odd 图形被填成实心,且被计入那 96.2% 的"可接住"。**
   这是全文最该补的一条:它是唯一"看起来跑通、其实画错"的缺陷。
4. **性能被自己的实现拖低了 1.6–2.6 倍。**(§6.2 复核框)
   独立实现(根裁剪走矩形交集、不分配 `Mask`)实测
   128/256/512/1024 = 0.35 / 0.64 / 1.20 / 2.34 ms(原文 0.55 / 0.99 / 2.24 / 6.18)。
   差距**随面积单调放大**,是 `Mask` 的指纹而非机器差异。
   **1024px 是 14% 预算不是 37%**,原文据此下的"1024 只配当启动画面"论据作废。
5. **`anim.rs` 集成会打穿定点更新。**(§7.1 复核框第 3 条)
   `update_style` 无条件 `bump()`(`sv-ui/lib.rs:658`)→ `doc.version()` 变 →
   `layout_full_cached` 缓存键失效(`render.rs:465-470`)→ **每帧重建整棵 taffy 树**。
   而 `App::paint` 的 `if unchanged && !animating`(`sv-shell/lib.rs:230`)
   **本来就允许"不 bump 也能重绘"**——白送的正确解法,原文没看见。
   顺带:现在的 `Anim` 是有限时长(`t>=1` 即出队),**装不下循环动画**。
6. **"三处小改"是 6+ 处穷尽 `match` + 金样重录。**(§7.1 复核框第 2 条)
   `ElementKind` 无 `#[non_exhaustive]`、无 `_` 兜底,新增变体点亮
   a11y ×2 / measure_leaf / paint_tree / dump_tree(**改金样文本**)/ create /
   sv-compiler `ElemKind` 镜像。外加 §7.1 自相矛盾:
   "节点持 composition 句柄"要 sv-ui 认识 velato,却又说"不进 sv-ui"。
   **工期由 3–5 人日修正为 6–9 人日。**

### 9.3 补充的遗漏(原文没写,但会影响裁决)

- **`catch_unwind` 的分析看错了边界。**(§6.4 复核框)
  velato 侧确实干净;会脏的是 `Painter` —— `TinySkiaPainter::clips`(`paint.rs:384`)
  与 `paint_tree::clip_stack`(`render.rs:717`)两条并行栈靠严格配对同步,
  半途 panic 就永久错位,该帧剩余节点全部带错误裁剪且不自愈。
  且 `panic = "abort"` **是下游 workspace 的 profile 说了算**,我们保证不了。
- **`push_layer` 忽略会把栈搞乱。**(§1.3 复核框)
  velato 的 pop 是**按数量配平**的(`render.rs:169`)。
  必须像 `velato_imaging` 那样压标记(`Clip`/`Group`/我们还要 `Elided`)。
- **track matte 是 Porter-Duff `Compose::SrcIn/SrcOut`,`mix` 全程 Normal**
  (`import/builders.rs:45-49`)。§7.2 第 2 行的 `push_layer(alpha, blend)` 少了一维;
  且 `PainterCaps` 现在只有两位(`paint.rs:147-152`),要加位。
- **依赖增量是 4 个不是 15 个**(§6.1 复核框):color/kurbo/peniko/polycool/
  linebender_resource_handle/serde/serde_repr/memchr/smallvec/arrayvec **已在 `Cargo.lock` 里**;
  真新增只有 velato / serde_json / zmij / itoa。
  **但**这几支此前只经 `optional` 的 vello 进树,加 velato 后首次进入默认 CPU 构建图
  → `sv-lottie` 自己也该做成默认关的 feature。
- **MSRV 零余量**:velato 1.88 == 本仓库 1.88,而 velato README 明说 MSRV 可在
  **patch 版本**里涨。本仓库有 MSRV 构建道,一次 `cargo update` 就可能打红。
- **图片资产 = 必炸**(`converters.rs:103`),原文列了这个行号却没测;复核实测 panic。
  **越界帧/NaN 帧反而安全**(实测不炸、画 0 次),这条是好消息,可以少写防御代码。
- **velato_imaging 不是即插模板**:锁 `velato "0.10.0"`(编不过 0.11)、331 行(非 ~200)、
  **且从不降级**——"接不住时怎么办"正是它一个字没演示、而我们全部工作量所在的部分。
- **`lottie-rs` 冒名包是 GPL-3.0-or-later**,不在 `deny.toml` allowlist 内(§2.3)。

### 9.4 被漏掉的"20% 力气拿 80% 收益"(新增 §7.1b)

原文的分档在 (a) 完整运行时与 (c) 不做之间**缺了一整档**,而且没有先问
"我们现在需要的是 lottie 吗":

- **构建期烘焙**(≠ §7.3 否掉的"转译成 `.sv`"):build.rs 跑 velato 出逐帧
  `PathCmd` 静态表,运行时零 velato / 零 JSON / 零 panic,velato 退成 `[build-dependencies]`。
  **实测体积:PolyStar 级 51 KB、Tiger 级 2.72 MB** —— 对 (a) 档自己点名的
  spinner / 空状态场景**明显更优**,对插画级不适用。
- **Spin 根本不需要 lottie**:调研 26 §3.2 原文说环形 Progress 要的是"圆弧路径(fill_path)",
  而 `fill_path`/`stroke_path` **都已落地**;缺的只是 `anim.rs` 一条 rotation 通道
  ——十几行,且是 CSS `transform: rotate` 的公共前置。
- **预渲染序列帧**:明确排除并写下理由(`Painter` 无 `draw_image`,
  要先造图片子系统,而调研 26 明写"无排期"),免得反复被人提起。

### 9.5 无法验证 / 未做的

见 §8 第 8–13 条(复核补充部分)。另外**未做**:GPU(`backend-vello`)路径实测
(不开窗)、真在本仓库跑 `cargo deny`(会改 Cargo.toml,越界)、
向 velato 上游提 issue/PR(§6.4 建议的那个 20 行修复,**复核认同其性价比判断**)。
