[中文](../zh-CN/styling.md) | **English**

# Styling: real CSS, closed subset

sv styles components with a `<style>` block containing **real CSS syntax** — but a deliberately closed subset of it, fully compiled at build time ([ADR-8 in ../DESIGN.md](../DESIGN.md), Chinese). The reasoning in three sentences: the industry's mental-migration pain line runs at "real CSS syntax + state pseudo-classes + variables", not at completeness — React Native's property-name objects and Flutter's zero-CSS both cost dearly there. Svelte's scoped-by-default model compresses what developers actually write down to a small surface: flat class rules, state pseudo-classes, box model, variables, inheritance. So sv parses genuine CSS and folds every rule into `Style` field assignments at compile time — there is **no runtime selector engine, no specificity computation, no style recalculation**; a class is just an index into a compile-time stylesheet.

This is an exploratory prototype. The subset grows in planned batches (C1 landed 2026-07-18; C2 is next) and APIs churn. Everything below is implemented and tested today (`css_c1_box_model_vars_nesting` and friends in the sv-compiler test suite); everything else is tracked in the [gap matrix](#what-does-not-exist-yet).

## The `<style>` block

Two selector forms exist: `.class` rules and element-type rules (`view` / `text` / `button` / `checkbox`). Anything else — descendant combinators, `#id`, `[attr]` — is a compile error today.

```svelte
<view class="card">
  <text class="card-title">{title}</text>
</view>

<style>
.card { padding: 16px; gap: 8px; border-radius: 10px; background-color: #f0f0f6; }
.card-title { font-size: 20px; color: #223344; }
text { color: #223344; }   /* element rule: baseline for every <text> in this component */
</style>
```

Rules are **scoped per component**, like Svelte: `.btn` in `Stepper.sv` and `.btn` in `Showcase.sv` are different stylesheets and never collide. Element rules apply below class rules (the familiar "element < class" intuition, achieved by application order, not specificity — see [Cascade](#cascade-declaration-order-not-specificity)). Defining the same rule twice in one component is a compile error.

## Applying styles in markup

| Form | Example | Notes |
|---|---|---|
| `class` attribute | `<view class="card row">` | Static string only; multiple classes merge in listed order. An unknown class is a compile error. |
| `class:` directive | `<text class:muted={done}>` | Conditional class; shorthand `class:muted` reads the same-named variable. |
| Inline `style` | `<view style="direction:row; gap:8">` | Same declaration parser as the `<style>` block; static string only. |
| Bare style attributes | `<text font-size="20" fg="#ff3e00">` | Any non-event attribute is parsed as a single style declaration. |
| `style:` directive | `<text style:fg={expr}>` | Dynamic per-field escape hatch with a Rust expression; see [components guide](./sv-components.md). |

A dynamic `class={expr}` or `style={expr}` is a compile error — the error message points you to `class:name={cond}` and `style:field={expr}` respectively.

## Supported properties

The authoritative list is the `match` in `crates/sv-compiler/src/style.rs`. Anything not listed is a compile error naming the supported set.

| Property | Aliases | Values |
|---|---|---|
| `background-color` | `background`, `bg` | any [color form](#colors) |
| `color` | `fg` | color, `currentColor`, `inherit` (the latter two mean "inherit") |
| `font-size` | `font_size` | length |
| `padding`, `margin` | — | 1–4 lengths, CSS shorthand expansion (top/right/bottom/left) |
| `padding-top` … `margin-left` | — | single length, all 8 longhands |
| `border` | — | `none`, or `[solid] <width> [solid] [<color>]` — solid only, color defaults to black; `dashed`/`dotted`/`double` are a compile error (P2) |
| `border-radius` | `corner-radius`, `radius` | single length (independent corners: P2) |
| `gap` | `row-gap`, `column-gap` (all set the same gap) | length |
| `flex-direction` | `direction` | `row` \| `column` |
| `width`, `height` | — | length |
| `opacity` | — | number 0.0–1.0 |
| `cursor` | — | `pointer` / `default` / `text` / `grab` / `not-allowed` |

## Units

| Unit | Status |
|---|---|
| `px`, bare number | Logical pixels; HiDPI scaling is automatic |
| `rem` | Folded to px at compile time (×16) |
| `em`, `%`, `vw`, `vh`, `vmin`, `vmax`, `pt`, `ch` | **Compile error by design**, with a message explaining why: `em` needs the dynamic font-size base (P2), `%`/`vw`/`vh` need the taffy layout system (C2) |

```
error: 单位 `%` 暂不支持——需要布局系统(taffy,C2);请用 px/rem/裸数(`width: 8px`)
```

## Colors

All color forms are folded to RGBA at compile time.

| Form | Examples |
|---|---|
| Hex, 3/4/6/8 digits | `#f06`, `#ff3e00`, `#ff3e0080` |
| `rgb()` / `rgba()` | comma syntax `rgb(255, 62, 0)` and modern space-slash syntax `rgb(255 62 0 / .5)` |
| `hsl()` / `hsla()` | `hsl(20 100% 50%)`, optional `deg` suffix on hue |
| `hwb()` | `hwb(20 10% 5%)` |
| Named | ~60 names: the CSS basic 16 plus common extended ones (`rebeccapurple`, `hotpink`, `steelblue`, …), `transparent` |
| `currentColor` | On `color`: resolves via [inheritance](#inheritance) |

Alpha accepts a number (`0.5`) or a percentage (`50%`).

## `:root` variables and `var()`

```svelte
<style>
:root { --accent: rgb(255, 62, 0); --btn-pad: 8px 14px; }

.btn {
  padding: var(--btn-pad);
  background-color: var(--accent);
  color: var(--missing, white);   /* fallback form */
}
</style>
```

`var(--x)` is **compile-time textual substitution** — no runtime custom-property chain exists. A `:root` block may appear anywhere in the style block; using an undefined variable without a fallback is a compile error. Runtime theming via variables is a C2 item.

## Nesting and state pseudo-classes

CSS nesting is supported in exactly one form: `&:hover` / `&:active` inside a rule. The standalone forms `.btn:hover { }` / `.btn:active { }` work too. Other pseudo-classes (`:focus`, `:disabled`) are a compile error, scheduled for C2 with the keyboard focus chain.

```svelte
<style>
.btn {
  background-color: var(--accent);
  cursor: pointer;
  &:hover  { background-color: orange; }
  &:active { opacity: 0.7; }
}
</style>
```

There is no selector matching at runtime. For each element that has a `:hover` rule, the compiler generates a private boolean signal and wires it to the element's pointer-enter/leave callbacks; `:active` gets a second bit wired to pointer-down/up. The element's whole style becomes one reactive closure (`bind_style`) that reapplies base declarations, then hover, then active — so the pressed state wins, matching CSS LVHA ordering. Your own `onpointerenter`/`onpointerleave` handlers are merged with the internal wiring, not overwritten.

## Inheritance

`color` and `font-size` inherit down the tree, like the web. Implementation: `Style { fg: None, font_size: f32::NAN }` are the "inherit" sentinels (the defaults); the renderer resolves them by walking the parent chain, with root fallbacks of black / 16.0. Writing `color: inherit` or `color: currentColor` clears the element's own value back to the sentinel. Other properties do not inherit; the inheritance whitelist grows with the text stack (e.g. `line-height`, C1/C2).

```svelte
<style>
view { color: #223344; }  /* every text below inherits this unless overridden */
.muted { color: #667; }
</style>
```

## Cascade: declaration order, NOT specificity

**This is a deliberate difference from the web.** There is no specificity counting and no `!important` — within a component, what wins is declaration order plus a fixed channel priority:

```
element rule < class (in class="a b" order) < inline style="" < class: conditional < :hover < :active < style: directive
```

This matches CSS's own tiebreak *within* equal specificity, and Svelte itself flattens specificity with `:where()` — the mental model transfers. The ultimate override is the `style:` directive. Rationale in ADR-8, [../DESIGN.md](../DESIGN.md) (Chinese).

## What does not exist yet

The full accounting is the 91-item gap matrix in [../CSS-SUPPORT.md](../CSS-SUPPORT.md) (Chinese), one verdict per feature against a 2026 Baseline definition of "modern CSS". Headline numbers after the C1 batch:

| Status | Count | Meaning |
|---|---|---|
| ✅ implemented | 24 | tested, used by `examples/showcase` |
| 📅 C2 scheduled | 13 | grid, `@media`, transitions, `:focus`, `%` units, margin `auto`… (flex batch 1 + `white-space` + `text-align` landed in R2 via taffy 0.12) |
| ✏️ P2 degraded/compile-time form | 14 | `calc()` constant folding, descendant combinators, `oklch()`… |
| ⏳ awaiting infrastructure | 17 | gradients/shadows/filters (vello backend), fonts (parley)… (scrolling landed in R2: `overflow: scroll` + wheel/scrollbar/clipping) |
| ❌ never, with documented alternative | 16 | specificity, `!important`, `@layer`, pseudo-elements, runtime selectors, `:has()`… |

C2 completion is defined as the "seamless migration line" for Svelte developers (44/91 implemented, covering the high-frequency surface).

To see all of the above running: `cargo run -p showcase` (or `-- --png out.png` for an offscreen frame). Style sources: `examples/showcase/src/Showcase.sv`, `Card.sv`, `Stepper.sv`, `TaskRow.sv`.

## See also

- [Components and templates](./sv-components.md) — `class:`/`style:` directives in context
- [../CSS-SUPPORT.md](../CSS-SUPPORT.md) (Chinese) — the full 91-item gap matrix
- [../DESIGN.md](../DESIGN.md) (Chinese) — ADR-8, the decision record behind all of this
