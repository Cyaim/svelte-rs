# svelte-rs(工作代号 `sv`)

Svelte 风格的 Rust 跨平台桌面 UI 库 — **探索原型**。

把 Svelte 5 的编译哲学搬到原生桌面:模板在编译期变成对 retained 场景树的
**定点更新代码**,运行时没有虚拟 DOM、没有 diff。目标平台:Windows / Linux /
macOS / 鸿蒙(HarmonyOS NEXT)。

```rust
let count = state(0);                       // $state
let double = derived(move || count.get() * 2); // $derived
effect(move || println!("{}", double.get()));  // $effect
count.set(1);                                // 精准触发,无 diff
```

## 仓库结构

| 路径 | 说明 |
|---|---|
| `crates/sv-reactive` | runes 响应式内核(push-pull 三态脏标记、effect 所有权树) |
| `crates/sv-ui` | retained 场景树(桌面版 DOM)+ 细粒度绑定原语 |
| `crates/sv-macro` | `view!` 宏前端(proc-macro 路线) |
| `crates/sv-compiler` | `.sv` 单文件组件编译器前端(编译器路线:runes 源变换 + 原汁 Svelte 模板语法) |
| `crates/sv-shell` | winit 窗口 + CPU 自绘渲染壳(原型;归宿是 wgpu/vello + Parley) |
| `examples/counter` | 计数器 · `view!` 宏写法 |
| `examples/counter-sfc` | 计数器 · `.sv` 文件写法(`src/Counter.sv`,build.rs 编译) |
| `examples/todo-sfc` | 待办 · `.sv` 特性集(组件+`$props`、`{#each}{:else}`、`{@const}`、`{#key}`、`style:` 指令、`$inspect`) |
| `examples/showcase` | **特性橱窗**(推荐先看):`$bindable` 双向绑定、children snippet、`{#snippet}/{@render}`、keyed `{#each}` 重排保状态、`<style>` scoped 类 |
| `docs/SVELTE-SUPPORT.md` | Svelte 5 语法/特性支持矩阵(77 项,终局 ✅43/🚧3/📋0/⏳11/❌20) |
| `docs/CSS-SUPPORT.md` | 现代 CSS 对比矩阵(91 项差距表:✅12 / C1+C2 排期 32 / P2 13 / ⏳18 / ❌16) |
| `docs/DESIGN.md` | 架构设计与决策记录(ADR) |
| `docs/research/` | 5 份联网核实的深度调研(Svelte 内核 / 生态 / 鸿蒙 / 编译器 / 渲染栈) |

## 快速开始

```sh
cargo test                    # 全部测试(77 个)
cargo run -p showcase         # 特性橱窗(推荐)
cargo run -p counter          # 计数器(view! 宏路线)
cargo run -p counter-sfc      # 计数器(.sv 编译器路线,UI 在 src/Counter.sv)
cargo run -p showcase -- --png out.png  # 离屏渲染一帧(无需窗口)
```

`.sv` 写法一瞥(script 里的 `count += 1` 会被编译器自动改写成句柄操作):

```text
<script>
let count = $state(0i32);
let double = $derived(count * 2);
</script>

<text>Count: {count} · 双倍 = {double}</text>
<button style="bg:#ff3e00; fg:#fff" on:click={|| count += 1}>+1</button>
{#if count > 5}
  <text fg="#ff3e00">超过 5 了!</text>
{/if}
```

> 注:检出目录在 OneDrive 等同步盘时,建议启用 `.cargo/config.toml` 里注释掉的
> `target-dir`,把构建产物移出同步目录。

## 现状与路线

M0 已完成:signal → 场景树 → 布局 → 光栅完整闭环(中文渲染、HiDPI、点击派发);
`.sv` 编译器支持 Svelte 5 主要语法面(runes 全家、全部块语法、组件模型 v0、
`$bindable` 双向绑定、keyed each、scoped `<style>`),经对抗性审查修复 17 个缺陷。
详见 [docs/DESIGN.md](docs/DESIGN.md) 与 [docs/SVELTE-SUPPORT.md](docs/SVELTE-SUPPORT.md)。

双许可:MIT OR Apache-2.0。
