[中文](../zh-CN/architecture.md) | **English**

# Architecture

> Status: exploratory prototype. APIs churn, crate names (`sv-` prefix) are working
> codenames, and several layers are explicitly temporary. Decision records and the
> roadmap live in [DESIGN.md](../DESIGN.md) (Chinese).

The one-line idea: **take Svelte 5's compilation philosophy to native desktop — templates
compile into pinpoint updates of a retained scene tree. At runtime there is no virtual
DOM, no diff, no rebuild.** Target platforms: Windows / Linux / macOS / HarmonyOS.

## Layers

```
┌──────────────────────────────────────────────────────────────┐
│ User components: view! templates + $state/$derived/$effect   │
├──────────────────────────────────────────────────────────────┤
│ sv-macro     view! proc-macro frontend (parse → IR → codegen)│
│ sv-compiler  .sv single-file-component frontend              │
│              → both emit calls into sv-ui binding primitives │
├──────────────────────────────────────────────────────────────┤
│ sv-reactive  runes kernel: Signal / Derived / Effect         │
│              thread-local arena + Copy handles, push-pull    │
│              three-state dirty marking, effect ownership tree│
├──────────────────────────────────────────────────────────────┤
│ sv-ui        retained scene tree ("desktop DOM") + binding   │
│              primitives: bind_text / bind_style / if_block / │
│              each_block / virtual_list / …                   │
├──────────────────────────────────────────────────────────────┤
│ sv-shell     window + renderer                               │
│              window: winit (desktop); HarmonyOS via a narrow │
│              window trait (planned, ADR-4)                   │
│              render: CPU (softbuffer + tiny-skia + swash) by │
│              default; vello/wgpu behind `backend-vello`      │
└──────────────────────────────────────────────────────────────┘
```

Try it:

```sh
cargo test                              # all crates
cargo run -p counter                    # windowed counter (view! route)
cargo run -p counter -- --png out.png   # render one frame offscreen, no window
```

## Data flow: no VDOM, no diff

A component function runs **once**, building scene-tree nodes and registering
bindings. This is real code from the `sv-ui` test suite — exactly the shape the
compilers emit:

```rust
use sv_reactive::state;
use sv_ui::{Doc, bind_text};

let doc = Doc::new();
let count = state(0);

let label = doc.create_text("");
doc.append(doc.root(), label);
bind_text(&doc, label, move || format!("Count: {}", count.get()));

let btn = doc.create_button("+1");
doc.append(doc.root(), btn);
doc.set_on_click(btn, move || count.update(|c| *c += 1));
```

What happens on a click:

1. `count.update(..)` writes the signal. The push phase marks subscribers dirty
   (`Clean`/`Check`/`Dirty` three-state marking, same scheme as Svelte 5).
2. The flush runs pending effects — here, the one effect `bind_text` created.
3. That effect calls `doc.set_text(label, ..)`: a pinpoint mutation of exactly one
   node. If the new text equals the old, it returns early and nothing else happens.
4. A real change bumps the `Doc` version counter and fires the `on_mutate` callback.
5. sv-shell wired `on_mutate` to the window's redraw request; the next frame repaints
   from the retained tree.

There is nothing to diff: which value changed determines which node gets touched,
and that wiring was decided at compile time. Equality pruning exists at two levels —
`derived` skips downstream work when the recomputed value is `==` the old one, and
`Doc` setters (`set_text` / `set_style` / `set_checked`) skip the version bump when
the value is unchanged, so the renderer never repaints for a no-op write.

One current limitation: writes flush synchronously (correct, but not frame-aligned).
Frame scheduling — batching writes into a pre → render → layout → paint pipeline —
is ADR-6 and **not implemented yet**.

## Two compiler frontends, one compile target (ADR-2, revised)

Both frontends compile to the same thing: imperative `Doc` tree building plus calls
into sv-ui binding primitives. Generated code from the two routes is nearly identical,
which is why a shared core is planned.

The `view!` proc-macro route (`examples/counter`):

```rust
view! { doc, root =>
    <text style(|s| s.font_size = 20.0)>"Count: " {count.get()}</text>
    <button style(btn) on_click(move || count.update(|c| *c += 1))>"+1"</button>
    if count.get() > 5 {
        <text style(|s| s.fg = Some(Color::rgb(255, 62, 0)))>"超过 5 了!"</text>
    }
}
```

The `.sv` single-file-component route (`examples/counter-sfc/src/Counter.sv`,
compiled by `build.rs` calling `sv_compiler::build("src")` into `OUT_DIR`):

```svelte
<script>
let count = $state(0i32);
let double = $derived(count * 2);
</script>

<view style="padding:24; gap:12">
  <text font-size="20">Count: {count} · 双倍 = {double}</text>
  <button on:click={|| count += 1}>+1</button>
  {#if count > 5}
    <text fg="#ff3e00">超过 5 了!</text>
  {/if}
</view>
```

| | `view!` macro (sv-macro) | `.sv` SFC (sv-compiler) |
|---|---|---|
| Runes | explicit: `count.get()`, `count.update(..)` | implicit: bare `count` reads, `count += 1` writes — a whole-script source transform |
| Template syntax | constrained by the Rust tokenizer (quoted text, Rust `if`/`for`) | genuine Svelte: unquoted text, `{#if}{:else}`, `on:click`, `bind:` |
| Diagnostics | spans point into your source | template errors carry `.sv` line/col; rustc type errors land in (readable, prettyplease-formatted) generated code |
| Build | expands in place | `build.rs` + `OUT_DIR` + `include!` |
| Compile target | sv-ui binding primitives | sv-ui binding primitives (same) |

The original ADR-2 said "start with proc-macro only". After building runnable
prototypes of both routes side by side, it was revised: **both frontends coexist over
one compiler core**. The M1 plan is three no-regret steps: merge sv-macro and
sv-compiler into a single core (shared IR/codegen, both frontends become thin shells),
make templates data rather than generated types, and split codegen into setup/render —
the latter two also serve hot reload. The biggest open risk of the `.sv` route is IDE
support (no rust-analyzer inside `.sv`; a Volar-style forwarding LSP is unbuilt).
See [sv-components](./sv-components.md).

## Single-threaded reactive model (ADR-1)

```rust
use sv_reactive::{state, derived, effect};

let count = state(0);                            // Signal<i32>: Copy + !Send
let double = derived(move || count.get() * 2);   // lazy, == pruning
effect(move || println!("{}", double.get()));    // first run is synchronous
```

- All reactive nodes live in a **thread-local** arena (slotmap). `Signal<T>` /
  `Derived<T>` are `Copy + !Send` generational handles — move them into as many
  closures as you like, no lifetimes, no `Rc` juggling. This is the standard solution
  to reactivity under the borrow checker (Leptos / Sycamore use the same shape).
- **No Send/Sync**, on purpose: a UI runtime is single-threaded, and paying for
  `Arc`/atomics on every read would be pure waste. Background threads send messages
  back to the UI thread, which writes the signal.
- Scheduling is push-pull: writes only mark (push); effects run in a flush; derived
  values recompute only when read (pull). Diamond dependencies are glitch-free and
  run each effect once.
- Effects form an **ownership tree**: a rerun first destroys child scopes and runs
  `on_cleanup` callbacks — which is why `{#if}` branch teardown is free.
- Writing state inside a derived computation **panics** (the equivalent of Svelte's
  `state_unsafe_mutation` error).
- Deliberate divergence from Svelte: effects run synchronously at creation instead of
  being deferred to a microtask. Frame-aligned scheduling comes later (ADR-6).

Details and the full API surface: [reactivity](./reactivity.md).

## What each crate owns

| Crate | Owns |
|---|---|
| `sv-reactive` | Runes kernel: `state` / `derived` / `effect` / `effect_pre` / `batch` / `untrack` / `on_cleanup` / `create_root` / `provide_context` / `use_context`; thread-local runtime and scheduling |
| `sv-ui` | Retained scene tree (`Doc`, `ViewNode`, `Style`) and the binding primitives both compilers target: `bind_text`, `bind_style`, `bind_style_patch`, `if_block`, `each_block`, `each_block_else`, `each_block_keyed`, `key_block`, `virtual_list`, `mount`; version counter + `on_mutate` |
| `sv-macro` | `view!` proc-macro frontend: parse → IR → codegen |
| `sv-compiler` | `.sv` SFC frontend: runes source transform, Svelte template syntax, style parsing, `build.rs`/`OUT_DIR` integration (`sv_compiler::build`), errors with `.sv` line/col |
| `sv-shell` | winit window + renderers: CPU stack (softbuffer + tiny-skia + swash) by default, vello/wgpu behind the `backend-vello` feature with `SV_RENDERER=cpu\|vello` override; `Painter` trait, layout, hit testing, `run_app` / `render_to_png` |
| `examples/counter` | Counter on the `view!` route (windowed + `--png` offscreen) |
| `examples/counter-sfc` | Counter on the `.sv` route (build.rs integration + end-to-end behavior test) |

The rendering layer is explicitly a placeholder: the current CPU stack gets replaced
by the vello family, Parley text, and taffy layout per the roadmap — the `Painter`
abstraction exists so backends can be swapped. See
[rendering-backends](./rendering-backends.md).

## ADR index

One line each; full records (Chinese) in [DESIGN.md](../DESIGN.md).

| ADR | Decision | Status |
|---|---|---|
| ADR-1 | Reactive graph: thread-local arena + `Copy` handles; push-pull three-state dirty marking; no Send/Sync | Implemented |
| ADR-2 (rev.) | Compile strategy: dual frontends (`view!` + `.sv`) sharing one compile target; merge into a single compiler core in M1 | Both frontends running; merge planned |
| ADR-3 | Rendering: start on a CPU stack, converge on the vello family (Parley text, taffy layout) | CPU stack running |
| ADR-3b | Backend verdict + switchable `Painter` abstraction; vello as second real backend; text stack moved to swash | Landed |
| ADR-4 | Window layer: narrow trait, winit is not an architectural premise (no winit HarmonyOS backend) | Planned |
| ADR-5 | HarmonyOS: technically feasible (Tier-2 targets, XComponent + GLES path proven by Flutter/Servo ports), second-tier priority | Planned (M3 spike) |
| ADR-6 | Frame scheduling: batch writes into a frame pipeline with `flush_sync` escape hatch | **Not implemented** — biggest open design point |
| ADR-7 | `each` blocks: keyed reconcile. `each_block_keyed` (key-based row reuse, state-preserving reorder) exists; per-item-signal reconcile is the target shape | Partially implemented |
| ADR-8 | CSS: real syntax, closed subset, compile-time stylesheets, never a runtime selector engine | C1 landed; C2 planned |
| ADR-9 | Scale: viewport virtualization (`virtual_list`) decouples frame cost from logical widget count — measured 1M controls, CPU backend, p99 5.28 ms / 1% low 174 fps | Landed |

## Related pages

- [reactivity](./reactivity.md) — the runes kernel in depth
- [sv-components](./sv-components.md) — the `.sv` component format and build integration
- [rendering-backends](./rendering-backends.md) — `Painter`, CPU vs vello, `SV_RENDERER`
- [performance](./performance.md) — measured numbers and how they were taken
- [DESIGN.md](../DESIGN.md) (Chinese) — full ADRs, roadmap, risk register
- [SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) (Chinese) — Svelte feature support matrix
- [CSS-SUPPORT.md](../CSS-SUPPORT.md) (Chinese) — CSS support, item by item
