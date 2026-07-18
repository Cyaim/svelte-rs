**中文** | [English](../en/rendering-backends.md)

# 渲染后端

sv 的渲染收敛在 `crates/sv-shell/src/paint.rs` 的一个小 trait —— `Painter` 上。边界之上(布局、样式解析、文本 shaping、共享的 `paint_tree` 遍历)完全不感知后端;边界之下是三个可互换的实现:CPU 光栅(默认)、vello/wgpu GPU 路径、以及用于金样测试的记录型后端。本项目是探索原型:trait 刻意保持最小、会继续演进,CPU 光栅栈明确是过渡方案、以替换为目标(见 [../DESIGN.md](../DESIGN.md) ADR-3/3b)。

## Painter trait

三个动词加一个能力探测。坐标是物理像素——调用方已经乘过窗口缩放。

```rust
pub trait Painter {
    /// 能力位(默认全 false;调用方按 caps 降级)
    fn caps(&self) -> PainterCaps { PainterCaps::default() }
    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color);
    fn stroke_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32,
                           width: f32, color: Color);
    /// 一段已定位字形(shaping 已在上游完成;后端只负责光栅/上屏)
    fn glyph_run(&mut self, glyphs: &[GlyphPos], color: Color);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PainterCaps {
    /// 能否合成外部 wgpu 纹理(`<surface3d>` 的前置)
    pub external_texture: bool,
    /// 能否做高斯模糊(box-shadow / backdrop-filter 的前置)
    pub blur: bool,
}
```

`render.rs` 里唯一的共享遍历器 `paint_tree(doc, placed, painter: &mut dyn Painter, scale)` 对所有后端发出同一条命令流。`dyn` 只存在于 sv-shell 边界内,绝不把类型参数上浮到 sv-ui 或编译器产物;每帧低千级动态调用的开销约为个位数微秒。

| caps | `TinySkiaPainter` | `VelloPainter` | `RecordingPainter` |
|---|---|---|---|
| `external_texture` | false | false(尚未接线) | false |
| `blur` | false | **true**(vello 有 `draw_blurred_rounded_rect`;等 box-shadow 落地才有消费方) | false |

## 为什么 shaping 在 trait 之上

`glyph_run` 拿到的是**定位好的字形,既不是字符串也不是位图**。shaping(文本 → 字形 id + 基线原点)在上游的 `shape_text`(`render.rs`)统一做一次;光栅化则在各后端内部完成。如果把字符串传下去,每个后端都得各自实现一遍 shaping;如果把位图传上来,GPU 路径就被堵死了——`Scene::draw_glyphs` 要的是字形 id。paint.rs 的文档注释称之为 Slint 软件渲染器与 GPU 渲染器文本处理的双重教训。

因此 `GlyphPos` 同时携带两套寻址:

```rust
pub struct GlyphPos {
    pub key: GlyphKey,      // CPU 路径:光栅缓存键(字形 id + 字号 f32 位模式)
    pub x: f32, pub y: f32, // CPU 路径:基线原点
    pub id: u16,            // GPU 路径:字形 id
    pub ox: f32, pub oy: f32, // GPU 路径:基线原点(draw_glyphs 语义)
}
```

M2 换 Parley 时,只动 trait 之上的 shaping 门面,任何后端都不用改。

## 三个实现

| 后端 | 类型 | 上屏方式 | 定位 |
|---|---|---|---|
| `TinySkiaPainter` | CPU(tiny-skia 0.11 + swash) | softbuffer | 默认;能力冻结的过渡栈与测试基准 |
| `VelloPainter` / `VelloWin` | GPU(vello 0.9,wgpu 29) | `render_to_texture` + blit | feature `backend-vello`;第二个真实后端 |
| `RecordingPainter` | 显示列表 | — | 金样测试;未来帧间 diff 载体 |

**TinySkiaPainter** 画进 `tiny_skia::Pixmap`,窗口路径把 pixmap 拷进 softbuffer surface。字形是 alpha 覆盖度位图逐像素混合,数据来自下文的字形缓存。离屏渲染(`render_to_png`、`cargo run -p counter -- --png out.png`)固定走这个后端。

**VelloPainter** 把三个动词 1:1 映射到 `vello::Scene`(`fill`/`stroke`/`draw_glyphs`)。窗口呈现器 `VelloWin` 持有 `RenderContext` + `Renderer` + `RenderSurface`;因为 vello 0.9 没有 `render_to_surface`,先渲染到纹理再 blit 到交换链(`TextureBlitter::copy` → `present`),present 模式为 `AutoVsync`。静止帧走 `render_cached` 跳过场景重编码、只重呈现。离屏入口 `render_frame_vello` 返回紧致 RGBA8 字节(无 GPU adapter 时返回 `None`),并自建 wgpu device、把 `max_storage_buffer_binding_size` 抬到 adapter 实际能力——vello 的 `RenderContext` 固定 `Limits::default()`(128MB),10 万控件档约 192MB 的 scene buffer 曾把它撑爆。对拍测试 `vello_offscreen_parity` 校验 GPU/CPU 出图的非白像素比落在 0.5–2.0 区间。

**RecordingPainter** 记录简化命令(`PaintCmd::FillRect` / `StrokeRect` / `Glyphs { count, color }`,数值取整以保证快照稳定)。`recording_painter_golden` 测试对一张带样式的卡片断言精确命令流——零像素、零 GPU——并验证重放是确定性的。新后端先对拍这条命令流;它也是增量场景编码的预留载体。

## 后端选择

三层机制:编译期 feature × 运行时环境变量覆盖 × 自动探测。

```rust
fn select_backend() -> Backend {
    match std::env::var("SV_RENDERER").ok().as_deref() {
        Some("cpu")   => Backend::Cpu,
        Some("vello") => vello_or_fallback(true),   // 显式指定:跳过预探测
        _             => vello_or_fallback(false),  // 自动:探测 adapter
    }
}
```

- 不开 `backend-vello` feature 时二进制只有 CPU 后端;`SV_RENDERER=vello` 会打印警告并继续用 CPU。
- 开了 feature,默认路径先探测 wgpu adapter,拿不到就自动回退 CPU(stderr 有一条提示——应用永远能启动)。显式 `SV_RENDERER=vello` 跳过预探测;若开窗时建 surface/renderer 失败,再回退一次 CPU。

```sh
cargo run -p showcase                                    # CPU(默认构建)
cargo run -p showcase --features backend-vello           # 编入 vello,自动探测
SV_RENDERER=cpu cargo run -p showcase --features backend-vello   # 运行时强制 CPU
cargo run -p membench --release --features backend-vello -- --backend vello --controls 3000
```

## 环境变量

| 变量 | 取值 | 作用 |
|---|---|---|
| `SV_RENDERER` | `cpu` \| `vello` | 覆盖后端选择(见上) |
| `SV_VELLO_AA` | `area` \| `msaa8` \| `msaa16` | vello 抗锯齿方式,默认 `msaa16`。`area` 是解析式 AA、零 MSAA 缓冲——本机实测对内存无影响(调研 17) |
| `SV_SHOW_FPS` | `1` | 连续重绘 + 每 30 帧向 stdout 打印一行 `FPS <n>`(基准/诊断用) |

## 文本栈

- **字体加载**(`font.rs`):全局单字体,按系统路径逐候选探测(Windows:`msyh.ttc` → `segoeui.ttf` → `arial.ttf`;macOS:PingFang/Helvetica;Linux:Noto CJK/DejaVu)。字节只读一次进 `Arc`,swash(`FontRef` 懒定位表偏移)与 vello 的 `peniko::FontData` 零拷贝共用同一份数据。
- **为什么是 swash 0.2.10**:此前的 fontdue 栈会急切解析全部 CJK 轮廓——约 173MB 常驻、首帧 573ms。迁移到 swash 后基线内存 198 → 27MB、首帧预热降到 11ms,30k 档 CPU 光栅还快了约一倍([../research/18-million-controls-144fps.md](../research/18-million-controls-144fps.md))。
- **shaping 仍是线性排版**:charmap 逐字映射 + advance 推进——无 kerning、无连字、无 bidi。M2 计划迁 Parley(fontique 回退链 + HarfRust 整形),届时只动 shaping 门面。
- **字形覆盖度缓存**(CPU 路径):以 `GlyphKey { id, px_bits }` 为键,线程级,热代上限 2048 条。淘汰是分代式的:热代满则整代降为冷代(旧冷代丢弃),活跃字形下次命中时从冷代零成本晋升。这替代了原先的整体清空——"清空后那一帧全量重光栅"正是 1% low 长尾的来源。内存上界约两代 ≈ 2.6MB @16px。

## 实测特征

开发机数据(Windows 11,DX12,release,1920×1080 离屏;全量建树口径,swash 迁移后)——出自 [../research/17-backend-memory-fps.md](../research/17-backend-memory-fps.md) 与 [../research/18-million-controls-144fps.md](../research/18-million-controls-144fps.md):

| 控件数 | CPU 帧均 | CPU WS | vello 帧均* | vello WS |
|---|---|---|---|---|
| 0 | 2.2ms | 27.4MB | 9.1ms | 625MB |
| 3 000 | 7.2ms | 29.7MB | 23.2ms | 636MB |
| 30 000 | 46.7ms | 40.2MB | 151.2ms | 648MB |
| 100 000 | 150.8ms | 68.8MB | 452.5ms | — |

\* vello 离屏含约 7ms 的串行回读地板——这是基准工具的成本,不是产品路径的成本。

对预期的校准:

- **CPU 基线约 27MB 工作集**,UI 边际成本约 0.5KB/控件。
- **vello 带来约 600MB 固定成本**(本机 DX12 的 device + 管线;一次性,无逐帧泄漏;MSAA 不是元凶——`SV_VELLO_AA=area` 对内存无影响)。该值随驱动/后端波动很大。
- **vello 窗口路径被 vsync 封顶**:`AutoVsync` 把轻负载钉在 60fps,而 CPU softbuffer 路径(无 vsync)在同一个百万控件虚拟列表冒烟里跑到约 800fps。mailbox/immediate present 是 ADR-6 的既定工程项,不是能力缺口。
- 30k+ 全量建树两个后端都到不了 60fps——规模问题的正解是视口虚拟化,不是换后端。见 [./performance.md](./performance.md)。

## 分档建议(ADR-3b)

- **轻量 UI(≤ 约 3000 控件):CPU 后端,压倒性胜出。** 3000 控件 7.2ms、增量内存约 2MB;vello 的固定成本在这里毫无回报。
- **GPU 的价值在能力,不在小矩形**:渐变、模糊/阴影、transform、大面积重绘才是 vello 的主场——那里 `caps().blur` 已经是 true。
- **规划中的档位**(未实现):鸿蒙用 `vello_hybrid`(GLES 级 GPU,匹配 wgpu-OHOS 现状);`vello_cpu` 替换能力冻结的 tiny-skia 栈作为 CPU 档——三档共享同一 imaging model。路线图见 [../DESIGN.md](../DESIGN.md)。

## 相关阅读

- [./architecture.md](./architecture.md) — 渲染层在数据流中的位置
- [./performance.md](./performance.md) — 虚拟化、帧预算与基准方法学
- [../DESIGN.md](../DESIGN.md) — ADR-3/3b:后端判决与可切换 Painter 决策
- [../research/14-switchable-painter.md](../research/14-switchable-painter.md) — 可切换性先例(Slint、iced、Flutter、anyrender)
