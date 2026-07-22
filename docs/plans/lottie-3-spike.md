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

> ⚠️ **复核:这不是"意外发现",它已经是本仓库的既有结论。**
> `lottie-1-ecology.md` §1.3 的标题就叫「`RenderSink` —— 不必绑 vello 的那扇门」,
> 正文引了 velato 0.9.0 的 CHANGELOG 原文("The `vello` dependency is now optional"),
> §6.1 贴了同一棵 `default-features = false` 的依赖树;`lottie-2-architecture.md`
> §5.1 已据此裁决 `sv-shell` 加
> `velato = { version = "0.11", default-features = false, optional = true }`。
> 本节把它写成"改变了 lottie 在路线图里的位置"是**重复记账**——路线图上的位置
> 在本文写之前就已经改过了。
>
> **本节仍有的价值**:lottie-1 的依赖树是文字誊录,本文是四个真项目的 `cargo tree`
> 实跑,并且额外验了 `velato + vello_cpu` 的共存(§2.4)。这一层"实跑复核"的价值
> 是真的,但它是**证实**,不是发现。

**真正的卡点只有一条,而且不是依赖问题,是健壮性问题**(§6.1):velato 在**合法
Lottie 输入**上 `todo!()` panic 而不是返回 `Err`。我独立复现了三种触发方式。
好消息:`catch_unwind` 能接住(实测),仓库也没开 `panic = "abort"`。

> ⚠️ **复核:这条同样已经在 lottie-1 §1.4 + §6.4 里。**
> 那份文档的 §0 就把它列为"死穴之一",给了同一张 6 行 `todo!()` 位置表、同一个
> "删掉 `r` 键即炸"的复现、同一条"仓库没设 `panic = "abort"` 所以 catch_unwind 可用"
> 的前提核实,还多一条本文没有的判读:**`:213` 是纯 bug**(schema 把 rotation
> 声明成 `Option`,却在 `None` 分支写 `todo!()`),因此
> **"给 velato 提一个 20 行的 PR 把 `todo!()` 改成 `Error` 变体,性价比高于
> catch_unwind"**。本文 §8.3 把 `catch_unwind` 排在第 1 位却完全没提上游修复,
> 是把创可贴当成了治疗(lottie-1 §6.4 原话)。

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

> ⚠️ **复核:「velato CI 用的同一批」是错的,而且漏了许可证。**
> 逐项查过:
> - `gh api repos/linebender/velato/contents/.github/workflows` 只有 `ci.yml` 与
>   `pages-release.yml` 两个 workflow,**两个文件里 grep `download|gstatic|noto`
>   零命中**——velato 的 CI **从不下载也从不使用**这批资产。
> - 这份名单的真实出处是 **`examples/scenes/src/download/default_downloads.rs`**
>   (`with_winit` 示例的按需下载器,`gh api search/code` 全仓唯一命中 gstatic 的文件),
>   里面是 `google_noto_asset!(名字, id, expected_size)` 宏展开的一张表,URL 模板
>   `https://fonts.gstatic.com/s/e/notoemoji/latest/{id}/lottie.json`。
> - **该文件逐字写着 `license: "CC BY 4.0"`**,`info` 指向
>   <https://googlefonts.github.io/noto-emoji-animation/>。
>   本文全篇没有提过任何资产/依赖的许可证。
>   顺带补齐另外两条(复核者查的):velato 0.11.0 crates.io 与仓库内
>   `LICENSE-APACHE`/`LICENSE-MIT` 均为 **Apache-2.0 OR MIT**,与本仓库
>   `license.workspace = "MIT OR Apache-2.0"` 相容;`velato_imaging` 0.0.1 同。
> - `expected_size` 表里 `1f602 = 59124` 字节,与本地 `assets/noto/1f602.json` 的
>   59124 字节逐字节相等——**同一来源可以确认,"CI 用的"不能**。
>
> **为什么要较这个真**:这批文件只留在 `%TEMP%` 就没问题;一旦有人照本文的
> "velato CI 同款"这句话把它们当测试固件 vendor 进仓库,进来的就是一批
> **CC BY 4.0(要求署名)** 的第三方素材,而仓库的 `deny.toml` 管不到
> `assets/*.json`。**要用就得在 `assets/LICENSE` 里写明出处与署名。**
> 另外 `2764_fe0f.json` 在下文若干表里被写成 `2764`,是同一个文件。

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

> ⚠️ **复核:签名转述有一处失真,以及漏了一个现成实现。**
>
> 1. **`draw` 的真实签名是 `Option<&fixed::Stroke>` / `&fixed::Brush`**,不是本节写的
>    `Option<&kurbo::Stroke>` / `&peniko::Brush`。二者在 0.11 里等价
>    (`runtime/model/fixed.rs` 逐字:`pub type Stroke = kurbo::Stroke;`
>    `pub type Brush = peniko::Brush;` `pub type Transform = kurbo::Affine;`),
>    所以结论不受影响 ——**但那是别名,不是同名**,而别名是 velato 保留的换实现口子。
>    照本文的签名去写适配器编译得过,照本文去推断"velato 永远给 kurbo/peniko 类型"
>    则没有依据。
> 2. **`vello::Scene` 不是唯一实现,还有 `velato_imaging` 0.0.1**
>    (crates.io API 实查:2026-05-21 发布,Apache-2.0 OR MIT,仓库
>    `github.com/forest-rs/imaging`,作者 waywardmonkeys = Bruce Mitchener,
>    Linebender 的人)。lottie-1 §1.3 已经把它点名为**"我们要写的东西的模板"**,
>    并指出它最值得抄的一点:**把图层栈失衡收成
>    `Error::UnbalancedLayerStack` 而不是 panic,出错后整体转 no-op。**
>    本文从零写了 395 行 `TinySkiaSink`(§1 说"约 280 行",实测 `wc -l` = 395)
>    却**通篇没提这个包**。这是本次复核认定的最大一处"20% 力气拿 80% 收益被漏掉"
>    ——详见 §10.5。

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

> ⚠️ **复核:本节这些数字产生于 §6.2 的 compose bug 修好之前,而本文没有说。**
> 用当前 `%TEMP%` 里的二进制(即修好之后的)重跑同一条命令:
>
> ```
> $ ./lottie-spike cpu assets/Tiger.json 60 out/verify-tiger.png
> [cpu] frame=60 1024x1024  34.73ms  fills=47 strokes=0 peak_layers=4  非白像素=345594
> ```
>
> **345 594,不是本文写的 351 051。** 把两张 PNG 逐像素比:
> `out/cpu-tiger.png`(10:16 产,本节引用的那张)与新产物有 **7 456 个像素不同
> (0.71%),最大通道差 224**——那不是舍入,那是"有东西画错了"。
> 时间线可以对上:`src/tsk_sink.rs` 的 mtime 是 10:36,compose 修复后的
> `cpu-1f525-fixed.png` 是 10:29,而本节引用的 PNG 全部生成于 10:16—10:18。
>
> 所以:**"是一只完整正确的老虎"这句话描述的是 §6.2 亲口承认画错了的那版代码的输出。**
> 修复确实让 Tiger 也变了(它有 4 组 track matte,见 lottie-1 §6.3),
> 说明这个 bug 的影响面比 §6.2 描述的"火焰那一张"更大。
>
> **§3.4 的 `p-cpu-*` 那批(10:30 产)是修好之后的,复核逐条重跑,数字一模一样**
> (Tiger 352139 / 1f525 425825 / 1f600 665513)。要引用 CPU 侧数字,引 §3.4,别引本节。

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

> ⚠️ **复核:「差值全在抗锯齿边缘」没有依据,「比现有 parity 带还紧」是错觉。
> 这张表证明不了它想证明的事。**
>
> **先说指标本身。** `main.rs:125` 与 `:98` 的口径是
> `filter(|p| !(r==255 && g==255 && b==255)).count()` ——
> **只数"非白像素有几个",不比"这些像素长什么样"**。它对颜色错、渐变插值错、
> 图层顺序错、镜像翻转几乎完全不敏感。用本文自己的产物就能把这一点钉死:
>
> ```
> out/cpu-tiger-plain.png  非白像素 351051
> out/cpu-tiger-fast.png   非白像素 351051      ← 一个数都没动
> 逐像素:4140 个像素不同(0.39%),最大通道差 2
> ```
>
> §5.2 那次"透传优化"真真切切改了 4140 个像素,**这个指标纹丝不动**。
> 一个连自己都测不出来的指标,不能拿来说"两条路互相对得上"。
>
> **再说真实差距。** 复核者自己写了个 PNG 解码 + 逐像素比对脚本(纯 Python,
> 无第三方依赖),对本文 §3.4 用的同一批 `p-cpu-*` / `p-gpu-*` 跑了一遍:
>
> | 资产 | 非白比(本文口径) | **像素不同占比** | **最大通道差** | 平均 \|Δ\| | Δ>8 的像素 |
> | --- | ---: | ---: | ---: | ---: | ---: |
> | Tiger | 0.9993 | **1.38%** | **133** | 0.197 | 0.77% |
> | noto 1f525 | 0.9997 | **26.11%** | **121** | 1.531 | 5.18% |
> | noto 1f600 | 1.0000 | **31.91%** | **100** | 1.731 | 7.09% |
> | noto 1f970 | 0.9999 | **30.38%** | **114** | 1.759 | — |
>
> **1f600 的非白比是 1.0000(表上最"完美"的一行),而它有 31.9% 的像素不同、
> 最大通道差 100。** 平均 \|Δ\| 只有 1.7,所以这不是"画错了",但也**绝不是
> "差值全在抗锯齿边缘"**——边缘像素在 1024² 的 emoji 上撑死几个百分点,
> 而 5–7% 的像素差超过 8 个色阶,那是**面**上的差,不是**线**上的差。
>
> **最可能的成因(复核者的推测,未验证)**:渐变色标的插值色彩空间不一致。
> `tsk_sink.rs:134` 的 `stops_to_ts` 把 peniko 色标 `to_alpha_color::<Srgb>()`
> 后交给 tiny-skia 在预乘 sRGB 8bit 里插值,vello 走的是另一条;而 velato README
> 的 missing features 里恰好有一条 **"Correct color stop handling"**。
> 要坐实得做单渐变最小样本对拍,**复核者没做**。
>
> **对决策的影响,分两半说清楚:**
> - **不推翻可行性结论。** 平均 \|Δ\| < 2、无结构性错位,肉眼看确实一致;
>   而且 `lottie-2-architecture.md` §5.4 已经把 lottie 的 parity 口径定成
>   "非白像素比落在 `[0.95, 1.05]`",本文这批数全部轻松通过**既定验收线**。
> - **推翻"对拍 ⇒ 两个后端画得一样"这个推论。** ADR-3b 的非白比口径是为
>   "纯色圆角矩形 + 字形"立的,那时候覆盖面积确实是好代理;lottie 一来就有大面积
>   渐变,代理关系断了。**真要在 CI 里守住 lottie 的双后端一致性,验收项必须
>   加一条逐像素的(平均 \|Δ\| 与 Δ>N 的像素占比),否则渐变整体偏色也是绿的。**
>   这条建议本文没有,应当补进 lottie-2 §5.4。

**结论:能。两条路都能离屏渲染出一帧,且互相对得上。**

> ⚠️ **复核:改成——两条路都能出图,覆盖面积一致,逐像素不一致(见上表)。**

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

> ⚠️ **复核:这笔账要打折,而且这条"发现"也不是本文的。**
>
> 1. **不是本文发现的。** `lottie-1-ecology.md` §7.0 的小标题就叫
>    「先更正一条既有事实」,正文点名 `7966785` 说任务书里"没有 fill_path"已经过期。
>    本文 §0 写的"**在我做这个 spike 的同时**,主线已经把 … 落地了",时间线上
>    没错(`git log`:`7966785` 与 `3ebe81c` 都在 `50a700c` 之前),但作为**发现**
>    是第二遍。
> 2. **"出账了"要打折:动词齐了,裁剪语义还是欠的。** 复核者读了
>    `crates/sv-shell/src/paint.rs`:`TinySkiaPainter::fill_path` / `stroke_path`
>    给 tiny-skia 的 `mask` 参数传的是 **`None`**,并且源码注释自己挂着缺口:
>
>    ```rust
>    // crates/sv-shell/src/paint.rs:659-662
>    // 裁剪:矩形裁剪走 Mask 太贵(见 push_clip 的矩形交集裁决),这里
>    // 用 tiny-skia 的 clip_mask 参数传 None,靠调用方保证路径在裁剪内。
>    // **已知缺口**:滚动容器内的路径图标不会被裁掉;等真有这个场景再补
>    ```
>
>    **lottie 的第一帧就会踩到它**:`Renderer::append` 开头无条件发一次
>    `push_clip_layer`(`render.rs:68`,本文 §5.2 自己引了这一行)。
>    `lottie-2-architecture.md` §2.4 已有同款复核结论:
>    "**所以「步骤 0 已完成」要打个折:动词齐了,裁剪语义还是欠的**"。
> 3. **顺带一条精度**:velato 的 `impl RenderSink for vello::Scene`
>    (`runtime/vello.rs`)对 `draw` / `push_layer` / `push_clip_layer`
>    **一律硬编码 `Fill::NonZero`** —— 也就是说 **lottie 这条线永远不会发出 EvenOdd**。
>    `PathFill::EvenOdd` 的价值在 SVG 图标(带孔图标)那边,本节把它算进 lottie
>    的验证收益是记错了账本。

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

> ⚠️ **复核:数字全部复现,但有一条跨文档矛盾必须当面对上,还有两处过于乐观。**
>
> **(a) 复现结果**(复核者原样重跑 `%TEMP%` 里的二进制):
>
> ```
> $ SPIKE_SCALE=0.0625 lottie-spike bench assets/noto/1f602.json 60
> velato->vello::Scene 编码       14.408 µs/帧      (本文 13–23 µs ✓)
> velato->tiny-skia 全 CPU 64x64  347.993 µs/帧     (本文 0.345 ms ✓)
> velato->vello GPU 渲染+回读     687.978 µs/帧     (本文 0.698 ms ✓)
> $ lottie-spike bench assets/noto/1f602.json 60            # 1024²,朴素
> 纯图层缓冲 19.096 ms/帧   CPU 37.483 ms/帧   GPU 5.471 ms/帧   (本文 18.69/37.7/5.72 ✓)
> $ SPIKE_FASTCLIP=1 …                                       # 1024²,透传
> 纯图层缓冲 20.118 µs/帧   CPU 29.552 ms/帧                    (本文 0.013ms/29.0 ✓)
> ```
>
> **性能这一块可以信。**
>
> **(b) 跨文档矛盾:同一个 Tiger、同一个尺寸,仓库里现在有三套数,差一个数量级。**
>
> | 出处 | Tiger 256² | Tiger 1024² | 那个 sink 做没做图层合成 |
> | --- | ---: | ---: | --- |
> | `lottie-1-ecology.md` §6.2 原文 | 0.99 ms | **6.18 ms**(自评"37% 预算,能跑") | **没做**(自述"`push_layer` 的 blend/alpha 降级为忽略") |
> | `lottie-1` 的复核块(并行复核者独立重写) | 0.64 ms | **2.34 ms**(14% 预算) | 同上 |
> | 本文 §5.1 | 2.69 ms | **29.0 ms**(自评"不可接受") | **做了**(`peak_layers=4`) |
>
> **差异不是测错,是测的不是同一件事。** lottie-1 §6.3 自己量出 Tiger 每帧有
> **121 条非平凡 `push_layer`** 被丢掉——不分配图层 Pixmap 当然便宜。
> 本文的 29 ms 才是**画对**的价钱,**本文在这一点上是对的**,
> 而且这正是它最重要的原创贡献之一(§10.4)。
>
> **但本文一个字都没提这件事。** 后果是仓库里三张表给出从"14% 预算"到
> "不可接受"的相反结论,谁都没引用谁。**这一条必须回填进 lottie-1 §6.2:
> 那两组数要标注"未实现图层合成,是下限而非预算"。**
>
> **(c) 「GPU 档几乎免费」是过于乐观的口径。** 15 µs 只是 **velato → `Scene` 的编码**。
> 它成立要靠一个本文没写的前提:**那一帧 GPU 本来就要重编码 + 重渲染**。
> 而 `lottie-2-architecture.md` §4.4 已经指出:`sv-shell/src/lib.rs:278`
> 现在是 `vw.render_cached(&self.doc, scale, unchanged)`,`scene_unchanged = true`
> 时**会跳过整个 `paint_tree` 重编码**。也就是说 lottie 一旦按 §4.2 的"每帧写不 bump"
> 接进来,GPU 后端上它会**停在第一帧**——"免费"的前提在今天的代码里恰好不成立,
> 得先改那一行。本文完全没有触及这条接缝(§9 也没列)。
>
> **(d) 「64px 0.35 ms 可接受」缺分母,且成分可疑。** 复核实测:64×64 那 348 µs 里,
> **纯图层缓冲对照组就占 144.84 µs(42%)**(本文 §5.2 记的是 81.3 µs,复核机上
> 是 144.84 µs——64px 档太小、噪声大,这个数别当准数用)。更要紧的是分母:
> **一个 lottie 在跑 = 每帧整窗重绘**,§5 全部数字都只是 lottie 自己那一块。
> 窗口其余部分的 CPU 重绘成本本文没测、也没引用仓库里已有的数
> ——**"可接受"这个词现在没有依据支撑,只有"0.35 ms 比 16.7 ms 小"这个直觉。**

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

> ⚠️ **复核:数字复现(37.48 → 29.55 ms,-21%;缓冲 19.10 ms → 20.1 µs),
> 结论也对,但"该特判"这条同样是 lottie-1 已经开过的方子。**
> lottie-1 §6.2 最后一条判读逐字:
>
> > 一个可优化点:`Renderer::append` 每帧都会先 `push_clip_layer` 一个合成画布
> > 矩形,我的实现给它分配了一张 w×h 的 `Mask`(1024px 时 1 MB/帧)。
> > **这个根裁剪是轴对齐矩形,应当特判走现有的 `push_clip`**,别落进路径裁剪。
>
> 本文把它实现并量化了(这是真增量:从"应该"到"-23%,像素差 0.164%"),
> 但**不是新的诊断**。
>
> 另外这一节的两张 PNG(`cpu-tiger-plain/fast.png`)产于 10:23,同样早于
> §6.2 的 compose 修复 —— 也就是说 **-23% 是在"画错的那版"上量的**。
> 复核者在修好之后的二进制上重量,降幅 -21%,结论不变,但报告里应当标明代次。
> 顺带:这一对 PNG 正是揭穿 §3.4 指标的证物(4140 像素不同 / 非白比一个数没动),
> 见 §3.4 的 ⚠️ 块。

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

> ⚠️ **复核:内存这一节复现,读法基本认可,只削一句。**
> 重跑 `lottie-spike mem assets/noto/1f602.json`:基线 7.0 MB → 解析后工作集
> 8.2 MB(**+1.0 MB ✓**)→ 1024² 光栅峰值 **23.0 MB ✓** → GPU 段工作集
> **301.2 MB / 峰值 609.2 MB ✓**。数字对得上。
>
> 要削的是 **"十几个图标动画同时驻留 ≈ 10 MB 量级,可接受"**:样本只有 4 个,
> 且 3 个来自同一来源;更要紧的是这 1 MB 只是**静态 `Composition`**,
> 不含播放期的图层 Pixmap。按同一节自己的数,**一个 1024² 的动画在播放瞬间就是
> 23 MB 峰值**;十几个同时播就不是 10 MB 量级。
> 正确的说法是:**常驻(不播)≈1 MB/个;在播的那个另外按渲染尺寸算**,
> 而这正是 §5.2"图层 Pixmap 池"要解决的东西。

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

> ⚠️ **复核:「第一次引入 serde」是错的,而且这份"新引入"名单同时高估和低估,
> 因为它没分清"`Cargo.lock` 里有没有"和"默认构建编不编"。**
>
> **实测**(`grep -c '^name = "X"$' Cargo.lock`,本仓库当前 `Cargo.lock`):
>
> ```
> serde 1   serde_core 1   serde_derive 1   serde_repr 1   ← 全都已经在
> kurbo 1   peniko 1   color 1   linebender_resource_handle 1   polycool 1   memchr 1
> serde_json 0   itoa 0   zmij 0                            ← 只有这三个是新的
> ```
>
> **`serde_repr` 本文自己列在"新引入"里,而它已经在 lock 里了**;
> `serde` 更是早就在。所以"第一次引入 serde/serde_json"这句话,前半句错、后半句对。
>
> **但反过来,真正的代价被低估了。** 上面那些"已经在"的条目**只在
> `backend-vello` 打开时才编**。复核者实跑默认特性的依赖树:
>
> ```
> $ cargo tree -p sv-shell --offline -e normal | grep -iE "kurbo|peniko|serde|color |polycool"
> (无输出;整棵默认树里与图形/序列化相关的只有 linebender_resource_handle,来自 parley)
> ```
>
> 也就是说 **对"默认 CPU 构建"而言,接 velato 会新增约 12 个 crate**
> (velato + kurbo + peniko + color + polycool + serde ×3 + serde_repr +
> serde_json + itoa + zmij),**其中 kurbo / peniko 是今天只在 GPU 特性下出现的
> Linebender 图形栈**。这不是记账口径问题,它直接顶到本仓库的一条明文纪律
> —— `crates/sv-shell/src/paint.rs:20-26` 对 `PathCmd` 的注释:
>
> > 为什么不直接用 kurbo 的 `BezPath`:vello 在本仓库是 **optional dependency**
> > …… 让接口签名依赖只在某个 feature 下存在的类型,等于把 GPU 后端焊死进 CPU 路径。
>
> `PathCmd` 本身当然不会因此改变(适配器内部摊平,本文 §8.4 说得对),
> **但"CPU 默认构建不含 kurbo/peniko"这个事实会消失。**
>
> **裁决(已有,本文漏引)**:`lottie-2-architecture.md` §5.1 早就把它按住了 ——
> `sv-shell` 侧写成
> `velato = { version = "0.11", default-features = false, optional = true }`,
> 挂在 `lottie` feature 下;§5.3 第 1 条还进一步要求"没开 feature 时
> `register_bytes` 这类 API **不存在**,而不是返回 `Err`"。
> **本文从头到尾没有出现过 `optional` / feature 门这两个词**,读起来像是可以直接
> 无条件加依赖 —— 这是本次复核认定的最严重的一处**与既有约束的冲突遗漏**。
>
> 顺带:`≈ 20 s` 是空项目里量的,不是本仓库的边际构建成本(本仓库已有
> kurbo/peniko/serde 的编译产物,真实增量只有 velato + serde_json + itoa + zmij)。
> **这个数偏保守,别当上限用,也别当下限用。**

### 5.5 velato 自己的 rustc 门槛

velato 0.11 的 `Cargo.toml`:`edition = "2024"`,`rust-version = "1.88"`。
本机 rustc 1.88.0 **正好卡在下限上**。这是需要记一笔的约束(仓库
`rust-version.workspace` 若低于 1.88 就要抬)。

> ⚠️ **复核:velato 的 `edition = "2024"` / `rust-version = "1.88"` / 许可证
> `Apache-2.0 OR MIT` 三项逐字核对无误(registry 里的 `Cargo.toml`)。
> 但"若低于 1.88 就要抬"这半句是白担心,真正的风险被漏了,而且方向是反的。**
>
> - 仓库根 `Cargo.toml` 写的就是 `rust-version = "1.88"`(还带一段注释解释为什么是
>   1.88 而不是 edition 2024 的 1.85:let-chains 晚稳定了三个版本)。**查一眼就知道,
>   不用"若"。**
> - **真正的风险在 CI 那条道上。** `.github/workflows/ci.yml` 有一个
>   `msrv` job,`MSRV: "1.88.0"` 钉死,而且**带 `--all-features` 构建**,
>   注释逐字写着:
>
>   > 现状核过:`--all-features` 依赖图里声明的最高 `rust-version` 恰好就是 1.88
>
>   velato 声明的正好是 1.88,**等于把这条 CI 道顶到天花板**:
>   velato 之后任何一次把 MSRV 抬到 1.89 的**补丁级发布**,都会直接把这条门禁打红,
>   而我们的 `Cargo.toml` 一个字没改。加上 §2.1 自己说的"velato 的发布节奏跟 vello 走"
>   ——**这是两条独立的上游追随税叠在一起**。
> - **结论应当改成**:接 velato 之前先决定这条 MSRV 道对第三方 crate 的容忍策略
>   (要么给 velato 钉 `=0.11.x`,要么把 MSRV 道改成只测本仓库 crate)。
>   本文把它写成"记一笔的约束",低估了。

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

> ⚠️ **复核:全部复现(OK=4 / Err=2 / PANIC=3,逐行一致),`panic = "abort"`
> 的前提也核过了(根 `Cargo.toml` 与全部 crate/example 的 `Cargo.toml` 里
> grep `panic` 零命中,只有 `[profile.dev] opt-level`)。三处要补:**
>
> 1. **上面那段控制台输出被删了一行。** 真实运行还会打印
>    `Err  velato_repo_ls.json (Error parsing lottie: invalid type: sequence, expected struc…)`
>    —— 这就是为什么合计写着 `Err=2` 而表里只看得见一个 Err。那个文件是作者
>    抓 velato 仓库目录列表存下的 JSON,混在 `assets/` 里被一起扫了。
>    不影响结论,**但本文开头说"所有…取自实际进程输出",那就不该手工删行**。
> 2. **§9.4 那个"没查明"的问题,lottie-1 §1.4 已经给了答案。** 原文:
>    "**Split positions 已经实现了,README 过期**。`converters.rs` 的
>    `conv_transform` 里 `AnyTransformP::SplitPosition(..) => Position::SplitValues(..)`
>    ……但 `conv_shape_transform`(形状组的 transform)里仍是 `todo!("split position")`。
>    **同一个概念两处实现,一处好一处炸。**"
>    复核者对着 registry 源码确认:`converters.rs:252` 确实位于形状组 transform 那条
>    路径上。**所以 `bad-split-pos.json` 没炸不是玄学,是它把 split position 放在了
>    图层 transform 上(已实现),没放在形状组 transform 上(未实现)。**
>    §9.4 应当从"未验证"里划掉,并改成一条更准的风险:**panic 面按"哪一层的
>    transform"分叉,构造边界样本时要两处都造。**
> 3. **`catch_unwind` 要罩两处,不是一处。** lottie-1 §6.4:六个
>    `todo!()`/`unimplemented!()` 全在导入期(`converters.rs`),罩住 `from_str` 就够;
>    **但渲染期(`Renderer::append`)也要罩,理由不同** —— velato 0.8.1 的
>    CHANGELOG 记着修过 "a panic on WASM when finding roots"(样条求根),
>    求值路径有算术 panic 的历史。本文的 `robust` 子命令确实两处都罩了(代码里),
>    但 §8.3 第 1 条只写"包住 velato 的解析与渲染",没写清为什么是两处,
>    容易被实现者简化成一处。

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

> ⚠️ **复核:这一节是本文最有价值的原创发现(它正面回答了 lottie-1 §8.6 挂着的
> "track matte 被忽略后的真实视觉损失程度——未验证"),但结论说得太满,
> 要补三条源码事实。**
>
> 1. **"两半都得映射"在 tiny-skia 上做不到。** `tiny_skia::BlendMode` 是一个**扁平枚举**,
>    表达不了 `(mix, compose)` 二元组。复核者读了本文自己的修法
>    (`tsk_sink.rs:98-129`):它对任何非 `SrcOver` 的 compose **提前 return**,
>    也就是**改成丢掉 `mix` 那一半了**。碰上 `BlendMode { mix: Multiply, compose: SrcIn }`
>    这类组合,新代码同样是错的,只是错在另一边。
>    **对未来 `Painter::push_layer` 的正确结论应当是**:签名可以收二元组,
>    但 CPU 后端必须显式声明"只支持 `mix` 为 Normal 时的 compose,其余降级并计数"
>    —— 而不是"两半都得映射"。
> 2. **matte 的编码位置在 `import/builders.rs`,不是 `render.rs`。** 逐字:
>    `MatteMode::Alpha | MatteMode::Luma => Compose::SrcIn.into()`、
>    `InvertedAlpha | InvertedLuma => Compose::SrcOut.into()`
>    (`builders.rs:45-49`,另有两处同款副本)。本文引的 `render.rs:109-127` 是**消费**处。
> 3. **顺带暴露一个上游正确性缺口,本文没注意到**:上面那两行把
>    **`Luma`(亮度遮罩)和 `Alpha`(透明度遮罩)映射到了同一个 `Compose::SrcIn`**。
>    亮度遮罩按亮度取值、透明度遮罩按 alpha 取值,二者不等价。
>    **所以 luma matte 的资产在我们这边无论怎么映射都是错的,错在 velato 里。**
>    再加上 velato 自己在 `render.rs:112` 留的注释
>    `// todo: re-enable masking when it is more understood` ——
>    **"补全 Porter-Duff 映射后 track matte 就对了"这个印象是不成立的**,
>    本节修好的只是 alpha matte 这一支(样本 `1f525` 恰好是这一支)。

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

> ⚠️ **复核:第 2 条"渐变笔刷不是可选项"与已入库的裁决正面冲突,必须当成
> **异议**提出来,而不是当成新发现写。**
> `lottie-2-architecture.md` §5.2 的支持子集表逐字裁决:
>
> | 特性 | v1 行为 |
> | --- | --- |
> | 渐变填充 | **降级:取色标平均色**(两个后端同样降级) |
> | 图层遮罩(`layer.masks`) | **降级:外接矩形裁剪**(两个后端同样降级) |
> | 轨道遮罩 / 混合模式(`push_layer(blend)`) | **不支持**,记账保证 push/pop 配平 |
>
> 而且给了理由:**故意让 vello 也一起降级,是为了让"换后端画面不变"成立**
> ——`SV_RENDERER=cpu` 与无 adapter 自动回退是既有行为,GPU 有渐变、CPU 没有
> 会让 parity 测试要么失效要么写满例外。
>
> **本文提供的证据(1f525 渐变+matte、Tiger 躯干渐变、以及 §6.2 那张被填满的
> 画布)是对这条裁决的有力反驳**——"取平均色"在 1f525 这种整幅径向渐变上
> 显然不是"轻微降级"。**但本文必须以"我要求重开 lottie-2 §5.2 的裁决"的姿态写,
> 并且回答那条裁决真正在乎的问题:两个后端怎么保持一致。** 现在的写法
> ("不是可选项")读起来像是没人裁决过。
>
> 同理第 1 条:`lottie-2` §2.3 已经有一节叫「为什么不带 `transform` 参数」,
> 本文的各向异性抽样是给那条裁决补证据(**有价值**),但不是新开的问题。

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

> ⚠️ **复核:这张表漏了本仓库最贵的三件事,而且第 6 条是 lottie-1 §0 的原话。**
>
> **(a) 第 6 条不是本文的判断。** lottie-1 §0「死穴 2」逐字:
> "**我们没有脏矩形。** 一个 lottie 在跑 = 每帧整窗重绘。…… 这条不该记在 lottie
> 账上(是全仓欠账),但 lottie 会是第一个把它逼出来的特性。"
> 本文这两句与之几乎同形。lottie-1 §7.2 还给了**不同的顺序建议**:
> `1 → 5 → 2 → 4 → 3`,理由是"(a) 档一落地,整窗重绘立刻成为最大的实际开销,
> 收益远超 lottie 本身"——**它把脏矩形提到第二位,本文把它放到最后一位,
> 两份文档给了相反的排序,谁都没提对方。**
>
> **(b) 表里完全没有"接进场景树"这一步。** 而这恰恰是本仓库最独特的成本:
> - `lottie-2` §3.1 已经比较过三条路,**否掉了 `ElementKind::Lottie`**
>   ——理由是加一个变体要连带改 `sv-ui/lib.rs` 的枚举 + `create()` 的
>   focusable/accepts_text/input 三张表 + `dump`、`render.rs` 的 `build_taffy` /
>   `measure_leaf` / `paint_tree` 三处 match、`a11y.rs` 的 role 映射、
>   `sv-compiler` 的 `ElemKind` + 属性名表、`sv-macro` 的标签表,**七处以上**,
>   与多行 textarea 那次的裁决冲突。复核者数了一下:全仓 `ElementKind`
>   出现在 **15 个文件、61 处**,这个"七处以上"是保守的。
>   **裁决是走 `View` + `paint_source` 槽。** 本文对此只字未提。
> - 注意:lottie-1 §7.1 的集成点写的却是"`sv-ui`:`ElementKind` 加 `Lottie`"
>   ——**lottie-1 与 lottie-2 在这一点上本身就打架**,lottie-2 是后者且给了论证,
>   应以 lottie-2 为准。本文本可以做这个仲裁,却绕开了。
>
> **(c) 表里没有"每帧写不 bump 版本"这一步 —— 那才是 60fps 的真正闸门。**
> `lottie-2` §4.2:`Doc::version()` 一 bump,`layout_full_cached` 就整份失效
> (30k 档 ≈130 ms),**"一个转圈图标 = 全应用卡顿"**。所以媒体时间必须走一个
> **刻意不 bump** 的写入口。复核者核过机制确实如此:
> `sv-shell/src/lib.rs:235` 的 `frame_key` 以 `doc.version()` 为键,
> `render.rs:458` 的 `layout_full_cached` 挂在同一条链上。
> **这条不做,§5 里所有 0.35 ms 的数字都没有意义** —— 因为每帧会先付一次全量布局。
> 它比本文表里的第 2/3/4 条(图层/裁剪/渐变)重要一个量级,却根本没进表。
>
> **(d) 第 1 条只写了创可贴,没写治疗。** 见 §6.1 的 ⚠️:
> lottie-1 §6.4 认为"给 velato 提 PR 把 `todo!()` 改成 `Error` 变体
> (`:213` 是纯 bug,大概率 20 行)"的性价比高于其它任何一项。
> 上游修 + `catch_unwind` 兜底是两件事,表里应当都有。

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

> ⚠️ **复核:三条都同意(`vello_cpu 0.0.9 / 2026-05-30` 与
> `vello_svg 0.10.0 → vello ^0.9.0(非 optional)`、`resvg 0.46.0 → tiny-skia ^0.11.4`
> 都经 crates.io API 复查无误)。但"不建议做的"这份清单缺了它的对偶
> ——「更省力的做法」,而漏掉的那几条恰恰是本次复核认为最该先做的:**
>
> 1. **抄 `velato_imaging`,别从零写 sink。**(§2.3 的 ⚠️ 已展开)
>    现成的第二个 `RenderSink` 实现,约 200 行,Linebender 自己人写的,
>    **而且已经解决了本文 §6.1 的一半问题**:把栈失衡收成 `Error` 而不是 panic,
>    出错整体转 no-op。本文写了 395 行,没有这条纪律。
> 2. **先只做 SVG 静态图标,lottie 押后。** 这不是绕路,是**同一条地基上更早的收益**:
>    `fill_path`/`stroke_path` 已经落地(§4),SVG 只差一个**构建期** `usvg` →
>    `&[PathCmd]` 数据表(`lottie-2` §2.7 已经写好了这条:
>    "生成数据而非类型,ADR-2 哲学,与 lottie 完全解耦"),
>    **零运行时依赖、零 panic 面、零帧预算、不碰场景树、不碰版本号 bump**。
>    调研 26 说的"没有这项,arco 视觉完成度上限约六成"指的正是图标而不是动画。
>    本文 §9.9 已经摸到这条线(还查证了 `resvg → tiny-skia ^0.11.4` 版本吻合),
>    却把它归进"没验证的部分",没有把它提为**替代方案**。
>    **按"20% 力气拿 80% 收益"排,这一条应该在 §8.3 的表里排第 0.5 位。**
> 3. **把 lottie 预渲染成序列帧 —— 明确写"不做",并写清理由。**
>    复核者查了:lottie-1 §7.3(4) 已经否掉了"构建期转译成 `.sv`/`view!`"
>    (理由:Qt 能这么干是因为 QML 底下有通用属性动画系统,我们的 `anim.rs`
>    只有 opacity 与 scrollY 两个通道)。**但"预渲染成位图序列帧"是另一件事,
>    两份文档都没写。** 它的否定理由更硬:
>    `crates/sv-shell/src/paint.rs` 文件头写着"**painter 不拿字符串也不拿位图
>    (Slint 软件渲染器与 GPU 灾难的双重教训)**",`Painter` 上根本没有位图动词,
>    要走序列帧得先造一个我们完全没有的图片子系统 —— 而 1024² × 60 帧 × 4B
>    = 252 MB 的解码后体量也不现实。**写进"不建议做的",免得下一个人再提。**

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
4. ~~**`bad-split-pos.json` 为什么没触发 `todo!("split position")`**(converters.rs:252)。~~
   ~~我构造的拆分位置 JSON 被正常接受了,没查明是我的 JSON 形状不对还是那条路径确实~~
   ~~没走到。**这意味着 §6.1 的 panic 清单可能不完整。**~~
   > ⚠️ **复核:已查明,划掉。** `lottie-1-ecology.md` §1.4(2) 早就答了:
   > split position 在**图层 transform**(`conv_transform`)里**已实现**,
   > 只有**形状组 transform**(`conv_shape_transform`)里还是 `todo!("split position")`
   > —— `converters.rs:252` 正在后者。**"同一个概念两处实现,一处好一处炸。"**
   > 改成一条更有用的风险记录:**panic 面按"哪一层的 transform"分叉,
   > 构造边界样本必须两处都造。**
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

> ⚠️ **复核:这份清单本身是诚实的(第 1/2/5 条尤其到位),但漏了五条,
> 而且漏掉的都比清单里的靠前:**
>
> 10. **`Painter` 门面下的双后端一致性没验。** §3.4 的对拍是"我的 tiny-skia sink
>     vs vello::Scene",走的是**两条各自独立的 velato 出口**;真实形态是**同一个
>     适配器**打到 `&mut dyn Painter`,两个后端的差异被压回 `Painter` 那一层
>     (`lottie-2` §5.1)。**本文测的不是将来要跑的那个拓扑。**
> 11. **feature 门与 API 缺席行为没验。** velato 该是 `optional = true`
>     (`lottie-2` §5.1),不开时相关 API 应当**不存在**而非返回 `Err`(§5.3)。
>     本文四个实验项目全都是无条件依赖,**"关掉 lottie feature 后仓库还编不编"
>     一次都没测过**(而这正是"CPU 后端必须能降级"这条约束的落点)。
> 12. **版本号 bump / 布局缓存的相互作用没验。** 见 §8.3 的 ⚠️ (c)。
>     这是本仓库特有的、也是最贵的一条,清单里应当排第 1。
> 13. **`.lottie`(dotLottie zip 容器)没验。** velato 只吃裸 JSON
>     (`Composition::from_slice` / `from_json`),而设计师交付里 `.lottie` 很常见。
>     lottie-1 §1.5 已记录,本文没提。
> 14. **上游 issue 与 panic 修复意愿没查。** velato 6 个 open issue 一条没读
>     (lottie-1 §8.7 也把这条挂在未核实里,两轮都没做)。
>     §8.3 若要把"提 PR"排进去,这一条是前置。

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

> ⚠️ **复核:这份清单混着两代产物,读的时候要分清(mtime 是硬证据):**
>
> | 时间 | 产物 | 代次 |
> | --- | --- | --- |
> | 10:16–10:18 | `tinyskia-path` / `cpu-rect-*` / `cpu-tiger` / `vello-*` / `mem-*` | **compose bug 修复前** |
> | 10:23 | `cpu-tiger-plain/fast` | **修复前**(§5.2 的 -23% 在这上面量的) |
> | 10:29–10:30 | `cpu-1f525-fixed` / `p-cpu-*` / `p-gpu-*` | 修复后 |
> | 10:34–10:36 | `bad-split-pos` / `dbg` | 修复后 |
>
> 也就是说 **`cpu-tiger.png` 与 `vello-tiger.png` 那对"肉眼一致"是修复前的对比**;
> 用当前二进制重跑 `cpu Tiger 60`,与 `cpu-tiger.png` 差 7456 像素、最大通道差 224。
> 引用 CPU 侧图像证据,只引 `p-cpu-*` 那一批。

---

## 10. 复核记录

> 复核日期:2026-07-22。复核者:独立 agent,**默认立场"这份产物有问题"**。
> 方法:(1) 全文逐条对着源码/接口原始返回重查;(2) `%TEMP%` 里的四个实验项目
> **原样重跑**,不信任报告里的任何一个数;(3) 自己写工具做本文没做的比对;
> (4) 与仓库既有约束和已入库的 `lottie-1` / `lottie-2` 两份文档交叉比对。
> **只改了 `docs/plans/lottie-3-spike.md` 一个文件**(权限所限),
> 原文一字未删,所有意见以 `⚠️ **复核**` 块就地插入。

### 10.1 复现结果:性能/内存/依赖三类数字全部对上

原样重跑,与原文逐项比对:

| 项 | 原文 | 复核重跑 | 判定 |
| --- | --- | --- | --- |
| `velato → vello::Scene` 编码(1f602) | 13–23 µs | **14.408 µs** | ✅ |
| CPU 64×64(1f602) | 0.345 ms | **0.348 ms** | ✅ |
| GPU 64×64(1f602,含同步回读) | 0.698 ms | **0.688 ms** | ✅ |
| CPU 1024²(朴素) | 37.7 ms | **37.48 ms** | ✅ |
| CPU 1024²(透传) | 29.0 ms | **29.55 ms** | ✅ |
| 纯图层缓冲 1024²(朴素→透传) | 18.69 ms → 0.013 ms | **19.10 ms → 20.1 µs** | ✅ |
| GPU 1024² | 5.72 ms | **5.47 ms** | ✅ |
| `robust assets` | OK=4 Err=2 PANIC=3 | **逐行一致** | ✅ |
| `robust assets/noto` | OK=12 Err=0 PANIC=0 | **一致** | ✅ |
| 内存 1f602(解析 / 光栅峰值 / GPU) | +1.0 MB / 23.0 MB / 301·609 MB | **一致** | ✅ |
| §3.4 CPU 非白像素(Tiger/1f525/1f600) | 352139 / 425825 / 665513 | **逐字一致** | ✅ |
| §5.2 透传前后像素差 | 0.164% 字节 / 最大差 2 | **0.1639% / 2** | ✅ |

**外部事实复查(crates.io API / GitHub API / registry 源码,全部一手):**
velato 0.11.0 发布时间 `2026-07-21T12:29:44Z`、依赖
`kurbo ^0.13.0` / `peniko ^0.6.0` / `serde ^1.0.228` / `serde_json ^1.0.149` /
`serde_repr ^0.1.20` / `vello ^0.9.0 (optional)`、
GitHub `pushed_at 2026-07-21T12:28:49Z` / 152 star / 6 open issue / 未归档 ——
**与原文逐字相符**。README 兼容矩阵、README 用了不存在的 `renderer.render(...)`、
`todo!/unimplemented!` 共 8 处、`Error` 只有 `Json` 一个变体、
`fixed::Brush = peniko::Brush`、tiny-skia `painter.rs` 非单位变换时
同时变换 path 与 `paint.shader` —— **全部核对无误**。
`vello_cpu 0.0.9 / 2026-05-30`、`vello_svg 0.10.0 → vello ^0.9.0(非 optional)`、
`resvg 0.46.0 → tiny-skia ^0.11.4` —— **全部无误**。
**没有发现任何编造的版本号或 API 名。** 这一点原文做得比大多数报告好。

### 10.2 改了什么(按严重度)

| # | 位置 | 问题 | 性质 |
| --- | --- | --- | --- |
| 1 | §3.4 | "非白像素比 0.9956–1.0000 / 差值全在抗锯齿边缘 / 比现有 parity 带还紧" —— **该指标只数像素个数**。逐像素实测:1f600 有 **31.9% 像素不同、最大通道差 100**,而它的非白比是 1.0000。反证:§5.2 的两张图差 4140 像素,该指标**一个数没动** | **结论口径错误** |
| 2 | §5.4 | "**这是仓库第一次引入 serde/serde_json**" —— `serde`/`serde_core`/`serde_derive`/**`serde_repr`(原文自己列为"新引入")** 全都已在 `Cargo.lock`。反过来对**默认特性构建**低估:实测默认树里没有 kurbo/peniko/serde,接 velato 会新增约 12 个 crate | **事实错误 + 低估** |
| 3 | 全文 | **没有一处提到 velato 必须 `optional = true` / 挂 feature 门**,而 `lottie-2` §5.1/§5.3 已裁决。这直接顶到"CPU 后端必须能降级""vello 是 optional"两条约束 | **致命遗漏** |
| 4 | §8.3 | 落地顺序表里**没有"接进场景树"**(`lottie-2` §3.1 已否掉 `ElementKind::Lottie`,全仓 `ElementKind` 涉 15 文件 61 处)**也没有"媒体时间不 bump 版本"**(`lottie-2` §4.2:一 bump 就让 `layout_full_cached` 每帧失效,30k 档 ≈130 ms)。后者是 60fps 的真闸门,比表里第 2/3/4 条重要一个量级 | **致命遗漏** |
| 5 | §1 | "Noto emoji 是 **velato CI 用的同一批**" —— velato 两个 workflow 里 grep `download\|gstatic\|noto` **零命中**;真实出处是 `examples/scenes/src/download/default_downloads.rs`(示例下载器),且该文件逐字写着 **`license: "CC BY 4.0"`**。全文无一处提许可证 | **无出处的断言 + 许可证漏检** |
| 6 | §0/§2.3/§4/§5.2/§6.1/§8.3 | 五条"发现"实为 `lottie-1-ecology.md`(已入库)的既有结论:RenderSink 解耦(§1.3)、panic 复现(§1.4+§6.4)、fill_path 已落地(§7.0)、根裁剪该特判(§6.2)、lottie 逼出脏矩形(§0)。原文用"最大的意外发现""不是引用别人的结论,是我改 JSON 跑出来的"等措辞主张原创性 | **重复记账 / 新颖性溢价** |
| 7 | §3.2 §5.2 附录 | 这些数字与 PNG 产生于 §6.2 compose 修复**之前**(mtime 铁证),原文未标。重跑 Tiger frame=60:**345594 ≠ 351051**,与旧图差 7456 像素、最大通道差 **224** | **代次混淆** |
| 8 | §5.1 | "GPU 档几乎免费(15 µs)" 的前提是"那一帧本来就要重编码",而 `sv-shell/src/lib.rs:278` 的 `render_cached(.., unchanged)` 恰好会跳过重编码(`lottie-2` §4.4)。"64px 0.35 ms 可接受"缺分母(整窗重绘没算,且其中 42% 是图层缓冲分配) | **过于乐观** |
| 9 | §5.1 vs lottie-1 §6.2 | **同一个 Tiger 同一尺寸,仓库里三套数差一个数量级**(1024²:6.18 / 2.34 / **29.0** ms),结论从"14% 预算"到"不可接受",互不引用。原因已查明:前两套的 sink 都不做图层合成(lottie-1 §6.3 自陈每帧丢 121 条 `push_layer`)。**本文的数是对的,但没说明自己在测不同的东西** | **跨文档矛盾** |
| 10 | §4 | "调研 26 点名的最大风险已出账" —— `fill_path`/`stroke_path` 给 tiny-skia 的 `mask` 传的是 `None`,paint.rs 自己挂着"已知缺口:滚动容器内的路径图标不会被裁掉";而 velato 每帧开头必发 `push_clip_layer` | **过于乐观** |
| 11 | §6.2 | "两半都得映射"在 tiny-skia 上做不到(`BlendMode` 是扁平枚举);原文修法实际是**改成丢 `mix` 那一半**。另:velato 把 **Luma matte 也映射成 `Compose::SrcIn`**(`builders.rs:45-49`),luma 遮罩无论怎么映射都是错的 | **结论说得太满** |
| 12 | §7.2 | "渐变笔刷不是可选项"与 `lottie-2` §5.2 的"渐变降级取平均色,两个后端同样降级"正面冲突,原文未以异议形式提出 | **与既有裁决冲突** |
| 13 | §9.4 | "`bad-split-pos` 为什么没炸没查明"—— lottie-1 §1.4(2) 已答:split position 在图层 transform 已实现,只有形状组 transform 是 `todo!`。已划掉并改写成更有用的风险 | **可关闭的未知项** |
| 14 | §5.5 | "仓库 `rust-version` 若低于 1.88 就要抬" —— 仓库本来就是 1.88。真风险是 CI 的 `msrv` job 钉死 1.88.0 且 `--all-features`,注释自陈"依赖图最高 rust-version 恰好就是 1.88",velato 声明 1.88 = **顶到天花板**,上游一次补丁级 MSRV 抬升就打红门禁 | **风险方向搞反** |
| 15 | §6.1 | 控制台输出被手工删了一行(真实运行有第 2 条 `Err velato_repo_ls.json`),与开篇"所有输出取自实际进程"的自述不符 | **证据不完整** |
| 16 | §8.4 / §9 | 漏了三条更省力的替代:抄 `velato_imaging`(现成 ~200 行 sink,把栈失衡收成 `Error` 而非 panic)、**先只做 SVG 静态图标**(`lottie-2` §2.7 的构建期 usvg→PathCmd,零运行时依赖零 panic 面)、明确否掉"预渲染序列帧"(`Painter` 无位图动词是明文纪律)。§9 另补 5 条未验证项 | **替代方案缺席** |

### 10.3 复核者自己做的实验(原文没做的)

1. **逐像素比对工具**:纯 Python 的 PNG 解码 + 差分(无第三方依赖),脚本在
   复核者的 scratchpad。用它跑出 §3.4 ⚠️ 块里那张表(像素不同占比 / 最大通道差 /
   平均 |Δ| / Δ>8 占比),以及"§5.2 优化前后 4140 像素不同而指标不动"这条反证。
2. **原样重跑全部 bench / robust / mem / cpu 子命令**(含 `SPIKE_FASTCLIP` 两档),
   得到 §10.1 那张对照表。
3. **默认特性依赖树实测**:`cargo tree -p sv-shell --offline -e normal`,
   确认默认构建里没有 kurbo/peniko/serde/color/polycool。
4. **`Cargo.lock` 逐 crate 计数**,推翻"第一次引入 serde"。
5. **velato CI 溯源**:`gh api` 拉两个 workflow 全文 + `search/code` 定位
   `default_downloads.rs`,证伪"CI 用的同一批",并挖出 CC BY 4.0。
6. **产物 mtime 取证**,建立 §3.2/§5.2 与 §3.4 的代次分界。
7. **velato registry 源码通读**:`error.rs` / `runtime/render.rs` /
   `runtime/vello.rs` / `runtime/model/fixed.rs` / `import/builders.rs` 的 matte 映射
   / 8 处 `todo!`;`tiny-skia-0.11.4/src/painter.rs` 的变换传播。

### 10.4 复核者认定的**真实原创贡献**(不该被上面的批评淹没)

1. **track matte 被忽略/映射错时的真实视觉损失,首次量化** —— 正面关闭了
   `lottie-1` §8.6 挂着的未核实项("要下结论需要一组带 matte 的样本做像素对拍")。
   "整幅画布被填满(1046528/1048576)"这个证据比任何论述都有力。
2. **第一份实现了图层合成的 CPU sink,因此第一次给出"画对"的 CPU 成本** ——
   lottie-1 的 6.18 ms 是丢掉 121 条 `push_layer` 换来的,本文的 29 ms 才是真价。
   这条**必须回填进 lottie-1 §6.2**,否则仓库里留着一张误导性的预算表。
3. **GPU 侧的量级** —— 关闭 lottie-1 §8.1("velato 在 GPU 端的实机帧时间未测")。
   即便只是离屏口径,"编码 14 µs 且与分辨率无关"这个结论是稳的。
4. **`peniko::BlendMode` 是 `{mix, compose}` 二元组** 这条对未来
   `Painter::push_layer` 签名的约束 —— lottie-1/lottie-2 都只写了 `push_layer(alpha, blend)`,
   没人注意到 compose 那一半。**这是本文对设计面最有价值的一句话。**
5. **样本量从 2 个扩到 15 个**,并诚实声明了同质性(§9.5)。
6. **根裁剪透传优化从"应该做"变成"-23%、像素差 0.164%、最大差 2"**。

### 10.5 无法验证 / 复核者也没做的

1. **窗口路径的真实帧率** —— 同原文,任务禁止开窗。原文 §9.1 的自述是准确的。
2. **接进 `&mut dyn Painter` + `PathCmd` 摊平后的真实数字** —— 需要动
   `crates/`,超出本次文件权限。原文标为"推断不是实测",这个标注是对的。
3. **§3.4 那 5–7% 像素差的成因** —— 复核者的假设是渐变色标插值色彩空间不一致
   (velato README 自认 "Correct color stop handling" 是缺口),
   **需要一个单渐变最小样本对拍才能坐实,没做**。
4. **`velato_imaging` 的源码** —— 只核实了 crates.io 元数据(0.0.1 / 2026-05-21 /
   Apache-2.0 OR MIT / `forest-rs/imaging`),**没读代码**;
   "约 200 行、把栈失衡收成 `Error`"这两点转引自 `lottie-1` §1.3,复核者未独立验证。
5. **velato 的 6 个 open issue** —— 没读(与原文、与 lottie-1 §8.7 同)。
6. **鸿蒙/Linux/macOS、长时间播放内存、`.lottie` 容器、`vello_hybrid`** —— 均未验证,
   同原文 §9。
7. **本复核只动了这一个文件。** §10.2 里点名要回填 `lottie-1-ecology.md` §6.2
   (性能表矛盾)与 `lottie-2-architecture.md` §5.4(parity 口径要加逐像素项)、
   §5.2(渐变降级裁决被本文的证据挑战)的三处,**复核者无权限改,留给主线。**
