# sv-compiler

> .svelte 单文件组件编译器 — [svelte-rs](https://github.com/Cyaim/svelte-rs) 的一个 crate。

script 块 runes 源变换(裸 `count` → `.get()`、`count += 1` → `.update`)+ 100% Svelte 模板语法 → Rust 代码生成,经 build.rs / OUT_DIR 集成。

单独使用意义不大:整套栈的入口、示例与中英双语指南都在
[仓库根目录](https://github.com/Cyaim/svelte-rs)(`docs/README.md` 是导航)。
架构分层与 ADR 决策记录见 `docs/DESIGN.md`。

## `sv check`:把 rustc 的报错指回 `.svelte`

生成代码在 `$OUT_DIR/<组件>.rs`,rustc 的类型错误默认报在那里,坐标形如
`target/debug/build/<pkg>-<hash>/out/counter.rs:44:44` —— 对写 `.svelte` 的人零价值。
`sv check` 跑一遍 `cargo check --message-format=json`,按 `.svmap`
(build.rs 与 `.rs` 同一次写盘)把诊断搬回 `.svelte`:

```sh
cargo run -q -p sv-compiler --bin cargo-sv -- check              # 默认 --workspace
cargo run -q -p sv-compiler --bin cargo-sv -- check -p counter-sfc
```

把 `Counter.svelte` 的 `{count}` 改成 `{count + "x"}`,实测输出(2026-07-22):

```text
...\src\Counter.svelte:12:38: error[E0277]: cannot add `&str` to `i32`  (位置由相邻锚点插值得出:…)
     |
  12 |   <text font-size="20">Count: {count + "x"} · 双倍 = {double}</text>
     |                                      ^
   = 生成文件对应位置: ...\out/counter.rs:44:44
```

主 span 是那个宽度 1 的 `+` —— 标点不在锚点表里,靠"同区相邻锚点之间"这一档
插值回去(不做按字节距离平移:生成侧 `count.get() + "x"` 比 `.svelte` 侧长,平移会
越过右锚点)。

`.vscode/tasks.json` 里配好了 problemMatcher(Ctrl+Shift+B),这一行会直接变成
Problems 面板条目 + 编辑器波浪线,**不需要任何 VS Code 扩展**。

**映射不到时绝不吞诊断**:位置退回生成文件并附一句 `[sv check: …]` 说明,
而且**说明必须是真理由**——降级成因有五种(落在胶水上 / `.svmap` 坏了 /
`.svelte` 原文找不到 / map 过期 / 锚点表整体作废),给错理由会把人支到错误的方向。
所以 Problems 面板里出现 `target/**/out/*.rs` 的条目是预期行为。
映射机制与三档降级见 `src/sourcemap.rs` 头部;方案背景见 `docs/plans/lsp-spike.md`。

已知降级(诚实列):包络**不是真嵌套的**,region 粒度是"一个 parse 入口的一行",
所以 `Envelope` 那一档给的是行级近似而非节点级(节点栈是 `lsp-spike.md` §3.2
点名要做、§6 批准第一版缓做的一项)。

---

**EN** — The `.svelte` single-file-component compiler: runes source transform over the script block plus Svelte template syntax, emitting Rust through a build.rs / OUT_DIR integration.
This crate is part of the [svelte-rs](https://github.com/Cyaim/svelte-rs) workspace;
start from the repository root for guides (bilingual) and runnable examples.

### `sv check` — put rustc's errors back on the `.svelte`

Generated code lives in `$OUT_DIR/<component>.rs`, so rustc reports type errors
at coordinates like `target/debug/build/<pkg>-<hash>/out/counter.rs:44:44` — worthless
to someone editing a `.svelte`. `sv check` runs `cargo check --message-format=json` and
relocates each diagnostic through the `.svmap` sidecar (written by build.rs in the
same pass as the `.rs`):

```sh
cargo run -q -p sv-compiler --bin cargo-sv -- check              # defaults to --workspace
cargo run -q -p sv-compiler --bin cargo-sv -- check -p counter-sfc
```

Output is one rustc-style line per diagnostic (`path:line:col: level[code]: message`),
which the problemMatcher in `.vscode/tasks.json` turns into Problems-panel entries and
editor squiggles — **no VS Code extension involved**.

**A diagnostic is never dropped.** When it cannot be relocated, the generated-file
position is kept and a `[sv check: …]` note explains *why* — and the reason has to be
the real one. There are five distinct causes (landed on generated glue / the `.svmap`
is unreadable / the recorded `.svelte` is gone / the map is stale / the anchor table was
voided wholesale); reporting the wrong one sends people digging in the wrong place.
Unparsable cargo output is echoed verbatim on stderr rather than skipped.

Known degradation, stated plainly: envelopes are **not** truly nested. A region is
"one line of one parse site", so the `Envelope` tier is a line-level approximation,
not a node-level one (the node stack called for in `docs/plans/lsp-spike.md` §3.2 is
deferred by §6 for the first cut).

## 许可 / License

MIT OR Apache-2.0
