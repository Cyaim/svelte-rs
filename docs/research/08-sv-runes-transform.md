# 08 · runes 源码变换的语义设计:在 .sv 的 Rust script 块里实现 Svelte 5 式反应性

> 调研日期:2026-07-17。核实方式:① 联网核实 Svelte 5 编译器实际行为(官方文档、compiler 源码 `AssignmentExpression.js`、编译输出讲解文章、5.25 writable derived 发布信息、官方 compiler-errors 清单);② **本地实验**(`scratchpad/dollar-exp`,proc-macro2 1.0.106 + syn 2.0.119)验证 `$` token 管线的每一个关键断言。未能联网核实、仅凭训练数据的点在文末单独标注。
>
> 前置语境:本报告服务于"独立编译器路线"(.sv 单文件组件,script 块是 Rust + runes,由我们自己的编译器变换成对 sv-reactive / sv-ui 的显式调用)。sv-reactive 现有 API:`state(v) -> Signal<T>`(`Copy + !Send` 句柄)、`.get()/.set()/.update()/.with()`、`derived(f)`、`effect(f)`、`batch/untrack/create_root`。

---

## 0. 结论速览(TL;DR)

1. **可行,且不是黑魔法**。Svelte 5 本身就是"**声明处显式(`$state`)、使用处隐式(裸 `count`)**"的源变换:编译器把读改写成 `$.get(count)`、写改写成 `$.set(count, ...)`(联网核实,见 §1)。我们做的是同一件事,只是宿主语言从 JS 换成 Rust。变换规则表可以**闭合定义**(§2),所有覆盖不到的模式都能落在**编译期拒绝清单**上(§3.6)——失败模式是编译错误而非运行时错误,方向安全。
2. **最重要的取舍:v0 采用"变量粒度"响应性,不追 Svelte 的 Proxy 深响应**。`$state` 可以包任意 `T`;读是快照(`.get()`,要求 `Clone`),字段赋值/方法调用等可变操作统一改写成 `.update(闭包)`(RHS 必须预求值,见 §2.3 的坑)。依赖追踪粒度是整个变量——比 Svelte 粗,但语义 100% 健全,over-notify 由 derived 的相等性剪枝兜底。字段级订阅(`#[derive(Store)]` 投影)是 v1 增量,不阻塞路线。
3. **shadowing 必须编译期拒绝(v0)**。这一条把作用域分析从"完整 Rust name resolution"降维成"标识符集合匹配 + 遮蔽检测即报错",是整个变换器可以在几周内做对的关键前提。Svelte 用完整 Scope 分析支持遮蔽,我们不需要陪跑。
4. **线程逃逸不需要新机制**:句柄已 `!Send`(`PhantomData<*const ()>`),`thread::spawn(move || count += 1)` 改写后自动撞 rustc E0277。成本在错误信息落点——用**行对齐生成**(改写只动列不动行)+ 编译器附注缓解(§4.5)。
5. **最深的坑在宏调用内部**:`println!("{count}")` 的隐式捕获藏在字符串字面量 token 里,token 级替换够不着,漏改的后果是把句柄当值用、错误信息莫名其妙。v0 方案:**白名单 std fmt 宏做捕获脱糖,其余宏内出现 rune 标识符一律硬错误**(建议用户先绑快照)。本地实验还证实 `macro_rules!` 体内的 `$state` 会被预处理误伤 → v0 直接禁止 script 内 `macro_rules!`(§4.3)。
6. **实施管线已实验验证**:`proc_macro2::TokenStream::from_str` 接受含 `$` 的输入(`$` 是合法 Punct);字符串/注释里的 `$state` 在词法层就与代码隔离,天然免疫;`$` 与 ident 的邻接可用 span 起止位置精确判定;token 级替换 `$state → __sv_state` 后 syn 全功能解析成功。**不需要任何文本正则预处理**(§4.1)。
7. **DX 判断:值得做,但要签"社会契约"**。Svelte 社区论战的真实教训不是"隐式不好",而是"**隐式的边界必须清晰**"——Svelte 4 的 `let count = 0` 隐式因为边界模糊(只在 .svelte 顶层生效)被 Rich Harris 亲手废掉,runes 把隐式收缩到"显式声明的变量"。我们的边界比 Svelte 更清晰:**文件扩展名就是契约**(.sv 里是 sv 方言,.rs 里是显式 API)。对冲措施:`sv expand` 一键查看生成 Rust、`$sig(x)` 逃生舱直通显式 API、变换器做成独立库(proc-macro 路线想复用也能用)。若 DX 反馈差,退回"显式 script + 只变换模板"的成本很低(§6)。

---

## 1. 对照基准:Svelte 5 编译器对 `$state` 的实际处理(联网核实)

### 1.1 编译输出形态

对这个组件(来源:[Compile Svelte 5 in your head](https://lihautan.com/compile-svelte-5-in-your-head)):

```svelte
<script>
  let count = $state(0);
  function increment() { count += 1; }
</script>
<button onclick={increment}>Clicked {count}</button>
```

Svelte 5 客户端编译输出的骨架(`$` 是 `svelte/internal/client` 的导入别名):

- 声明:`let count = $.state(0)` —— 创建 signal 对象;
- 读:一律改写为 `$.get(count)`(模板、事件处理器、闭包内皆同);
- 写:`count = x` → `$.set(count, x)`;`count += 1` 经 `build_assignment_value` 脱糖后同样落到 `$.set`(核实自编译器源码 [`AssignmentExpression.js`](https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/compiler/phases/3-transform/client/visitors/AssignmentExpression.js):`=`/`||=`/`&&=`/`??=` 之外的复合算符按"先取值再运算"处理;对象成员赋值在特定上下文走 `$.assign(object, property, operator, ...)`);
- 模板 `{count}` → `$.template_effect(() => $.set_text(text, $.get(count)))`;
- 类字段 `#done = $.state(false)` + 编译器生成的 `get done() { return $.get(this.#done) }` / `set done(v) { $.set(this.#done, v, true) }`(类不走 Proxy,走 getter/setter 变换,核实自 [$state 文档](https://svelte.dev/docs/svelte/$state))。

### 1.2 对我们最重要的五条语义事实

| 事实 | 出处 | 对 sv 的意义 |
|---|---|---|
| 对象/数组走**深层 Proxy**,`array.push(...)` 等方法内的读写都被拦截,粒度到字段 | [$state 文档](https://svelte.dev/docs/svelte/$state) | 这是我们**唯一无法照搬**的部分,Rust 没有 Proxy。§3.4 给替代方案 |
| **传函数是快照**:"When you call a function, the arguments are the *values* rather than the *variables*"——文档专门有 "Passing state into functions" 一节解释 JS 传值语义,响应性不跨函数边界(除非传 getter 或对象) | 同上 | 我们的 `f(count)` → `f(count.get())` 快照语义**与 Svelte 原语义一致**,不是妥协(§3.3) |
| **解构失去响应性**:"If you destructure a reactive value, the references are not reactive... they are evaluated at the point of destructuring" | 同上 | 我们对解构读同样给快照;`let (a,b) = $state(..)` 这种声明位解构直接拒绝(Svelte 对 `$props` 也强制特定 pattern) |
| **重赋值的 state 不可导出**(`state_invalid_export`:"Cannot export state from a module if it is reassigned...") | [compiler-errors](https://svelte.dev/docs/svelte/compiler-errors) | 佐证"编译器只能改写本文件内的引用"这一根本约束,我们同样只在单个 .sv 内闭合 |
| `$derived` 自 **5.25 起可写**(临时覆盖,依赖变化后回退;用于 optimistic UI) | [HN 讨论](https://news.ycombinator.com/item?id=43444999)、[svelte.dev/docs/svelte/$derived](https://svelte.dev/docs/svelte/$derived) | v0 仍拒绝写 derived(实现简单、语义保守);writable derived 列入观察项,不承诺 |

### 1.3 Svelte 的 rune 错误码清单(我们的拒绝清单直接对标)

核实自 [compiler-errors](https://svelte.dev/docs/svelte/compiler-errors),对设计 §3.6 直接有用的:

- `state_invalid_placement`:"`%rune%(...)` can only be used as a variable declaration initializer, a class field declaration, ..."
- `effect_invalid_placement`:"`$effect()` can only be used as an expression statement"
- `props_invalid_placement` / `props_invalid_identifier`("`$props()` can only be used with an object destructuring pattern")/ `props_duplicate`
- `bindable_invalid_location`:"`$bindable()` can only be used inside a `$props()` declaration"
- `rune_missing_parentheses` / `rune_invalid_arguments`

**要点**:Svelte 对 rune 的"位置合法性"检查非常严格——rune 不是表达式,是**只在特定语法位置合法的编译器指令**。我们照抄这个哲学:`$state` 只在 `let` 初始化器位置合法,`foo($state(0))` 直接报错,杜绝"rune 当函数传来传去"的整类歧义。

---

## 2. 变换规则表

约定:`count`、`user`、`xs` 是 `$state` 绑定;`e'` 表示"对 e 递归应用本规则表后的结果";生成代码通过 `use ::sv_reactive as __sv_rt;` 引用运行时。

### 2.1 声明位

| .sv script 源码 | 生成 Rust | 说明 |
|---|---|---|
| `let count = $state(0);` | `let count = __sv_rt::state(0);` | `count: Signal<i32>`,`Copy + !Send`。**只接受 `let` 初始化器位置**(对标 `state_invalid_placement`) |
| `let mut count = $state(0);` | 同上(丢弃 `mut`)+ **警告** | 句柄不需要 mut;保留会误导 |
| `let count: i32 = $state(0);` | `let count: __sv_rt::Signal<i32> = ...` | 用户写的类型标注是**值类型**,编译器搬进 `Signal<_>`。这是"读者看到的类型 = 值类型"契约的一部分 |
| `let doubled = $derived(count * 2);` | `let doubled = __sv_rt::derived(move \|\| count.get() * 2);` | 表达式整体 move 闭包化,内部递归改写(§2.4) |
| `$effect(\|\| { ... });` | `__sv_rt::effect(move \|\| { ...' });` | 只在**表达式语句位**合法(对标 `effect_invalid_placement`);参数必须是闭包字面量,自动补 `move` |
| `let (a, b) = $state((0, 1));` | **拒绝** sv0008 | 声明位解构不允许(Svelte 解构即失去响应性;我们直接不给这个坑) |
| `foo($state(0))` / `return $state(0)` | **拒绝** sv0001 | rune 不是表达式 |

### 2.2 读位置(裸 `count`)

裸标识符改写的前提:该标识符是**单段路径**(`syn::Expr::Path` 且无 qualifier),且在当前作用域解析到 rune 绑定(由于禁止 shadowing,这退化为集合成员判断,§3.1)。

| 读形态 | 生成 Rust | 说明 |
|---|---|---|
| 表达式里的裸 `count` | `count.get()` | 快照,要求 `T: Clone`。建立依赖(在 effect/derived 内)|
| `user.name`(字段读) | `user.with(\|__v\| __v.name.clone())` | 只要求**字段**类型 `Clone`,不 clone 整个结构体。依赖粒度仍是整个 `user`(v0 无字段级订阅) |
| `xs.len()`、`xs.iter().count()` 等**表达式位**方法调用 | `xs.with(\|__v\| __v.len())` | 读语义(tracked)。返回值若借用 `__v` 会被 borrowck 挡住(闭包返回值不能引用参数)——这是**特性**:阻止借用逃逸 |
| `&count` | `&count.get()` | 快照的引用;临时生命周期由 Rust 常规规则管 |
| `f(count)` | `f(count.get())` | 快照传参,与 Svelte 语义一致(§3.3) |
| `match count { ... }` | `match count.get() { ... }` | scrutinee 快照进 match,**arm 内允许写 `count` 自身**(状态机常见);scrutinee 临时存活整个 match,借用模式也安全 |
| `Point { count }`(结构体简写) | `Point { count: count.get() }` | 简写必须展开(实验证实 syn 的 `FieldValue.colon_token == None` 可精确识别) |
| 模板 `{count}` / `{user.name}` | `bind_text(doc, id, move \|\| format!("{}", count.get()))` 等 | **模板与 script 共用同一张读规则表、同一个改写器**——这是选择"script 也变换"的核心一致性论据 |
| `$untrack(expr)` | `__sv_rt::untrack(\|\| expr')` | 显式脱追踪 |
| `$sig(count)` | `count`(句柄本体) | **逃生舱**:拿到 `Signal<T>` 后显式 API 全开放(`.update/.with/.get_untracked/...`)。不特判任何方法名,正交且无歧义 |

### 2.3 写位置

**核心坑(必须先讲)**:sv-reactive 的 `.update()` 在执行闭包期间会把值 take 走,闭包内再读同一 signal 会重入 panic。所以**一切进入 update 闭包的改写,RHS / 参数 / 索引必须先在闭包外求值**。`count += count` 若朴素改写成 `count.update(|v| *v += count.get())` 就是运行时炸弹;正确形态是先绑 `__rhs`。

| 写形态 | 生成 Rust | 说明 |
|---|---|---|
| `count = e;` | `count.set(e');` | e' 先于 set 求值,e' 里读 `count` 自身也安全 |
| `count += e;`(及 `-= *= /= %= &= \|= ^= <<= >>=`) | `{ let __rhs = e'; count.update(\|__v\| *__v += __rhs); }` | **RHS 预求值**,规避重入 |
| `user.name = e;` | `{ let __rhs = e'; user.update(\|__v\| __v.name = __rhs); }` | 变量粒度通知 |
| `xs[i] = e;` / `xs[i].done = true;` | `{ let __i = i'; let __rhs = e'; xs.update(\|__v\| __v[__i] = __rhs); }` | 索引也预求值(i 可能读别的 rune) |
| `xs.push(e);`(**语句位**方法调用) | `{ let __a0 = e'; xs.update(\|__v\| { __v.push(__a0); }); }` | 启发式:语句位 = 写意图(§3.4 论证) |
| `let n = xs.pop();`(**表达式位**可变方法) | 改写为 `with` → 闭包内 `&T` 调 `&mut self` 方法 → **rustc E0596 编译错误** + 编译器附注:"改用 `$sig(xs).update(...)` 或 `$mut`" | 启发式失败的方向是**编译错误**,不是静默错语义 |
| `count = e` 其中 count 是 `$derived` | **拒绝** sv0005 | v0 不做 writable derived |
| `&mut count` | **拒绝** sv0007 | 无法健全地交出内部值的 `&mut`(值在 arena 里) |

运行时配套:事件回调由模板 codegen 自动包 `__sv_rt::batch(|| ...)`,循环写(`while count.get() < 10 { count += 1 }` 每次迭代 notify)不至于每写一次 flush 一轮 effect;组件 init 整体跑在 `create_root` + batch 内。

### 2.4 `$derived(expr)` 的自动闭包化

`$derived(count * price + tax)` → `derived(move || count.get() * price.get() + tax)`:

- 闭包**必须 `move`**:rune 句柄是 `Copy`,move 零成本;非 rune 的本地变量(如 `tax: f64` 若非 Copy)会被夺走所有权——后续使用处 rustc 报 use-after-move,落点在用户代码,可读。v0 不自动插 clone(魔法过头),文档给出 `let tax = tax.clone();` 惯用法。
- 闭包体内递归应用读规则表;**写任何 rune 在 derived 表达式内**:直接赋值静态可检出 → sv0006;经函数调用间接写 → 运行时已有 `state_unsafe_mutation` panic 兜底。
- `$derived.by(闭包)` 形态(Svelte 同名):用户想写多语句就用它,`$derived` 本体只收表达式。

### 2.5 与 Svelte 5 的语义差异总表

| 场景 | Svelte 5 | sv v0 | 评估 |
|---|---|---|---|
| `user.name = x` 深改 | Proxy 拦截,字段级更新 | `update` 整变量通知 | 语义等价(effect 重跑结果一致),性能粗一档;derived 相等剪枝 + 未来 `#[derive(Store)]` 收窄 |
| 读 `user.name` 的依赖粒度 | 字段级 | 变量级 | 同上 |
| `f(count)` | 快照(原始值) | 快照 | **一致** |
| `f(user)`(对象) | Proxy 引用同行,深改仍响应 | 快照(clone) | **不一致**,是差异最大点;文档明示 + 需要共享改用 `$sig(user)` 传句柄 |
| 解构 | 失去响应(文档明示) | 读位解构 = 快照;声明位解构 rune 拒绝 | 更严格 |
| writable `$derived` | 5.25+ 支持 | 拒绝 | 观察项 |
| `count++` | `$.update(count)` | 不存在(Rust 无 `++`) | — |

---

## 3. 健全性深挖

### 3.1 shadowing:v0 一律编译期拒绝

问题全景:`let count = $state(0); let count = count + 1;`(重绑定)、`match x { Some(count) => ... }`(模式绑定)、`|count| ...`(闭包参数)、`for count in ..`。任何一处引入同名绑定后,后续裸 `count` 到底指 rune 还是新绑定,需要完整的词法作用域分析才能回答——Svelte 就是靠其 Scope pass 做对的。

**判断:v0 不值得陪跑完整 name resolution。** 规则:script 块内(含所有嵌套块、闭包、match arm、for/if-let pattern),**引入与任何 rune 绑定同名的新绑定 = 编译错误 sv0002**。理由:

1. 遮蔽 rune 从来不是需求,是事故:用户十有八九是想"取快照",给他的正确拼法是 `let snapshot = count;`(读位改写后就是快照)——换个名字,意图更清楚。
2. 拒绝遮蔽后,"该不该改写这个 ident"从作用域问题退化成 `HashSet<String>` 查询 + 模式遍历检查,visit_mut 实现量降一个数量级,且**不可能改写错**(没有歧义存在的空间)。
3. 错误信息可以做得极好:"`count` 已是响应式绑定,不能在内层重新绑定;若想取当前值快照,写 `let count_now = count;`"。

代价:合法但被误伤的代码(内层纯局部变量恰好同名)需要改名。可接受;v1 若做了 Scope 分析可放开。

### 3.2 闭包捕获与线程逃逸

**捕获规则:改写不动用户闭包的捕获方式。** 裸读改写成 `count.get()` 后,非 move 闭包按引用捕获 `count` 句柄(`&Signal<T>`),需要 `'static` 的场合(事件回调存进场景树)由用户自己写 `move`——与今天手写显式 API 完全一致;**模板 codegen 自己生成的闭包**(bind_text、事件包装)一律 `move`。不自动给用户闭包加 `move`:句柄虽 Copy 无所谓,但闭包里若还捕获了非 Copy 的普通变量,强插 `move` 会改变它们的语义——这是"变换必须保持非 rune 代码语义不变"红线。

**线程逃逸:现有 `!Send` 已经够。** `Signal<T>` 带 `PhantomData<*const ()>`,`std::thread::spawn(move || count += 1)` 改写后闭包捕获句柄 → 不满足 `F: Send` → rustc E0277"`Signal<i32>` cannot be sent between threads safely"。不需要任何编译器特判(也不该做:syntactic 检测 `thread::spawn` 挡不住 rayon/tokio/自定义 API)。两个配套动作:

1. 错误落点治理:E0277 指向生成文件里的闭包——靠 §4.5 的行对齐 + `sv` CLI 的路径回映,用户看到的是 .sv 的行号;
2. 文档收录这条预期错误样例并解释"跨线程改状态请发消息回主线程"(sv-reactive 既有约束)。

另一个逃逸面:**嵌套 `fn` item**。`fn helper() { count += 1 }`(item fn 不能捕获环境)改写后撞 rustc E0434("can't capture dynamic environment in a fn item"),信息尚可但没说人话。编译器预检:嵌套 fn 体内出现 rune 标识符 → sv0009:"`fn` 项不能捕获响应式绑定,改用闭包 `let helper = move || ...;`"。这正是 Svelte 鼓励的形态(函数捕获 state 是 JS 闭包;Rust 里对应物是闭包而非 fn)。

### 3.3 把 `count` 传给函数:快照语义,与 Svelte 一致

`fn f(x: i32)` 收到 `count.get()` 的快照——**这不是我们的妥协,是 Svelte 的文档化语义**:$state 文档专门有 "Passing state into functions" 一节,明确 JS 传值,"the arguments are the *values* rather than the *variables*",响应性不跨函数边界。Svelte 的官方建议(传 getter / 传含 getter 的对象)映射到我们这边就是:

- 要传"活的"引用 → `f($sig(count))`,签名 `fn f(x: Signal<i32>)`——比 Svelte 的 getter 约定更显式、类型可查;
- 唯一分叉:Svelte 里传**对象** state 时 Proxy 同行、深改仍响应;我们一律快照。差异表已列(§2.5),文档必须用对比例子讲透,这是从 Svelte 迁移过来的人最可能踩的一条。

### 3.4 结构体字段:四个候选方案与 v0 决策

Rust 没有 Proxy,`$state(struct)` 的深层可变性有四条路:

| 方案 | 形态 | 评估 |
|---|---|---|
| A. `$state` 只准包"叶子值"(Copy/原子类型) | 结构体必须逐字段声明 rune | 语义最干净,但直接毙掉 `$state(Vec<Todo>)`,TodoMVC 都写不了。**否** |
| B. 统一 `update` 闭包改写(变量粒度) | §2.3 规则表 | 语义健全、实现閉合、over-notify 可被 derived 剪枝吸收。**v0 选它** |
| C. write guard(drop 时 notify) | `xs.write().push(v)`,Dioxus 生产验证(`Signal::write() -> WritableRef`,"When the guard goes out of scope, reactive updates trigger",[docs.rs/dioxus-signals](https://docs.rs/dioxus-signals/latest/dioxus_signals/struct.Signal.html)) | 表达力最强(表达式位可变方法也能写),但 sv-reactive 现在是"take 值"模型,要重构成 per-node RefCell,且 guard 跨语句持有会引入新的重入 panic 面。**v1 备选**,作为 `$mut` 的实现基础 |
| D. `#[derive(Store)]` 字段投影(Leptos `reactive_stores` 路线,04 报告 §6.1) | `todo.done()` 返回字段级信号 | 唯一能拿到**字段级订阅粒度**的方案,但改变 props/each 的类型面貌,工程量大。**v1/v2**,与 B 不冲突(B 是缺省,D 是 opt-in 精化) |

**方案 B 的读写判别启发式**(无类型信息下的语法判别,§2.2/2.3 已列):方法调用在**表达式位 = 读**(`with`,tracked)、**语句位 = 写**(`update`,notify)。两个失败模式都可控:

- 表达式位的可变方法(`let n = xs.pop()`):`with` 闭包内拿 `&T` 调 `&mut self` 方法 → **rustc E0596 编译错误**,编译器再附注指路 `$sig(xs).update(...)`。错误方向安全。
- 语句位的纯读方法(`xs.len();`):被当成写,假通知一次。语义仍正确(effect 多跑一轮、derived 剪枝兜底),且"语句位丢弃返回值的纯读调用"本身就是无意义代码,极罕见。接受。

### 3.5 循环与 match 里的读写

- `while count.get() < 10 { count += 1; }`:每次迭代 `update` → notify;若不在 batch 内,当前运行时会同步 flush,effect 跑 O(n) 轮。**结论**:正确性没问题(sv-reactive 的 `MAX_FLUSH_PASSES` 防失控),但必须配套"事件回调自动 batch + 组件 init 自动 batch"(§2.3);热循环文档建议 `$sig(count).update(|v| *v = 10)` 一次完成。
- `for x in xs { ... }`(xs 是 rune):读位改写 `for x in xs.get() { ... }`——快照迭代,**循环体内写 `xs` 是允许且安全的**(迭代的是 clone)。这与 Svelte(迭代 proxy,循环中改数组是经典坑)相比反而更可预测,值得写进卖点。
- `for x in &mut xs`:撞 sv0007(`&mut` rune),指路 `$sig(xs).update(|v| for x in v.iter_mut() { ... })`。
- `match count { ... }`:scrutinee 快照(§2.2),arm 里写 `count` 自身合法——状态机模式(`match state.get() { Idle => state = Running, ... }`)一等公民。
- effect 内循环读同一 rune:依赖登记去重(runtime 已做),无性能悬崖。

### 3.6 编译期拒绝清单(v0)与错误信息设计

原则:每条错误必须 ① 指到 .sv 源位置,② 说清"为什么不行",③ 给出**可以直接抄的替代拼法**。错误码对标 Svelte 的命名风格。

| 错误码 | 触发模式 | 错误信息(草案) |
|---|---|---|
| sv0001 `rune_invalid_placement` | `$state/$derived` 不在 `let` 初始化器位;`$effect` 不在语句位 | "`$state(...)` 只能作为 `let` 声明的初始化器使用" |
| sv0002 `rune_shadowing` | 任何位置引入与 rune 同名的绑定(let/闭包参数/match/for/if-let 模式) | "`count` 是响应式绑定,不能在内层重新绑定;取当前值快照请换名:`let count_now = count;`" |
| sv0003 `rune_in_macro` | rune 标识符出现在非白名单宏调用的 token 流中 | "宏内部无法改写响应式读取;先绑定快照:`let c = count;` 再传入宏" |
| sv0005 `derived_reassign` | 对 `$derived` 绑定赋值 | "`$derived` 是只读的(派生值);需要可覆盖的值请改用 `$state` + `$effect` 同步" |
| sv0006 `state_write_in_derived` | `$derived` 表达式内静态可检出的 rune 写 | "不能在 `$derived` 计算中写入状态(对应 Svelte 的 state_unsafe_mutation)" |
| sv0007 `mut_borrow_of_rune` | `&mut count`(含 `for _ in &mut xs`) | "不能对响应式绑定取 `&mut`;原地修改请用 `$sig(count).update(\|v\| ...)`" |
| sv0008 `rune_destructure` | `let (a, b) = $state(..)` / `let Pat {..} = $derived(..)` | "rune 声明不支持解构;逐个声明,或对读取结果解构(得到快照)" |
| sv0009 `rune_in_fn_item` | 嵌套 `fn` 体内引用 rune | "`fn` 项不能捕获响应式绑定;改用闭包:`let helper = move \|\| ...;`" |
| sv0010 `rune_in_static` | `static`/`const` 初始化器内出现 rune | "静态项不能引用响应式状态" |
| sv0011 `macro_rules_in_script` | script 块内定义 `macro_rules!` | "v0 不支持在组件内定义宏(`$` 元变量与 rune 预处理冲突);请移到普通 .rs 模块" |
| sv0012 `reserved_prefix` | 用户标识符以 `__sv_` 开头 | "`__sv_` 前缀保留给编译器生成代码" |
| sv0013 `props_invalid` | `$props()` 不在组件顶层 / 重复调用 / 非 `let Props {..} =` 形态 | 对标 Svelte `props_invalid_placement` / `props_duplicate` |

**不由我们报、但要在文档收录预期样例的 rustc 错误**:E0277(句柄 `!Send`,线程逃逸)、E0596(表达式位可变方法,§3.4)、E0382(derived move 捕获非 Copy 变量后继续使用)、Clone 缺失(裸读非 Clone 类型 → 提示改用字段读/`with`)。编译器在能识别的场合追加 note(§4.5)。

---

## 4. syn 实现策略(本地实验验证)

### 4.1 预处理:token 级替换,不是文本替换

实验(`scratchpad/dollar-exp`,proc-macro2 1.0.106 / syn 2.0.119)证实的事实链:

1. **`$` 是合法 token**:`TokenStream::from_str` 对含 `$state(0)` 的整段代码解析成功,`$` 是 `Punct('$')`(macro_rules 体本来就是 token 流,词法器必须收它)。**结论:预处理在 token 层做,不碰文本**。
2. **字符串/注释天然免疫**:`"字符串里的 $state"` 是单个 `Literal` token,普通注释被词法器丢弃,doc 注释变成 `#[doc = "..."]`(内容也是 Literal)——正则文本替换要处理的三大坑在词法层直接消失。
3. **邻接判定可行**:`span().end()`(`$`)与 `span().start()`(ident)的 LineColumn 相等 ⇔ 紧邻。`$ state`(有空格)被正确跳过。需要 proc-macro2 `span-locations` feature。
4. **替换后 syn 全功能**:`$state → __sv_state`(span 取原 ident 的 span)后,`syn::parse2::<syn::Block>` 直接成功,span 保真(不经字符串往返)。
5. **macro_rules 会被误伤(实锤)**:实验里 `macro_rules! my { ($state:expr) => { $state + 1 }; }` 体内的 `$state` 被**静默替换**——`$state` 恰好邻接、名字撞 rune。这就是 sv0011(v0 禁 script 内 macro_rules)的直接依据;若未来放开,预处理需识别 `macro_rules` ident + `!` 并跳过其后整个 token 树。

管线:`.sv 文本 → 切出 script 块 → TokenStream::from_str(带行列偏移修正)→ token 遍历替换 $rune → syn::parse2<syn::File 或 Vec<Stmt>> → 语义分析(rune 表、拒绝清单)→ visit_mut 改写 → prettyplease 输出`。`__sv_state(...)` 在替换后就是普通函数调用表达式,syn 正常成树;`$bindable(i32)` 在类型位替换后是 `__sv_bindable(i32)`,恰好能被 syn 按 Fn-sugar 路径(`TypePath` + `ParenthesizedGenericArguments`)解析——props 类型标注(§5)搭这个便车。

### 4.2 visit_mut 改写的坑(清单)

- **只改单段 `Expr::Path`**:`module::count`、`Self::count` 不碰;`Expr::Field` 的 member ident(`s.count` 的 `count`)不碰;方法名不碰(实验第 7 项确认 syn AST 里三者节点类型不同,不会误判)。
- **结构体简写必须展开**:`Point { count }` 的 `FieldValue` 是 `colon_token: None`,改写成 `count: count.get()`(实验确认可识别)。
- **改写顺序**:自底向上(先改子表达式再包外层),复合赋值的 RHS 预求值必须在 RHS 已改写之后做,否则 `__rhs` 里还有裸 rune。
- **语句位/表达式位判别**:`Stmt::Expr(e, Some(semi))` 且值被丢弃 = 语句位;块尾表达式、`let` RHS、参数位 = 表达式位。
- **模式遍历**:所有 `Pat` 节点收集绑定名与 rune 表求交 → sv0002。这是拒绝 shadowing 后唯一要做的"作用域工作"。
- **属性/泛型/类型位置**:类型位置的 ident(用户类型恰好叫 `count`?类型与值不同命名空间)不改——visit_mut 只走 expression/statement visitor,不进 `Type`。

### 4.3 宏调用内部:白名单 + 硬错误

syn 把宏调用参数保留为原始 token 流,visit_mut 的表达式改写进不去。三层处理:

1. **std fmt 宏白名单**(`format! println! print! eprintln! eprint! write! writeln! panic! assert! assert_eq! assert_ne! format_args! todo! unreachable!` + 可配置追加 `log/tracing` 系):把参数 token 流按 fmt 语法解析,**隐式捕获脱糖**——`println!("{count}")` → `println!("{}", count.get())`;位置参数/命名参数里的 rune 读同规则改写。隐式捕获藏在字符串字面量里,这是唯一够得着它的层面;fmt 语法稳定且有 `format_args` 规范可依。
2. **`view!`/模板宏**:不存在——独立编译器路线里模板不是宏,template 块由同一改写器处理。
3. **其他宏**:token 流中出现 rune 名 ident → **sv0003 硬错误**,建议先绑快照。不做"盲改 ident → `(count.get())`"——宏可能 token 匹配 ident(如 `matches!`、自定义 DSL),盲改会静默破坏,违反"失败必须是编译错误"原则。

### 4.4 hygiene 与命名冲突

独立编译器生成的是普通文本 Rust,没有宏 hygiene 可用,靠约定封死:

- 生成的辅助绑定/项一律 `__sv_` 前缀(`__sv_rhs`、`__sv_v`、`__sv_rt`),用户侧 `__sv_` 前缀被 sv0012 拒绝 → 冲突不可能;
- 运行时引用走 `use ::sv_reactive as __sv_rt;`(crate 名绝对路径),用户 `use` 什么都不受影响;
- rune 改写用的方法名(`.get()/.set()/.update()/.with()`)是 `Signal<T>` 固有方法,接收者类型确定,不受用户 trait 引入影响(除非用户给 `Signal<T>` 写扩展 trait 制造歧义——文档警告即可,rustc 会报歧义错而非静默错)。

### 4.5 span / 错误映射:行对齐生成

独立编译器的老大难(04 报告 §1b 已论)。具体方案:

1. **script 块行对齐**:生成文件中,script 部分逐行对应 .sv 的 script 行号(读写改写只改列内容,不增删行;预求值块 `{ let __sv_rhs = ...; ... }` 写在同一行)。文件头对齐填充 `//` 行。rustc/clippy 的行号误差为零,列号近似。
2. **模板生成代码**放在 script 之后,每段前注释 `// sv:src=Counter.sv:37:9`,`sv build` 包装 cargo,用 `--message-format=json` 拿诊断,按行映射回 .sv 再渲染(SvelteKit/Volar 均此思路)。
3. **常见 rustc 错误的补充注释**:对 E0277(!Send)、E0596(with 内 &mut)、E0382(derived move)做 pattern 匹配,在回显时附加 sv 侧解释与 fix 建议(§3.6 尾表)。
4. **`sv expand` 子命令**:打印某组件生成的 Rust(prettyplease 格式化)。这是隐式变换的"透明度阀门",DX 论战里最有效的缓解措施(§6)。
5. 长期:r-a 不认识 .sv,需要 sv-lsp 做转发(svelte-language-server / Volar 模式)。**注意**:整套变换器是独立库,输入输出都是 TokenStream——若最终折回 proc-macro 路线(04 报告推荐),`sv! { }` 宏内做同样变换技术上完全可行(`$` 在宏输入里合法,实验已证),只是 r-a 展开后用户看到的 `count` 是 `Signal<i32>`,补全语义撕裂,DX 反而更差。**script 变换与独立编译器路线是强绑定的组合**。

---

## 5. props 与组件组合

### 5.1 `$props` 的 SFC 形态:显式 `struct Props` + 解构

Svelte 用 TS 类型标注 + 对象解构(`let { a = 1, children }: Props = $props()`,强制 destructuring pattern,见 `props_invalid_identifier`)。Rust 没有匿名结构体类型,硬造会脱离 syn 可解析范围。v0 形态:

```rust
<script>
struct Props {
    label: String,
    #[prop(default = 1)]
    step: i32,
    count: $bindable(i32),      // 预处理后 __sv_bindable(i32),syn 按 Fn-sugar 类型解析
    #[prop(optional)]
    on_change: Option<Callback<i32>>,
    children: Snippet,
}
let Props { label, step, count, children, .. } = $props();
</script>
```

- `struct Props` 是**编译器指令**而非真实类型:编译器读字段表后生成真正的 props 结构(见下),`$props()` 解构行改写为对生成结构的解构。每个 .sv 至多一个(对标 `props_duplicate`)。
- 类型标注就是普通 Rust 类型,r-a 未来在 sv-lsp 下可完整理解;default/optional 走属性,对标 Svelte 解构默认值。

### 5.2 props 的响应性:一律包 reactive 读端

Svelte 里 props 是响应的(父更新自动流入)。对应生成(概念形态):

```rust
pub struct CounterProps {
    pub label: __sv_rt::PropValue<String>,   // enum { Static(T), Reactive(Derived<T>) }
    pub step:  __sv_rt::PropValue<i32>,
    pub count: __sv_rt::Signal<i32>,         // $bindable → 直接要求可写句柄
    pub children: Snippet,
}
```

- 子组件内,prop 标识符进 rune 表,读位同规则改写(`label` → `label.get()`);普通 prop **不可赋值**(拒绝,对标 Svelte 对非 bindable prop 写入的警告/错误),`$bindable` prop 可读可写、写即回流父级——因为它就是父级传进来的 `Signal` 句柄本体,**双向绑定零胶水**,且"`bind:` 只接受可写信号"由类型系统保证(04 报告 §2.3 的静态优势在此兑现)。
- 父侧模板:`<Counter label={name} bind:count={total} />` → 静态实参走 `PropValue::Static`,含 rune 读的表达式包成 `PropValue::Reactive(derived(move || ...))`;`bind:` 位置的 rune **不做读改写**,直接传 `$sig` 本体。

### 5.3 children 与 snippet

Svelte 5 的 snippet 映射为**闭包 prop**(04 报告 §2.4 结论,此处落地类型):

```rust
pub type Snippet        = std::rc::Rc<dyn Fn(&Doc, ViewId)>;
pub type Snippet1<A>    = std::rc::Rc<dyn Fn(&Doc, ViewId, A)>;
```

- 模板里 `{#snippet row(todo)} ... {/snippet}` → 编译成 `Rc::new(move |doc, parent, todo| { ...模板 codegen... })`,捕获全是 Copy 句柄,无借用问题;
- `{@render children()}` / `{@render row(item)}` → codegen 处直接调用闭包,把当前 `Doc`/挂载点传入;
- 隐式 `children`:标签体非 snippet 内容自动包成缺省 `children` prop,与 Svelte 语义一致;
- snippet 参数若需响应(each 行内),v0 传快照值 + 行重建(each_block 现状),D 方案(Store 投影)落地后升级为行级句柄。

---

## 6. 结论:v0 子集、拒绝清单、DX 判断

### 6.1 v0 支持的 runes 子集

| rune | v0 语义 |
|---|---|
| `$state(v)` | 任意 `T`(读整体需 `Clone`);变量粒度响应;读=快照、写=set/update(RHS 预求值) |
| `$derived(expr)` / `$derived.by(闭包)` | move 闭包化,惰性 + 相等剪枝(运行时已有);只读 |
| `$effect(闭包)` | 语句位;同步首跑(运行时既有语义,与 Svelte 微任务时序的差异沿用 sv-reactive 文档) |
| `$props()` | `struct Props` + 解构形态;prop 读端响应 |
| `$bindable(T)` | props 类型位;可写句柄直传 |
| `$untrack(expr)` | 脱追踪读 |
| `$sig(x)` | **逃生舱**:取句柄,显式 API 全开放(sv 特有,Svelte 无对应物也不需要) |

**明确不进 v0**:`$state.raw` / `$state.snapshot`(快照本来就是缺省语义,无 Proxy 即无此需求)、writable `$derived`(5.25 特性,观察)、`$effect.pre/.tracking/.root`(等场景树时序需求明确后再定)、`$inspect`(dev 工具期)、`$host`(无 custom element 语境)、script 内 `macro_rules!`、非白名单宏内的 rune。

### 6.2 隐式反应性值不值得?——论战与判断

**Svelte 社区的正方(显式声明+隐式使用是对的)**:Rich Harris 在 [Introducing runes](https://svelte.dev/blog/runes) 给出的废除全隐式(`let count = 0`)的理由——"As applications grow in complexity, figuring out which values are reactive and which aren't can get tricky",且启发式只在 .svelte 顶层生效,"Having code behave one way inside `.svelte` files and another inside `.js` can make it hard to refactor code"。即:**Svelte 亲手验证过"边界模糊的隐式"不可持续,而"$声明 + 裸使用"是他们迭代出的平衡点**。支持者的总结(见 [runes 争论汇总](https://biggo.com/news/202503181123_Svelte_5_Runes_Debate)、[Kodaps 分析](https://www.kodaps.dev/en/blog/runes-svelte-magic-problem)):有了 runes,"look at any component and instantly know what it does"。

**反方(值得认真对待的三条)**:① rune 只在 .svelte/.svelte.ts 里是活的,"这不再是 JavaScript"([HN: Svelte 5: Runes](https://news.ycombinator.com/item?id=37584224)、[HN 43093037](https://news.ycombinator.com/item?id=43093037))——照进我们:.sv 里的"Rust"不是 Rust,`&mut` 被拒、`f(count)` 是快照、derived 里 move,**资深 Rust 用户会经历 uncanny valley**;② 变体膨胀(`$state`/`$state.raw`/`$state.snapshot` 三选一的心智负担)——照进我们:v0 坚决不加变体,快照语义唯一;③ 读代码时看不见成本——`count` 一个 clone 藏在裸标识符后面,这在 JS 里无所谓,在 Rust 文化里是原则问题(Leptos 坚持显式 `.get()`;Dioxus 走中间路线:`Copy` Signal + `count()` 调用语法 + write guard,证明 Rust 社区对"少写但可见"的接受度存在光谱)。

**判断:值得做,给三个前提。**

1. **契约边界必须是文件级**:.sv 是另一门语言(Svelte 同款社会契约),.rs 里永远是显式 API;`$sig` 保证两个世界零成本互通。这比 Svelte 的 ".svelte.ts 后缀魔法" 边界更硬。
2. **透明度工具先行**:`sv expand` 与 v0 同期交付;所有 sv 错误信息展示**改写后的形态**,让用户随时能"看穿"变换。隐式的每一分 DX 红利都要用可见性赎回。
3. **失败模式全部落在编译期**(拒绝清单 + rustc 类型/借用检查 + `!Send`),唯一的运行时新风险(update 重入)被 RHS 预求值规则从根上消除。这是我们相对 Svelte(Proxy 运行时行为)反而更强的健全性论据。

反过来说,**如果不做 script 变换**,独立编译器路线的价值主张会塌一半:模板 `{count}` 反正要变换(编译器已在场),script 里却要求 `count.get()`——同一个文件两套读写规则,比"全显式"更糟。所以真正的二选一是:"独立编译器 + script/template 统一变换" vs "proc-macro + 全显式 API"(04 报告路线)。本报告证明前者语义上站得住;两条路线共享 sv-reactive、sv-ui 与(设计为独立库的)变换器核心,切换成本被架构吸收。

### 6.3 落地顺序建议

1. **M-a**:变换器独立库 `sv-compiler-script`:token 预处理($ 替换、macro_rules 检测)→ rune 表 → 拒绝清单(sv0001–sv0013)→ visit_mut 读写改写。纯函数库,snapshot 测试(输入 .sv script,输出 Rust 文本)。
2. **M-b**:sv-reactive 增补 `Signal::mutate<R>`(update 返回 R)与(评估)per-node RefCell 化为 guard 方案铺路;事件回调/组件 init 自动 batch。
3. **M-c**:`.sv` 文件切分器 + template 块共用读规则 + 行对齐生成 + `sv build`(cargo 包装、诊断回映)+ `sv expand`。
4. **M-d**:`$props`/snippet 生成;计数器与 TodoMVC 两个 .sv 样例端到端(TodoMVC 专门压测 §3.4 变量粒度的实际体感,为 `#[derive(Store)]` 的优先级提供数据)。

---

## 7. 来源

**联网核实**:

- Svelte `$state` 官方文档(深层 Proxy、Passing state into functions、解构、$state.raw/snapshot):https://svelte.dev/docs/svelte/$state
- Svelte compiler-errors 官方清单(state_invalid_placement 等全部错误码原文):https://svelte.dev/docs/svelte/compiler-errors
- Svelte 编译器源码 · client 赋值变换(compound assignment 脱糖、$.set/$.assign):https://github.com/sveltejs/svelte/blob/main/packages/svelte/src/compiler/phases/3-transform/client/visitors/AssignmentExpression.js
- Compile Svelte 5 in your head(编译输出逐段讲解,$.state/$.get/$.set/template_effect):https://lihautan.com/compile-svelte-5-in-your-head
- Introducing runes(Rich Harris,隐式→显式的动机原文):https://svelte.dev/blog/runes
- writable $derived(5.25):https://news.ycombinator.com/item?id=43444999 · https://svelte.dev/docs/svelte/$derived
- runes 论战:https://news.ycombinator.com/item?id=37584224 · https://news.ycombinator.com/item?id=43093037 · https://biggo.com/news/202503181123_Svelte_5_Runes_Debate · https://www.kodaps.dev/en/blog/runes-svelte-magic-problem
- Dioxus `Signal`(Copy、调用语法、write guard drop 通知——方案 C 的生产先例):https://docs.rs/dioxus-signals/latest/dioxus_signals/struct.Signal.html

**本地实验**(可复跑):`scratchpad/dollar-exp`(proc-macro2 1.0.106 + syn 2.0.119,Windows,2026-07-17)——`$` 词法接受性、字符串/注释免疫、span 邻接判定、替换后 syn 解析、macro_rules 误伤实锤、FieldValue 简写识别、doc 注释 token 形态。

**仅凭训练数据、未逐条联网核实的点(风险自评:低,均为语言/库稳定事实)**:Rust fmt 宏隐式捕获的脱糖规则(RFC 2795,稳定于 1.58);match scrutinee 临时值存活到 match 结束的生命周期规则;Leptos 社区对显式 `.get()` 的立场表述细节;Svelte `count++` 编译为 `$.update(count)` 的具体形态(UpdateExpression.js 未抓取,但与 AssignmentExpression 同族,不影响任何结论——Rust 无 `++`);syn 对 `Ident(Type)` 按 Fn-sugar 解析 `$bindable(i32)` 的断言(基于 syn 文法知识,落地时用一行测试锁定)。
