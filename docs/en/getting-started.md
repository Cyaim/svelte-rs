[中文](../zh-CN/getting-started.md) | **English**

# Getting Started

svelte-rs (working name `sv`) is an **exploratory prototype** of a Svelte-style desktop UI library for Rust. The core idea is Svelte 5's compilation philosophy applied to native desktop: templates are compiled at build time into code that performs pinpoint updates on a retained scene tree — no virtual DOM, no runtime diffing. Windows, Linux and macOS run today through a winit-based shell; HarmonyOS NEXT is a roadmap target (see [DESIGN.md](../DESIGN.md) (Chinese)). APIs churn freely at this stage — nothing here is stable.

The whole reactive model in four lines:

```rust
let count = state(0);                          // $state
let double = derived(move || count.get() * 2); // $derived
effect(move || println!("{}", double.get()));  // $effect
count.set(1);                                  // pinpoint trigger, no diff
```

## Prerequisites

- **Rust stable.** The workspace uses `edition = "2024"` (see `Cargo.toml`); there is no `rust-toolchain` pin, so a recent stable toolchain is all you need.
- A desktop environment. The prototype shell is pure CPU rendering (winit + softbuffer + tiny-skia + swash), so no GPU is required. An optional GPU backend exists behind the `backend-vello` cargo feature (see below).

## Clone and test

```sh
git clone https://github.com/Cyaim/svelte-rs
cd svelte-rs
cargo test          # full workspace test suite
```

> **Checkout inside OneDrive / Dropbox / another sync folder?** Uncomment and adjust the `target-dir` line in `.cargo/config.toml`. File syncers lock build artifacts (causing link failures and broken incremental builds on Windows) and `target/` generates gigabytes of pointless sync traffic:
>
> ```toml
> [build]
> # target-dir = "C:/cargo-target/svelte-rs"
> ```

The dev profile is tuned for UI iteration: workspace code builds at `opt-level = 1` and dependencies at `opt-level = 2`, so debug-mode demos stay responsive without release-build compile times.

## Run the examples

```sh
cargo run -p showcase       # feature tour — start here
cargo run -p counter        # counter, view! macro route
cargo run -p counter-sfc    # counter, .svelte single-file-component route
cargo run -p todo-sfc       # todo app, wider .svelte feature set
```

| Example | Front end | What it demonstrates |
|---|---|---|
| `showcase` | `.svelte` compiler | **Recommended tour**: `$bindable` two-way binding, children snippets, `{#snippet}/{@render}`, keyed `{#each}` (state survives reorder), scoped `<style>`, `{#await}`, `in:fade` |
| `counter` | `view!` macro | Minimal counter written directly in Rust with the proc-macro template |
| `counter-sfc` | `.svelte` compiler | Same counter, but the UI lives in `src/Counter.svelte`, compiled by `build.rs` into readable Rust in `$OUT_DIR` |
| `todo-sfc` | `.svelte` compiler | Components + `$props`, `{#each}{:else}`, `{@const}`, `{#key}`, `style:` directive, `$inspect` |
| `membench` | direct `sv-ui` API | Memory/frame-time benchmark harness (no `--png`, own CLI — see below) |

Windows open at a 480×400 logical size and are HiDPI-aware. The two counter examples build the same UI through the two template front ends — diff their sources for a feel of both routes.

Where the `.svelte` route looks like this (`count += 1` in the script is rewritten by the compiler into handle operations):

```text
<script>
let count = $state(0i32);
let double = $derived(count * 2);
</script>

<text>Count: {count} · double = {double}</text>
<button style="bg:#ff3e00; fg:#fff" onclick={|| count += 1}>+1</button>
```

## Offscreen rendering: `--png`

All four windowed examples accept `--png [path]` — render one frame to a PNG with no window, useful for CI and quick visual checks:

```sh
cargo run -p showcase -- --png out.png
cargo run -p counter -- --png out.png
```

If you omit the path, the default file names are `showcase.png`, `counter.png`, `counter-sfc.png` and `todo.png`. `showcase` and `todo-sfc` simulate a few clicks before the shot (incrementing a stepper, checking a row, reversing a list) so the PNG shows a non-empty state. `membench` does **not** take `--png`.

## The benchmark harness: `membench`

`membench` builds a synthetic UI of N controls (rows of view + checkbox + two texts + button, 5 nodes per row, with live bindings), optionally renders offscreen frames, prints one stats line, then sleeps so an external sampler can read process memory:

```sh
cargo run -p membench -- --controls 3000 --frames 3
cargo run --release -p membench --features backend-vello -- --backend vello
cargo run -p membench -- --windowed --mutate      # real window; pair with SV_SHOW_FPS=1
```

| Flag | Default | Meaning |
|---|---|---|
| `--controls N` | 3000 | Total control count (5 per row) |
| `--frames N` | 3 | Timed frames after a warm-up frame |
| `--hold-secs N` | 6 | Sleep after printing, for external memory sampling |
| `--backend cpu\|vello` | `cpu` | Offscreen render backend (`vello` needs the `backend-vello` feature) |
| `--no-render` | off | Build the tree only, skip rendering |
| `--mutate` | off | Drive incremental updates between frames |
| `--virtual` | off | Virtual-list mode: 30 instantiated rows over N logical rows |
| `--windowed` | off | Open a real window instead of offscreen |

Output is a single machine-readable line: `READY backend=… mutate=… virtual=… nodes=… signals=… build_ms=… warmup_ms=… frame_avg_ms=… p99_ms=… low1_fps=… fps=… frames=…`. Measured results live in [research/16-memory-benchmarks.md](../research/16-memory-benchmarks.md) (Chinese) and [research/17-backend-memory-fps.md](../research/17-backend-memory-fps.md) (Chinese).

## Workspace layout

| Path | Role |
|---|---|
| `crates/sv-reactive` | Runes reactive core: push-pull three-state dirty marking, effect ownership tree |
| `crates/sv-ui` | Retained scene tree (the "desktop DOM") + fine-grained binding primitives |
| `crates/sv-macro` | `view!` proc-macro template front end (parser only; template IR and codegen live in the shared sv-compiler core) |
| `crates/sv-compiler` | `.svelte` single-file-component compiler plus the template IR/codegen core shared by both front ends (runes source transform, Svelte template syntax) |
| `crates/sv-shell` | winit window + CPU raster shell; optional vello/wgpu backend behind `backend-vello` |
| `examples/*` | The five examples listed above |

Data flow: `state`/`derived` (sv-reactive) → effects mutate the scene tree (sv-ui) → version bump → `on_mutate` → redraw (sv-shell). Details in [architecture](./architecture.md).

## GPU backend in sixty seconds

The default renderer is the CPU stack. A vello/wgpu backend is compiled in with the `backend-vello` feature — among the examples, only `showcase` and `membench` forward that feature:

```sh
cargo run -p showcase --features backend-vello
```

Backend selection at startup (`SV_RENDERER` env var):

| `SV_RENDERER` | Behavior |
|---|---|
| unset | With the feature: probe for a GPU adapter, use vello if found, else warn and fall back to CPU. Without the feature: CPU |
| `cpu` | Force the CPU backend |
| `vello` | Use vello without pre-probing; falls back to CPU if surface creation fails (or warns and uses CPU if the feature wasn't compiled in) |

`SV_SHOW_FPS=1` disables the still-frame skip, redraws continuously, and prints `FPS …` every 30 frames — for diagnostics and benchmarking:

```powershell
# PowerShell
$env:SV_RENDERER = "vello"; $env:SV_SHOW_FPS = "1"
cargo run -p showcase --features backend-vello
```

```sh
# bash
SV_RENDERER=vello SV_SHOW_FPS=1 cargo run -p showcase --features backend-vello
```

More on backend architecture and the fallback chain in [rendering backends](./rendering-backends.md).

## Where next

- [Architecture](./architecture.md) — the crate layers and the no-VDOM data flow
- [Reactivity](./reactivity.md) — `state` / `derived` / `effect` and the single-threaded runtime rules
- [.svelte components](./sv-components.md) — the single-file-component format and build.rs integration
- [Rendering backends](./rendering-backends.md) — CPU stack, vello, and the migration plan
- [DESIGN.md](../DESIGN.md) (Chinese) — ADRs, roadmap, risk register
- [SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) (Chinese) — the 77-item Svelte 5 syntax support matrix
- [CSS-SUPPORT.md](../CSS-SUPPORT.md) (Chinese) — the 91-item modern-CSS gap matrix
