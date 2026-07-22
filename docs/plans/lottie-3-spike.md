# Lottie 可行性 spike:仓库外最小可运行验证

> 生成:2026-07-22。**方法:不是查资料,是跑代码。** 在 `%TEMP%` 下建了四个临时
> cargo 项目真跑,本仓库 `crates/` 一行没动。所有版本号/日期取自 crates.io 与
> GitHub API 原始返回,所有耗时/内存/像素数取自实际进程输出,PNG 产物逐张看过。
> 没跑通的、没测的,写在 §9。
>
> 问题:**velato 到底能不能在本仓库的版本约束下跑起来;跑起来是什么代价。**
>
> ---
>
> ⚠️ **对抗性复核(2026-07-22,复核者独立重跑了本文全部可复现实验)。**
> 结论:**性能/内存/依赖三类数字全部复现,可以信;三处事实要改,一处"意外发现"
> 不是新的,一条核心结论(CPU/GPU 对拍)被我用真正的逐像素比对推翻了口径。**
> 逐条见各节的 ⚠️ 块与文末 §10「复核记录」。读之前先知道三件事:
>
> 1. **本文不是这条线上的第一份文档。** `docs/plans/lottie-1-ecology.md`(已入库,
>    提交 `419d447`)与 `lottie-2-architecture.md` 在本文之前已经做过生态核实与
>    架构裁决。本文标为"最大的意外发现"的 `RenderSink` 解耦、"我独立复现"的
>    `todo!()` panic、"主线已补 fill_path"、"根裁剪该特判"、"lottie 逼出脏矩形"
>    ——**这五条在 lottie-1 里都已经有了**。本文的真实增量在别处(§10 列了)。
> 2. **§3.4 的"非白像素比"不是像素级对拍。** 它只比"非白像素的个数"。复核实测:
>    §5.2 那次优化前后两张图有 4140 个像素不同,而这个指标**一个数都没动**。
>    真正的逐像素比对结果见 §3.4 的 ⚠️ 块。
> 3. **§3.2、§5.2 的数字产生于 §6.2 那个 compose bug 修复之前**,§3.4 之后的才是修好的。
>    本文没有说明这件事,附录的 PNG 清单也混着两代产物。

**机器**:i5-12400(6C12T)/ 31.7 GB / Intel UHD 730(集显,Vulkan)/ Win11 26200 /
rustc 1.88.0 + cargo 1.88.0(x86_64-pc-windows-msvc)。所有耗时都是 `--release`。

---

## 0. TL;DR 判决

**四个问题的答案:能 / 能 / 能 / 比预想便宜。没有找到死穴级的依赖冲突。**

1. **版本共存:完全没冲突。** velato 0.11.0(2026-07-21 发布,昨天)的依赖恰好是
   `vello ^0.9.0` + `kurbo ^0.13.0` + `peniko ^0.6.0` —— 与本仓库 `Cargo.lock`
   锁定的 vello 0.9.0 / kurbo 0.13.1 / peniko 0.6.1 **逐项对上**。`cargo tree -d`
   里没有第二份 vello/peniko/kurbo(§2)。
2. **离屏渲染:两条路都跑通了。** GPU 档(velato → `vello::Scene` → wgpu 离屏 →
   PNG)与 CPU 档(velato → **我自己写的 tiny-skia RenderSink** → PNG)都出图,
   15 个资产逐张对拍,非白像素比 **0.9956–1.0000**(§3)。
3. **tiny-skia 任意路径填充:本来就有,`fill_path`/`stroke_path` 都在**(§4)。
   而且**在我做这个 spike 的同时,主线已经把 `Painter::fill_path` 与
   `stroke_path` 落地了**(提交 `7966785` / `3ebe81c`)——调研 26 点名的"步骤 0"
   已经不是风险项。
4. **量级:比"lottie 很重"的直觉便宜一个量级。** 典型 UI 图标档(64×64)纯 CPU
   **0.35 ms/帧**;1024×1024 大图 CPU 29 ms、GPU 7.7 ms;一个 57 KB 的 lottie
   解析后常驻 **约 1 MB**(§5)。

**但是,最大的意外发现不在上面四条,而是这个:**

> **velato 0.11 的 `RenderSink` trait 让 lottie 与 vello 彻底解耦。**
> `velato = { version = "0.11", default-features = false }` 的完整依赖树里
> **没有 vello、没有 wgpu、没有 naga**,只有 kurbo + peniko + serde(§2.3)。
> 也就是说 **lottie 不是"GPU 特性",CPU 默认后端可以原生支持它。**
> 这一条直接推翻了"lottie 要等 vello 成为默认后端"的隐含前提。

**真正的卡点只有一条,而且不是依赖问题,是健壮性问题**(§6.1):velato 在**合法
Lottie 输入**上 `todo!()` panic 而不是返回 `Err`。我独立复现了三种触发方式。
好消息:`catch_unwind` 能接住(实测),仓库也没开 `panic = "abort"`。

---

## 1. 实验环境与复现方式

四个临时项目,都在 `C:\Users\DELL\AppData\Local\Temp\claude\` 下:

| 目录 | 用途 |
| --- | --- |
| `lottie-spike\` | 主实验体。velato + vello 0.9 + tiny-skia + wgpu 全都在 |
| `lottie-spike-nogpu\` | `velato default-features = false`,验证"无 vello 也能跑" |
| `lottie-spike-vellocpu\` | velato + `vello_cpu 0.0.9` 共存性(只做依赖解析) |
| `velato-only\` | 只依赖 velato,量构建时间 |

主实验体源码:

- `C:\Users\DELL\AppData\Local\Temp\claude\lottie-spike\src\main.rs`
  —— 子命令 `parse` / `cpu` / `vello` / `path` / `bench` / `mem` / `robust`
- `C:\Users\DELL\AppData\Local\Temp\claude\lottie-spike\src\tsk_sink.rs`
  —— **`impl velato::RenderSink for TinySkiaSink`**,约 280 行,是整个 spike 的核心

资产(`lottie-spike\assets\`):

| 文件 | 来源 | 规格 |
| --- | --- | --- |
| `rect-slide.json` | **我手写的**(圆角矩形 60 帧从左移到右,带描边) | 1 KB / 1 层 / 400×200 |
| `PolyStarTest.json` | velato 仓库 `examples/assets/` | 2 KB / 2 层 / 512×256 |
| `Tiger.json` | velato 仓库 `examples/assets/google_fonts/` | 418 KB / 14 层 / 1024×1024 |
| `noto/*.json`(12 个) | Google Noto 动画 emoji(velato CI 用的同一批,`fonts.gstatic.com`) | 8–132 KB / 1024×1024 |
| `bad-*.json` | **我构造的畸形/边界输入**(见 §6.1) | —— |

选 Noto emoji 不是随便选的:那批就是"UI 里的小动画图标",与 sv-arco 的实际用法同构。

---

## 2. 实验 1:velato 能不能与 vello 0.9 共存

### 2.1 先核实版本事实(crates.io API 原始返回)

```
GET https://crates.io/api/v1/crates/velato
  max_version: 0.11.0
  0.11.0 - 2026-07-21     0.10.0 - 2026-05-01     0.9.0 - 2026-01-18
  0.8.1  - 2025-12-23     0.8.0  - 2025-12-08     0.7.0 - 2025-10-12

GET https://crates.io/api/v1/crates/velato/0.11.0/dependencies
  kurbo       ^0.13.0   normal  必需
  peniko      ^0.6.0    normal  必需
  serde       ^1.0.228  normal  必需
  serde_json  ^1.0.149  normal  必需
  serde_repr  ^0.1.20   normal  必需
  vello       ^0.9.0    normal  **optional**
  vello       ^0.9.0    dev

GET https://api.github.com/repos/linebender/velato
  pushed_at: 2026-07-21T12:28:49Z    stars: 152    open_issues: 6    archived: false
```

velato README 自带兼容矩阵,同样是一手证据:

```
| velato     | vello |
| main, 0.11 | 0.9   |
| 0.10       | 0.7   |
| 0.9        | 0.7   |
```

本仓库 `Cargo.lock`:`vello 0.9.0` / `peniko 0.6.1` / `kurbo 0.13.1` / `wgpu 29.0.4`。
**velato 0.11 是唯一与我们对得上的版本,而它恰好是最新版,且昨天刚发。**

> 顺带一提:这是运气,不是必然。velato 0.10 配 vello 0.7 —— 如果这个 spike 早两个
> 月做,结论就是"版本对不上,要么降 vello 要么等"。**velato 的发布节奏跟 vello 走,
> 我们每次升 vello 都要重新对一次这张表**,这是长期税。

### 2.2 实测依赖树

`lottie-spike/Cargo.toml` 同时钉死四个:

```toml
vello = "0.9.0"
tiny-skia = "0.11.4"
wgpu = "29.0.4"
velato = { version = "0.11.0", features = ["vello"] }
```

```
$ cargo tree -i vello
vello v0.9.0
├── lottie-spike v0.1.0
└── velato v0.11.0
    └── lottie-spike v0.1.0

$ cargo tree -i peniko
peniko v0.6.1
├── velato v0.11.0
├── vello v0.9.0
└── vello_encoding v0.9.0

$ cargo tree -i kurbo
kurbo v0.13.1
├── peniko v0.6.1
└── velato v0.11.0
```

**单版本,共享。** `cargo tree -d` 的重复项清单里没有任何图形栈 crate:

```
$ cargo tree -d | grep -E '^[a-z_-]+ v' | sort -u
bitflags v1.3.2 / v2.13.1     foldhash v0.1.5 / v0.2.0
hashbrown v0.15.5/0.16.1/0.17.1               naga v29.0.4
once_cell v1.21.4             png v0.17.16 / v0.18.1
syn v2.0.119 / v3.0.3
```

`png` 双份是 **tiny-skia 0.11.4 → png 0.17** 撞 **vello 0.9 → png 0.18** 造成的,
与 velato 无关;本仓库 `Cargo.lock` 里**现在就已经有这两份**(第 2240 行与 2253 行)。

**结论:能。没有版本冲突,不是死穴。**

### 2.3 意外发现:velato 可以完全不带 vello

velato 0.11 的 `default = ["vello"]`,但 vello 是 `optional`。关掉之后:

```
$ cd lottie-spike-nogpu    # velato = { version = "0.11.0", default-features = false }
$ cargo tree
lottie-spike-nogpu
├── kurbo v0.13.1 → arrayvec, polycool, smallvec
├── peniko v0.6.1 → color v0.3.3, kurbo, linebender_resource_handle, smallvec
├── tiny-skia v0.11.4
└── velato v0.11.0 → kurbo, peniko, serde, serde_json, serde_repr

$ cargo tree | grep -iE "vello|wgpu|naga"
(无输出)
```

支撑它的是 velato 0.11 新增的 `RenderSink` trait
(`velato-0.11.0/src/runtime/render.rs:14`,**四个必需方法**):

```rust
pub trait RenderSink {
    fn push_layer(&mut self, blend: impl Into<peniko::BlendMode>, alpha: f32,
                  transform: Affine, shape: &impl kurbo::Shape);
    fn push_clip_layer(&mut self, transform: Affine, shape: &impl kurbo::Shape);
    fn pop_layer(&mut self);
    fn draw(&mut self, stroke: Option<&kurbo::Stroke>, transform: Affine,
            brush: &peniko::Brush, shape: &impl kurbo::Shape);
    // 两个有默认实现的钩子:begin_layer_group / end_layer_group
}
```

`vello::Scene` 只是它的**一个实现**(`runtime/vello.rs:9`,整个文件 56 行)。
README 首句就写着:*"Render with the (optional) built-in Vello integration, or
implement the `RenderSink` trait to bring your own renderer."* —— 我这个 spike 走的
正是官方设想的第二条路。

**这一条改变了 lottie 在路线图里的位置**:它不再是"vello 专属能力",CPU 默认后端
可以原生吃下。

### 2.4 附带核实:velato + vello_cpu 也不冲突

ADR-3b 写了"兜底从 tiny-skia 迁 vello_cpu"。顺手验证这条路不会被 lottie 堵死:

```
$ cd lottie-spike-vellocpu    # velato 0.11 (no default) + vello_cpu 0.0.9
$ cargo tree -i peniko
peniko v0.6.1
├── glifo v0.1.1 → vello_cpu v0.0.9
├── velato v0.11.0
└── vello_common v0.0.9
$ cargo tree -d | grep -E '^[a-z_-]+ v'
syn v2.0.119 / syn v3.0.3     ← 只有 proc-macro 层重复
```

`vello_cpu 0.0.9`(2026-05-30 发布,仍是 0.0.x)→ `vello_common 0.0.9` → `peniko ^0.6.1`,
与 velato 同族。**只是 velato 没有为 vello_cpu 的场景类型实现 `RenderSink`,那部分要自己写。**

---

## 3. 实验 2:能不能离屏渲染出一帧

### 3.1 手写一个最小 lottie 先验证链路

`assets/rect-slide.json`(我写的):400×200,60fps,一个带 12px 圆角 + 4px 描边的
80×60 矩形,`ks.p` 两个关键帧从 `[60,100]` 到 `[340,100]`,带缓动手柄。

velato 的 serde schema 相当严格(`schema/` 目录 60+ 文件),第一次就过了:

```
$ ./lottie-spike parse assets/rect-slide.json
[parse] assets/rect-slide.json  1 KB  ->  500.2µs  layers=1  400x200  frames=0..60  fps=60
```

### 3.2 CPU 档:自建 tiny-skia RenderSink

`src/tsk_sink.rs` 的核心映射:

| velato 给的 | 我怎么落到 tiny-skia |
| --- | --- |
| `&impl kurbo::Shape` | `shape.path_elements(0.1)` → `PathBuilder`(Move/Line/Quad/Cubic/Close 五个命令一一对应,零缺口) |
| `kurbo::Affine` | `Transform::from_row(a,b,c,d,e,f)`,`as_coeffs()` 顺序直接对上 |
| `peniko::Brush::Solid` | `Paint::set_color` |
| `peniko::Brush::Gradient` | `LinearGradient::new` / `RadialGradient::new` + 色标 |
| `Option<&kurbo::Stroke>` | `tiny_skia::Stroke { width, cap, join, miter_limit }` |
| `push_layer(blend, alpha, ...)` | 新开一张 `Pixmap`,pop 时 `draw_pixmap` 带 `PixmapPaint{opacity, blend_mode}` |
| `push_clip_layer` | `tiny_skia::Mask` + 与父层逐像素求交 |

跑通:

```
$ ./lottie-spike cpu assets/rect-slide.json 0  out/cpu-rect-0.png
[cpu] frame=0  400x200  677.7µs  fills=1 strokes=1 skipped_brush=0 peak_layers=2  非白像素=5240
$ ./lottie-spike cpu assets/rect-slide.json 30 out/cpu-rect-30.png
[cpu] frame=30 400x200  620.2µs  fills=1 strokes=1 skipped_brush=0 peak_layers=2  非白像素=5240
$ ./lottie-spike cpu assets/rect-slide.json 59 out/cpu-rect-59.png
[cpu] frame=59 400x200  627.2µs  fills=1 strokes=1 skipped_brush=0 peak_layers=2  非白像素=5242
```

三张 PNG 各 400×200,矩形分别在左 / 中 / 右 —— **动画插值是对的**(我逐张看了图)。

复杂资产也一次过:

```
$ ./lottie-spike cpu assets/Tiger.json 60 out/cpu-tiger.png
[parse] 418 KB -> 17.07ms  layers=14  1024x1024  frames=0..306
[cpu] frame=60 1024x1024  34.55ms  fills=47 strokes=0 skipped_brush=0 peak_layers=4  非白像素=351051
```

产物 1024×1024 / 113 KB,是一只完整正确的老虎(渐变、多层、47 个填充全在)。

**12 个 Noto emoji 全部渲染成功,零 skipped brush**:

```
$ for f in assets/noto/*.json; do ./lottie-spike cpu $f 20 out/noto-$(basename $f .json).png; done
[cpu] 1f44d 16.83ms fills=7  strokes=0    [cpu] 1f525 31.20ms fills=7  strokes=1
[cpu] 1f600 30.05ms fills=6  strokes=2    [cpu] 1f602 31.01ms fills=15 strokes=0
[cpu] 1f60d 63.29ms fills=22 strokes=0    [cpu] 1f62d 28.27ms fills=16 strokes=4
[cpu] 1f634 27.70ms fills=8  strokes=0    [cpu] 1f923 39.81ms fills=16 strokes=0
[cpu] 1f970 67.83ms fills=29 strokes=0    [cpu] 1f973 38.11ms fills=26 strokes=0
[cpu] 1fae0 35.82ms fills=10 strokes=0    [cpu] 2764  1.08ms  fills=5  strokes=0
```

(这些都是 1024×1024 原生尺寸;真实 UI 里按 64px 用,见 §5.1。)

### 3.3 GPU 档:velato → vello::Scene → wgpu 离屏

按 `crates/sv-shell/src/vello_backend.rs` 的现成配方(自建 device 抬高
`max_storage_buffer_binding_size`、`render_to_texture` + `copy_texture_to_buffer` 回读):

```
$ ./lottie-spike vello assets/rect-slide.json 30 out/vello-rect-30.png
[vello] adapter: Intel(R) UHD Graphics 730 (IntegratedGpu, Vulkan)
[vello] frame=30 400x200  scene构建=56.4µs  GPU渲染+回读=1.78s  非白像素=5240

$ ./lottie-spike vello assets/Tiger.json 60 out/vello-tiger.png
[vello] frame=60 1024x1024  scene构建=89.3µs  GPU渲染+回读=514.5ms  非白像素=345786
```

那个 1.78 s / 514 ms 是 **device 创建 + 管线编译的一次性开销**,不是帧耗时;
稳态见 §5.1(7.7 ms/帧)。`scene构建=56–89 µs` 才是 velato 本身的代价。

### 3.4 CPU / GPU 对拍(15 个资产,frame=20)

| 资产 | CPU 非白像素 | GPU 非白像素 | 比值 |
| --- | ---: | ---: | ---: |
| rect-slide | 5 284 | 5 288 | 0.9992 |
| PolyStarTest | 16 819 | 16 894 | 0.9956 |
| Tiger | 352 139 | 352 373 | 0.9993 |
| noto 1f44d | 457 224 | 457 377 | 0.9997 |
| noto 1f525 | 425 825 | 425 942 | 0.9997 |
| noto 1f600 | 665 513 | 665 512 | 1.0000 |
| noto 1f602 | 639 152 | 639 335 | 0.9997 |
| noto 1f60d | 555 026 | 555 017 | 1.0000 |
| noto 1f62d | 636 397 | 636 644 | 0.9996 |
| noto 1f634 | 691 131 | 691 247 | 0.9998 |
| noto 1f923 | 640 023 | 640 211 | 0.9997 |
| noto 1f970 | 699 166 | 699 229 | 0.9999 |
| noto 1f973 | 579 646 | 579 776 | 0.9998 |
| noto 1fae0 | 533 590 | 533 685 | 0.9998 |
| noto 2764 | 365 313 | 365 374 | 0.9998 |

口径与 ADR-3b 里那条"GPU/CPU 非白像素比 1.001"一致。**0.9956–1.0000,比现有几何
渲染的 parity 带(1.001–1.017)还紧。** 差值全在抗锯齿边缘。

**结论:能。两条路都能离屏渲染出一帧,且互相对得上。**

---

## 4. 实验 3:tiny-skia 路径填充

这是 lottie 与 SVG 图标共享的地基,单独验一遍。`./lottie-spike path out/tinyskia-path.png`
画一条**故意难看**的路径(两段三次贝塞尔 + 一段二次贝塞尔闭合,带自交与尖角),
`FillRule::EvenOdd` 填充后再 3px 圆角描边:

```rust
let mut pb = tiny_skia::PathBuilder::new();
pb.move_to(20.0, 170.0);
pb.cubic_to(40.0, 20.0, 140.0, 20.0, 150.0, 120.0);
pb.cubic_to(160.0, 20.0, 260.0, 20.0, 280.0, 170.0);
pb.quad_to(150.0, 100.0, 20.0, 170.0);
pb.close();
pm.fill_path(&path, &paint, FillRule::EvenOdd, Transform::identity(), None);
pm.stroke_path(&path, &spaint, &Stroke { width: 3.0, line_join: LineJoin::Round, .. }, ..);
```

```
[path] 300x200 -> out/tinyskia-path.png  非白像素=20119  (EvenOdd 填充 + 3px 描边)
```

产物 300×200 / 12 350 字节,填充与描边都正确(尖角、EvenOdd 空洞、圆角折点都对)。

**tiny-skia 0.11.4 的 `Pixmap::fill_path` / `stroke_path` 本来就是完整的任意路径
API**,签名 `(path, paint, fill_rule, transform, mask)`。而且读源码确认:非单位变换
时它会 `path.transform(t)` **同时** `paint.shader.transform(t)`
(`tiny-skia-0.11.4/src/painter.rs:305–312`)—— 渐变笔刷跟着路径一起变换,不用自己补。

**结论:能,而且从来就没缺过。** 缺的是**我们自己 `Painter` trait 上的动词** ——
而这一条**主线已经在我做 spike 期间补上了**:

```
$ git log --oneline -3 -- crates/sv-shell/src/paint.rs
3ebe81c feat(painter): 补 stroke_path——路径动词齐活(lottie/SVG 图标的"步骤 0")
7966785 feat(adr-2 ②): 模板数据面落地(Template/stamp)+ Painter::fill_path
```

```rust
// crates/sv-shell/src/paint.rs 当前
fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color);
fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color);
```

**调研 26 说的"最大风险是需要路线图外新增 fill_path"—— 这条风险已经出账了。**

---

## 5. 实验 4:量级

### 5.1 单帧耗时 vs 渲染尺寸

Noto `1f602`(「笑哭」,57 KB,15 个填充)—— 最像"真实 UI 图标动画"的样本,
`bench` 跑 60 帧取平均(GPU 档预热一帧不计):

| 渲染尺寸 | CPU(tiny-skia) | GPU(vello,含每帧同步回读) |
| --- | ---: | ---: |
| 1024×1024 | 39.8 ms | 5.72 ms |
| 256×256 | 2.48 ms | 1.06 ms |
| 128×128 | 0.896 ms | 0.808 ms |
| **64×64** | **0.345 ms** | 0.698 ms |
| 32×32 | 0.161 ms | 0.721 ms |

Tiger(14 层 / 397 个 path segment,复杂度上限样本):

| 渲染尺寸 | velato→Scene 编码 | CPU 光栅 | GPU 渲染+回读 |
| --- | ---: | ---: | ---: |
| 1024×1024 | 14.4 µs | 29.0 ms | 7.67 ms |
| 256×256 | 16.0 µs | 2.69 ms | 1.30 ms |
| 64×64 | 13.7 µs | 0.398 ms | 0.769 ms |

三个必须说清楚的口径问题:

1. **GPU 档那条 ~0.7 ms 的地板不是 GPU 算力,是我每帧 `device.poll(wait)` 强同步
   回读的往返成本。** 真实开窗路径不做这个同步,GPU 的真实增量会更低。所以
   "128px 以下 CPU 比 GPU 快"这个结论**只在离屏对拍口径下成立**,别拿去做架构决策。
2. **`velato → vello::Scene` 的编码只要 13–23 µs,与尺寸无关。** 这是 GPU 后端上
   lottie 的**真实边际成本** —— 场景合并进现有 `Scene` 里,GPU 那一趟本来就要跑。
   **一个 lottie 在 vello 后端上几乎是免费的。**
3. **CPU 耗时随像素面积线性增长**(1024→256 是 16 倍面积,耗时 16.1 倍),
   符合软件光栅的预期。

### 5.2 CPU 档一半的开销是"图层缓冲",而且可以去掉

朴素实现下,`velato::Renderer::append` **每帧开头必发一次覆盖整幅的
`push_clip_layer`**(`render.rs:68`),我照实新开了一张全尺寸 `Pixmap`。
量一下纯缓冲开销(建 4 层 + 合成,零绘制):

```
1024×1024:  18.69 ms/帧     ← 占总 37.7 ms 的一半
256×256:     1.40 ms/帧
64×64:      81.3 µs/帧
```

加一个 8 行的判断(裁剪形状是轴对齐矩形且已盖住画布 → 不分配,直接透传):

```
                      朴素      透传优化    降幅
1024×1024 总耗时     37.7 ms    29.0 ms    -23%
1024 纯缓冲开销      18.69 ms   0.013 ms   -99.9%
256×256  总耗时       3.45 ms    2.69 ms   -22%
peak_layers            4          3
```

产物像素级对比:**4 194 304 字节里 6 876 字节不同(0.164%),最大通道差 = 2** ——
肉眼无差,差值来自多走一次预乘往返的舍入。

**结论:CPU 档剩下的 29 ms 里,还有相当一部分是"每层都开全画布 Pixmap"造成的。
真做的话要上图层 Pixmap 池 + 按图层包围盒开小图,不要照抄我的 spike。**

### 5.3 内存

Windows `K32GetProcessMemoryInfo` 在进程内分阶段采样(不是事后读 `Process` 对象,
那样只会得到 0):

**Tiger.json(418 KB,1024×1024)**

```
0 进程基线                                工作集   7.0 MB   峰值   7.0 MB
1 JSON 文本入内存(418 KB)                工作集   7.5 MB   峰值   7.5 MB
2 Composition 解析完成(已丢 JSON 文本)   工作集   9.7 MB   峰值  14.0 MB
3 CPU 光栅一帧 1024×1024(峰值 4 层)     工作集  13.9 MB   峰值  25.0 MB
4 释放 CPU pixmap                         工作集   9.9 MB   峰值  25.0 MB
5 velato → vello::Scene(仅编码)         工作集  10.0 MB   峰值  25.0 MB
6 vello GPU 渲染 + 回读(含 device/管线) 工作集 302.0 MB   峰值 610.1 MB
```

**Noto emoji**

```
1f602(57 KB)   解析后 +1.0 MB   1024² 光栅峰值 23.0 MB
1fae0(132 KB)  解析后 +1.1 MB   1024² 光栅峰值 22.8 MB
2764(8 KB)     解析后 +0.7 MB    800² 光栅峰值 13.6 MB
rect-slide(1 KB) 解析后 +0.6 MB
```

读法:

- **一个典型 UI lottie 常驻约 1 MB**(解析后的 `Composition`),与 JSON 大小的相关性
  不强(8 KB 和 132 KB 的资产都在 0.7–1.1 MB),说明有固定开销地板。**十几个图标
  动画同时驻留 ≈ 10 MB 量级,可接受。**
- **第 6 步那 302 MB / 610 MB 峰值不能记在 lottie 账上** —— 那是 wgpu device +
  vello 管线编译的固定成本,本仓库开 `backend-vello` 时**现在就已经在付**。
- CPU 光栅的峰值几乎全是全尺寸 `Pixmap`(1024²×4B = 4 MB/层);按 §5.2 优化后
  这块会明显缩。

### 5.4 构建成本

```
$ cd velato-only && cargo clean && time cargo build --release
Finished `release` profile in 20.33s          # velato(无 vello)+ 25 个 crate

$ cd lottie-spike-nogpu && cargo clean && time cargo build --release
Finished `release` profile in 18.29s          # 上面 + tiny-skia(仓库已有)
```

velato(无 vello)给 CPU 路径新引入的 crate:velato / kurbo / peniko / color /
linebender_resource_handle / polycool / serde / serde_core / serde_derive /
serde_json / serde_repr / itoa / memchr / zmij(smallvec / arrayvec / syn 等仓库已有)。
**≈ 20 s 干净构建增量。** 注意:**这是仓库第一次引入 serde/serde_json**
(`Cargo.lock` 里 `serde_json` 现在计数为 0)。

### 5.5 velato 自己的 rustc 门槛

velato 0.11 的 `Cargo.toml`:`edition = "2024"`,`rust-version = "1.88"`。
本机 rustc 1.88.0 **正好卡在下限上**。这是需要记一笔的约束(仓库
`rust-version.workspace` 若低于 1.88 就要抬)。

---

## 6. 卡点

### 6.1 【真卡点】velato 在合法 Lottie 上 panic,不返回 Err

这是整个 spike 里唯一让我停下来的东西。`velato::Error` 枚举**只有一个变体**
(`Json(serde_json::Error)`),所有"不支持的特性"走的是 `todo!()` / `unimplemented!()`。
源码里数得出来:

```
$ grep -rn "todo!\|unimplemented!" velato-0.11.0/src --include=*.rs | wc -l
8
velato-0.11.0/src/import/converters.rs:103   unimplemented!("asset {:?} is not yet implemented")
velato-0.11.0/src/import/converters.rs:211   AnyTransformR::SplitRotation { .. } => todo!()
velato-0.11.0/src/import/converters.rs:213   None => todo!("split rotation")
velato-0.11.0/src/import/converters.rs:243   todo!("split rotation")
velato-0.11.0/src/import/converters.rs:252   todo!("split position")
velato-0.11.0/src/import/converters.rs:805   Add => unimplemented!()
velato-0.11.0/src/import/converters.rs:806   HardMix => unimplemented!()
```

我自己造输入复现(**不是引用别人的结论,是我改 JSON 跑出来的**):

| 我做的改动 | 结果 |
| --- | --- |
| 从图层 `ks` 里**删掉 `r`(旋转)键** | **PANIC** `converters.rs:213` |
| 把 `r` 换成 `rx`/`ry`/`rz`(拆分旋转) | **PANIC** `converters.rs:213` |
| 图层 `bm: 16`(Add 混合模式) | **PANIC** `converters.rs:805` |
| 位置改成 `{"s":true,"x":...,"y":...}` | OK(未触发,原因未查明) |
| JSON 截断到 200 字节 | 正常 `Err`:`EOF while parsing a value at line 16` |

**"删掉 `r`"不是恶意输入 —— 那是所有 Lottie 优化器(lottie-web 的压缩、
LottieFiles 的 optimizer)对"旋转恒为 0"图层的常规处理。** 资产是设计师给的、
可能过工具链,这等于把第三方数据接到进程存活性上。

**兜底可行,我实测了。** `robust` 子命令把解析和渲染都包进 `catch_unwind`:

```
$ ./lottie-spike robust assets
  OK        PolyStarTest.json          OK        Tiger.json
  解析PANIC bad-bm-add.json  (catch_unwind 已接住)
  解析PANIC bad-no-r.json    (catch_unwind 已接住)
  解析PANIC bad-split-rot.json (catch_unwind 已接住)
  Err       bad-truncated.json (Error parsing lottie: EOF while parsing a value)
  OK        bad-split-pos.json         OK        rect-slide.json
[robust] 共 9 个:OK=4 Err=2 PANIC=3

$ ./lottie-spike robust assets/noto
[robust] 共 12 个:OK=12 Err=0 PANIC=0
```

12 个真实 Noto 资产 **0 panic**;3 个我故意造的畸形输入全被 `catch_unwind` 接住。
仓库 `Cargo.toml` 没设 `panic = "abort"`(默认 unwind),这条兜底成立。
**但要写进约束:一旦哪天为了体积开了 `panic = "abort"`,这个兜底就失效。**

### 6.2 【我踩到的坑】把 `peniko::BlendMode` 的 compose 半边丢掉 → track matte 渲染成不透明背景

第一版 `mix_to_ts` 我图省事写成:

```rust
match blend.compose {
    Compose::SrcOver | Compose::Copy => {}
    _ => return tiny_skia::BlendMode::SourceOver,   // ← 这里
}
```

结果 Noto 火焰 `1f525` 渲染成"整幅径向渐变背景 + 一个浅黄色火焰"—— **非白像素
1 046 528 / 1 048 576,整张画布都被填满了**,而 vello 渲染同一帧是 425 942。

打开调试才看清:

```
$ SPIKE_DEBUG=1 ./lottie-spike cpu assets/noto/1f525.json 20 ...
  push_layer blend: mix=Normal compose=SrcOver
  push_layer blend: mix=Normal compose=SrcIn      ← track matte
```

velato 处理 lottie 的 track matte(`tt` 字段)时,**把 matte 模式编在
`BlendMode.compose`(Porter-Duff)里,而不是 `mix`**(`render.rs:109–127`)。
丢掉 compose = 遮罩层被当成普通图层画成背景。

修法很短(tiny-skia 的 `BlendMode` 有完整 Porter-Duff 集:`SourceIn`/
`DestinationIn`/`SourceOut`/`Xor`/`Plus`…),补上之后:

```
[cpu] 1f525 frame=20  非白像素=425825      vs   vello 425942     比值 0.9997
```

**教训**:`peniko::BlendMode` 是 `{ mix, compose }` 二元组,**两半都得映射**;
只看 `mix` 会在真实资产上静默画错(而不是报错)。这是"看起来对了 90%,剩下 10%
最刺眼"的典型。写进任何未来的 `Painter::push_layer` 设计里。

### 6.3 【设计接缝】`RenderSink` 不是 dyn-compatible

`push_layer(blend: impl Into<BlendMode>, ..., shape: &impl kurbo::Shape)` 有
`impl Trait` 参数 → **trait object 不可用**。而本仓库 `paint_tree` 的签名是
`painter: &mut dyn Painter`(`render.rs:664`)。

不是障碍,但要多一层:写一个**具体类型**的适配器
`struct SinkAdapter<'a> { p: &'a mut dyn Painter }`,`impl RenderSink for SinkAdapter<'_>`,
泛型方法内部把 `shape.path_elements()` 摊成 `Vec<PathCmd>` 再喂给 `dyn Painter`。
单态化只发生在适配器内部,`dyn` 边界不上浮 —— 与 ADR-3b 的"dyn 只收在 sv-shell
边界内"是相容的。

### 6.4 【小坑】velato README 的示例代码是过期的

README 写 `renderer.render(&composition, frame, transform, alpha, &mut scene)`,
但 0.11 的公开 API 只有:

```
runtime/render.rs:54   pub fn Renderer::new()
runtime/render.rs:59   pub fn Renderer::append(&mut self, animation, frame, transform, alpha, scene: &mut impl RenderSink)
runtime/vello.rs:45    pub fn Renderer::render_to_vello_scene(...) -> vello::Scene    // 仅 vello feature
runtime/mod.rs:38      pub fn Composition::from_slice / from_json
```

**没有 `render`。** `lib.rs` 顶部的 doctest 是对的(用 `render_to_vello_scene`),
README 没跟上。照 README 抄会编译失败 —— 别信 README,信 `docs.rs` / 源码。

### 6.5 【非阻塞】velato 官方声明的能力缺口

`lib.rs` 顶部原文列的 missing features:

> Position keyframe (`ti`, `to`) easing / Time remapping (`tm`) / **Text** /
> **Image embedding** / Advanced shapes (stroke dash, zig-zag, etc.) /
> Advanced effects (motion blur, drop shadows, etc.) / Correct color stop handling /
> Split rotations / Split positions

对 UI 图标动画这批(12/12 全过)不构成阻塞;对"设计师给的任意 lottie"是硬上限。
注意 `Correct color stop handling` 是官方自认有问题的项 —— 我的渐变对拍没暴露它,
但那只说明我的样本没踩到。

---

## 7. 对照当前 `Painter`:还差什么动词

主线补完 `fill_path`/`stroke_path` 之后,拿 velato 的 `RenderSink` 四个方法逐条比:

| velato 要的 | 当前 `Painter` | 差距 |
| --- | --- | --- |
| `draw(None, transform, Brush::Solid, shape)` | `fill_path(&[PathCmd], PathFill, Color)` | **变换要上层烘焙进坐标**(可行) |
| `draw(Some(stroke), transform, ...)` | `stroke_path(&[PathCmd], &StrokeStyle, Color)` | 同上 + **线宽要跟着变换缩放** |
| `draw(_, _, Brush::Gradient, _)` | 只有 `color: Color` | **缺渐变笔刷** |
| `push_clip_layer(transform, shape)` | `push_clip(x,y,w,h,radius)` 只吃矩形 | **缺任意路径裁剪** |
| `push_layer(blend, alpha, transform, shape)` | 无 | **缺图层(alpha + Porter-Duff/Mix)** |
| `pop_layer` | `pop_clip` | 语义不同(pop_layer 要合成) |

三条具体的、有实测支撑的补充说明:

1. **"没有 transform 参数"对填充无害,对描边有害。** 把仿射烘焙进 `PathCmd` 坐标后,
   `StrokeStyle.width` 是标量,没法表达非均匀缩放/斜切下的椭圆笔。
   **我量了真实资产**(`SPIKE_DEBUG=1` 打印每次描边的变换分解):

   ```
   1f600  stroke w=10.000  sx=1.0000 sy=1.0000  各向异性=1.0000  斜切=0.0000
   1f62d  stroke w= 5.000  sx=1.5292 sy=1.5292  各向异性=1.0000  斜切=0.0000
   1f62d  stroke w=39.979  sx=1.5292 sy=1.5292  各向异性=1.0000  斜切=0.0000
   1f525  stroke w= 8.000  sx=1.6391 sy=1.6391  各向异性=1.0000  斜切=0.0000
   ```

   **抽样到的全是各向同性缩放**(各向异性恒 1.0,斜切恒 0)→ `width * sx` 就够。
   非均匀的情况我**没抽到**,不代表不存在(§9)。真遇到就用 kurbo 把描边展开成填充。

2. **渐变笔刷不是可选项。** 12 个 Noto 资产里 `1f525`(火焰)是渐变 + track matte;
   Tiger 的躯干也是渐变。只支持纯色的话,这类资产会视觉塌陷(而不是报错)。

3. **图层是 §6.2 那个坑的正主。** `push_layer` 的签名里必须带 **compose(Porter-Duff)
   而不只是 mix**,否则 track matte 静默画错。

---

## 8. 给主线的建议

### 8.1 可行性判决

> **可行,而且比 DESIGN.md 现有风险描述里假定的便宜。**
> 依赖冲突这条被排除了(§2);"步骤 0 缺 fill_path"这条主线已经出账了(§4);
> 真实成本落在"图层/渐变/裁剪三个动词"和"velato 的 panic 兜底"上,都是有界工作量。

### 8.2 三条实验事实,建议直接写进决策

1. **lottie 不是 GPU 特性。** velato 关掉 `vello` feature 后依赖树里连 wgpu 都没有
   (§2.3)。**默认 CPU 后端就能支持 lottie**,不必等 vello 成为默认。这一条应当
   修正"lottie ⊂ vello 路线"的隐含假设。
2. **在 vello 后端上,lottie 的边际成本约 15 µs/帧**(场景编码,与分辨率无关,§5.1)。
   合进现有 `Scene` 后 GPU 那一趟本来就要跑。**GPU 档几乎免费。**
3. **在 CPU 后端上,64px 图标档 0.35 ms/帧,可接受;1024px 全屏档 29 ms/帧,不可接受。**
   分档能力协商(`PainterCaps` 已有这个先例)比"CPU 一律不支持"更合适。

### 8.3 建议的落地顺序(按"能解锁多少真实资产"排,不是按难度)

| 序 | 动作 | 解锁什么 | 依据 |
| --- | --- | --- | --- |
| 0 | ~~`Painter::fill_path` / `stroke_path`~~ | —— | **已完成**(`7966785` / `3ebe81c`) |
| 1 | `catch_unwind` 包住 velato 的解析与渲染 | 把"第三方资产 panic 掉进程"降级为"这个动画不显示" | §6.1 实测 3/3 接住 |
| 2 | `push_layer(alpha, BlendMode{mix,compose}) / pop_layer` | track matte、图层不透明度 —— 真实资产必需 | §6.2 踩坑实证 |
| 3 | 任意路径裁剪(`push_clip_path`) | mask 图层 | §7 |
| 4 | 渐变笔刷(线性/径向 + 色标 + extend) | 火焰/躯干这类资产 | §5、§7 |
| 5 | 图层 Pixmap 池 + 按包围盒开小图 | CPU 档 -23%~-50% | §5.2 实测 |
| 6 | 脏矩形 | 不记在 lottie 账上,但 lottie 会第一个逼出它 | 见下 |

第 6 条要单独说:**一个 lottie 在跑 = 每帧整窗重绘**。§5 里所有 CPU 数字都只是
lottie 自己那一块,窗口其余部分的重绘没算。这是全仓欠账,lottie 只是第一个把它
逼出来的特性。

### 8.4 不建议做的

- **不建议为 lottie 引入第二套图形类型体系。** 主线 `PathCmd` 刻意不借 kurbo 的
  判断(paint.rs 注释里写得很清楚:vello 是 optional,接口不能依赖只在某 feature 下
  存在的类型)是对的。适配器里做 `kurbo::Shape → &[PathCmd]` 的摊平即可,
  `path_elements()` 的五种命令与 `PathCmd` 一一对应,**零语义损失**(§3.2)。
- **不建议现在就上 `vello_cpu`。** 它还是 0.0.9(2026-05-30),而且 velato 没为它
  实现 `RenderSink`,要自己写 —— 那和给 tiny-skia 写一个是同样的工作量,却多担一份
  0.0.x 的不稳定。tiny-skia 这条路今天就通(§3.2)。
- **不建议把 velato 直接暴露给用户 API。** 它的 `todo!()` 分布(§6.1)和"README
  过期"(§6.4)说明它还在快速演进期;中间隔一层我们自己的 `sv-lottie` 门面,
  将来换实现(rlottie / ThorVG / 自研)不动上层。

---

## 9. 我没能验证的部分

按"影响决策的程度"排:

1. **窗口路径的真实帧率。** 任务约束不跑开窗程序,所有数字都是离屏 + 每帧强同步
   回读。GPU 档那条 ~0.7 ms 的地板**几乎肯定是同步开销而不是 GPU 工作量**,
   真实开窗会更好,但**好多少我没测**。
2. **接进本仓库 `Painter` 之后的实际数字。** 我的 spike 直接调 tiny-skia,没有经过
   `&mut dyn Painter` 和 `PathCmd` 摊平这两层。摊平会多一次 `Vec<PathCmd>` 分配
   (可池化),`dyn` 调用按 ADR-3b 的口径是个位数 µs —— **但这是推断,不是实测**。
3. **非均匀缩放/斜切下的描边。** §7 抽样到的真实资产全是各向同性(各向异性恒 1.0)。
   **我没有找到反例样本,也就没能验证"标量 width 不够用"这个担心是否会真的发生。**
4. **`bad-split-pos.json` 为什么没触发 `todo!("split position")`**(converters.rs:252)。
   我构造的拆分位置 JSON 被正常接受了,没查明是我的 JSON 形状不对还是那条路径确实
   没走到。**这意味着 §6.1 的 panic 清单可能不完整。**
5. **velato 的能力缺口对"设计师给的任意 lottie"的真实通过率。** 我的样本是 15 个,
   其中 12 个来自同一来源(Google Noto,风格高度同质:纯形状图层、无文本、无图片)。
   **这不是一个有代表性的抽样。** 真实设计交付里文本层/图片层/表达式的比例我没有数据。
6. **鸿蒙/Linux/macOS 上的表现。** 全部实验只在 Windows + Intel 集显上跑过。
   velato 本身是纯计算无平台代码(依赖里没有任何 sys crate),**理论上无平台风险,
   但没验证**。
7. **长时间播放的内存行为。** §5.3 都是单帧采样。连续播放 N 分钟是否有增长
   (velato `Renderer` 内部的 `batch` / `mask_elements` 复用是否真的稳态)**没测**。
8. **`vello_hybrid` 路径。** 只核实了版本(0.0.9,2026-05-30),没跑。
9. **SVG 图标管线。** 顺手核实了 `vello_svg 0.10.0`(2026-07-19)依赖 `vello ^0.9.0`
   —— 与我们对得上,但它把 vello 作为**非可选**依赖,没有 velato 那样的 `RenderSink`
   抽象,**CPU 路径要走 `usvg` + 自绘**。附带好消息:`resvg 0.46.0` 依赖
   `tiny-skia ^0.11.4`,与本仓库锁定版本完全一致。**这条线我只做了依赖核实,没跑代码。**

---

## 附:实验产物清单

`C:\Users\DELL\AppData\Local\Temp\claude\lottie-spike\out\` 下的 PNG(全部逐张看过):

```
tinyskia-path.png       300×200     12 350 B   实验 3:任意贝塞尔 EvenOdd 填充 + 描边
cpu-rect-0/30/59.png    400×200     ~4 450 B   实验 2:手写 lottie 三帧,矩形左→中→右
cpu-tiger.png          1024×1024   113 464 B   CPU 档老虎
vello-tiger.png        1024×1024    87 642 B   GPU 档老虎(与上图肉眼一致)
cpu-tiger-plain/fast.png 1024×1024             §5.2 透传优化前后(差 0.164% 字节,最大差 2)
cpu-1f525-fixed.png    1024×1024               §6.2 修 compose 之后的火焰(与 vello 一致)
noto-*.png(12 张)     1024×1024               12 个 Noto emoji 全渲染成功
p-cpu-*/p-gpu-*.png(各 15 张)                 §3.4 对拍用
```
