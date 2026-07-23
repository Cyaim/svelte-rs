**中文** | [English](../en/reactivity.md)

# 响应式:runes 内核(sv-reactive)

`sv-reactive` 是按 Svelte 5 runes 建模的细粒度响应式内核,面向单线程桌面 UI。
它是整个栈的最底层:模板(不论 `view!` 宏还是 `.svelte` 文件,见
[sv-components](./sv-components.md))最终都编译成 `state`/`derived`/`effect`
调用,直接精准修改保留模式场景树——没有虚拟 DOM,运行时不做 diff(见
[architecture](./architecture.md))。本项目是探索原型,API 随时可能变。

## Svelte runes ↔ Rust API 对照

| Svelte | sv-reactive | 说明 |
|---|---|---|
| `$state` | `state` / `Signal<T>` | 显式 `get`/`set`,没有 Proxy 魔法 |
| `$derived` | `derived` / `Derived<T>` | 惰性求值 + `PartialEq` 剪枝;支持可写覆盖 |
| `$effect` | `effect` | 创建时同步首跑(刻意差异,见下文) |
| `$effect.pre` | `effect_pre` | 两阶段 flush 的第一相位 |
| `$effect.tracking()` | `is_tracking` | |
| `$effect.root` | `create_root`(最接近的对应) | 所有权作用域,手动 `dispose` |
| `$effect.pending()` | `sv_ui::tasks::pending_count` | 响应式的在途任务数 |
| `$props.id()` | `unique_id` | 线程内自增:`"sv-1"`、`"sv-2"`… |
| `tick` | `tick` | 帧对齐下是 flush 逃生舱;离屏路径写入本就同步 flush |
| `setContext` / `getContext` | `provide_context` / `use_context` | 按 Rust 类型索引,不用字符串键 |
| `untrack` | `untrack` | |

刻意放弃的特性(见 [DESIGN.md](../DESIGN.md) ADR-1):隐式赋值响应(`count += 1`
触发更新)与 Proxy 深层响应。裸 Rust 里用显式 `set`/`update`;`.svelte` 编译器前端
则通过源变换把隐式写法找回来。

## `#[derive(Store)]`:字段级信号

`Signal<整个结构体>` 粒度太粗——改一个字段会把只读别的字段的 effect 一起叫醒。
Svelte 用 Proxy 做深层响应,这里走编译期:**每个字段一个 `Signal`**。

```rust
use sv_macro::Store;

#[derive(Store, Clone, PartialEq)]
struct Settings { theme: String, volume: f32 }

let s = Settings { theme: "dark".into(), volume: 0.8 }.into_store();
s.volume.set(0.5);                  // 只叫醒读 volume 的 effect
let snap: Settings = s.snapshot();  // 读回整值(会订阅**所有**字段)
s.apply(next);                      // 整体写回,只写值变了的字段
```

生成的 `SettingsStore` 是 `Copy` 的(字段都是 `Signal` 句柄),可以随手塞进闭包。
要求:具名字段、无泛型、字段类型 `Clone + PartialEq + 'static`。
**不做嵌套 store**:内层结构体仍是一个整体信号,想更细就给内层也 derive 一次
——自动递归会让类型与所有权难以预料。

## state

```rust
use sv_reactive::state;

let count = state(0);           // Signal<i32>,Copy 句柄
count.get();                    // 读取并建立依赖(需要 T: Clone)
count.with(|v| v.to_string());  // 借用读取并建立依赖,不 clone
count.get_untracked();          // 读取但不订阅(还有 with_untracked)
count.set(1);                   // 写入并通知——不做相等性检查,写同样的值也会触发
count.update(|v| *v += 10);     // 原地修改并通知
```

`Signal<T>` 是指向 thread-local arena 的 `Copy + !Send` 世代句柄,随意塞进任意多个
闭包。句柄上的 `PartialEq`/`Hash` 比较的是**身份**(是否同一节点),不是值。

## derived

```rust
use sv_reactive::{state, derived};

let a = state(1);
let doubled = derived(move || a.get() * 2);  // Derived<i32>,要求 T: PartialEq
assert_eq!(doubled.get(), 2);
```

`derived` 是惰性的:没人读就不计算,标脏了但没人读也不会重算。重算后若新值与旧值
`==` 相等,不惊动下游(相等性剪枝)。

### 可写 derived(乐观 UI)

对应 Svelte 5.25 的 writable `$derived`:可以从**外部**临时覆盖派生值:

```rust
let a = state(1);
let d = derived(move || a.get() * 2);
d.set(100);                    // 乐观覆盖,下游立即看到 100
d.update(|v| *v += 1);         // 在最新派生值基础上原地修改覆盖值
a.set(5);                      // 任一依赖变化 → 重算结果盖回,覆盖自动回退
assert_eq!(d.get(), 10);
```

语义要点:`set`/`update` 会先把派生值拉到最新(顺带建立依赖边,保证从未读过的
derived 覆盖后也能回退)。`set` 在覆盖值与当前派生值相等时剪枝;`update` 不剪枝
(不 clone 拿不到旧值副本,无从比较)。在 derived 计算过程中写 derived 照样
panic——允许的只是"从外部"写。

## effect 与 effect_pre

```rust
use sv_reactive::{state, effect, effect_pre, on_cleanup};

let count = state(0);
let handle = effect(move || {
    println!("count = {}", count.get());   // 依赖自动追踪
    on_cleanup(|| println!("重跑前 / 销毁时执行"));
});
count.set(1);       // effect 同步重跑(先执行 cleanup)
handle.dispose();   // 可选的提前销毁;丢弃句柄不影响 effect 运行
```

**与 Svelte 的刻意差异**:Svelte 把 effect 推迟到微任务,这里选择**创建时同步
首跑**——对桌面事件循环更直观。首跑视作一次原子刷新:期间写入的 state 在首跑
结束后统一 flush 一轮。

依赖每次运行都重新收集,分支切换后旧依赖自然退订。重跑前会先级联销毁上次运行
创建的子节点(嵌套 effect、临时 signal)并执行 `on_cleanup` 回调——`{#if}` 分支
的销毁逻辑由此免费获得。

`effect_pre` 即 `$effect.pre`:除调度顺序外与 `effect` 完全一致——同一轮 flush 里
所有 pre effect 先于普通 effect 执行。本模型里普通 effect 承担"渲染"写入(改
场景树),pre 用于在渲染前读取旧状态。

## batch 与 tick

```rust
use sv_reactive::{state, batch, tick};

let a = state(1);
let b = state(2);
batch(|| {
    a.set(10);
    b.set(20);   // 两次写入只在闭包结束后触发一轮 effect
});
tick();          // 立即冲刷待决 effect;batch 内调用是 no-op
```

`tick` 在**开窗应用**里是逃生舱,在离屏/测试里是 API 对齐——差别来自帧调度:

### 帧对齐(ADR-6)

`sv_shell::run_app` 开窗后会调 `set_frame_scheduler`,此后**写入不再当场跑
effect**:入队 + 请求一帧,由渲染壳在帧前统一冲刷,然后才布局、绘制。
一次事件里连写十个 state,只重绘一帧、只跑一轮 effect
(Svelte 用 microtask flush 达成同一件事,桌面端的等价物是帧边界)。

```rust
count.set(1);
count.set(2);
assert_eq!(derived_total.get(), /* 仍是旧值 */);  // 帧还没到
tick();                                           // 逃生舱:现在就要结果
```

**只有开窗路径会开启**。离屏渲染(`render_to_png`)与测试保持"写入即同步
flush",行为与过去一致——所以离屏测试里 `tick` 依旧基本是空操作。
需要显式关闭时用 `clear_frame_scheduler()`。

## untrack 与 is_tracking

```rust
use sv_reactive::{state, effect, untrack, is_tracking};

let a = state(1);
let b = state(2);
effect(move || {
    let _ = a.get() + untrack(|| b.get());  // b 被读到但不建立依赖
    assert!(is_tracking());                 // effect / derived 内为 true
    assert!(!untrack(is_tracking));         // untrack 内(以及顶层)为 false
});
```

## 所有权作用域:create_root、on_cleanup、detached

```rust
use sv_reactive::{create_root, state, effect, detached};

let (value, root) = create_root(|| {
    // 回调内创建的所有节点都挂在这个根下
    let s = state(0);
    effect(move || { s.get(); });
    42
});
root.dispose();   // 级联销毁:effect 停跑、signal 回收、cleanup 执行

// 逃生舱:永不挂进任何作用域的节点(线程级单例)
let global = detached(|| state(0usize));
```

`create_root` 是卸载原语:组件和 keyed `{#each}` 的行都活在 root 里,拆除只需
一次调用。`on_cleanup` 注册到**当前**作用域(effect 或 root),在每次重跑前和
销毁时执行。`detached` 在无所有者、无追踪的环境下执行闭包——异步桥的在途计数
信号就靠它:该信号可能在某个 effect 运行期间被惰性初始化,不游离创建就会随那个
effect 的重跑被误销毁。

## Context:provide_context / use_context

```rust
use std::rc::Rc;
use sv_reactive::{create_root, provide_context, use_context};

struct Theme(&'static str);

let (_, root) = create_root(|| {
    provide_context(Theme("dark"));                    // 按 TypeId 挂到当前作用域
    let theme: Option<Rc<Theme>> = use_context::<Theme>();
    assert_eq!(theme.unwrap().0, "dark");
});
```

查找沿 **owner** 链向上,取最近一层提供的值;内层同类型就近覆盖外层。owner 边
记录的是节点**创建时刻**的作用域,所以查找能穿过 `create_root` 边界——keyed each
的行作用域照样能取到组件层的 context。作用域重跑时上一轮挂的 context 会一并
清掉(由重跑重新 provide)。

## 护栏

| 你做了什么 | 结果 |
|---|---|
| 在 `derived` 计算过程中写 state(或 derived) | panic——对应 Svelte 的 `state_unsafe_mutation` |
| effect 里循环写自己的依赖 | 超过 `MAX_FLUSH_PASSES`(1000)轮不收敛后 panic |
| effect 内同步触发自身重跑 | panic(effect 重入执行) |
| 在 `with` 回调里重入读取**同一个**节点 | panic(读其它节点没问题) |
| 作用域销毁后继续使用句柄 | panic,报"已随作用域销毁" |

## 调度:push-pull,glitch-free

调度采用与 Svelte 5 / reactively 同款的 push-pull 三态脏标记:`Clean` / `Check`
(上游 derived **可能**变了)/ `Dirty`(确定要重算)。写 signal 只向下游推标记并
把 effect 入队;derived 被读到时才拉取重算,`Check` 态节点会先逐个确认上游再决定
是否重跑。效果:菱形依赖(`a → b, a → c, effect(b, c)`)每次变化 effect 恰好跑
一次,永远不会读到"半更新"的 glitch 状态,相等性剪枝还能提前掐断传播。

## 线程模型

整张响应式图放在 **thread-local** arena(slotmap)里,`Signal`/`Derived` 句柄
`Copy + !Send`。没有锁、没有 `Arc` 开销——也从构造上杜绝了跨线程访问。后台工作
通过消息把结果送回 UI 线程,这座桥就是 `sv_ui::tasks`:

```rust
use std::time::Duration;
use sv_reactive::{create_root, state};
use sv_ui::tasks;

let (_, _root) = create_root(|| {
    let got = state(0i32);
    tasks::spawn(async { 41 + 1 }, move |v| got.set(v)); // Future 在后台线程跑
    assert_eq!(tasks::pending_count(), 1);               // 响应式:$effect.pending()
    assert!(tasks::pump_until_idle(Duration::from_secs(5)));
    assert_eq!(got.get_untracked(), 42);                 // 回调在 UI 线程执行
});
```

`spawn(fut, on_done)` 把 `Send` Future 丢到独占的后台线程(park/unpark 实现的
极简 `block_on`);完成值经通道回传,`on_done` 在 UI 线程 `pump()` 时执行——窗口
壳每帧/空闲时调用 `pump`,并通过 `set_waker` 注册的唤醒器把事件循环拍醒。
`cancel(id)` 丢弃回调(后台线程照常跑完,完成值被丢弃)。`pump_until_idle(timeout)`
是无窗测试用的阻塞版本。

再往上是 `tasks::await_block` / `await_block_result`(`{#await}` 的运行时):
Future **工厂**在 effect 里求值,追踪到的依赖一变就取消旧任务、视图回到 pending
分支、派发新任务——依赖重启语义,与 Svelte `{#await}` 换新 promise 的行为一致。

## 动画(简述)

`sv_ui::anim` 是极简动画驱动:`transition_in_fade(&doc, node, dur)` 实现 v0 的
淡入(`transition:fade` / `in:fade`,目前只有 opacity 通道);shell 每帧调
`anim::pump(now_ms)` 推进,`anim::active()` 为 true 时继续排帧。出场过渡(`out:`)
需要 INERT 延迟销毁机制,暂缓——见 [SVELTE-SUPPORT.md](../SVELTE-SUPPORT.md)。

## 相关页面

- [sv-components](./sv-components.md) — 两个模板前端如何编译到这些原语
- [architecture](./architecture.md) — 内核在 crate 分层中的位置
- [DESIGN.md](../DESIGN.md) — ADR-1:thread-local arena + Copy 句柄,以及路线图
