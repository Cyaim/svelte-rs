# 调研报告 01:Svelte 5 编译模型与响应式内核,及其到 Rust 的映射

> 调研日期:2026-07-17。除特别标注外,以下事实均已联网核实(核实对象:Svelte 主仓库 `main` 分支源码、官方文档、官方博客;leptos / reactive_graph / sycamore / floem 的 docs.rs 与 crates.io)。
> 目标读者:svelte-rs 项目(Svelte 风格 Rust 跨平台桌面 UI 库,目标 Windows / Linux / macOS / HarmonyOS NEXT)的架构设计者。

---

## 0. 结论速览(TL;DR)

1. **Svelte 5 的响应式内核是一个教科书级的 push-pull 混合 signal 系统**:写入时向下游"push"脏标记(`DIRTY` / `MAYBE_DIRTY` 三态),读取时"pull"惰性重算,用整数写版本号(`wv`)裁剪无效重算。整套算法**不依赖 JS 的任何动态特性**(除 `$state` 的 Proxy 深层代理),核心数据结构是普通对象 + 侵入式链表 + 位标志,**非常适合 1:1 翻译成 Rust**。
2. **Svelte 编译器的产物就是"命令式细粒度更新代码"**:静态结构走 `<template>` + `cloneNode` 一次性实例化,动态部分编译成一个个 `template_effect` 闭包,控制流块(`{#if}`/`{#each}`)编译成管理子 effect 树的 block effect。把 "cloneNode" 替换成"预编译的场景树构造函数",把 "DOM anchor" 替换成"稳定的子节点插槽",**整个编译策略可以原样搬到 retained-mode 原生场景树上**。
3. **Rust 侧映射已有两条被生产验证的路线**:leptos `reactive_graph`(Arc 引用计数原语 + arena 分配的 `Copy` 句柄双轨,Send+Sync)与 Sycamore 0.9(单例 `Root` + slotmap,全 `Copy`、无生命周期参数)。桌面单 UI 线程场景下,**推荐 thread-local runtime + 世代 arena + `Copy` 句柄**,不引入 Send/Sync 约束,这是复杂度最低、心智模型最接近 Svelte 的方案。floem(Lapce 编辑器的 UI 库)证明了该模型在原生桌面(wgpu 渲染)可行。
4. **必须放弃的 Svelte 特性**:基于赋值语法的隐式响应式(`count += 1` 触发更新)与 Proxy 深层响应式。替代方案:显式 `sig.set(..)` / `sig.update(..)` API + `#[derive(Store)]` 字段级信号派生宏。这不是损失——Svelte 本身也是编译器把赋值改写成 `$.set()` 调用,我们只是把"改写"变成"显式或宏内改写"。
5. **调度层是唯一没有现成答案、必须原型验证的部分**:浏览器有 microtask,原生侧没有。需要把 Svelte 的 Batch/flush 模型嫁接到 winit 事件循环与 ArkUI(HarmonyOS)的 VSync 管线上,保证 "state 写入 → 下一帧渲染前 flush render effects → 渲染 → user effects" 的顺序。

---

## 1. 版本与时效性核实(2026-07)

| 项目 | 核实结果 | 来源 |
|---|---|---|
| Svelte | 5.x 线,2026-05 时约 **5.55.0**,runes 模型稳定;`await` in components(async Svelte)仍在 `experimental.async` 旗标下,计划 Svelte 6 转正 | 官方博客 May 2026、docs |
| leptos | **0.8.19**(2026-04),**0.9.0-alpha**(2026-05);响应式核心为独立 crate `reactive_graph` **0.2.14** | crates.io / docs.rs |
| Sycamore | **0.9.2**,Reactivity v3(slotmap + Copy 信号)已落地 | crates.io / 官方博客 |
| floem | pre-1.0,持续活跃(2026-02 仍有更新),原生桌面 + 细粒度响应式(源自 leptos_reactive)+ wgpu/tiny-skia 渲染 | GitHub |

注意:本文引用的 Svelte 内部实现细节抓取自 `sveltejs/svelte` 主仓库 `main` 分支(领先于 5.55 发布版,包含 async 相关的 `Batch` 重构)。内部 API 无稳定性保证,但**核心算法(三态脏标记、push-pull、effect 树)自 5.0 起未变**,作为移植蓝本是安全的。

---

## 2. Svelte 5 响应式内核剖析

### 2.1 三类图节点:Source / Derived / Effect

内核里只有三种节点,全部是普通 JS 对象(源码:`internal/client/reactivity/{sources,deriveds,effects}.js`):

- **Source(`$state`)**:`{ f: flags, v: value, reactions: Reaction[] | null, equals, rv: read_version, wv: write_version }`。`reactions` 是"谁读过我"的反向边;`wv` 每次成功写入递增(全局单调计数器);`equals` 默认严格相等,值未变则**不传播**。
- **Derived(`$derived`)**:同时是 Value 和 Reaction。字段含 `fn`(计算函数)、`deps`(正向依赖边)、`reactions`(下游)、`effects`(在 derived 内部创建的子 effect,由 derived 负责销毁:`destroy_derived_effects`)、`parent`、`wv`。初值为 `UNINITIALIZED`,**完全惰性**:创建时不求值,第一次被 `get()` 时才执行。
- **Effect(`$effect` 及一切 DOM 更新)**:Reaction + **树节点**。字段含 `parent / first / last / prev / next`(侵入式双向链表构成 effect 树)、`fn`、`teardown`、`deps`、`nodes_start/nodes_end`(该 effect 拥有的 DOM 区间)、`ctx`(组件上下文)、`b`(boundary)。

状态用位标志压缩在 `f` 字段(源码 `internal/client/constants.js`),关键的有:节点种类 `DERIVED / EFFECT / RENDER_EFFECT / BLOCK_EFFECT / BRANCH_EFFECT / ROOT_EFFECT / BOUNDARY_EFFECT / MANAGED_EFFECT / USER_EFFECT`;脏三态 `CLEAN / MAYBE_DIRTY / DIRTY`;生命周期 `DESTROYING / DESTROYED / INERT / CONNECTED / REACTION_RAN`;以及优化标志 `WAS_MARKED`(避免重复标记遍历)、`EFFECT_PRESERVED`、`EAGER_EFFECT` 等。

> **Rust 映射提示**:这一整套就是 `bitflags!` + arena 索引。`reactions`/`deps` 是 `Vec<NodeId>`,effect 树的 `first/last/prev/next` 是 `Option<EffectId>`。没有任何东西需要 GC。

### 2.2 依赖收集:active_reaction + 版本号去重 + 前缀复用

依赖跟踪是经典的"执行期动态收集"(源码 `runtime.js`):

- 全局(运行时级)变量 `active_reaction` 指向当前正在执行的 reaction。`get(signal)` 时若存在 `active_reaction` 且未处于 `untrack()`,就登记依赖。
- **去重**:每个 Value 有 `rv`(read version),与全局 `read_version` 比较,同一次执行内重复读同一 signal 不会重复登记。
- **前缀复用优化(`skipped_deps`)**:重跑 reaction 时,若本次读取序列与上次 `deps` 数组的前缀一致,只递增计数器不重建数组;分叉之后才开始新建 `new_deps`,结束时对被移除的旧依赖调用 `remove_reactions()` 摘除反向边。这把"依赖不变的重跑"的分配开销降到零。
- 依赖是**最后一次执行的快照**:条件分支没走到的读取不会成为依赖(官方文档明确:"Values that are read asynchronously — after an `await` or inside a `setTimeout` — will not be tracked")。
- `untrack(fn)` 通过全局 `untracking = true` 整体旁路收集。

> **Rust 映射提示**:`active_reaction` 变成 runtime 里的 `Cell<Option<ReactionId>>`(thread-local)。`rv/wv` 版本号、`skipped_deps` 前缀优化可原样照抄,且在 Rust 里 `Vec` 复用比 JS 更自然。

### 2.3 Push-Pull 混合传播与三态脏标记

这是 Svelte 5(以及 Solid 2.0、Vue 3.4+ alien-signals 一族)共同收敛到的算法,官方文档直接称之为 push-pull reactivity:

**Push 阶段(写入时,只标记不计算)** —— `set()` → `internal_set()`(源码 `sources.js`):
1. `equals` 检查,值没变直接返回;
2. `wv = increment_write_version()`,把旧值记入当前 `Batch`;
3. `mark_reactions(source, DIRTY)`:遍历 `reactions`,**直接依赖者标 `DIRTY`**;若依赖者是 Derived,则递归把 Derived 的下游标 **`MAYBE_DIRTY`**(注意:不重算 derived 本身!),并用 `WAS_MARKED` 避免菱形依赖里的重复遍历;
4. 途中遇到的 Effect 交给 `schedule_effect()` 进入批次队列(不立即执行;仅 `EAGER_EFFECT`/block 类为了 async 探测会即时跑)。

**Pull 阶段(读取时 / flush 时,按需计算)** —— `is_dirty()`(源码 `runtime.js`):
- `DIRTY` → 必须重跑;
- `MAYBE_DIRTY` → 遍历 `deps`:对 derived 依赖递归 `is_dirty()`,脏则 `update_derived()` 真正重算;随后比较 `dependency.wv > reaction.wv`——**只有某个依赖的写版本比我上次执行时新,我才真的脏**;全部检查通过则降级回 `CLEAN`,整棵下游子树被剪枝。
- `update_derived()` 重算后走 `equals`:值没变则不递增 `wv`,下游因此在版本比较处被短路。这就是官方文档说的 "if the new value is referentially identical to the previous, downstream updates will be skipped"。

这套"三色标记"(CLEAN/MAYBE_DIRTY/DIRTY)+ 版本号的组合解决了纯 push(过度重算、菱形依赖 glitch)与纯 pull(每次读都要全图询问)各自的缺陷,并保证无 glitch(同一 tick 内永远读到一致值)。

> **Rust 映射提示**:算法与语言无关,可逐函数翻译。`is_dirty` 的递归在深图上可能需要显式栈(Rust 默认栈较浅的目标平台注意)。`equals` 对应 Rust 的 `PartialEq`,但要允许用户对非 `PartialEq` 类型退化为"总是不等"(Svelte 的 `safe_equals` 语义)。

### 2.4 Effect 树:ownership、teardown 与嵌套销毁

Svelte 的 effect 不是平铺的,而是一棵**所有权树**(源码 `effects.js`),树形即 UI 结构:

- `create_effect()` 把新 effect 通过 `push_effect()` 挂到 `active_effect` 名下;组件、`{#if}` 分支、`{#each}` 项都是 `BRANCH_EFFECT`/`BLOCK_EFFECT` 节点,普通 DOM 更新是 `RENDER_EFFECT`,用户 `$effect` 是 `USER_EFFECT`,`$effect.root` 创建游离的 `ROOT_EFFECT`(返回手动 dispose 函数)。
- **内存优化**:一个 effect 若"不会重跑、不拥有 DOM、无 teardown",创建后不保留在树上(源码注释:"we skip it and go to its child")。
- **重跑语义**:effect 重跑前先 `destroy_effect_children()` 销毁全部子 effect(ROOT 例外,会被摘为独立根)。这就是"嵌套 effect 的 dispose":子 effect 的生命周期严格嵌套于父 effect 的单次执行。
- **teardown 顺序**:`execute_effect_teardown()` 在 `set_is_destroying_effect(true)` + `active_reaction = null` 环境下执行,防止 cleanup 里再注册依赖;子先于父清理。
- `destroy_effect()` 的完整阶段:移除 DOM 区间(`remove_effect_dom`,靠 `nodes_start/nodes_end`)→ 置 `DESTROYING` → 递归销毁子树 → 摘除 signal 反向边 → 停止 transition → 执行 teardown → `unlink_effect()` 从父链表摘除 → 清空引用。
- **pause/resume(`INERT`)**:`{#if}` 切换时旧分支不是立即销毁,而是 `pause_effect()` 置 `INERT`(不再更新、DOM 保留)等 outro 过渡完成再销毁;条件回切则 `resume_effect()` 复活并补跑变脏的子 effect。**这对原生 UI 的退场动画同样是刚需**,值得完整移植。

> **Rust 映射提示**:effect 树 = leptos/sycamore 的 **Owner/Scope 树**。所有权语义(父销毁 ⇒ 子销毁;重跑 ⇒ 子销毁重建)正是 Rust drop 语义的舒适区:arena 里按 `EffectId` 递归 dispose,teardown 存 `Option<Box<dyn FnOnce()>>`。`nodes_start/nodes_end` 对应"场景树子节点区间句柄"。

### 2.5 调度:Batch、microtask flush 与执行顺序

源码 `reactivity/batch.js`(2025 年为支持 async 由 Rich Harris 重构,PR #15348 "simplify flushing" 及后续):

- 所有写入发生在一个 **`Batch`** 里:`Batch.ensure()` 惰性创建当前批次,并在非 `flushSync`、非正在处理时 `queue_micro_task(() => batch.flush())` —— 同一同步代码段内的多次赋值合并为一次 flush("changing color and size in the same moment won't cause two separate runs")。
- `Batch` 记录 `previous`/`current` 两张 source→值 的映射(供 async 时间旅行/回放),维护 `#dirty_effects` / `#maybe_dirty_effects` 队列。
- flush 时 `#process()` 从各 effect 根做**树序遍历**(`#traverse()`),按标志分类:**render effects(DOM 更新)先执行,user effects 后执行**——保证 `$effect` 看到的是更新后的 DOM。`$effect.pre` 则在 DOM 更新前。
- `flushSync()` 循环 `flush_tasks(); current_batch.flush()` 直到批清空,是同步逃生舱(测试与命令式代码用)。
- **async(实验性)**:含 `await` 的 derived/块会让 batch 进入 pending;`#defer_effects()` 暂存 effect,boundary 显示 pending 态;全部 settle 后 `#commit()` 一次性落地 DOM 增删,并把已提交值 rebase 到更早的未完成批次上。此机制 2026-07 仍在 `experimental.async` 下,Svelte 6 转正。

> **Rust 映射提示(关键差异点)**:原生侧没有 microtask。见 §4.5。async-Svelte 的 Batch 回放机制复杂度很高,**v1 不建议移植**,先实现同步 Batch(= Svelte 5.0–5.35 的行为)。

### 2.6 `$state` 的 Proxy 深层响应式与逃生舱

- `$state` 对 **数组与 plain object 递归代理**(Proxy),每个被读到的属性惰性生成内部 source;class 实例不代理,而是编译器把 class 字段上的 `$state` 改写成私有 source + getter/setter。
- `$state.raw`:不做深层代理,只能整体重赋值——大集合的性能出口。
- `$state.snapshot`:去代理化拷贝,交给外部库。
- 解构会失去响应性(普通 JS 求值语义)。
- 跨模块导出直接重赋值的 `$state` 不允许——编译器单文件工作,导入方无法被改写。

这一块是**整个 Svelte 里唯一深度依赖 JS 动态特性的部分**,也是 Rust 映射的主要"断点",见 §5。

### 2.7 `$derived` / `$effect` / `$props` 语义要点(文档核实)

- `$derived(expr)` / `$derived.by(fn)`:惰性、缓存、可在读取端 pull;**在未被任何 effect 观察时(unowned/disconnected)有专门处理**(断连的 derived 在 effect 内再次被读时 `reconnect()`)。Svelte 5 后期还允许对 derived **临时写入**(乐观 UI),下次依赖变化时恢复——v1 可不支持。
- `$effect`:mount 后运行;重跑批量化、DOM 更新后;teardown 在重跑前/销毁时;`$effect.pre`(DOM 前)、`$effect.tracking()`、`$effect.root()`。文档明确警告:读写同一状态会无限循环,应 `untrack` 或改用 `$derived`——Rust 侧同样需要这个运行时检测(dev 模式 `effect_update_depth` 上限)。
- `$props`:编译成对 `$$props` 对象的细粒度访问器;子组件读 props = 读父组件传入的表达式信号,天然细粒度。`$bindable` 提供双向绑定。

---

## 3. 编译产物:模板如何变成命令式更新代码

### 3.1 静态骨架:`from_html` + `cloneNode`

对每个连续静态 HTML 片段,编译器生成模块级模板工厂(参考 Tan Li Hau "Compile Svelte 5 in your head" 与源码 `dom/template.js`):

```js
var root = $.from_html(`<h1> </h1>`);           // 建 <template>,返回克隆工厂
export default function App($$anchor) {
  let name = $.state('world');
  var h1 = root();                               // cloneNode(true) 实例化
  var text = $.child(h1);                        // 按编译期算好的路径定位动态孔位
  $.template_effect(() => $.set_text(text, `Hello ${$.get(name)}`));
  $.append($$anchor, h1);
}
```

要点:
- **一次 parse,N 次 clone**:浏览器里 `cloneNode` 远快于逐个 `createElement`(Firefox 场景改用 `importNode`,PR #15272)。动态孔位在模板里留占位文本/注释,克隆后按**编译期确定的路径**(firstChild/nextSibling 序列)直取引用,零查询。
- **每个动态表达式 = 一个 `template_effect`**(本质是 render effect):`set_text`、`set_attribute`、`set_class` 等参数化 helper + effect 内闭包读 signal。更新粒度精确到单个文本节点/属性,**没有 diff**。
- **事件委托**:常见事件不逐节点 `addEventListener`,而是 `h1.__click = handler; $.delegate(['click'])`,根节点统一分发。
- **组件即 setup 函数,只执行一次**;`$.push()/$.pop()` 维护组件上下文栈(context、生命周期宿主)。组件边界在运行时几乎零成本——没有组件实例对象树。

### 3.2 控制流块:编译成"管理子 effect 树的 block effect"

- **`{#if}`**(源码 `dom/blocks/if.js`):编译成 `$.if(anchor, fn)`。内部是一个 `block(...)` effect,重跑时求条件,按分支 key 走 `update_branch()`:新分支用 `branch(() => …)` 创建 `BRANCH_EFFECT` 子树,旧分支 `pause_effect()`(播 outro,完了销毁)。DOM 用注释节点做 anchor 定位插入点。
- **`{#each}`**(源码 `dom/blocks/each.js`):每项对应 `EachItem { v, i, e }`——`v` 是**该项值的 source 信号**,`i` 是索引 source,`e` 是分支 effect。列表更新时:
  - 已存在的 key:**不重建 effect,直接 `internal_set(item.v, value)` 原地更新项信号**,项内部的细粒度 effect 自行响应——这是"列表 diff 只 diff 键,内容更新走信号"的关键设计;
  - 重排:`reconcile()` 用 keyed map + `seen` 集合探测乱序,比较"把后面的往前挪"和"把前面的往后挪"哪个操作少(非 LIS,工程化的启发式),`move()` 搬移 effect 拥有的 DOM 区间;
  - 删除:`pause_effects()` 走退场过渡后销毁。
  - `EACH_ITEM_REACTIVE / EACH_INDEX_REACTIVE` 等标志由编译器按模板是否用到 `item`/`index` 静态决定——**用不到就不建信号**,零成本抽象。
- `{#key}`、`{#await}`、组件、snippet 同理:都是"一个 block effect + 若干 branch effect"的模式。

### 3.3 哪些思想能搬到 retained-mode 原生场景树

| Svelte/DOM 概念 | 原生场景树对应物 | 判断 |
|---|---|---|
| `<template>` + `cloneNode` | **预编译的静态子树构造函数**(直线代码 `let n1 = Text::new(); n1.append(...)`)。DOM 里 clone 快是因为绕过 JS↔DOM 边界;Rust 原生**没有这个边界**,直线构造代码就是最优解,无需真的实现"克隆"。例外:**HarmonyOS ArkUI NDK 的 C-API 是 FFI 边界**,批量建树值得做成一次跨界调用(节点描述数组一次提交) | 思想保留,机制换掉 |
| `$.child/$.sibling` 路径定位 | 构造函数直接返回孔位句柄(编译期已知,连路径都不用走) | 更简单 |
| 注释节点 anchor | 场景树节点的**稳定子节点插槽/区间**(block 拥有 `[start, end)` 子区间句柄) | 必须设计,场景树 API 要原生支持"区间插入/搬移/摘除" |
| `template_effect` + `set_text/set_attribute` | render effect + **属性 setter 直调**(`node.set_text(..)`,retained-mode 场景树打脏矩形/重排标记) | 原样保留,粒度一致 |
| 事件委托 `__click` | 原生事件本来就要自己做 hit-test + 冒泡,委托与否是事件系统内部实现细节 | 不需要模仿,自然获得 |
| `{#if}`/`{#each}` block effect + pause/resume | 完全同构:分支挂/摘子树,INERT + 退场动画 | 原样移植,动画系统按 INERT 语义设计 |
| hydration(SSR 注水) | 桌面端无此需求 | 删除,省掉源码里约 1/3 的分支复杂度 |
| 组件 = setup 函数一次执行 | 同构,组件无实例、无 rerender | 原样移植 |

---

## 4. Rust 侧映射设计

### 4.1 所有权问题与两条已验证路线

响应式图天然是**多所有者、循环引用**(source↔reaction 互指)结构,与借用检查直接冲突。生态收敛出的解法:

1. **Arena + `Copy` 句柄(推荐主路线)** —— Sycamore 0.9(单例 `Root` + slotmap,信号全 `Copy + 'static`,砍掉了 0.8 的生命周期参数与 `cx` 传递)与 leptos 经典模式("交出值的所有权给响应式系统,换回一个 `Copy + 'static` 的位置标识",见 leptos ARCHITECTURE.md)。节点存 slotmap/世代 arena,句柄 = 世代化索引,闭包随便 move,不需要 `clone()` 仪式。悬垂句柄在运行时被世代号拦截(报错或静默失败,策略见 §7)。
2. **Arc 双轨(leptos 0.7+ `reactive_graph` 0.2.x)**:`ArcRwSignal`(引用计数、Send+Sync、跨 async 边界)+ arena 包装的 `Copy` 版 `RwSignal`(内部仍是 Arc,注册到 Owner 上以便统一 dispose)。换来了跨线程能力,代价是每节点原子计数 + 锁,以及 API 双份。

**判断**:桌面 UI 库的响应式图只活在 UI 线程,Svelte 本身也是单线程模型。选 **thread-local runtime + 世代 arena + `Copy` 句柄**,不给核心类型加 `Send/Sync`,内部用 `Cell/RefCell` 免锁。后台线程通过消息(`post_to_ui(move |cx| sig.set(..))`,类似 winit `EventLoopProxy`)回 UI 线程写状态——这比"图本身线程安全"简单一个数量级,floem 已验证该形态在原生桌面成立。HarmonyOS 侧 ArkUI 也要求 UI 操作在 UI 线程,模型一致。

### 4.2 核心类型与 API 草案

```rust
// 句柄:8–16 字节,Copy,零生命周期参数
pub struct Signal<T> { id: NodeId, _ty: PhantomData<fn() -> T> }
pub struct Memo<T>   { id: NodeId, _ty: PhantomData<fn() -> T> }  // = $derived
pub struct Effect    { id: EffectId }                              // = $effect(.root)

impl<T: 'static> Signal<T> {
    pub fn get(&self) -> T where T: Clone;        // track + clone(小类型主 API)
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R;  // track,免 clone
    pub fn set(&self, value: T);                   // = 赋值改写后的 internal_set
    pub fn update(&self, f: impl FnOnce(&mut T));  // 原地改 + 标记
    pub fn get_untracked(&self) -> T where T: Clone;
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R;
}
pub fn untrack<R>(f: impl FnOnce() -> R) -> R;
```

设计取舍(对照 Svelte 语义逐条):
- **`get`/`with` 双 API** 是 Rust 特有的必要妥协(JS 值语义 vs Rust 移动语义);`get` 要求 `Clone` 覆盖 90% 场景,大对象用 `with`。leptos/Sycamore 均如此。
- **equality 短路**:`set` 内部若 `T: PartialEq` 则值等短路(对应 Svelte `equals`)。Rust 没有特化,方案:`Signal::new` 走 `PartialEq` 约束版构造器、另提供 `new_always`(= `safe_equals` 语义,总是通知);或用 `impl` 分离的 builder。**必须在 API 层露出**,因为 memo 的 `wv` 短路依赖它(§2.3)。
- **`update` 与借用**:`update` 执行期间该信号处于独占借用,若用户在闭包里再读同一信号会 `RefCell` panic——比 JS 的静默怪异行为更好,dev 报错信息要友好。
- **Memo**:惰性 + `UNINITIALIZED` 哨兵 + `wv` 比较,照抄 §2.3。注意 Svelte 的 unowned/disconnected derived 处理(无人观察的 derived 断连、再读时 reconnect)v1 可简化为"memo 常连接",代价是无人观察的 memo 依赖变化时多一次标记,可后续优化。
- **`$props` 映射**:组件宏生成 props struct;"细粒度 props"用 `impl Into<MaybeSignal<T>>`(静态值或信号皆可传),对应 Svelte 里"props 就是读父组件表达式"。

### 4.3 Owner/Scope 树与嵌套 dispose

直接移植 Svelte effect 树(§2.4),它同时充当 leptos 的 Owner:

- arena 中 `EffectNode { flags, parent, first, last, prev, next, deps, teardown: Option<Box<dyn FnOnce()>>, node_range: Option<(SceneNodeId, SceneNodeId)>, cx }`;
- `create_effect` 挂到 `current_owner`;组件、if 分支、each 项即 BRANCH/BLOCK 节点;
- **重跑前递归 dispose 子树**(先子后父跑 teardown,摘反向边,释放 arena 槽,世代号 +1)——严格对应 `destroy_effect_children`;
- context(`getContext/setContext`)挂在 BRANCH 层的 `cx`,查找沿 parent 链;
- `INERT` pause/resume 原样保留,退场动画依赖它;
- 提供 `create_root(|dispose| …)`(= `$effect.root`)作为顶层入口与测试工具。

### 4.4 编译器/宏层:模板 → 场景树构造 + 细粒度更新

形态建议(v1 → v2 演进):
1. **v1:proc-macro DSL**(`view! {}` 风格但语义按 Svelte 块设计:`if/each/key` 块、事件、双向绑定糖)。宏展开产物与 §3.1 的形状一致:直线构造静态子树 → 孔位句柄 → 若干 `render_effect` 闭包 → 返回 `(node_range, ())`。控制流展开成 `if_block(anchor, |cx| cond.get(), |cx| {...}, Some(|cx| {...}))`、`each_keyed(anchor, list, |item| key, |cx, item: Signal<Item>, i| {...})`——运行时块函数逐一移植 Svelte `dom/blocks/*`。
2. **v2(可选):外部单文件格式**(`.svelte` 式模板 + `<script>` 内 Rust)走 build.rs/编译器插件。表达力更强(真正的赋值改写、编译期依赖分析),但 IDE/rust-analyzer 支持成本极高。**先不做**,等 v1 验证运行时后再评估。

each 块务必保留 Svelte 的关键设计:**每项一个 `Signal<Item>`,reconcile 只处理 key 的增删移,内容变化走 `internal_set(item_sig, new_value)`**(要求 `Item: PartialEq` 或用户提供 diff 粒度),这让"列表更新"与"项内细粒度更新"解耦——这是它比 VDOM 列表 diff 快的本质。

### 4.5 调度:把 microtask 换成帧管线(最大的开放设计点)

Svelte 的顺序契约:同步代码内任意多次写 → 一个 Batch → microtask flush(render effects → DOM → user effects)。原生侧建议:

- **写入只标记 + 入队**,并向事件循环 `request_flush()`(winit:`EventLoopProxy::send_event` / `Window::request_redraw`;ArkUI:VSync 回调/`postTask`);
- **每帧管线**:输入事件处理(可能多次 set)→ flush:`$effect.pre` → render effects(改场景树,打脏)→ layout → paint → user effects;
- 提供 `flush_sync()`(= `flushSync`)给测试与命令式读取"更新后状态"的场景;
- effect 内写 state 触发的级联,循环 flush 直至 settle(带 dev 深度上限,对应 Svelte 的无限循环检测);
- **不移植 async-Svelte 的 Batch 回放/`fork`**(§2.5),v1 的异步靠"后台任务完成后回 UI 线程 set signal" + 手写 loading 态,足够覆盖桌面场景。

### 4.6 深层响应式的替代:`#[derive(Store)]`

无 Proxy,则"深层可变状态"改为**编译期展开字段级信号**(leptos `Store`、Sycamore deep reactivity 同思路):

```rust
#[derive(Store)]
struct Todo { done: bool, text: String }
// 生成 TodoStore { done: Signal<bool>, text: Signal<String> } + lens 访问
todos.field(i).done.set(true);   // 只有读过 done 的 effect 重跑
```

配合内建响应式集合类型(对应 `svelte/reactivity` 的 `SvelteMap/SvelteSet`):`RwVec<T>`/`RwMap<K,V>`,内部按 key/长度维护细粒度信号。

---

## 5. Rust 里做不了 / 不值得做的 Svelte 特性与替代方案

| Svelte 特性 | 为什么 Rust 不行/不值 | 替代方案 |
|---|---|---|
| **赋值语法隐式响应**(`count += 1` 触发更新;编译器把赋值改写为 `$.set`) | proc-macro 理论上也能改写宏体内的赋值,但:改写范围只能覆盖宏内(跨函数就失效,和 Svelte "跨模块导出 $state 不允许" 同源的问题,而 Rust 函数拆分远比 JS 频繁);且掩盖 `Copy` 句柄的真实语义,调试反直觉 | 显式 `sig.set()/update()`;模板宏内部可对 `bind:` 等有限位置做糖 |
| **Proxy 深层响应式**(`obj.a.b.c = 1` 精确触发) | 无运行时代理机制;`Deref` 魔法脆弱且骗不过借用检查 | `#[derive(Store)]` 字段级信号(§4.6);`$state.raw` 的对应物反而是 Rust 默认行为(整体 set) |
| **类字段 `$state` 的 getter/setter 改写** | 同上,需要改写所有访问点 | Store 派生宏统一解决 |
| **无类型/动态 props、`{...rest}` 展开、动态组件 `<svelte:component>`** | 静态类型下 props 是 struct;动态组件需 trait object | `Box<dyn Component>` / enum 分发;rest-props 用 builder + `..Default` |
| **async-Svelte 的 Batch fork/回放、boundary pending 树** | 复杂度极高,浏览器语义(microtask/Promise)绑定深,实验性未稳 | v1 不做;异步 = 后台任务 + 回 UI 线程写信号 |
| **SSR/hydration 双输出** | 桌面无此需求 | 直接删除,简化所有块实现 |
| **`$inspect`、HMR、devtools 深度集成** | 可做但依赖工具链投入 | v1 提供 dev 模式日志钩子(信号命名 + 变更 trace),HMR 后置 |
| **`bind:` 任意深路径双向绑定** | 需要路径改写 | 限定为 `Signal<T>` 或 Store lens 的双向绑定 |

值得强调:**放弃隐式赋值 ≠ 放弃 Svelte 心智模型**。Svelte 5 runes 本身已经把响应式显式化($state/$derived/$effect 是显式声明,只有"写入"还是赋值语法);Rust 版把最后一步也显式化,与 runes 的设计方向一致。

---

## 6. 对本项目的落地建议(有先后序)

1. **第一步只做 `reactive` crate**:Source/Memo/Effect 三节点 + 三态脏标记 + `wv/rv` 版本号 + effect 树 + Batch/flush_sync,逐函数对照 Svelte `sources.js / deriveds.js / effects.js / runtime.js / batch.js` 移植,配合 Svelte 仓库的 signal 测试用例(`signals/test.ts` 有大量边界 case)做对照测试。
2. **场景树 API 按"块运行时"的需求反向设计**:节点区间(range)插入/搬移/摘除、稳定插槽、INERT 子树。先出 trait(`SceneTree`),Windows/Linux/macOS 用自绘(wgpu/vello 或 skia)一套实现,HarmonyOS 走 ArkUI NDK 节点 API 一套实现,块运行时(if/each/key)只依赖 trait。
3. **宏 DSL 后行**:先手写"编译产物形状"的代码跑通 TodoMVC 级 demo,确认运行时 API 形状,再写 proc-macro 固化它(Svelte 团队同样是先定 runtime helper 再定编译产物)。
4. **调度原型优先验证**(见 §7):在 winit 上先跑通"事件 → 批 → 帧前 flush → 渲染"闭环,再抽象成可嫁接 ArkUI VSync 的 `Scheduler` trait。
5. **明确非目标**:SSR/hydration、async-Svelte、HMR、Proxy 式深响应,v1 全部不做。

---

## 7. 未决问题(需原型验证)

1. **悬垂句柄策略**:世代 arena 拦截 use-after-dispose 后,`get()` 返回什么?panic(fail-fast,leptos 老版本被诟病)/ `Option`(污染 API)/ 静默返回缓存值 + dev 警告(leptos 新版 unwrap-or-warn 路线)。需在真实组件代码中体感验证。
2. **帧调度细节**:输入事件处理中多次 `flush_sync`(如受控输入框需要同步读回)与每帧一次 flush 的交互;effect 级联的 settle 上限;与 ArkUI VSync/`postTask` 的实际嫁接方式(NDK 文档层面未完全核实,需要写 demo)。
3. **`with` vs `get` 的人体工学**:模板宏展开代码里大量读取,闭包借用(`RefCell` 借用叠加,如 effect 内读 A 时 A 的 update 正在进行)是否会在真实 UI 模式下频繁踩 panic,需要 demo 验证并可能引入读写分离的内部借用策略。
4. **`#[derive(Store)]` 的粒度与嵌套集合**:`Vec<Struct>` 的 lens 设计(索引稳定性 vs key 稳定性)、增量 diff 的 `PartialEq` 成本,需要在 each 块原型里实测。
5. **HarmonyOS ArkUI NDK 能力边界**:C-API 自定义节点的属性集、测量/布局回调、事件注入是否足以承载自绘混合方案,还是需要"ArkUI 原生控件树"整套映射(两条路成本差异巨大)。本报告未做深入核实,应作为独立调研题(建议 02 号报告)。
6. **each reconcile 算法选型**:Svelte 的 seen-set 启发式 vs 传统 LIS(Solid/ivi 派)在场景树搬移成本模型下(原生搬移可能比 DOM 便宜/昂贵)孰优,可后置基准测试。

---

## 8. 来源

### Svelte 官方(文档/博客/PR)
- Svelte Docs: [$state](https://svelte.dev/docs/svelte/$state) / [$derived](https://svelte.dev/docs/svelte/$derived) / [$effect](https://svelte.dev/docs/svelte/$effect) / [await expressions](https://svelte.dev/docs/svelte/await-expressions) / [v5 migration guide](https://svelte.dev/docs/svelte/v5-migration-guide) / [svelte/reactivity](https://svelte.dev/docs/svelte/svelte-reactivity)
- [What's new in Svelte: May 2026](https://svelte.dev/blog/whats-new-in-svelte-may-2026)(版本号核实)
- [Releases · sveltejs/svelte](https://github.com/sveltejs/svelte/releases) / [CHANGELOG](https://github.com/sveltejs/svelte/blob/main/packages/svelte/CHANGELOG.md)
- PR [#15844 feat: allow `await` in components](https://github.com/sveltejs/svelte/pull/15844) · PR [#15348 chore: simplify flushing](https://github.com/sveltejs/svelte/pull/15348) · PR [#15272 use importNode for Firefox](https://github.com/sveltejs/svelte/pull/15272) · Discussion [#15845 Asynchronous Svelte](https://github.com/sveltejs/svelte/discussions/15845)

### Svelte 内部源码(main 分支,2026-07 抓取)
- [internal/client/constants.js](https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/internal/client/constants.js)(flags)
- [reactivity/sources.js](https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/internal/client/reactivity/sources.js) · [reactivity/deriveds.js](https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/internal/client/reactivity/deriveds.js) · [reactivity/effects.js](https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/internal/client/reactivity/effects.js) · [reactivity/batch.js](https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/internal/client/reactivity/batch.js) · [runtime.js](https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/internal/client/runtime.js)
- [dom/blocks/if.js](https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/internal/client/dom/blocks/if.js) · [dom/blocks/each.js](https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/internal/client/dom/blocks/each.js)

### 解析文章
- Tan Li Hau, [Compile Svelte 5 in your head](https://lihautan.com/compile-svelte-5-in-your-head)(编译产物)
- [Svelte 5 signals fix its glitchy and inconsistent reactivity](https://www.webdevladder.net/blog/svelte-5-signals-fix-its-glitchy-and-inconsistent-reactivity)
- Willy Brauner, [Signals, the push-pull based algorithm](https://willybrauner.com/journal/signal-the-push-pull-based-algorithm)

### Rust 生态
- leptos: [ARCHITECTURE.md](https://github.com/leptos-rs/leptos/blob/main/ARCHITECTURE.md) · [docs.rs/leptos 0.8.19](https://docs.rs/crate/leptos/latest) · [reactive_graph 0.2.14](https://docs.rs/reactive_graph/latest/reactive_graph/) · [Discussion #2552 Non-'static signals](https://github.com/leptos-rs/leptos/discussions/2552)
- Sycamore: [Announcing v0.9.0](https://sycamore.dev/post/announcing-v0-9-0) · [PR #612 Reactivity v3](https://github.com/sycamore-rs/sycamore/pull/612) · [crates.io(0.9.2)](https://crates.io/crates/sycamore)
- floem: [lapce/floem](https://github.com/lapce/floem)(原生桌面 + 细粒度响应式参照)

### 未联网核实、仅基于训练数据的内容
- §4.5 中 ArkUI VSync/`postTask` 的嫁接方式与 §7.5 ArkUI NDK 能力边界(需专项调研)
- Solid 2.0 / alien-signals 与 Svelte 算法同族的横向对比表述(方向性结论,非关键依据)
- leptos 旧版 panic 行为与新版 unwrap-or-warn 策略的细节演进(§7.1)
