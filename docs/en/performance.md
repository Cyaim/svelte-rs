[中文](../zh-CN/performance.md) | **English**

# Performance

> Status: exploratory prototype. Every number on this page was measured in July 2026 on one dev machine (Windows 11, DX12, release build, 1920×1080 offscreen) and is recorded in [research note 18](../research/18-million-controls-144fps.md) (Chinese). APIs churn; read this as a snapshot of the model, not a benchmark contract.

sv's performance story is architectural rather than constant-tuning: templates compile to pinpoint updates of a retained scene tree, so per-frame cost scales with *what changed*, not with *how big the UI is*. There is no virtual DOM and no diff pass.

## The performance model

The data flow is: `state`/`derived` → an effect mutates the scene tree → version bump → `on_mutate` → redraw. On top of it, these mechanisms keep frame cost down:

| Mechanism | Where | What it does |
|---|---|---|
| Pinpoint updates | sv-reactive + sv-ui bindings (`bind_text`, …) | A signal write patches exactly the nodes bound to it; no tree-wide diff ever runs |
| Version-keyed layout cache | `sv_shell::render::layout_tree_cached(doc, logical_w, logical_h)` | Layout result cached per (doc identity, doc version, size); a static frame does zero measure/place work |
| Static-frame skip | sv-shell window loop | If (version, size, scale) equal the previous frame's and nothing is animating, the frame is skipped entirely — idle cost is zero |
| vello scene-encode skip | `render_cached(doc, scale, scene_unchanged)` in the vello backend | On a static frame, only re-presents the surface; the vello scene is not re-encoded |
| Glyph coverage cache with generational eviction | sv-shell paint layer | swash alpha-coverage bitmaps cached in two generations (hot/cold, 2048 entries each). A full hot generation is demoted wholesale to cold (the old cold generation is dropped); a cold hit promotes back for free. A frame can at most re-rasterize glyphs unused for an entire generation — unlike a full cache clear, the active working set is never wiped, so there is no frame-time spike hurting 1% lows. Upper bound 2 × 2048 entries ≈ 2.6 MB at 16 px |

One caveat: the layout cache is all-or-nothing. Any version bump relayouts the whole tree (partial layout is planned, see below). Under virtualization the live tree is tiny, so this is currently unnoticeable.

## Virtualization: `sv_ui::virtual_list`

Full materialization is linear in tree size (table below), so at scale the answer is to shrink the per-frame working set. The primitive, from `crates/sv-ui/src/lib.rs`:

```rust
pub fn virtual_list<T: Clone + 'static>(
    doc: &Doc,
    parent: ViewId,
    count: impl Fn() -> usize + 'static,
    offset: sv_reactive::Signal<usize>,
    viewport_rows: usize,
    item_at: impl Fn(usize) -> T + 'static,
    row: impl Fn(&Doc, ViewId, sv_reactive::Signal<Option<T>>, usize) + 'static,
)
```

How it works:

- **Fixed slots** — `viewport_rows` real rows are built exactly once; each slot holds a `Signal<Option<T>>` that the row builder binds against.
- **Scrolling = data swap** — when `offset` (or `count`) changes, one effect writes each slot via `.set()`; the row's bindings update in place. Zero node creation/destruction, zero structural change — this is where stable 1% lows come from (no "occasional rebuild on a scroll frame" tail).
- **Lazy items** — `item_at(i)` is called only for visible indices; a million-row list is never materialized.
- `None` in a slot means out of range; the row renders its own empty state.

The unit test `virtual_list_million_rows_few_nodes` pins this down: 1,000,000 logical rows with a 30-slot viewport produce a scene tree of at most 34 nodes, and `offset.set(500_000)` updates in place without adding or removing a single node.

```rust
use sv_reactive::{create_root, state};
use sv_ui::{Doc, bind_text, virtual_list};

let doc = Doc::new();
let offset = state(0usize);
let (_, _scope) = create_root(|| {
    virtual_list(
        &doc,
        doc.root(),
        || 1_000_000,           // logical row count (a closure — may be reactive)
        offset,                 // scroll position, in rows
        30,                     // viewport slots: the only rows that ever exist
        |i| format!("row {i}"), // lazy fetch, called for visible indices only
        |doc, parent, slot, _i| {
            let label = doc.create_text("");
            doc.append(parent, label);
            bind_text(doc, label, move || slot.get().unwrap_or_default());
        },
    );
});

offset.set(500_000); // scroll: 30 slot writes, zero nodes created or destroyed
```

(Inside `sv_shell::run_app` the build closure already runs in a root scope; the explicit `create_root` is only needed standalone.)

**When to reach for it**: lists beyond a few thousand rows. At 3,000 fully materialized controls the CPU backend already spends ≈7.2 ms/frame — the entire 144 Hz budget — and at 10,000 it misses even 60 Hz. Below a few thousand, full materialization is fine.

## The million-widget result

Setup (membench, research 18): 200,000 logical rows × 5 controls each (row container + checkbox + two texts + button) = 1,000,000 logical controls; 30-row viewport; `--mutate` advances the scroll offset by one row *every frame* — continuous scroll, the worst case for virtualization.

| Configuration | frame avg | p99 | **1% low** | working set |
|---|---|---|---|---|
| CPU offscreen 1920×1080, 500 frames | 3.41 ms | **5.28 ms** | **174 fps** | 28.2 MB |
| CPU windowed (softbuffer, no vsync cap) | — | — | ~800 fps (FPS counter) | 37.9 MB |
| vello offscreen (≈7 ms readback floor) | 9.26 ms | 16.3 ms | 56 fps | 633 MB |
| vello windowed | — | — | 60 fps (vsync-pinned) | 598 MB |

The 144 Hz frame budget is 6.94 ms. CPU p99 = 5.28 ms and 1% low = 174 fps clear it, with a 28 MB working set that does not grow with the logical control count. The two windowed rows measure presentation, not capacity: softbuffer has no vsync cap (hence ~800 fps), while the vello surface runs AutoVsync and is pinned at 60 — lifting that needs a mailbox/immediate present mode (ADR-6, not implemented). Offscreen vello also carries a ≈7 ms texture-readback floor the windowed path doesn't have; subtracting it leaves ≈2.3 ms/frame.

## Why virtualization: full-materialization costs

Same row mix, no virtualization, CPU backend (tiny-skia + swash):

| Controls | frame avg | p99 | 1% low | working set |
|---|---|---|---|---|
| 0 | 2.2 ms | 3.6 ms | 275 fps | 27.4 MB |
| 1,000 | 5.0 ms | 6.1 ms | 163 fps | 28.4 MB |
| 3,000 | 7.2 ms | 9.1 ms | 110 fps | 29.7 MB |
| 10,000 | 17.5 ms | 20.5 ms | 49 fps | 32.2 MB |
| 30,000 | 46.7 ms | 51.9 ms | 19 fps | 40.2 MB |
| 100,000 | 150.8 ms | 154.8 ms | 6 fps | 68.8 MB |

Frame cost is linear in tree size because every frame relayouts and re-encodes the full tree. Constant-factor work moves the curve but not its slope — the swash migration roughly halved CPU raster time, yet 100k controls still cost 150.8 ms. Full materialization on vello is strictly worse in this benchmark (3,000 ≈ 23.2 ms offscreen; 100,000 ≈ 452.5 ms — slow but no longer crashing after raising the wgpu storage-buffer limits); see [Rendering backends](./rendering-backends.md) for where the GPU backend actually pays off.

## Memory rules of thumb

| Item | Cost |
|---|---|
| UI side (scene tree + signals) | ≈0.5 KB per control; 100k fully materialized controls add ≈41 MB over baseline |
| Text stack baseline (swash) | ≈27 MB working set at `--controls 0` (20.5 MB private, 11 ms first-frame warmup); was ~200 MB with fontdue |
| vello device cost | ≈600 MB fixed wgpu/DX12 pipeline cost on the dev machine, independent of content |
| Glyph coverage cache | ≤ 2 generations × 2048 entries ≈ 2.6 MB at 16 px |

## The membench harness

`examples/membench` builds N controls of a typical mixed UI, optionally renders timed offscreen frames, prints one stats line, then sleeps so an external tool (e.g. PowerShell) can sample the process WorkingSet/Private.

```sh
# 1M logical controls, virtualized, continuous scroll, 500 timed frames
cargo run -p membench --release -- --controls 1000000 --virtual --mutate --frames 500

# vello backend (requires the feature)
cargo run -p membench --release --features backend-vello -- --controls 3000 --backend vello

# windowed smoke run with live FPS counter
SV_SHOW_FPS=1 cargo run -p membench --release -- --controls 1000000 --virtual --mutate --windowed
```

| Flag | Default | Meaning |
|---|---|---|
| `--controls N` | 3000 | Total logical controls; rows = N / 5 (view + checkbox + 2 texts + button per row) |
| `--backend cpu\|vello` | `cpu` | Offscreen renderer; `vello` needs `--features backend-vello` |
| `--frames N` | 3 | Timed frames, measured after one untimed warmup frame (font parsing, pipeline compile) |
| `--no-render` | off | Build the tree only — for build-time and memory numbers |
| `--mutate` | off | One mutation per frame: bump a row signal, or advance the scroll offset by one row in `--virtual` mode |
| `--virtual` | off | Use `virtual_list` (30-row viewport) instead of materializing all rows |
| `--windowed` | off | Open a real window (true presentation path); combine with `SV_SHOW_FPS=1` and `SV_RENDERER=cpu\|vello` |
| `--hold-secs N` | 6 | Sleep after `READY` so an external sampler can read memory |

Output is a single line (then the process holds):

```
READY backend=<cpu|vello> mutate=<bool> virtual=<bool> nodes=<n> signals=<n> build_ms=<n> warmup_ms=<n> frame_avg_ms=<f> p99_ms=<f> low1_fps=<f> fps=<f> frames=<n>
```

| Field | Meaning |
|---|---|
| `nodes` | Scene-tree node count |
| `signals` | Live reactive-graph node count (`sv_reactive::debug_node_count()`) |
| `build_ms` | Time to build the tree |
| `warmup_ms` | The untimed first frame (includes font parsing / pipeline compilation) |
| `frame_avg_ms` / `fps` | Mean timed-frame cost and its fps equivalent |
| `p99_ms` | 99th-percentile frame time |
| `low1_fps` | Mean of the worst 1% of frames, converted to fps — the 144 Hz acceptance metric |

Caveat: offscreen vello timing includes texture readback (≈7 ms), slightly overstating frame cost relative to the windowed path.

## Landed since the first draft of this page

- **Frame pacing — batched flush (ADR-6, landed 2026-07-22).** Windowed writes no longer run effects on the spot; they accumulate to the frame boundary and the shell flushes once. What is still open is the *deep* vsync alignment (a mailbox present mode) that would let the vello window break past 60 fps — see DESIGN.md ADR-6 / ADR-9.
- **Change classification + partial layout (this round).** A version bump is now graded Paint / Position / rebuild; scroll, typing and colour changes no longer trigger a full relayout (≈6k-node scroll list: full 29 ms → scroll frame 0.66 ms). The remaining gap is incremental `mark_dirty` for structural/text changes, which still rebuild the whole layout tree.

## Planned, not implemented

The roadmap and ADRs live in [DESIGN.md](../DESIGN.md) (Chinese):

- **Dirty-rectangle painting** — the current true bottleneck. A scroll frame is 12.45 ms end-to-end but only 0.66 ms of that is layout; the other ~12 ms is paint, redrawn full-window every frame. This is also the shared prerequisite for animation power savings (Lottie/PAG/VAP redraw the whole window while playing).
- **Incremental scene encoding** — diffing the RecordingPainter command stream, to cut frame cost for full-materialization scenes too.
- **Mailbox present mode (ADR-6 / ADR-9)** — the present-mode choice that pins the vello window at 60 fps.
- **Scroll physics** — inertia and pixel-level offsets on top of `virtual_list`.

## See also

- [Rendering backends](./rendering-backends.md) — CPU vs vello, backend selection, `SV_RENDERER`.
- [Research note 18: a million controls at 144 fps](../research/18-million-controls-144fps.md) (Chinese) — the full measurement log behind this page.
- [DESIGN.md](../DESIGN.md) (Chinese) — ADR-9 (scale strategy) and ADR-6 (frame pacing).
