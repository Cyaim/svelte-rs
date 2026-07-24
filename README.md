[中文](./README.zh-CN.md) | **English**

# svelte-rs (working name `sv`)

[![CI](https://github.com/Cyaim/svelte-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Cyaim/svelte-rs/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

A Svelte-style cross-platform desktop UI library in Rust — **exploratory prototype**.

The idea: bring Svelte 5's compilation philosophy to native desktop. Templates compile
into **pinpoint updates** against a retained scene tree — no virtual DOM, no diffing,
no rebuild at runtime. Target platforms: Windows / Linux / macOS / HarmonyOS NEXT.

```rust
let count = state(0);                          // $state
let double = derived(move || count.get() * 2); // $derived
effect(move || println!("{}", double.get()));  // $effect
count.set(1);                                  // precise trigger, no diff
```

Or the same thing in a `.svelte` single-file component — real Svelte template syntax,
real Rust expressions (the compiler rewrites `count += 1` into handle operations):

```text
<script>
let count = $state(0i32);
let double = $derived(count * 2);
</script>

<text>Count: {count} · doubled = {double}</text>
<button style="bg:#ff3e00; fg:#fff" onclick={|| count += 1}>+1</button>
{#if count > 5}
  <text fg="#ff3e00">Past five!</text>
{/if}
```

## Highlights

- **Runes kernel in Rust** — `state` / `derived` (writable) / `effect` / `batch` /
  `untrack` / context, push-pull three-state dirty marking, effect ownership tree.
- **Two compiler frontends, one target** — a `view!` proc-macro and a `.svelte`
  single-file-component compiler (build.rs integration) both emit calls to the same
  scene-tree binding primitives.
- **Real CSS, compiled** — a closed subset of actual CSS syntax (`:hover`, `:root`
  variables, nesting, inheritance) resolved at build time; zero runtime selector engine.
- **Switchable render backends** — CPU (tiny-skia + swash) and GPU (vello 0.9 / wgpu)
  behind one `Painter` trait; select per build, per env (`SV_RENDERER`), with automatic
  fallback.
- **Scales to a million widgets** — viewport virtualization (`virtual_list`) measured at
  1,000,000 logical controls: p99 = 5.28 ms, 1% low = 174 fps, 28 MB working set
  (CPU backend, continuous-scroll worst case).

## Quick start

```sh
cargo test                    # run the whole test suite
cargo run -p showcase         # feature tour (recommended first stop)
cargo run -p counter          # counter, view! macro route
cargo run -p counter-sfc      # counter, .svelte compiler route (UI in src/Counter.svelte)
cargo run -p showcase -- --png out.png  # render one frame offscreen, no window needed
```

> If the checkout lives inside OneDrive or another synced folder, enable the
> commented-out `target-dir` in `.cargo/config.toml` to keep build artifacts out
> of the sync scope.

## Documentation

The documentation center lives in [`docs/`](docs/README.md) — guides in
[English](docs/en/getting-started.md) and [中文](docs/zh-CN/getting-started.md):

| Guide | |
|---|---|
| [Getting started](docs/en/getting-started.md) | Install, run the examples, repo tour |
| [Architecture](docs/en/architecture.md) | Layers, data flow, why no VDOM |
| [Reactivity](docs/en/reactivity.md) | The runes kernel as a user guide |
| [.svelte components](docs/en/sv-components.md) | Template syntax, props, build integration |
| [Styling](docs/en/styling.md) | The compiled CSS subset |
| [Render backends](docs/en/rendering-backends.md) | Painter trait, CPU/vello, knobs |
| [Performance](docs/en/performance.md) | virtual_list, membench, measured numbers |

Reference material (Chinese): [design & ADRs](docs/DESIGN.md) ·
[Svelte support matrix, 77 items](docs/SVELTE-SUPPORT.md) ·
[modern-CSS gap matrix, 91 items](docs/CSS-SUPPORT.md) ·
[research reports ×27](docs/README.md#research)

## Repository layout

| Path | What it is |
|---|---|
| `crates/sv-reactive` | Runes reactive kernel |
| `crates/sv-ui` | Retained scene tree + fine-grained binding primitives |
| `crates/sv-macro` | `view!` proc-macro frontend |
| `crates/sv-compiler` | `.svelte` single-file-component compiler frontend (+ `sv check`) |
| `crates/sv-shell` | winit window shell + CPU/vello renderers |
| `crates/sv-vap` · `sv-pag` · `sv-lottie` | Animation-format parsers (VAP / PAG / Lottie) |
| `crates/sv-lsp` | `.svelte` language server (LSP): live compiler diagnostics |
| `crates/sv-arco-tokens` | Arco Design tokens: palette algorithm port + `global.less` transliteration (Rust consts + `:root` CSS) |
| `crates/sv-arco` | Arco-style component library (`.svelte` components; Button landed, more en route) |
| `examples/` | showcase · counter(-sfc) · todo-sfc · settings-sfc · input-demo · overlay-demo · membench · vap-gift · arco-gallery |

## Status

M0 exploration is complete: full loop from signal to pixels (CJK text, HiDPI, hit
testing), `.svelte` compiler covering the main Svelte 5 syntax surface (43 of 77 matrix
items ✅), dual render backends, and the million-widget virtualization result.
This is a prototype — APIs churn, and several subsystems (layout, text shaping,
frame pacing) are placeholders with planned replacements. See
[docs/DESIGN.md](docs/DESIGN.md) (Chinese) for the roadmap and ADRs.

## Versioning & releases

Nothing is published to crates.io yet. Naming is settled (ADR-10 in
[docs/DESIGN.md](docs/DESIGN.md): umbrella crate `svelte-rs`, sub-crates keep
the `sv-*` prefix). Once published, the workspace ships all crates at the same
version, in dependency order.

**0.x policy: a minor bump (`0.X.0`) means breaking changes, a patch bump
(`0.0.X`) is backwards compatible.** Every breaking change documents its
migration in [CHANGELOG.md](CHANGELOG.md). All three breaking changes
scheduled before 1.0 is even discussed have landed: the two template
front-ends share a single compilation kernel (ADR-2 M1), the `on:` event
directive has been removed in favor of Svelte 5's `onclick={..}` attribute
form, and frame-pacing semantics (ADR-6) batches signal writes to the frame
boundary. What remains before 1.0 is the first crates.io release plus a
stabilization period.

MSRV is **1.88** — set by let-chains (`if let ... && ...`), not by edition 2024 — and pinned by a CI lane.

## License

Dual-licensed: MIT OR Apache-2.0.
