[中文](../zh-CN/rendering-backends.md) | **English**

# Rendering backends

sv renders through a small `Painter` trait in `crates/sv-shell/src/paint.rs`. Everything above it — layout, style resolution, text shaping, the shared `paint_tree` traversal — is backend-agnostic; everything below it is one of three interchangeable implementations: a CPU rasterizer (default), a GPU path on vello/wgpu, and a recording backend for golden tests. This is an exploratory prototype: the trait is deliberately minimal and will grow, and the CPU raster stack is explicitly a placeholder slated for replacement (see [../DESIGN.md](../DESIGN.md) (Chinese), ADR-3/3b).

## The Painter trait

Three verbs plus a capability probe. Coordinates are physical pixels — the caller has already multiplied by the window scale factor.

```rust
pub trait Painter {
    /// Capability bits (all false by default; callers degrade per caps)
    fn caps(&self) -> PainterCaps { PainterCaps::default() }
    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color);
    fn stroke_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32,
                           width: f32, color: Color);
    /// A run of already-positioned glyphs (shaping done upstream;
    /// the backend only rasterizes / uploads)
    fn glyph_run(&mut self, glyphs: &[GlyphPos], color: Color);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PainterCaps {
    /// Can composite an external wgpu texture (prerequisite for `<surface3d>`)
    pub external_texture: bool,
    /// Can do gaussian blur (prerequisite for box-shadow / backdrop-filter)
    pub blur: bool,
}
```

The one shared traversal, `paint_tree(doc, placed, painter: &mut dyn Painter, scale)` in `render.rs`, emits the same command stream to every backend. `dyn` stays inside the sv-shell boundary — it never leaks type parameters into sv-ui or compiler output; at the low thousands of dynamic calls per frame the cost is single-digit microseconds.

| caps | `TinySkiaPainter` | `VelloPainter` | `RecordingPainter` |
|---|---|---|---|
| `external_texture` | false | false (not wired up yet) | false |
| `blur` | false | **true** (vello has `draw_blurred_rounded_rect`; no consumer until box-shadow lands) | false |

## Why shaping sits above the trait

`glyph_run` takes **positioned glyphs, not strings and not bitmaps**. Shaping (text → glyph ids + baseline origins) happens once, upstream, in `shape_text` in `render.rs`; rasterization happens inside each backend. Passing strings down would force every backend to reimplement shaping; passing bitmaps up would foreclose the GPU path, which wants glyph ids for `Scene::draw_glyphs` — the paint.rs doc comment calls these the twin lessons of Slint's software renderer and GPU-renderer text handling.

`GlyphPos` therefore carries both addressing schemes:

```rust
pub struct GlyphPos {
    pub key: GlyphKey,      // CPU path: raster cache key (glyph id + f32 px bits)
    pub x: f32, pub y: f32, // CPU path: baseline origin
    pub id: u16,            // GPU path: glyph id
    pub ox: f32, pub oy: f32, // GPU path: baseline origin (draw_glyphs semantics)
}
```

When the M2 Parley migration replaces the linear shaper, only the shaping facade above the trait changes — no backend is touched.

## The three implementations

| Backend | Kind | Presents via | Role |
|---|---|---|---|
| `TinySkiaPainter` | CPU (tiny-skia 0.11 + swash) | softbuffer | Default; capability-frozen transition stack and test baseline |
| `VelloPainter` / `VelloWin` | GPU (vello 0.9 on wgpu 29) | `render_to_texture` + blitter | Feature `backend-vello`; second real backend |
| `RecordingPainter` | Display list | — | Golden tests; future inter-frame diff carrier |

**TinySkiaPainter** draws into a `tiny_skia::Pixmap`; the window path copies the pixmap into a softbuffer surface. Glyphs are alpha-coverage bitmaps blended per pixel, served from the glyph cache described below. Offscreen rendering (`render_to_png`, `cargo run -p counter -- --png out.png`) always uses this backend.

**VelloPainter** maps the three verbs 1:1 onto a `vello::Scene` (`fill`/`stroke`/`draw_glyphs`). The window presenter `VelloWin` holds a `RenderContext` + `Renderer` + `RenderSurface` and, because vello 0.9 has no `render_to_surface`, renders to a texture and blits it to the swapchain (`TextureBlitter::copy` → `present`), with `PresentMode::AutoVsync`. On static frames `render_cached` skips scene re-encoding and only re-presents. The offscreen entry `render_frame_vello` returns raw RGBA8 bytes (or `None` without a GPU adapter) and builds its own wgpu device with `max_storage_buffer_binding_size` raised to the adapter's actual limit — vello's `RenderContext` pins `Limits::default()` (128 MB), which the ~192 MB scene buffer of the 100k-control benchmark used to overflow. A parity test (`vello_offscreen_parity`) checks GPU and CPU output agree within a 0.5–2.0 non-white-pixel ratio.

**RecordingPainter** records simplified commands (`PaintCmd::FillRect` / `StrokeRect` / `Glyphs { count, color }`, values rounded to integers for stable snapshots). The `recording_painter_golden` test asserts the exact command stream for a styled card — zero pixels, zero GPU — and that replaying it is deterministic. New backends are validated against this stream first; it is also the planned carrier for incremental scene encoding.

## Backend selection

Three layers: compile-time feature × runtime env override × automatic probe.

```rust
fn select_backend() -> Backend {
    match std::env::var("SV_RENDERER").ok().as_deref() {
        Some("cpu")   => Backend::Cpu,
        Some("vello") => vello_or_fallback(true),   // explicit: skip pre-probe
        _             => vello_or_fallback(false),  // auto: probe adapter
    }
}
```

- Without the `backend-vello` feature the binary is CPU-only; `SV_RENDERER=vello` prints a warning and runs on CPU.
- With the feature, the default path probes for a wgpu adapter and falls back to CPU when none is found (a note goes to stderr — the app always starts). An explicit `SV_RENDERER=vello` skips the pre-probe; if surface/renderer creation then fails at window open, it falls back to CPU once more.

```sh
cargo run -p showcase                                    # CPU (default build)
cargo run -p showcase --features backend-vello           # vello compiled in, auto-probe
SV_RENDERER=cpu cargo run -p showcase --features backend-vello   # force CPU at runtime
cargo run -p membench --release --features backend-vello -- --backend vello --controls 3000
```

## Environment knobs

| Variable | Values | Effect |
|---|---|---|
| `SV_RENDERER` | `cpu` \| `vello` | Override backend selection (see above) |
| `SV_VELLO_AA` | `area` \| `msaa8` \| `msaa16` | vello antialiasing; default `msaa16`. `area` is analytic AA with zero MSAA buffers — measured memory-neutral on the dev machine (research 17) |
| `SV_SHOW_FPS` | `1` | Continuous redraw + print `FPS <n>` to stdout every 30 frames (benchmark/diagnostics) |

## The text stack

- **Font loading** (`font.rs`): one global UI font, found by probing OS paths (Windows: `msyh.ttc` → `segoeui.ttf` → `arial.ttf`; macOS: PingFang/Helvetica; Linux: Noto CJK/DejaVu). Bytes are read once into an `Arc` and shared zero-copy between swash (`FontRef` lazily locates table offsets) and vello's `peniko::FontData`.
- **Why swash 0.2.10**: the previous fontdue stack eagerly parsed all CJK outlines — ~173 MB resident and a 573 ms first frame. The swash migration dropped baseline memory 198 → 27 MB and first-frame warmup to 11 ms, and roughly doubled CPU raster speed at the 30k tier ([../research/18-million-controls-144fps.md](../research/18-million-controls-144fps.md) (Chinese)).
- **Shaping is still linear**: charmap lookup + advance per character — no kerning, no ligatures, no bidi. Parley (fontique fallback chains + HarfRust shaping) is the M2 plan; only the shaping facade changes.
- **Glyph coverage cache** (CPU path): keyed by `GlyphKey { id, px_bits }`, thread-local, 2048-entry hot generation. Eviction is generational: when hot fills up, the whole generation demotes to cold (old cold is dropped); active glyphs re-promote from cold at zero cost on next hit. This replaced clear-all eviction, whose "re-rasterize everything" frame was a 1%-low tail spike. Memory upper bound ≈ 2 generations ≈ 2.6 MB at 16 px.

## Measured characteristics

Dev machine numbers (Windows 11, DX12, release, 1920×1080 offscreen; full-tree build, post-swash) — from [../research/17-backend-memory-fps.md](../research/17-backend-memory-fps.md) and [../research/18-million-controls-144fps.md](../research/18-million-controls-144fps.md) (both Chinese):

| Controls | CPU frame avg | CPU WS | vello frame avg* | vello WS |
|---|---|---|---|---|
| 0 | 2.2 ms | 27.4 MB | 9.1 ms | 625 MB |
| 3 000 | 7.2 ms | 29.7 MB | 23.2 ms | 636 MB |
| 30 000 | 46.7 ms | 40.2 MB | 151.2 ms | 648 MB |
| 100 000 | 150.8 ms | 68.8 MB | 452.5 ms | — |

\* Offscreen vello includes a ~7 ms serialized buffer-readback floor — a benchmark-tool cost, not a product-path cost.

Set expectations accordingly:

- **CPU baseline is ~27 MB working set.** UI marginal cost is ~0.5 KB per control.
- **vello adds a fixed ~600 MB** device + pipeline cost on the dev machine (DX12; one-time, no per-frame leak; MSAA is not the culprit — `SV_VELLO_AA=area` was memory-neutral). The value varies widely with driver/backend.
- **The vello window path is vsync-capped**: `AutoVsync` pins light loads at 60 fps, while the CPU softbuffer path (no vsync) showed ~800 fps in the same 1M-control virtual-list smoke test. A mailbox/immediate present mode is a planned ADR-6 work item, not a capability gap.
- Neither backend survives 30k+ full-tree controls at 60 fps — the answer at scale is viewport virtualization, not backend choice. See [./performance.md](./performance.md).

## Which tier to pick (ADR-3b)

- **Light UI (≤ ~3000 controls): CPU backend, decisively.** 7.2 ms @ 3000 controls, ~2 MB incremental memory; vello's fixed cost buys nothing here.
- **GPU pays off for capability, not for small rectangles**: gradients, blur/shadows, transforms, and large-area repaints are where vello wins — `caps().blur` is already true there.
- **Planned tiers** (not implemented): `vello_hybrid` for HarmonyOS (GLES-level GPU, matching wgpu-on-OHOS reality) and `vello_cpu` to replace the capability-frozen tiny-skia stack as the CPU tier — three tiers sharing one imaging model. Roadmap in [../DESIGN.md](../DESIGN.md) (Chinese).

## See also

- [./architecture.md](./architecture.md) — where the render layer sits in the data flow
- [./performance.md](./performance.md) — virtualization, frame budgets, benchmark methodology
- [../DESIGN.md](../DESIGN.md) (Chinese) — ADR-3/3b: backend verdicts and the switchable-Painter decision
- [../research/14-switchable-painter.md](../research/14-switchable-painter.md) (Chinese) — switchability precedents (Slint, iced, Flutter, anyrender)
