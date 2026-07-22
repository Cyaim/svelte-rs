[中文](../zh-CN/sv-components.md) | **English**

# Authoring UI with `.sv` Components

svelte-rs has two template front ends that compile to the same target — pinpoint update calls against the retained scene tree in `sv-ui` (no virtual DOM, no runtime diff): **`.sv` single-file components**, compiled by `sv-compiler` from your `build.rs` (this page's focus), and **the `view!` proc-macro** from `sv-macro` (last section).

This is an exploratory prototype; APIs churn. The per-feature status of every Svelte 5 construct lives in the support matrix [../SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) (Chinese) — when this page and the matrix disagree, the matrix wins.

## Anatomy of a `.sv` file

Up to three blocks: `<script>` (plain Rust plus runes, must be the first block), the markup (Svelte template syntax over a closed element set), and an optional `<style>` block (scoped classes, conventionally last). The element set is closed: `<view>`, `<text>`, `<button>`, `<checkbox>` (a leaf — must self-close). Unknown lowercase tags are compile errors; capitalized tags like `<TodoItem />` are component calls.

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

Each `.sv` file compiles to one human-readable Rust file (formatted with `prettyplease`) exporting `pub fn <fn_name>(doc, parent[, props])`; the function name is the file stem, snake_cased (`Counter.sv` → `counter`, `TodoItem.sv` → `todo_item`). Compile errors carry the 1-based line/column of the `.sv` source. Full runnable examples: `examples/counter-sfc`, `examples/todo-sfc`, `examples/showcase`.

## The runes source transform

The compiler rewrites the entire `<script>` scope so reactivity is implicit, like Svelte 5 — the one thing a proc-macro cannot do. What you write:

```rust
let count = $state(0i32);
let double = $derived(count * 2);
$effect(|| println!("count = {}", count));
let reset = || count = 0;
let bump  = || count += count;
```

What the generated code contains:

```rust
let count = ::sv_reactive::state(0i32);
let double = ::sv_reactive::derived(move || count.get() * 2);
::sv_reactive::effect(move || println!("count = {}", count.get()));
let reset = move || count.set(0);
let bump = move || {
    let __sv_rhs = count.get();              // RHS pre-evaluated: no re-entrant
    count.update(|__v| *__v += __sv_rhs);    // read inside the update closure
};
```

Rules (see `crates/sv-compiler/src/script.rs`): bare reads `count` → `count.get()`, including inside arguments of whitelisted macros (`format!`, `println!`, assertions, `vec!`, …) — a reactive variable in a *non*-whitelisted macro is a hard compile error, not a silent miss; writes `x = v` → `x.set(v)` and `x += v` → `x.update(..)` with the right-hand side pre-evaluated; closures referencing reactive variables get `move` added (handles are `Copy`, so it's free); explicit `.get()/.set()/.update()/.with()/.get_untracked()/.with_untracked()` calls pass through untouched; shadowing by closure/fn parameters, `match` arms, and `for` / `if let` patterns is respected, and string literals/comments are immune.

Known v0 limits: field/index assignment (`pos.x = 1`) is not rewritten — use `pos.update(|v| v.x = 1)`; likewise method-call writes (`items.push(v)`) — use `items.update(|v| v.push(item))`; `format!("{count}")` inline capture is not rewritten — use `"{}", count`. The core runes are covered in [./reactivity.md](./reactivity.md); the exhaustive per-variant status (`$state.raw`, `$derived.by`, `$inspect`, the `$sig(x)` escape hatch, …) lives in the support matrix [../SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) (Chinese).

## Template syntax

Everything below is verified by a compiler test in `crates/sv-compiler/src/lib.rs`.

### Interpolation, attributes, events

```svelte
<text>Count: {count} · doubled = {double}</text>    <!-- {expr}: any Rust expression -->
<text fg="#ff3e00" font-size="28">static style attributes</text>
<Card {title} />                                    <!-- shorthand ≡ title={title} -->
<button onclick={|| count += 1}>+1</button>         <!-- Svelte 5 attribute form, preferred -->
<button on:click={|| count = 0}>reset</button>      <!-- legacy alias, still accepted -->
<text onpointerenter={|| hovers += 1}>hover area</text>
```

Mixed static/interpolated text compiles to a single `bind_text` binding; fully static text gets zero bindings. Attribute values are `name="static string"` or `name={rust_expr}`. The event set today: `onclick`/`on:click`, `onpointerenter`, `onpointerleave`, plus keyboard/focus (R1): `onkeydown={|e| ...}` (automatically makes the element focusable; `e.stop_propagation()` cuts bubbling, `e.prevent_default()` cancels the Tab/Enter default layer), `onfocus`/`onblur`, and the boolean attribute `autofocus`. Buttons are Tab-focusable and Enter/Space-activatable with no annotation. Text entry uses `<input>` (a self-closing leaf): `placeholder="..."`, two-way `bind:value={x}`, `oninput={|v| ...}`/`onsubmit={|v| ...}` (signature `Fn(&str)`); caret/selection/IME preedit/Ctrl+C/X/V work out of the box, as do drag-select, double-click-to-select-word, triple-click-to-select-all, word motion (Ctrl/⌥+←/→, Ctrl+Backspace/Delete) and undo/redo (Ctrl+Z / Ctrl+Y). Caret and hit-testing geometry come from the same Parley layout that draws the text, so they stay exact under kerning and CJK/Latin font fallback. Unsupported events are a compile error, not a silent no-op. Inline `style="k:v; ..."` and shorthand style attributes (`fg=`, `font-size=`, …) are the styling mini-language — see [./styling.md](./styling.md).

### `{#if}` / `{#each}` / `{#key}`

```svelte
{#if count > 5}
  <text>over 5</text>
{:else if count < 0}
  <text>negative</text>
{:else}
  <text>early days</text>
{/if}

{#each todos as label, i}          <!-- item + optional index; pattern can destructure -->
  <text>{i}: {label}</text>
{:else}
  <text>nothing here</text>        <!-- empty-list branch -->
{/each}

{#each items as it (it.0)}         <!-- keyed: same key ⇒ row reused, row state preserved -->
  <TaskRow id={it.0} label={it.1} />
{/each}

{#key count}                       <!-- destroy & recreate whenever the expr changes -->
  <text>rebuilt on every count change</text>
{/key}
```

`{#each expr}` without `as` renders the block N times. Keyed `{#each}` cannot yet combine with an index or `{:else}` (compile error). Branch/row destruction disposes state and bindings created inside the block.

**The binding of a keyed row is reactive** (ADR-7): inside the row, `it` is that row's
`Signal<T>`, so a row whose key stayed the same but whose content changed **updates in
place** — no rebuild, no lost row state — and when the order is unchanged the tree isn't
touched at all. The cost: the binding must be a **single identifier** (destructure with
`{@const}` inside the row instead), and the item type needs `Clone + PartialEq`.
Non-keyed `{#each}` is unchanged: still a whole-block rebuild with a plain-value binding.

⚠️ **Component props are snapshots**: `<TaskRow label={it.1} />` reads the value once,
when the row is built. Markup written directly in the row (`<text>{it.1}</text>`) follows
content updates; a prop passed into a child component does not — pass a signal for that
(see `$bindable`).

### `{#await}` / `{:then}` / `{:catch}`

```svelte
{#await async move { base + 1 }}
  <text>loading…</text>
{:then v}
  <text>{v}</text>
{/await}
```

The awaited expression is a *future factory*: reactive reads inside it are rewritten, so a dependency change cancels the in-flight task and restarts it. With a `{:catch e}` branch the future must yield a `Result`. Execution runs on the background-thread async bridge in `sv_ui::tasks`.

### Snippets: `{#snippet}` / `{@render}`

```svelte
{#snippet badge(label: String, n: i32)}
  <text>{label}: {n}</text>
{/snippet}
{@render badge(String::from("count"), count)}
```

Snippets compile to local closures; parameters are typed Rust patterns. Argument reactivity works via a keyed rebuild on the argument tuple — coarser-grained than Svelte, same semantics — so parameter types need `Clone + PartialEq`.

### Tags: `{@const}`, `{@attach}`, `{@debug}`, comments

```svelte
{@const summary = format!("{} items", count)}   <!-- compiles to a block-scoped derived -->
<text>{summary}</text>
<view {@attach |d: &sv_ui::Doc, id: sv_ui::ViewId| { /* imperative escape hatch */ }}></view>
{@debug count, double}                          <!-- Debug-prints whenever a dependency changes -->
<!-- comments are stripped at compile time; <svelte:options … /> is accepted and ignored -->
```

`{@attach}` runs the closure on mount inside an effect and re-runs it when reactive dependencies change; it covers the roles of Svelte's `use:` and `bind:this` (both deliberately not implemented).

### Directives: `class:` and `style:`

```svelte
<text class="title" class:muted class:big={count > 5} style:padding={size * 2.0}>styled</text>
```

`class="…"` is a static string; the names must exist in the file's `<style>` block (unknown classes are compile errors). `class:name={cond}` toggles a class; the shorthand `class:muted` uses the same-named variable as the condition. `style:field={expr}` binds one style field reactively; accepted fields (from `codegen.rs`): `padding`, `margin`, `gap`, `font-size`/`font_size`, `radius`/`corner-radius`, `opacity`, `width`, `height`, `direction`, `bg`, `fg`/`color`. Precedence, weakest first: class < `style=""` < conditional classes < `:hover` < `style:` directives.

### Two-way binding: `bind:`

```svelte
<checkbox bind:checked={done} />     <!-- element binding: state → view and click → state -->
<Stepper bind:value={count} />       <!-- component binding: raw Signal handle, see $bindable -->
```

Element-level `bind:` supports `bind:checked` on `<checkbox>`, `bind:value` on `<input>`, and `bind:scrolly` on a scroll container. The remaining Svelte targets (`bind:this`, dimension bindings, media bindings) are a compile error pointing at the support matrix.

### Transitions

```svelte
<view transition:fade><text>fades in, 200 ms default</text></view>
<view in:fade={500}><text>fades in over 500 ms</text></view>
```

Only the enter direction exists, so `transition:fade` and `in:fade` are equivalent; the duration is milliseconds (`u32`). `out:` (needs INERT deferred destruction) and `animate:` (needs FLIP) are rejected with an explanatory error.

## Components

Component tags are PascalCase and resolve to the snake_case function of the matching `.sv` file — no import statement inside `.sv` files, because `sv_compiler::build` scans the whole source dir and registers every `$props` signature in a first pass (all generated files must then be `include!`-d into one Rust scope, see below).

### Declaring props with `$props`

```svelte
<script>
$props {
    label: String,
    index: usize,
    on_remove: std::rc::Rc<dyn Fn()>,                     // callbacks are just props
    accent: sv_ui::Color = sv_ui::Color::rgb(255, 62, 0), // default value
}
</script>
```

This generates a `pub struct TodoItemProps` and adds a `props` parameter to the component function. At the call site, missing required props and unknown props are compile errors; omitted defaulted props call a callee-side `default_<name>()` associated function. Props are passed **by value — a snapshot**: `label={name}` where `name` is `$state` compiles to `label: name.get()` and does *not* track later changes. To pass live reactivity, pass a `Signal` handle (`$sig(x)`) or use `$bindable`.

### Two-way props: `$bindable`

```svelte
<!-- Stepper.sv (callee) -->
<script>
$props { value: $bindable(i32), step: i32 = 1 }
</script>
<view style="direction:row; gap:8">
  <button onclick={|| value -= step}>-{step}</button>
  <text font-size="20">{value}</text>
  <button onclick={|| value += step}>+{step}</button>
</view>

<!-- caller:  <Stepper bind:value={count} step={2} /> -->
```

`$bindable(i32)` makes the field a `::sv_reactive::Signal<i32>`; inside the callee, `value` takes part in the runes rewrite like any `$state` variable. The caller passes the raw handle — both sides read and write the same signal, zero glue. `bind:` on a non-bindable prop is a compile error.

### `children` and snippet props

```svelte
<!-- Card.sv (callee) -->
<script>
$props { title: String, children: sv_ui::Snippet }
</script>
<view class="card">
  <text class="card-title">{title}</text>
  {@render children()}
</view>

<!-- caller: <Card title={…}><text>body {n}</text></Card> — the tag body becomes `children` -->
```

A named `{#snippet}` can also be passed explicitly as a prop (`<Card body={hello} />`); zero-arg snippets are auto-wrapped into the `Rc`-based `sv_ui::Snippet` type.

## Build integration: `build.rs` + `include!`

Exactly as in `examples/counter-sfc` — build dependency `sv-compiler`; runtime dependencies `sv-reactive`, `sv-ui`, `sv-shell`:

```rust
// build.rs
fn main() {
    // 扫描 src/ 下所有 .sv,编译成 $OUT_DIR/<组件名>.rs
    sv_compiler::build("src");
}
```

```rust
// src/main.rs  (examples/counter-sfc/src/main.rs)
include!(concat!(env!("OUT_DIR"), "/counter.rs"));

fn main() {
    sv_shell::run_app("sv 计数器(SFC)", |doc, root| counter(doc, root)).expect("运行失败");
}
```

`sv_compiler::build("src")` recursively collects `*.sv`, registers all `$props` signatures, then compiles each file to `$OUT_DIR/<fn_name>.rs`, emitting `cargo::rerun-if-changed` per file; a compile failure panics with `file:line:col: message`. Multi-component apps `include!` every generated file into one scope — e.g. `examples/todo-sfc/src/main.rs` includes both `todo.rs` and `todo_item.rs`.

```sh
cargo run -p counter-sfc                    # open a window
cargo run -p counter-sfc -- --png out.png   # render one frame offscreen, no window needed
```

## The `view!` macro route

The proc-macro front end (`crates/sv-macro`) targets the same `sv-ui` binding primitives (`bind_text` / `bind_style` / `if_block` / `each_block`), but the template is Rust-native and reactivity is **explicit** — no runes transform, since a proc-macro cannot rewrite the code around it:

```rust
use sv_macro::view;

let count = sv_reactive::state(0i32);
view! { doc, root =>
    <text style(|s| s.font_size = 20.0)>"Count: " {count.get()}</text>
    <button on_click(move || count.update(|c| *c += 1))>"+1"</button>
    if count.get() > 5 {
        <text>"Over 5!"</text>
    }
}
```

|  | `.sv` (sv-compiler) | `view!` (sv-macro) |
|---|---|---|
| Template syntax | Svelte (`{#if}`, `{#each}`, `onclick`, …) | Rust-native (`if`/`else if`/`else`, `for item, i in expr`) |
| Reactivity | implicit (runes source transform) | explicit `.get()` / `.set()` / `.update()` |
| Elements | `view`/`text`/`button`/`checkbox` + components | `view`/`text`/`button` only |
| Feature surface | everything on this page | text interpolation, `if`/`for` blocks, `style(closure)`, `on_click(closure)` |
| Build setup | `build.rs` + `include!` | none — inline macro |

Prefer `view!` when embedding a small reactive UI inline in existing Rust code with zero build-script setup, or when you want everything to stay plain Rust for the IDE. Per ADR-2 (revised), both front ends coexist; merging their compilation kernels is an M1 goal — see [../DESIGN.md](../DESIGN.md) (Chinese).

## See also

- [./reactivity.md](./reactivity.md) — signals, runes, effects, the reactive runtime.
- [./styling.md](./styling.md) — the inline `style=""` mini-language and `<style>` blocks.
- [../SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) (Chinese) — the full 77-item Svelte 5 support matrix.
- [../CSS-SUPPORT.md](../CSS-SUPPORT.md) (Chinese) — the CSS subset accepted in `<style>` blocks.
