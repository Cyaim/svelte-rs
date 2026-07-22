**中文** | [English](../en/architecture.md)

# 架构

> 状态:探索原型。API 随时会变,crate 名(`sv-` 前缀)是工作代号,若干层明确是
> 临时实现。决策记录与路线图见 [DESIGN.md](../DESIGN.md)。

一句话:**把 Svelte 5 的编译哲学搬到原生桌面——模板在编译期变成对 retained
场景树的定点更新代码,运行时没有虚拟 DOM、没有 diff、没有重建。**
目标平台:Windows / Linux / macOS / 鸿蒙。

## 分层

```
┌──────────────────────────────────────────────────────────────┐
│ 用户组件:view! 模板 + $state/$derived/$effect 风格 API       │
├──────────────────────────────────────────────────────────────┤
│ sv-macro     view! 过程宏前端(parse → IR → codegen)          │
│ sv-compiler  .sv 单文件组件前端                               │
│              → 两者都编译成对 sv-ui 绑定原语的调用             │
├──────────────────────────────────────────────────────────────┤
│ sv-reactive  runes 内核:Signal / Derived / Effect            │
│              thread-local arena + Copy 句柄,push-pull        │
│              三态脏标记,effect 所有权树                       │
├──────────────────────────────────────────────────────────────┤
│ sv-ui        retained 场景树(桌面版 DOM)+ 绑定原语:         │
│              bind_text / bind_style / if_block /              │
│              each_block / virtual_list / …                    │
├──────────────────────────────────────────────────────────────┤
│ sv-shell     窗口 + 渲染器                                    │
│              窗口:桌面 winit;鸿蒙走窄窗口 trait(规划中,    │
│              ADR-4)                                          │
│              渲染:默认 CPU 栈(softbuffer + tiny-skia +      │
│              swash);vello/wgpu 在 `backend-vello` feature 后 │
└──────────────────────────────────────────────────────────────┘
```

上手:

```sh
cargo test                              # 全部测试
cargo run -p counter                    # 开窗跑计数器(view! 路线)
cargo run -p counter -- --png out.png   # 离屏渲染一帧,无需窗口
```

## 数据流:没有 VDOM,没有 diff

组件函数只跑**一次**:建场景树节点、登记绑定。下面是 `sv-ui` 测试里的真实代码——
也正是两个编译前端的生成物形态:

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

点一次按钮会发生什么:

1. `count.update(..)` 写入 signal。push 阶段给订阅者标脏
   (`Clean`/`Check`/`Dirty` 三态标记,与 Svelte 5 同款)。
2. flush 执行待决 effect——这里就是 `bind_text` 创建的那一个。
3. 该 effect 调 `doc.set_text(label, ..)`:对**恰好一个**节点的定点修改。
   新文本与旧文本相同则提前返回,后面什么都不发生。
4. 真实变更会 bump `Doc` 的版本号并触发 `on_mutate` 回调。
5. sv-shell 把 `on_mutate` 接到窗口的重绘请求上,下一帧从 retained 树重画。

没有任何东西需要 diff:哪个值变了就动哪个节点,这条连线在编译期就定死了。
相等剪枝存在于两层——`derived` 重算结果与旧值 `==` 时不惊动下游;`Doc` 的
setter(`set_text` / `set_style` / `set_checked`)值未变时不 bump 版本号,
渲染端不会为无效写入白白重绘。

当前的局限:写入即同步 flush(正确,但未对齐帧)。帧调度——把写入攒到
pre → render → layout → paint 管线里——是 ADR-6,**尚未实现**。

## 双编译前端,同一编译目标(ADR-2 修订版)

两个前端编译产物相同:命令式的 `Doc` 建树代码 + 对 sv-ui 绑定原语的调用。
两条路线的生成代码形态几乎一致——这正是合并内核成立的依据。

`view!` 过程宏路线(`examples/counter`):

```rust
view! { doc, root =>
    <text style(|s| s.font_size = 20.0)>"Count: " {count.get()}</text>
    <button style(btn) on_click(move || count.update(|c| *c += 1))>"+1"</button>
    if count.get() > 5 {
        <text style(|s| s.fg = Some(Color::rgb(255, 62, 0)))>"超过 5 了!"</text>
    }
}
```

`.sv` 单文件组件路线(`examples/counter-sfc/src/Counter.sv`,由 `build.rs`
调 `sv_compiler::build("src")` 编译进 `OUT_DIR`):

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

| | `view!` 宏(sv-macro) | `.sv` SFC(sv-compiler) |
|---|---|---|
| runes | 显式:`count.get()`、`count.update(..)` | 隐式:裸 `count` 读、`count += 1` 写——整个 script 作用域的源变换 |
| 模板语法 | 受 Rust tokenizer 约束(文本要引号,Rust 的 `if`/`for`) | 原汁 Svelte:免引号文本、`{#if}{:else}`、`on:click`、`bind:` |
| 诊断 | span 精确指向用户源码 | 模板域错误带 `.sv` 行列;rustc 类型错误落在(prettyplease 格式化的可读)生成代码里 |
| 构建 | 原地展开 | `build.rs` + `OUT_DIR` + `include!` |
| 编译目标 | sv-ui 绑定原语 | sv-ui 绑定原语(相同) |

ADR-2 原版结论是"proc-macro 起步"。两条路线的可运行原型并排实证之后修订为:
**双前端共存,共享同一编译器核心**。三步无悔:① sv-macro 与 sv-compiler
合并为单一内核、② 模板数据化(生成数据而非生成类型)、③ codegen 拆
setup/render——后两步同时服务热重载。

**① 已落地(2026-07-22)**:对 sv-ui 的**发射口收敛为一处**
`sv_compiler::emit`(绑定原语调用词汇表 + 重建闭包协议),`view!` 宏改为
依赖 sv-compiler 并从同一词汇表发射;原语签名变更从此只改一处。
**刻意没有合并的是解析与 IR**:`view!` 的表达式是带真 span 的 Rust token,
`.sv` 的是带偏移的源码串(还要过 runes 改写)——硬合成一份 IR 会把宏路径的
span 精度赔进去,而那正是 ADR-2 保留双前端的理由。`.sv` 路线最大的
悬置风险是 IDE 体验(`.sv` 内没有 rust-analyzer,Volar 式转发 LSP 未 spike)。
详见 [sv-components](./sv-components.md)。

## 单线程响应式模型(ADR-1)

```rust
use sv_reactive::{state, derived, effect};

let count = state(0);                            // Signal<i32>:Copy + !Send
let double = derived(move || count.get() * 2);   // 惰性求值,== 剪枝
effect(move || println!("{}", double.get()));    // 创建时同步首跑
```

- 所有响应式节点放在 **thread-local** 的 arena(slotmap)里,`Signal<T>` /
  `Derived<T>` 只是 `Copy + !Send` 的世代句柄——随意塞进任意多个闭包,没有
  生命周期、不用摆弄 `Rc`。这是 Rust 借用检查下做响应式图的标准解法
  (Leptos / Sycamore 同款)。
- 刻意**不做 Send/Sync**:UI 运行时就是单线程的,每次读取都付 `Arc`/原子操作
  的钱纯属浪费。后台线程通过消息回到 UI 线程,由 UI 线程写 signal。
- 调度是 push-pull:写入只标记(push),effect 统一在 flush 里跑,derived
  被读到才重算(pull)。菱形依赖天然 glitch-free,effect 只跑一次。
- effect 构成**所有权树**:重跑前先销毁子作用域并执行 `on_cleanup`——
  `{#if}` 分支的销毁因此免费获得。
- derived 计算过程中写 state 会 **panic**(等价于 Svelte 的
  `state_unsafe_mutation` 错误)。
- 与 Svelte 的刻意差异:effect 创建时同步首跑,而非推迟到微任务;帧对齐的
  调度留给 ADR-6。

细节与完整 API 面见 [reactivity](./reactivity.md)。

## 各 crate 的职责

| crate | 职责 |
|---|---|
| `sv-reactive` | runes 内核:`state` / `derived` / `effect` / `effect_pre` / `batch` / `untrack` / `on_cleanup` / `create_root` / `provide_context` / `use_context`;thread-local runtime 与调度 |
| `sv-ui` | retained 场景树(`Doc`、`ViewNode`、`Style`)+ 两个编译器共同瞄准的绑定原语:`bind_text`、`bind_style`、`bind_style_patch`、`if_block`、`each_block`、`each_block_else`、`each_block_keyed`、`key_block`、`virtual_list`、`mount`;版本号 + `on_mutate` |
| `sv-macro` | `view!` 过程宏前端:parse → IR → codegen |
| `sv-compiler` | `.sv` SFC 前端:runes 源变换、Svelte 模板语法、样式解析、`build.rs`/`OUT_DIR` 集成(`sv_compiler::build`),错误带 `.sv` 行列 |
| `sv-shell` | winit 窗口 + 渲染器:默认 CPU 栈(softbuffer + tiny-skia + swash),vello/wgpu 在 `backend-vello` feature 后,`SV_RENDERER=cpu\|vello` 覆盖;`Painter` trait、布局、命中测试、`run_app` / `render_to_png` |
| `examples/counter` | 计数器 · `view!` 路线(开窗 + `--png` 离屏) |
| `examples/counter-sfc` | 计数器 · `.sv` 路线(build.rs 集成 + 端到端行为测试) |

渲染层明确是占位实现:当前 CPU 栈按路线图迁往 vello 家族、Parley 文本与 taffy
布局——`Painter` 抽象的存在就是为了让后端可替换。见
[rendering-backends](./rendering-backends.md)。

## ADR 索引

每条一行;完整记录见 [DESIGN.md](../DESIGN.md)。

| ADR | 决策 | 状态 |
|---|---|---|
| ADR-1 | 响应式图:thread-local arena + `Copy` 句柄;push-pull 三态脏标记;不做 Send/Sync | 已实现 |
| ADR-2(修订) | 编译策略:双前端(`view!` + `.sv`)共享同一编译目标;M1 合并为单一编译器核心 | 双前端已跑通,合并规划中 |
| ADR-3 | 渲染:CPU 栈起步,归宿是 vello 家族(Parley 文本、taffy 布局) | CPU 栈已跑通 |
| ADR-3b | 后端对比判决 + 可切换 `Painter` 抽象;vello 成为第二个真实后端;文本栈迁 swash | 已落地 |
| ADR-4 | 窗口层:窄抽象 trait,不以 winit 为架构前提(winit 没有鸿蒙 backend) | 规划中 |
| ADR-5 | 鸿蒙:技术可行(Tier-2 目标、XComponent + GLES 路径有 Flutter/Servo 先例),列第二梯队 | 规划中(M3 spike) |
| ADR-6 | 帧调度:写入攒批进帧管线,配 `flush_sync` 逃生舱 | **未实现**——最大开放设计点 |
| ADR-7 | `each` 块:keyed reconcile。`each_block_keyed`(按 key 复用行、重排保状态)已存在;每项一个 signal 的 reconcile 是目标形态 | 部分实现 |
| ADR-8 | CSS:真语法、封闭子集、编译期样式表,永不引入运行时选择器引擎 | C1 已落地,C2 规划中 |
| ADR-9 | 规模:视口虚拟化(`virtual_list`)让帧成本与逻辑控件数解耦——实测 100 万控件,CPU 后端,p99 5.28 ms / 1% low 174 fps | 已落地 |

## 相关页面

- [reactivity](./reactivity.md) — runes 内核深入
- [sv-components](./sv-components.md) — `.sv` 组件格式与构建集成
- [rendering-backends](./rendering-backends.md) — `Painter`、CPU 与 vello、`SV_RENDERER`
- [performance](./performance.md) — 实测数字与测法
- [DESIGN.md](../DESIGN.md) — 完整 ADR、路线图、风险清单
- [SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md) — Svelte 特性支持矩阵
- [CSS-SUPPORT.md](../CSS-SUPPORT.md) — CSS 支持逐项对照
