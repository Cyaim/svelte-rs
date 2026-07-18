[中文](../zh-CN/reactivity.md) | **English**

# Reactivity: the runes kernel (sv-reactive)

`sv-reactive` is a fine-grained reactive kernel modeled on Svelte 5 runes, sized for a
single-threaded desktop UI. It is the bottom layer of the stack: templates (whether the
`view!` macro or `.sv` files, see [sv-components](./sv-components.md)) compile down to
`state`/`derived`/`effect` calls that mutate a retained scene tree — no virtual DOM, no
runtime diff (see [architecture](./architecture.md)). This is an exploratory prototype;
APIs churn.

## Svelte runes ↔ Rust API

| Svelte | sv-reactive | Notes |
|---|---|---|
| `$state` | `state` / `Signal<T>` | explicit `get`/`set`, no proxy magic |
| `$derived` | `derived` / `Derived<T>` | lazy + `PartialEq` pruning; writable override |
| `$effect` | `effect` | runs synchronously on creation (deliberate difference, see below) |
| `$effect.pre` | `effect_pre` | first phase of the two-phase flush |
| `$effect.tracking()` | `is_tracking` | |
| `$effect.root` | `create_root` (closest equivalent) | ownership scope with manual `dispose` |
| `$effect.pending()` | `sv_ui::tasks::pending_count` | reactive in-flight task count |
| `$props.id()` | `unique_id` | `"sv-1"`, `"sv-2"`, … per thread |
| `tick` | `tick` | kept for API parity; writes already flush synchronously |
| `setContext` / `getContext` | `provide_context` / `use_context` | keyed by Rust type instead of string |
| `untrack` | `untrack` | |

Deliberately dropped (per ADR-1 in [DESIGN.md](../DESIGN.md) (Chinese)): implicit
assignment reactivity (`count += 1` triggering updates) and deep proxy reactivity. In
plain Rust you call `set`/`update` explicitly; the `.sv` compiler front-end restores the
implicit syntax via source transform.

## state

```rust
use sv_reactive::state;

let count = state(0);           // Signal<i32>, Copy handle
count.get();                    // read + track (needs T: Clone)
count.with(|v| v.to_string());  // borrow-read + track, no clone
count.get_untracked();          // read without subscribing (also: with_untracked)
count.set(1);                   // write + notify — no equality check, same value still fires
count.update(|v| *v += 10);     // in-place mutation + notify
```

`Signal<T>` is a `Copy + !Send` generational handle into a thread-local arena — move it
into as many closures as you like. `PartialEq`/`Hash` on the handle compare *identity*
(same node), not the value.

## derived

```rust
use sv_reactive::{state, derived};

let a = state(1);
let doubled = derived(move || a.get() * 2);  // Derived<i32>, requires T: PartialEq
assert_eq!(doubled.get(), 2);
```

`derived` is lazy: the closure does not run until someone reads the value, and a dirty
derived that nobody reads is never recomputed. After a recompute, if the new value is
`==` the old one, downstream subscribers are not woken (equality pruning).

### Writable derived (optimistic UI)

Like Svelte 5.25 writable `$derived`, you can temporarily override a derived from the
*outside*:

```rust
let a = state(1);
let d = derived(move || a.get() * 2);
d.set(100);                    // optimistic override; downstream sees 100 immediately
d.update(|v| *v += 1);         // in-place override on top of the freshest derived value
a.set(5);                      // any dependency change → recompute wins, override reverts
assert_eq!(d.get(), 10);
```

Semantics: `set`/`update` first pull the derived up to date (this also establishes the
dependency edges, so the override can revert even if the derived was never read before).
`set` prunes when the override equals the current derived value; `update` cannot prune
(no old copy to compare against). Writing a derived *inside* a derived computation still
panics — only external writes are allowed.

## effect and effect_pre

```rust
use sv_reactive::{state, effect, effect_pre, on_cleanup};

let count = state(0);
let handle = effect(move || {
    println!("count = {}", count.get());   // dependencies tracked automatically
    on_cleanup(|| println!("before rerun / on destroy"));
});
count.set(1);       // effect reruns synchronously (cleanup first)
handle.dispose();   // optional early teardown; dropping the handle does nothing
```

**Deliberate difference from Svelte:** Svelte defers effects to a microtask; here an
effect **runs synchronously on creation** — more direct for a desktop event loop. The
first run counts as one atomic flush: state written during it is flushed once at the end.

Dependencies are re-collected on every run, so branches unsubscribe naturally. Rerunning
first destroys any child nodes created by the previous run (nested effects, temporary
signals) and executes `on_cleanup` callbacks — `{#if}` branch teardown falls out of this
for free.

`effect_pre` is `$effect.pre`: identical in every way except scheduling — within each
flush pass, all pre effects run before normal effects. In this model normal effects do
the "render" writes to the scene tree, so pre is the place to read old state before that.

## batch, tick

```rust
use sv_reactive::{state, batch, tick};

let a = state(1);
let b = state(2);
batch(|| {
    a.set(10);
    b.set(20);   // one effect flush for both writes, after the closure returns
});
tick();          // flush pending effects now; inside batch it is a no-op
```

`tick` exists mostly for API parity: writes outside a batch already flush synchronously.

## untrack and is_tracking

```rust
use sv_reactive::{state, effect, untrack, is_tracking};

let a = state(1);
let b = state(2);
effect(move || {
    let _ = a.get() + untrack(|| b.get());  // b is read but not subscribed
    assert!(is_tracking());                 // true inside effect/derived
    assert!(!untrack(is_tracking));         // false under untrack (and at top level)
});
```

## Ownership scopes: create_root, on_cleanup, detached

```rust
use sv_reactive::{create_root, state, effect, detached};

let (value, root) = create_root(|| {
    // every node created in here belongs to this root
    let s = state(0);
    effect(move || { s.get(); });
    42
});
root.dispose();   // cascades: effects stop, signals die, cleanups run

// Escape hatch: nodes that must never be scope-owned (thread-level singletons)
let global = detached(|| state(0usize));
```

`create_root` is the unmount primitive: components and keyed-`{#each}` rows live in
roots so teardown is one call. `on_cleanup` registers on the *current* scope (effect or
root) and runs before each rerun and on destroy. `detached` runs a closure with no owner
and no tracking — used e.g. by the async bridge for its process-lifetime pending counter,
which must not die with whatever effect happened to initialize it lazily.

## Context: provide_context / use_context

```rust
use std::rc::Rc;
use sv_reactive::{create_root, provide_context, use_context};

struct Theme(&'static str);

let (_, root) = create_root(|| {
    provide_context(Theme("dark"));                    // keyed by TypeId, on current scope
    let theme: Option<Rc<Theme>> = use_context::<Theme>();
    assert_eq!(theme.unwrap().0, "dark");
});
```

Lookup walks the *owner* chain upward and returns the nearest provider; inner scopes
shadow outer ones. The owner edge is recorded at node **creation** time, so lookup
crosses `create_root` boundaries — a keyed-each row scope still sees component-level
context. Reruns clear the scope's contexts (they are re-provided by the rerun).

## Guardrails

| You do | Result |
|---|---|
| write `state` (or a derived) inside a `derived` computation | panic — mirrors Svelte's `state_unsafe_mutation` |
| effect writes its own dependency in a loop | panic after `MAX_FLUSH_PASSES` (1000) non-converging passes |
| effect synchronously triggers its own rerun | panic (reentrant effect execution) |
| re-enter `with` on the *same* node inside its own callback | panic (reading other nodes is fine) |
| use a handle after its scope was disposed | panic with a "destroyed with its scope" message |

## Scheduling: push-pull, glitch-free

Scheduling uses the same push-pull three-state dirty marking as Svelte 5 / reactively:
`Clean` / `Check` (an upstream derived *might* have changed) / `Dirty` (definitely
stale). A signal write only pushes marks downstream and queues effects; deriveds recompute
lazily when pulled, and a `Check` node first asks its sources before rerunning. The
result: in a diamond (`a → b, a → c, effect(b, c)`) the effect runs exactly once per
change, never observing a half-updated ("glitched") state, and equality pruning stops
propagation early.

## Threading model

The whole graph lives in a **thread-local** arena (slotmap); `Signal`/`Derived` handles
are `Copy + !Send`. There are no locks and no `Arc` overhead — and no cross-thread
access, by construction. Background work sends results back to the UI thread as messages;
that bridge is `sv_ui::tasks`:

```rust
use std::time::Duration;
use sv_reactive::{create_root, state};
use sv_ui::tasks;

let (_, _root) = create_root(|| {
    let got = state(0i32);
    tasks::spawn(async { 41 + 1 }, move |v| got.set(v)); // Future runs on a worker thread
    assert_eq!(tasks::pending_count(), 1);               // reactive: $effect.pending()
    assert!(tasks::pump_until_idle(Duration::from_secs(5)));
    assert_eq!(got.get_untracked(), 42);                 // callback ran on the UI thread
});
```

`spawn(fut, on_done)` runs the `Send` future on its own background thread (a minimal
park/unpark `block_on`); the completion value crosses a channel and `on_done` runs on the
UI thread during `pump()` — which the window shell calls every frame/idle, waking the
event loop via `set_waker`. `cancel(id)` drops the callback (the worker finishes, the
value is discarded). `pump_until_idle(timeout)` is the blocking variant for headless
tests.

On top of this sit `tasks::await_block` / `await_block_result` (the `{#await}` runtime):
the future *factory* is evaluated inside an effect, so when a tracked dependency changes
the old task is cancelled, the view returns to the pending branch, and a new task is
spawned — dependency-restart semantics, matching Svelte's `{#await}` on a new promise.

## Animation, in brief

`sv_ui::anim` is a minimal animation driver: `transition_in_fade(&doc, node, dur)`
implements the v0 fade-in (`transition:fade` / `in:fade`, opacity channel only); the
shell calls `anim::pump(now_ms)` each frame and keeps scheduling frames while
`anim::active()`. Outro transitions (`out:`) need delayed-destroy (INERT) machinery and
are deferred — see [SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) (Chinese).

## See also

- [sv-components](./sv-components.md) — how the two template front-ends compile to these primitives
- [architecture](./architecture.md) — where the kernel sits in the crate stack
- [DESIGN.md](../DESIGN.md) (Chinese) — ADR-1: thread-local arena + Copy handles, and the roadmap
