**中文** | [English](../en/sv-components.md)

# 用 `.sv` 组件写界面

svelte-rs 有两个模板前端,编译目标相同——对 `sv-ui` 保留式场景树的定点更新调用(没有虚拟 DOM,运行时零 diff):**`.sv` 单文件组件**,由 `sv-compiler` 在 `build.rs` 阶段编译(本页主角);**`view!` 过程宏**,来自 `sv-macro`(见最后一节)。

本项目是探索原型,API 随时会变。每一项 Svelte 5 语法的支持状态以支持矩阵 [../SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) 为准——本页与矩阵冲突时,以矩阵为准。

## `.sv` 文件的结构

一个 `.sv` 文件最多三个块:`<script>`(纯 Rust + runes,必须是第一个块)、模板标记(Svelte 模板语法 + 封闭元素集)、可选的 `<style>` 块(scoped 类,习惯放最后)。元素集是封闭的:`<view>`、`<text>`、`<button>`、`<checkbox>`(叶子元素,必须自闭合)。未知小写标签直接编译报错;首字母大写的标签(如 `<TodoItem />`)是组件调用。

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

每个 `.sv` 文件编译成一份人类可读的 Rust 源码(`prettyplease` 格式化),导出 `pub fn <fn_name>(doc, parent[, props])`;函数名取文件名 snake_case 化(`Counter.sv` → `counter`,`TodoItem.sv` → `todo_item`)。编译错误带 `.sv` 源文件内 1-based 的行/列定位。完整可运行示例:`examples/counter-sfc`、`examples/todo-sfc`、`examples/showcase`。

## runes 源变换

编译器会改写整个 `<script>` 作用域,让反应式像 Svelte 5 一样隐式生效——这是 proc-macro 做不到的事。你写的代码:

```rust
let count = $state(0i32);
let double = $derived(count * 2);
$effect(|| println!("count = {}", count));
let reset = || count = 0;
let bump  = || count += count;
```

生成代码里实际是:

```rust
let count = ::sv_reactive::state(0i32);
let double = ::sv_reactive::derived(move || count.get() * 2);
::sv_reactive::effect(move || println!("count = {}", count.get()));
let reset = move || count.set(0);
let bump = move || {
    let __sv_rhs = count.get();              // RHS 预求值:update 闭包内
    count.update(|__v| *__v += __sv_rhs);    // 不会再发生重入读
};
```

变换规则(见 `crates/sv-compiler/src/script.rs`):裸读 `count` → `count.get()`,包括白名单宏(`format!`、`println!`、断言、`vec!` 等)的参数——反应式变量出现在**非**白名单宏里是硬编译错误,不会静默漏改;写位置 `x = v` → `x.set(v)`,复合赋值 `x += v` → `x.update(..)` 且右侧先求值;引用反应式变量的闭包自动加 `move`(句柄是 `Copy`,零成本);显式 `.get()/.set()/.update()/.with()/.get_untracked()/.with_untracked()` 调用原样放行,绝不二次包装;闭包/fn 参数、`match` 臂、`for` / `if let` 模式的遮蔽都正确处理,字符串字面量与注释免疫改写。

v0 已知限制:字段/索引赋值(`pos.x = 1`)不改写——请用 `pos.update(|v| v.x = 1)`;方法调用式写入(`items.push(v)`)同理——请用 `items.update(|v| v.push(item))`;`format!("{count}")` 行内捕获不改写——请用 `"{}", count`。核心 runes 见 [./reactivity.md](./reactivity.md);逐变体的完整支持状态(`$state.raw`、`$derived.by`、`$inspect`、逃生舱 `$sig(x)` 等)见支持矩阵 [../SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md)。

## 模板语法

以下每一项都有 `crates/sv-compiler/src/lib.rs` 里的编译器测试作实证。

### 插值、属性、事件

```svelte
<text>Count: {count} · doubled = {double}</text>    <!-- {expr}:任意 Rust 表达式 -->
<text fg="#ff3e00" font-size="28">静态样式属性</text>
<Card {title} />                                    <!-- 简写 ≡ title={title} -->
<button onclick={|| count += 1}>+1</button>         <!-- Svelte 5 事件属性形态,推荐 -->
<button on:click={|| count = 0}>reset</button>      <!-- 遗留别名,仍然接受 -->
<text onpointerenter={|| hovers += 1}>悬停区</text>
```

静态与插值混排的文本编译成单个 `bind_text` 绑定;全静态文本零绑定。属性值形态是 `name="静态字符串"` 或 `name={rust_表达式}`。当前事件集只有 `onclick`/`on:click`、`onpointerenter`、`onpointerleave`——其它事件(键盘、输入)等焦点链基建,写了会编译报错而不是静默失效。内联 `style="k:v; ..."` 与样式简写属性(`fg=`、`font-size=` 等)属于样式迷你语言,见 [./styling.md](./styling.md)。

### `{#if}` / `{#each}` / `{#key}`

```svelte
{#if count > 5}
  <text>超过 5 了</text>
{:else if count < 0}
  <text>负数啦</text>
{:else}
  <text>还早</text>
{/if}

{#each todos as label, i}          <!-- 项 + 可选索引;模式可解构 -->
  <text>{i}: {label}</text>
{:else}
  <text>空空如也</text>             <!-- 空列表分支 -->
{/each}

{#each items as it (it.0)}         <!-- keyed:key 相同 ⇒ 行复用,行内状态保留 -->
  <TaskRow id={it.0} label={it.1} />
{/each}

{#key count}                       <!-- 表达式一变,整块销毁重建 -->
  <text>count 变化时我销毁重建</text>
{/key}
```

`{#each expr}` 省略 `as` 时按长度渲染 N 次。keyed `{#each}` 暂不能与索引或 `{:else}` 组合(编译报错)。分支/行销毁时,块内创建的状态和绑定一并回收。

### `{#await}` / `{:then}` / `{:catch}`

```svelte
{#await async move { base + 1 }}
  <text>加载中…</text>
{:then v}
  <text>{v}</text>
{/await}
```

被 await 的表达式是 **future 工厂**:其中的反应式读会被改写,依赖一变就取消在途任务并重启。带 `{:catch e}` 分支时 future 必须产出 `Result`。执行跑在 `sv_ui::tasks` 的后台线程异步桥上。

### Snippet:`{#snippet}` / `{@render}`

```svelte
{#snippet badge(label: String, n: i32)}
  <text>{label}: {n}</text>
{/snippet}
{@render badge(String::from("计数"), count)}
```

Snippet 编译成局部闭包;参数是带类型的 Rust 模式。参数的响应式通过"以参数元组为 key 的重建"实现——粒度比 Svelte 粗一档、语义一致——因此参数类型需要 `Clone + PartialEq`。

### 标签:`{@const}`、`{@attach}`、`{@debug}`、注释

```svelte
{@const summary = format!("共 {} 项", count)}   <!-- 编译成块级 derived -->
<text>{summary}</text>
<view {@attach |d: &sv_ui::Doc, id: sv_ui::ViewId| { /* 命令式逃生舱 */ }}></view>
{@debug count, double}                          <!-- 依赖一变就 Debug 打印 -->
<!-- 注释在编译期剥除;<svelte:options … /> 接受并忽略 -->
```

`{@attach}` 的闭包在挂载时于 effect 内运行,反应式依赖变化即重跑;它一并承担 Svelte `use:` 与 `bind:this` 的职责(这两个刻意不做)。

### 指令:`class:` 与 `style:`

```svelte
<text class="title" class:muted class:big={count > 5} style:padding={size * 2.0}>有样式的字</text>
```

`class="…"` 只收静态字符串,类名必须在本文件 `<style>` 块里定义过(未知类编译报错)。`class:name={cond}` 条件开关类;简写 `class:muted` 以同名变量作条件。`style:字段={expr}` 响应式绑定单个样式字段;可用字段(见 `codegen.rs`):`padding`、`margin`、`gap`、`font-size`/`font_size`、`radius`/`corner-radius`、`opacity`、`width`、`height`、`direction`、`bg`、`fg`/`color`。优先级从弱到强:类 < `style=""` < 条件类 < `:hover` < `style:` 指令。

### 双向绑定:`bind:`

```svelte
<checkbox bind:checked={done} />     <!-- 元素绑定:状态→视图 与 点击→状态 -->
<Stepper bind:value={count} />       <!-- 组件绑定:直传 Signal 句柄,见 $bindable -->
```

元素级 `bind:` 目前**只支持 `<checkbox>` 上的 `bind:checked`**。`bind:value` 及其余 Svelte 目标依赖尚不存在的文本输入控件与布局测量——写了会编译报错并指向支持矩阵。

### 过渡

```svelte
<view transition:fade><text>淡入,默认 200 ms</text></view>
<view in:fade={500}><text>500 ms 淡入</text></view>
```

目前只有进场方向,所以 `transition:fade` 与 `in:fade` 等价;时长单位是毫秒(`u32`)。`out:`(需要 INERT 延迟销毁)与 `animate:`(需要 FLIP)会被拒绝并给出说明性错误。

## 组件

组件标签用 PascalCase,解析到同名 `.sv` 文件 snake_case 化的函数——`.sv` 文件内不需要任何 import,因为 `sv_compiler::build` 会先整体扫描源码目录、第一遍注册全部 `$props` 签名(组件可以互相引用);生成的所有 `.rs` 文件随后要 `include!` 进同一个 Rust 作用域(见下文)。

### 用 `$props` 声明 props

```svelte
<script>
$props {
    label: String,
    index: usize,
    on_remove: std::rc::Rc<dyn Fn()>,                     // 回调就是普通 prop
    accent: sv_ui::Color = sv_ui::Color::rgb(255, 62, 0), // 默认值
}
</script>
```

这会生成 `pub struct TodoItemProps` 并给组件函数加上 `props` 参数。调用侧:缺必填 prop、传未知 prop 都是编译错误;省略有默认值的 prop 会调用 callee 侧的 `default_<name>()` 关联函数。props 是**按值传递的快照**:`label={name}`(`name` 是 `$state`)编译成 `label: name.get()`,之后的变化*不会*跟进。要传活的响应式,请传 `Signal` 句柄(`$sig(x)`)或改用 `$bindable`。

### 双向 prop:`$bindable`

```svelte
<!-- Stepper.sv(callee)-->
<script>
$props { value: $bindable(i32), step: i32 = 1 }
</script>
<view style="direction:row; gap:8">
  <button onclick={|| value -= step}>-{step}</button>
  <text font-size="20">{value}</text>
  <button onclick={|| value += step}>+{step}</button>
</view>

<!-- 调用侧:  <Stepper bind:value={count} step={2} /> -->
```

`$bindable(i32)` 让该字段成为 `::sv_reactive::Signal<i32>`;callee 内 `value` 与 `$state` 变量一样参与 runes 改写。调用侧直传裸句柄——两边读写同一个信号,零胶水。对非 bindable 的 prop 用 `bind:` 是编译错误。

### `children` 与 snippet prop

```svelte
<!-- Card.sv(callee)-->
<script>
$props { title: String, children: sv_ui::Snippet }
</script>
<view class="card">
  <text class="card-title">{title}</text>
  {@render children()}
</view>

<!-- 调用侧:<Card title={…}><text>正文 {n}</text></Card> —— 标签体自动成为 children -->
```

具名 `{#snippet}` 也可以显式当 prop 传(`<Card body={hello} />`);零参 snippet 会自动包成基于 `Rc` 的 `sv_ui::Snippet` 类型。

## 构建集成:`build.rs` + `include!`

与 `examples/counter-sfc` 完全一致——构建依赖 `sv-compiler`,运行时依赖 `sv-reactive`、`sv-ui`、`sv-shell`:

```rust
// build.rs
fn main() {
    // 扫描 src/ 下所有 .sv,编译成 $OUT_DIR/<组件名>.rs
    sv_compiler::build("src");
}
```

```rust
// src/main.rs (examples/counter-sfc/src/main.rs)
include!(concat!(env!("OUT_DIR"), "/counter.rs"));

fn main() {
    sv_shell::run_app("sv 计数器(SFC)", |doc, root| counter(doc, root)).expect("运行失败");
}
```

`sv_compiler::build("src")` 递归收集 `*.sv`,先注册全部 `$props` 签名,再逐个编译到 `$OUT_DIR/<fn_name>.rs`,并逐文件发出 `cargo::rerun-if-changed`;编译失败直接 panic,格式 `文件:行:列: 消息`。多组件应用把每个生成文件都 `include!` 进同一作用域——例如 `examples/todo-sfc/src/main.rs` 同时 include 了 `todo.rs` 与 `todo_item.rs`。

```sh
cargo run -p counter-sfc                    # 开窗运行
cargo run -p counter-sfc -- --png out.png   # 离屏渲染一帧,无需窗口
```

## `view!` 宏路线

过程宏前端(`crates/sv-macro`)编译目标同为 `sv-ui` 绑定原语(`bind_text` / `bind_style` / `if_block` / `each_block`),但模板是 Rust 原生语法,反应式**显式**书写——proc-macro 改写不了宏外的代码,所以没有 runes 变换:

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

|  | `.sv`(sv-compiler) | `view!`(sv-macro) |
|---|---|---|
| 模板语法 | Svelte(`{#if}`、`{#each}`、`onclick` 等) | Rust 原生(`if`/`else if`/`else`、`for item, i in expr`) |
| 反应式 | 隐式(runes 源变换) | 显式 `.get()` / `.set()` / `.update()` |
| 元素 | `view`/`text`/`button`/`checkbox` + 组件 | 仅 `view`/`text`/`button` |
| 特性面 | 本页全部内容 | 文本插值、`if`/`for` 块、`style(闭包)`、`on_click(闭包)` |
| 构建配置 | `build.rs` + `include!` | 无——宏内联展开 |

想在现有 Rust 代码里内联嵌一小块响应式 UI、不想碰构建脚本,或希望一切保持纯 Rust 以获得完整 IDE 体验时,选 `view!`。按 ADR-2(修订版),双前端共存;合并两者的编译内核是 M1 目标——见 [../DESIGN.md](../DESIGN.md)。

## 相关阅读

- [./reactivity.md](./reactivity.md) — 信号、runes、effect 与反应式运行时。
- [./styling.md](./styling.md) — 内联 `style=""` 迷你语言与 `<style>` 块。
- [../SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) — 全部 77 项 Svelte 5 支持矩阵。
- [../CSS-SUPPORT.md](../CSS-SUPPORT.md) — `<style>` 块接受的 CSS 子集。
