[中文](./README.zh-CN.md) | **English**

# svelte-rs Documentation Center

Guides are bilingual (English / 中文). Reference material — the design record and
research reports — is currently Chinese-only and linked as-is.

## Guides

| English | 中文 | Covers |
|---|---|---|
| [Getting started](en/getting-started.md) | [快速上手](zh-CN/getting-started.md) | Install, run the examples, repo tour |
| [Architecture](en/architecture.md) | [架构](zh-CN/architecture.md) | Layers, data flow, no-VDOM design |
| [Reactivity](en/reactivity.md) | [响应式](zh-CN/reactivity.md) | `state`/`derived`/`effect`, context, async bridge |
| [.sv components](en/sv-components.md) | [.sv 组件](zh-CN/sv-components.md) | Template syntax, props, build.rs integration |
| [Styling](en/styling.md) | [样式](zh-CN/styling.md) | The compiled CSS subset, `:hover`, variables |
| [Render backends](en/rendering-backends.md) | [渲染后端](zh-CN/rendering-backends.md) | Painter trait, CPU/vello, env knobs |
| [Performance](en/performance.md) | [性能](zh-CN/performance.md) | `virtual_list`, membench, measured numbers |

## Reference (Chinese)

- [DESIGN.md](DESIGN.md) — architecture design and the ADR record (ADR-1..9). Read this before changing architecture.
- [SVELTE-SUPPORT.md](SVELTE-SUPPORT.md) — Svelte 5 syntax/feature support matrix, 77 items.
- [CSS-SUPPORT.md](CSS-SUPPORT.md) — modern-CSS gap matrix, 91 items.

## Research

Twenty-five internet-verified research reports (Chinese) that back the ADRs, 2026-07:

| # | Report |
|---|---|
| 01 | [Svelte 5 compilation model → Rust mapping](research/01-svelte-model.md) |
| 02 | [Rust GUI landscape & differentiation](research/02-rust-gui-landscape.md) |
| 03 | [HarmonyOS self-rendering feasibility](research/03-harmonyos.md) |
| 04 | [Compiler strategy (proc-macro vs external file)](research/04-compiler-strategy.md) |
| 05 | [Rendering/text/layout/a11y stack selection](research/05-rendering-stack.md) |
| 06 | [.sv build integration (build.rs/OUT_DIR)](research/06-sv-build-integration.md) |
| 07 | [.sv IDE/LSP strategy (Volar-style forwarding)](research/07-sv-ide-lsp.md) |
| 08 | [Runes source-transform semantics & soundness](research/08-sv-runes-transform.md) |
| 09 | [.sv format design + hot-reload architecture](research/09-sv-sfc-format-hotreload.md) |
| 10 | [Hands-on dual-route comparison](research/10-route-comparison-hands-on.md) |
| 11 | [Industry CSS strategies + Rust infrastructure](research/11-css-industry-strategies.md) |
| 12 | [CSS semantics item-by-item mapping](research/12-css-semantics-mapping.md) |
| 13 | [Seven render-backend classes compared](research/13-render-backends.md) |
| 14 | [Switchable Painter abstraction](research/14-switchable-painter.md) |
| 15 | [Three-scenario status analysis](research/15-scenario-analysis.md) |
| 16 | [Per-scenario memory benchmarks](research/16-memory-benchmarks.md) |
| 17 | [Per-backend × per-scenario memory & fps](research/17-backend-memory-fps.md) |
| 18 | [Million controls @ 144fps: swash + virtualization](research/18-million-controls-144fps.md) |
| 19 | [Commercialization gap: four-way audit & staged verdict](research/19-commercialization-gap.md) |
| 20 | [Keyboard events + focus chain + shortcuts](research/20-keyboard-focus.md) |
| 21 | [Text input + IME + clipboard](research/21-text-input-ime-clipboard.md) |
| 22 | [Scroll system (clip, wheel, scrollbar, virtual_list bridge)](research/22-scroll-system.md) |
| 23 | [taffy layout + text wrapping](research/23-taffy-text-wrap.md) |
| 24 | [Parley migration + AccessKit](research/24-parley-accesskit.md) |
| 25 | [Overlay system + release engineering](research/25-overlay-release-engineering.md) |

## Conventions

- Guides mirror each other: `en/<page>.md` ↔ `zh-CN/<page>.md`, same sections, same facts.
- Every guide starts with a language-switcher line.
- Contributions should update **both** language versions of a guide in the same change.
