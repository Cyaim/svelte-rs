# 04 · "Svelte 编译器"在 Rust 里的实现策略

> proc-macro 内嵌 DSL vs 独立模板文件 vs 完全自定义语言
>
> 调研日期:2026-07-17。关键事实已联网核实(版本号、维护状态、热重载现状);未能核实的结论已单独标注。

---

## 0. 结论先行(TL;DR)

1. **推荐路线:(a) proc-macro 内嵌 DSL(`view!` 宏)起步**,但从第一天起把编译器做成**独立普通库 crate**(parser → 模板 IR → codegen),proc-macro 只是一层薄壳。这样路线 (b) 外部文件、热重载解释器、未来的 fmt/LSP 工具都能复用同一个编译器核心——这正是 Slint(compiler lib 被 macro / build.rs / interpreter 三个前端共用)和 Svelte 本身(`svelte/compiler` 是独立包)的架构。
2. **最重要的一条工程判断:"生成数据,而不是生成类型"**。把模板的静态结构编译成 `const` 的 `Template` 静态数据(节点表 + 动态绑定路径表),运行时批量实例化;只有动态表达式编译成闭包。这一个决定同时买到三样东西:编译时间可控(避免 Leptos 类型级视图树的泛型爆炸——Leptos 0.8 专门加 `--cfg=erase_components` 来救编译时间,就是反面教材)、**模板级热重载几乎免费**(Template 是数据,可序列化、可在运行时替换,Dioxus 已验证此路)、二进制体积可控。
3. **IDE 体验的核心不是"选不选宏",而是"错误恢复解析"**。rust-analyzer 对函数式 proc-macro 的补全依赖宏在残缺输入下仍能成功展开(speculative expansion);宏一 panic 或只吐 `compile_error!`,补全就全灭。rstml 的 recoverable parsing、Dioxus rsx 的部分展开都是围绕这一点设计的。这必须是 parser 的第一性需求,不是后期优化。
4. **路线 (c) 完全自定义语言 + LSP 明确不推荐**:那是 Slint 用整个团队多年投入才做好的事(编译器 + LSP + live-preview + 格式化 + 文档语法高亮全家桶),且会丢掉"模板里就是真 Rust 表达式"这一核心卖点。除非项目定位变成"语言产品",否则不碰。
5. 借用检查友好的前提是**信号(signal)必须是 `Copy` 的 arena 句柄**——Leptos 与 Dioxus 殊途同归的设计。事件闭包 `move` 进去零借用冲突,`bind:value` 才可能是纯语法糖。
6. 最大的未决难点是**深层响应性**(`bind:checked=todo.done` 这种行内字段绑定):Svelte 5 靠 Proxy,Rust 没有等价物,需要 reactive store/lens(参考 Leptos `reactive_stores`)或行级信号投影,必须原型验证。

---

## 1. 三条路线对比

### 1a. proc-macro 内嵌 DSL(`view!` / `rsx!`,类 Leptos / Dioxus)

**生态现状(2026-07 联网核实):**

- Leptos 当前 0.8.19,`view!` 宏基于 **rstml**(syn-rsx 的社区继任 fork)解析;rstml 最新版 **v0.12.0(2024-07-28 发布)**,维护节奏偏慢但功能完整:recoverable parsing(带错误仍返回节点树,专为 IDE 友好设计)、自定义节点/属性解析、span 保真的错误报告。使用者含 Leptos、Sauron、leptosfmt。
- Dioxus 0.7 已于 **2025-09-08 正式发布**,`rsx!` 用自研 parser(`dioxus-rsx`),配套 `dx` CLI 实现两级热重载(详见下文)。
- 格式化:社区已趟出路——`leptosfmt`(Leptos 官方 DX 文档推荐)、Dioxus 的 autofmt 内建于 CLI/编辑器插件;语法高亮有 `tree-sitter-rstml` 现成 grammar。

**IDE 体验(rust-analyzer 机制,核实过):**

rust-analyzer 计算补全的方式是:复制当前文件、在光标处插入占位标识符、尝试展开所有宏,然后在**展开结果**上分析上下文。推论:

- 只要宏在"半截输入"下仍能展开出包含用户表达式(且 span 保留)的代码,闭包体内、属性值里的 Rust 表达式就有**完整**的补全/跳转/类型提示——这是路线 (a) 相对 (b)(c) 的决定性优势:模板里嵌的是真 Rust,r-a 全程在场。
- 反之,输入残缺时宏若 panic 或只输出 `compile_error!`,r-a 拿到空展开,补全直接消失。Leptos DX 文档甚至建议用户配置 r-a 忽略 `#[server]`/`#[component]` 属性宏来救补全——教训:**属性宏比函数式宏更容易伤 IDE**,`#[component]` 应做得尽量"透明"(仅包装,不重写函数体)。
- 标签名/属性名(非 Rust 标识符)的补全 r-a 给不了,需要 tree-sitter 高亮 + 后续可选的轻量 LSP 扩展补足——但这是"锦上添花"级别的缺口,不是塌方。

**编译时间与增量:**

- proc-macro 本身的展开开销极小;syn/quote 的编译是一次性成本(秒级)。真正决定编译时间的是**生成代码的形态**:Leptos 的 tachys 用类型级嵌套视图树(每个模板是一个巨型泛型类型),单态化开销大到官方要提供 `--cfg=erase_components`(类型擦除的 dev 模式)来改善——这不是宏的锅,是 codegen 策略的锅。生成"静态数据 + 少量闭包"(Dioxus Template 模式)可以从根上避开。
- 增量编译:模板改动只重编该 crate,与普通 Rust 一致;比 build.rs 路线的"改一个模板 → 重跑 build script → 重编整个生成模块"更细。

**热重载(核实过,这是 2026 年格局变化最大的一块):**

Dioxus 0.7 的两级方案已是事实标准:

1. **模板热重载(不重编译)**:`rsx!` 的静态结构编译成可序列化的 Template 数据;dev CLI 监听源码、重新解析 rsx、diff 模板、把新模板推给运行中的 app。能即时生效的:元素增删改、字符串属性、字面量 props、格式化字符串里挪变量、if/for 体内的标记结构。
2. **Rust 逻辑热补丁(Subsecond)**:`dx serve --hotpatch`,增量链接/二进制补丁(思路近 Zig 的方案),支持 Web/桌面三平台/iOS/Android;限制:只追踪 tip crate(workspace 里依赖 crate 的改动不感知)、静态初始化器变更不生效、结构体字段增删无法自动迁移状态。**Subsecond 已被 Bevy 和 Iced 采用**,说明它作为独立机制可被第三方框架集成——我们也可以。

结论:proc-macro 路线的热重载天花板已经被 Dioxus 抬到接近 Svelte/Vite 的水平,"宏 = 没热重载"是 2023 年的旧认知。

### 1b. 独立模板文件 + 编译期代码生成(类 askama / Slint 的 .slint + build.rs)

两种子形态:

- **derive 宏读外部文件**(askama:`#[template(path="foo.html")]`,宏在编译期读文件生成渲染代码);
- **build.rs 代码生成**(slint-build:`.slint` → OUT_DIR 里的 Rust 模块,`include!` 进来)。

**IDE 体验:**

- Rust 侧:r-a 会跑 build script 并索引 OUT_DIR 生成代码,所以**生成出来的组件 API**(结构体、属性 setter)在 Rust 代码里有补全跳转——这点没问题。
- 模板文件内:**零支持**,除非自己造工具。没有 r-a、没有类型检查、嵌入的 Rust 表达式是"字符串"。askama 的解法是根本不嵌 Rust——模板表达式是自己的 Jinja 风格迷你语言;Slint 的解法是干脆做整套 LSP。也就是说:**路线 (b) 想要好的模板内体验,最终一定滑向路线 (c) 的成本**。
- span/错误映射:proc-macro 的 `Span` 无法指向外部文件,错误要么指向宏调用点(用户看不懂),要么靠错误消息里手工拼 `file:line`(askama 的现状),要么像 Slint 一样完全自己发诊断。这是持续的工程税。

**热重载:**

天然强项——文件本来就在编译流程外,运行时挂个解释器即可(Slint 的 live-preview 就是 interpreter 驱动;Slint 1.17 甚至加了 `slint-viewer --remote`,桌面上改 `.slint`、手机/平板上实时看效果——这个"远程真机预览"思路对鸿蒙调试很有参考价值)。但注意:**路线 (a) 用"Template 是数据"的 codegen 后,同样能做到模板级热重载**,(b) 的这项优势不再独占。

**适用判断:** 当"设计师/非 Rust 人员写 UI"成为真实需求时才值得上;作为主路线启动会同时吃"模板内无 IDE"和"要自己造工具链"两笔成本。

### 1c. 完全自定义语言 + LSP(Slint 路线)

Slint 的实际投入(核实):自研语言 + 编译器(供 Rust/C++/JS/Python 四语言绑定)+ 全功能 LSP(诊断、补全、跳转、live-preview)+ 格式化 + 文档站,由一家公司多年全职维护,2026 年仍在高速迭代(1.15 → 1.17)。另一个数据点是 Makepad(1.0 于 2025-05 发布):live design DSL + 自研 IDE,同样是"平台级"投入。Svelte 本身其实也是这条路(`.svelte` 是自定义语言 + svelte-language-server),但它背后是巨大的社区。

**对本项目的判断:** 成本结构完全不匹配一个探索期项目;且我们的差异化卖点恰恰是"心智模型是 Svelte、表达式是真 Rust、借用检查器全程在场"——自定义语言把第三条直接砍掉。**排除。**

### 对比矩阵

| 维度 | (a) proc-macro DSL | (b) 外部文件 + codegen | (c) 自定义语言 + LSP |
|---|---|---|---|
| Rust 表达式嵌入 | ★★★ 真 Rust,r-a 在场 | ★ 字符串,或退化为迷你语言 | ★ 需在语言里重造表达式 |
| 补全/跳转/类型提示 | ★★☆(依赖错误恢复展开;标签名补全缺) | Rust 侧 ★★ / 模板内 ☆ | ★★★ 但须自建全套 |
| 错误 span 精度 | ★★★(quote_spanned 可做到表达式级) | ★(跨文件映射靠手工) | ★★★ 自己发诊断 |
| 编译时间/增量 | 取决于 codegen 形态;数据式可很好 | build.rs 粒度粗 | 同 (b),另有解释器选项 |
| 热重载 | ★★★(Dioxus 已验证:模板 diff + Subsecond) | ★★★(解释器天然支持) | ★★★(Slint live-preview) |
| Rust 开发者学习曲线 | ★★★ 最低 | ★★ | ★ |
| 非 Rust 协作者友好 | ☆ | ★★ | ★★★ |
| 工具链自建成本 | 低(fmt + tree-sitter 即可及格) | 中(错误映射 + 高亮) | 极高(LSP 全家桶) |
| 演进灵活性 | 高(编译器做成库即可加前端) | 中 | 低(语言即承诺) |

---

## 2. 模板语法设计:Svelte 概念的 Rust 对应形态

前提:Svelte 5/6 现状核实——**Svelte 5 仍是当前大版本**(5.56.6,2026-07-16;网传"Svelte 6 已发布"出自 SEO 农场文,不实)。对标基准即 Svelte 5 runes:`$state/$derived/$effect/$props/$bindable` + snippets(取代 slots)。

### 2.1 控制流:用宏内原生关键字,不用"控制流组件"

Svelte 的 `{#if}/{#each}` 本质是编译到分支/列表块。Rust DSL 里有两派:

- Leptos 派:`<Show when=...>`、`<For each=... key=...>` 组件 + 闭包——缺点是嵌套闭包噪音大、`children` 类型复杂、错误信息差;
- Dioxus 派:宏内直接写 `if` / `for` / `match` 关键字,parser 特判——语法就是 Rust,条件/模式是真表达式,r-a 可分析。

**推荐 Dioxus 派**,对应关系:

| Svelte | 本项目 DSL |
|---|---|
| `{#if c}...{:else if}...{:else}` | `if c { ... } else if ... { ... } else { ... }`(编译为记忆化分支效果,切换时销毁/重建子树) |
| `{#each xs as x (x.id)}` | `for x in xs [key: x.id] { ... }`(`[key: ...]` 为扩展语法,编译为 keyed reconciler;缺省退化为 index-keyed) |
| `{#await p}...{:then v}...{:catch e}` | `match load.state() { Pending => {...}, Ready(v) => {...}, Failed(e) => {...} }` —— 复用 `match` + 一个 `AsyncState<T,E>` 枚举,而非发明 `await` 块;配套 `resource()` 原语与可选 `<Suspense>` 边界 |
| `{#key expr}` | `keyed(expr) { ... }` |

### 2.2 事件绑定与借用检查

`on:click=move |e| ...`。可行性的关键在响应式原语的设计而非宏:**signal 句柄必须 `Copy + 'static`**(arena/slotmap 索引,Leptos 与 Dioxus 的共同收敛点)。闭包 `move` 捕获的是句柄拷贝,无生命周期纠缠;`FnMut + 'static` 即可存进 retained 节点。宏必须把闭包 token **原样透传**(span 不动),r-a 才能在闭包体内正常工作。

### 2.3 双向绑定 `bind:value`

纯语法糖,展开为"读 + 写"两条:

```text
<TextInput bind:value=input />
   ⇩ 展开
<TextInput value={input.get()} on:input=move |e| input.set(e.value) />
```

组件侧需要协议支持:props 里声明可绑定属性(对应 Svelte 5 的 `$bindable`),类型上要求传入的是**可写**信号(`RwSignal<T>` / `impl SignalWrite`),编译期即可拒绝 `bind:` 到只读 derived——这是比 Svelte 更强的静态保证,值得当卖点。

**难点(未决)**:`bind:checked=todo.done`(绑定到集合元素的字段)。Svelte 5 靠深层 Proxy;Rust 需要 store/lens 机制(如 Leptos `reactive_stores` 的字段级订阅)或 `for` 编译时为每行投影出行级信号。必须原型验证,见 §6。

### 2.4 组件 props / children / snippet

- `#[component] fn Button(...) -> impl View`:属性宏生成 typed-builder 风格的 props 结构(缺省值 `#[prop(default)]`、`#[prop(into)]` 转换),但**不得重写函数体**(保 IDE)。
- Svelte 5 snippet 的对应物就是**闭包 prop**:`children: impl Fn() -> View`(缺省 snippet)与命名带参 snippet `row: impl Fn(&Todo) -> View`。宏内提供声明语法(见 §4 样例),编译为闭包传参。捕获规则与事件闭包一致(Copy 句柄),无借用问题。

---

## 3. proc-macro 工程实践要点

### 3.1 解析器:rstml 可作起点,但按"会 fork"预期

- syn 2 / quote / proc-macro2 是地基,无争议。
- rstml v0.12 提供现成的:JSX 风格节点树、**recoverable parsing(带错、仍出树)**、自定义节点与属性解析、span 化错误。可直接支撑 MVP。
- 但两点注意:① rstml 面向 HTML 语义(doctype/fragment/unquoted text),我们是**自定义元素集合的 retained scene tree**,要用其 custom node 扩展点并砍掉 HTML 包袱;② 其发布节奏慢(上一版 2024-07),核心维护人少,**做好 fork 自维护的预算**。Dioxus 的先例:干脆自研 parser 换取对 `if/for/match`、热重载元数据的完全控制。建议:MVP 用 rstml fork,IR 稳定后视需要重写 parser——因为编译器是独立库,parser 可替换。

### 3.2 错误恢复 = IDE 体验的生命线

具体做法(结合 r-a 机制推导 + Leptos/Dioxus 实践):

1. parser 永不 panic、永不"只报错不出树";残缺输入产出"尽量完整"的 AST + 错误列表。
2. codegen 对残缺 AST 仍生成**类型上可编译**的代码骨架,把所有已解析的用户表达式(带原 span)嵌进去,再附上 `compile_error!`(带精确 span)报告问题——这样 r-a 的 speculative expansion 能在半截输入里找到光标对应的表达式位置,补全存活。
3. 属性名拼错这类"领域错误"用 span 指向属性名本身,并生成"最近似候选"提示(did-you-mean)。
4. 测试基建:`trybuild` UI 测试锁错误消息与 span;再加一组"残缺输入必须仍展开成功"的 snapshot 测试——这类测试业界少见,但它是我们 DX 的护城河。

### 3.3 span 保真

- 用户表达式 token 原样透传;框架生成代码用 `quote_spanned!(user_span => ...)` 让类型错误落回用户写的位置。
- 典型陷阱:属性值经过多层辅助函数包装后错误指到框架内部——对包装函数加 `#[track_caller]`、并让包装调用本身携带用户 span。

### 3.4 静态部分提取:template cloning 在 retained scene tree 的等价物

Svelte 的做法:静态 HTML 拼成 `<template>`,运行时 `cloneNode(true)`,再按预计算路径找到动态节点挂 effect。retained 场景树的等价物:

1. **编译期**:把每个模板的静态结构编译成 `const` 数据——节点类型表、层级(父子索引)、静态属性值、动态绑定表(第 i 个绑定 → 节点 j 的属性 k)。这正是 Dioxus `&'static Template` 的思路。
2. **运行期实例化("stamp")**:按模板一次性在 arena 里批量分配节点、写入静态属性(静态样式对象可全实例共享一份,静态子树的 layout 约束可预计算),返回动态槽位的节点句柄数组。
3. **更新**:每个动态绑定编译成一个细粒度 effect,直接持有节点句柄写属性——**无 diff、无 VDOM**,与 Svelte 编译输出同构。
4. **红利**:Template 是纯数据 ⇒ 可序列化 ⇒ dev 模式下 CLI 重新解析宏、diff 模板、热推送到运行中的 app(Dioxus 已验证);未来做鸿蒙真机远程预览(参考 Slint 1.17 remote viewer)也是同一条数据通道。

---

## 4. 推荐语法样例

假想包名 `svelte-rs`,runes 风格 API:`state()` / `derived()` / `effect()`,句柄皆 `Copy`。

### Counter

```rust
use svelte_rs::prelude::*;

#[component]
fn Counter() -> impl View {
    let count = state(0i32);
    let doubled = derived(move || count.get() * 2);

    view! {
        <Column gap=8 padding=16>
            <Text>"count = " {count} " · doubled = " {doubled}</Text>
            <Button on:click=move |_| count.update(|n| *n += 1)>"+1"</Button>
            if count.get() > 10 {
                <Text color=theme::WARN>"That's a lot of clicks!"</Text>
            }
        </Column>
    }
}
```

### Todo List(keyed each、bind:value、snippet/children)

```rust
use svelte_rs::prelude::*;

#[derive(Clone, Store)]          // Store 派生:字段级响应式投影(见未决问题)
struct Todo { id: u64, text: String, done: bool }

#[component]
fn TodoApp() -> impl View {
    let todos = store(Vec::<Todo>::new());
    let input = state(String::new());
    let next_id = state(0u64);

    let remaining = derived(move || todos.iter().filter(|t| !t.done().get()).count());

    let add = move |_| {
        let text = input.get();
        if text.trim().is_empty() { return; }
        let id = next_id.get();
        next_id.set(id + 1);
        todos.push(Todo { id, text, done: false });
        input.set(String::new());
    };

    view! {
        <Column gap=12 padding=16>
            <Row gap=8>
                <TextInput bind:value=input placeholder="What needs doing?" on:submit=add />
                <Button on:click=add>"Add"</Button>
            </Row>

            for todo in todos [key: todo.id().get()] {
                <Row gap=8 align=Align::Center>
                    <Checkbox bind:checked=todo.done() />
                    <Text strikethrough={todo.done().get()}>{todo.text()}</Text>
                    <Button kind=ButtonKind::Ghost
                            on:click=move |_| todos.retain(|t| t.id != todo.id().get())>
                        "✕"
                    </Button>
                </Row>
            }

            if remaining.get() == 0 {
                <Text dim=true>"All done 🎉"</Text>
            } else {
                <Text dim=true>{remaining} " item(s) left"</Text>
            }
        </Column>
    }
}
```

### 带 snippet 的组件(Svelte 5 snippet 的对应)

```rust
view! {
    <DataList items=todos>
        // 命名 snippet:编译为 `row: impl Fn(StoreRow<Todo>) -> View` prop
        snippet row(todo) {
            <Text>{todo.text()}</Text>
        }
        snippet empty() {
            <Text dim=true>"Nothing here yet."</Text>
        }
    </DataList>
}
```

要点:`for ... [key: ...]` 编译为 keyed reconciler;`bind:` 只接受可写信号(编译期检查);`store()` 行句柄 `todo` 是 `Copy` 投影,闭包捕获零借用冲突;所有用户表达式 span 原样保留。

---

## 5. 演进路径:原型 → 1.0

**M0 · 先写"编译输出",再写编译器**(Svelte 团队的方法论):手写 3~5 个组件的目标展开代码(Template 静态数据 + effect 绑定),跑通 retained tree + signals 运行时,确认 codegen 形态(数据式、单态化轻)达到编译时间与性能目标。此阶段零宏。

**M1 · `view!` MVP**:编译器独立库 crate(`svelte-rs-compiler`:parser/IR/codegen),proc-macro 壳 crate 调它。parser 基于 rstml fork(custom nodes + `if/for/match/snippet` 扩展)。`#[component]` 只做 props builder,不碰函数体。trybuild 错误快照测试起步。

**M2 · DX 及格线**:错误恢复展开(残缺输入补全存活测试)、`svelte-rs-fmt`(对标 leptosfmt/dioxus autofmt)、tree-sitter grammar(高亮,可直接参考 tree-sitter-rstml)。

**M3 · 热重载**:Template 序列化 + dev CLI(watch → 重解析 → diff → 推送);评估集成 Subsecond 做逻辑热补丁(已有 Bevy/Iced 集成先例);远期做鸿蒙真机远程预览通道(对标 Slint 1.17 `--remote`)。

**M4 ·(可选)外部文件前端**:仅当出现真实的设计师协作需求时,以 askama 式 derive(`#[template(path="app.svrs")]`)复用同一编译器库落地;届时模板内工具链缺口(高亮/诊断映射)按需补。**不预置承诺。**

**1.0 门槛**:语法冻结(if/for/match/snippet/bind/on)、错误恢复覆盖率指标、四平台(Win/Linux/macOS/OHOS)模板热重载可用、编译时间基准(千组件级 app 冷/热编译)达标。

---

## 6. 未决问题(需原型验证)

1. **深层响应性**:`Store` 派生 + 字段级投影(lens)能否在保持 `Copy` 句柄与借用检查友好的同时,支撑 `bind:checked=todo.done()` 与 keyed each 的行级更新?对标 Leptos `reactive_stores`,需专门原型。
2. **rstml fork 的长期成本** vs 自研 parser 的时点:`if/for/match/snippet/bind:` 扩展在 rstml custom-node 框架内做到什么程度会开始别扭?
3. **Subsecond 在 OpenHarmony 上的可行性**:官方支持 Web/桌面/iOS/Android,OHOS 的动态加载与签名策略是否允许二进制热补丁,完全未验证。
4. **数据式 codegen 的量化收益**:同一组件集在"typed-builder 泛型树"与"Template 数据 + dyn 回调"两种输出下的编译时间/二进制体积/运行时开销对比基准。
5. **`match`-based `{#await}`** 的人体工学:`AsyncState` 枚举 + `<Suspense>` 边界在真实异步 UI(取消、竞态、重试)下是否够用。
6. 标签名/属性名补全的最终形态:tree-sitter 之外,是否值得做一个只管模板域符号的轻量 LSP 扩展(明确不做 Slint 全家桶)。

---

## 7. 来源

- Dioxus 0.7 发布公告(2025-09-08,Subsecond 细节与限制):https://dioxuslabs.com/blog/release-070/
- Dioxus 0.7 热重载文档(rsx 模板级热重载能力边界):https://dioxuslabs.com/learn/0.7/essentials/ui/hotreload/
- Dioxus v0.7.0 Release / 热补丁 PR #3797:https://github.com/DioxusLabs/dioxus/releases/tag/v0.7.0 · https://github.com/DioxusLabs/dioxus/pull/3797
- rstml(recoverable parsing、custom nodes、v0.12.0):https://github.com/rs-tml/rstml · https://crates.io/crates/rstml
- tree-sitter-rstml:https://github.com/rayliwell/tree-sitter-rstml
- Leptos 0.8(erase_components 等):https://github.com/leptos-rs/leptos/releases/tag/v0.8.0 · https://docs.rs/crate/leptos/latest
- Leptos DX 文档(r-a 配置、leptosfmt):https://book.leptos.dev/getting_started/leptos_dx.html
- rust-analyzer 与宏的 IDE 机制:https://rust-analyzer.github.io/blog/2021/11/21/ides-and-macros.html · https://lukaswirth.dev/posts/ide-proc-macros/ · https://github.com/rust-lang/rust-analyzer/issues/11014
- Slint LSP / live-preview:https://github.com/slint-ui/slint/blob/master/tools/lsp/README.md
- Slint 1.15 / 1.17(remote viewer):https://slint.dev/blog/slint-1.15-released · https://slint.dev/blog/slint-1.17-released
- Svelte runes 介绍 / 版本现状(Svelte 5 为当前大版本,5.56.6):https://svelte.dev/blog/runes · https://endoflife.date/svelte
- Makepad(自定义 live DSL 的投入规模参照):https://github.com/makepad/makepad · https://makepad.nl/
- Rust OpenHarmony 目标与 ohos-rs(平台上下文):https://doc.rust-lang.org/rustc/platform-support/openharmony.html · https://github.com/ohos-rs/ohos-rs

**仅基于训练数据、未联网核实的点**:Leptos tachys 类型级视图树导致单态化开销的归因细节;askama 的错误映射方式;Dioxus `Template` 的具体数据结构;Leptos `reactive_stores` 的字段级订阅机制;r-a 索引 OUT_DIR 生成代码的行为。以上均为训练期内稳定事实,风险低,但引用前建议抽查源码确认。
