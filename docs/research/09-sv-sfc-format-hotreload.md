# 09 · `.sv` 单文件组件格式设计 + 编译器路线的热重载红利

> 探索方向:不走 proc-macro,走**独立编译器**——`.sv` 单文件组件(script 是真 Rust + runes,
> template 是 Svelte 语法)由自研编译器变换为 Rust 代码 + 模板数据。
>
> 调研日期:2026-07-17。关键事实已联网核实:Dioxus 0.7.9(2026-07-12)与 subsecond 0.7.9
> 的热重载机制与限制、Slint 1.17.1(2026-07-07)interpreter/live-preview 架构、Svelte 5
> 现状(仍为当前大版本,2026-07 仍在发 5.x patch,无 Svelte 6)与完整指令清单、Vue SFC spec。
> 未能联网核实的点在文末单独标注。

---

## 0. 结论先行(TL;DR)

1. **文件结构选 Svelte 型,不选 Vue 型**:顶层内容即模板,`<script>` / `<style>` 是文件中的
   块,各至多一个。不引入 Vue 的 `<template>` 包裹、custom blocks、`src` 外链——模板是主角,
   少一层嵌套,与"模板语法 100% Svelte"的心智一致。
2. **script 块 = 100% 合法 Rust(syn 可整体解析)**。编译器魔法一律长成宏调用形状
   (`sv::props! {}`),runes 是普通函数(`state()/derived()/effect()`,`$` 不是合法 Rust
   token,`$state` 进不了 Rust 语法)。这一条保住三样东西:rustfmt 直接可用、未来
   Volar 式虚拟文档 LSP 可行、错误映射只需行映射不需语法映射。
3. **模板语法可以也应该做到"语法层 100% Svelte 5"**——独立编译器不受 Rust tokenizer 约束
   (裸文本、`{#if}`、`bind:value`、`it's` 这类撇号文本在 proc-macro 里全是雷)。但**语义层
   按桌面场景裁剪**:保留 `{#if}{#each}{#key}{#snippet}{@render}{@const}{@attach}`、
   `bind:` / `class:` / `style:`、Svelte 5 事件属性(`onclick={...}`,不是遗留 `on:` 指令);
   `{#await}` 与 `transition:` 家族**保留语法、推迟实现**;砍掉 `{@html}`、`use:`(被
   `{@attach}` 取代,Svelte 5.29+ 官方方向)、`<svelte:element>`、遗留 slot。
4. **style 块 = 极简"类 + 属性"静态样式语言**,不是 CSS 也不是 Rust 表达式:封闭属性集 =
   `sv_ui::Style` 字段(未来扩到 taffy 属性),类名天然 scoped(编译成组件私有样式表索引,
   零运行时选择器匹配),值只允许字面量与 `@theme` token。**动态样式一律走模板里的
   `style:` 指令**。这条边界同时就是热重载的"数据面/代码面"分界线——style 块 100% 落在
   数据面,改任何样式都不碰 rustc。
5. **热重载是编译器路线的最大红利,架构核心是"两面产物"**:编译器把每个 `.sv` 编成
   **数据面**(模板结构表 + 样式表,可序列化,不经 rustc 可替换)+ **代码面**(表达式闭包、
   事件 handler、script 体,经 rustc)。codegen 强制拆 `setup`(script,持有 signals)/
   `render`(按模板数据 stamp + 按槽位接线):数据面变更 → 只重放 render,setup 作用域不动,
   **状态天然保留**。表达式槽位按(源码 hash,出现序)匹配——已有表达式可移动/复制/删除,
   新表达式必须 rustc。这个能力边界与 Dioxus 0.7 实测一致(联网核实);**明确不做 Rust
   表达式解释器**——Slint 能全解释是因为表达式是自家小 DSL,我们的表达式是真 Rust,
   解释器等于再造半个 rustc,死路。
6. **生成代码必须人类可读**(rustfmt 级排版 + 源位置注释 + 行映射表),放 OUT_DIR:
   `sv-build`(build.rs)兜底保证裸 `cargo build` 可用,`svc` CLI 叠加 dev 服务器/热重载/
   诊断映射。`.sv` 之间的 import **就是 Rust `use`**(生成模块树镜像 src 目录),不发明
   import 语法;组件签名统一 `Component` trait(`Props` 结构 + `mount`),兼作热重载所需的
   组件注册表。
7. **与 view! 宏共享同一编译器核心(同一 IR、同一 codegen)**,双前端并存:`.sv` 前端 =
   完整 Svelte 语法 + 热重载 + 设计师可读;`view!` 前端 = rust-analyzer 全程在场的 IDE 体验。
   IDE 是 `.sv` 路线的最大税(近期 tree-sitter 高亮 + 诊断映射及格,远期 Volar 式 LSP),
   双前端策略让这个税不是赌注而是选项。

---

## 1. 文件结构:Svelte 型 vs Vue 型

两家 SFC 的结构差异(Vue SFC spec 与 Svelte 5 文档,均已核实):

| 维度 | Vue `.vue` | Svelte `.svelte` | `.sv` 取舍 |
|---|---|---|---|
| 模板位置 | 必须包在 `<template>` 里 | 顶层裸写,非 script/style 的内容即模板 | **Svelte 型**:模板是主角,少一层缩进 |
| script | `<script>` 与 `<script setup>` 二选一 | `<script>` + 可选 `<script module>` | 单一 `<script>`;**不设 module 块**(Rust fn 体内本来就能声明 item;跨组件共享的 item 写普通 `.rs`,少一个概念) |
| style | 多个,`scoped`/`module` 可选 | 至多一个,默认 scoped | 至多一个,**永远 scoped**(无全局样式概念,主题走 token) |
| 外链/扩展 | `src` import、`lang` 预处理、custom blocks | 无 | 全不要。`lang` 无意义(script 只能是 Rust),custom blocks 是生态税 |
| 元数据 | — | `<svelte:options>` | `<sv:options edition="..." name="..."/>` |

**块提取规则**(与 Svelte 相同的两阶段解析):先按顶层标签切出 `<script>`/`<style>`,余下顶层
内容按出现顺序拼成模板。script 内容切取到**首个不在 Rust 字符串/字符/注释字面量中的**
`</script>`——比 Svelte 对 JS 的处理更严谨(我们反正要做 Rust 词法扫描),Rust 代码里含
`"</script>"` 字符串的病例因此天然无害。

一个完整的 `counter.sv`,后文所有讨论都锚定它:

```svelte
<script>
  sv::props! { start: i32 = 0 }

  let count = state(start);
  let doubled = derived(move || count.get() * 2);
  let add = move |_| count.update(|n| *n += 1);
</script>

<view class="card">
  <text>count = {count} · doubled = {doubled}</text>
  <button onclick={add}>+1</button>
  {#if count.get() > 10}
    <text class="warn">That's a lot of clicks!</text>
  {/if}
</view>

<style>
  .card { direction: column; gap: 8; padding: 16; bg: #202028; radius: 6; }
  .warn  { fg: @theme.warn; }
</style>
```

注意 `That's` 里的撇号:proc-macro 路线里这是词法错误(`'s` 被 Rust lexer 当 lifetime/字符
字面量),必须写成字符串字面量;独立编译器路线裸文本随便写——这是"不受 Rust tokenizer
约束"最直观的收益。

---

## 2. script 块:真 Rust + 宏形状的编译器魔法

**铁律:script 块整体必须能被 `syn` 解析为语句序列。** 编译器不发明任何 Rust 之外的语法,
所有需要编译器介入的地方都伪装成宏调用——syn 把宏调用当不透明 token 树收下,我们再改写:

- `sv::props! { name: Type [= default] [, ...] }`:声明 props。生成 `pub struct XxxProps`
  (typed-builder 风格,带默认值的字段可省略)。`#[bind]` 标注对应 Svelte 5 `$bindable`:
  ```rust
  sv::props! {
      label: String,
      start: i32 = 0,
      #[bind] value: Signal<String>,   // 允许父组件 bind:value;类型必须是可写信号
  }
  ```
  编译期检查:`bind:` 只能指向 `#[bind]` 字段,且模板中 `bind:` 到只读 `Derived` 直接报错
  ——比 Svelte 的运行时警告更强的静态保证(04 号报告已论证,此处继承)。
- runes 就是 `sv-reactive` 的函数:`state/derived/effect/batch/untrack/on_cleanup`。
  **不引入 `$state` 符号**:`$` 在 Rust 词法里只存在于宏定义内部,强行支持意味着 script
  不再是合法 Rust,rustfmt/未来 LSP 全部报废——为了一个符号丢掉整条工具链,不值。
- script 顶层语句序列 = 组件 setup 体,原样(span/行号保真)进入生成代码。Rust 允许 fn 体内
  声明 `use`/`struct`/`fn`/`impl`,所以不需要 Svelte 的"module/instance"两级——这是 Rust
  白送的简化。

**作用域规则**:script 顶层绑定对模板全部可见(同 Svelte);模板中 `{#each ... as item}`、
`{#snippet name(x)}`、`{:then v}` 引入块级绑定;`{@const}` 在块内声明局部量。snippet 仅模板
作用域可见(同 Svelte 5),script 不能引用模板里定义的 snippet。

---

## 3. 模板语法:100% Svelte 语法层,桌面语义裁剪

Svelte 5 全量清单(svelte.dev 文档导航,已核实):块 `{#if}{#each}{#key}{#await}{#snippet}`;
标签 `{@render}{@html}{@attach}{@const}{@debug}`;指令 `bind: use: transition: in: out:
animate: style: class`;runes;特殊元素 `<svelte:boundary/window/document/body/head/element/
options>`。事件在 Svelte 5 已从 `on:` 指令改为**事件属性**(`onclick={...}`),指令清单里
已无 `on:`。

逐条裁决(判断依据:retained 场景树没有 HTML/CSS/DOM 事件冒泡,但有布局、交互态、动画、
异步):

| Svelte 构造 | v0 裁决 | 桌面语义 / 理由 |
|---|---|---|
| 文本插值 `{expr}` | **保留** | 编译为 `bind_text` 槽位。自动追踪靠 trait:生成 `move \|\| (expr).sv_display()`,`SvDisplay` 对 `T: Display` 与 `Signal<T: Display>` 双实现——`{count}` 直接写信号即可,不用 `.get()`,拿到 Svelte 的顺手且不需要编译器知道类型 |
| `{#if}{:else if}{:else}` | **保留** | → `if_block`(已有)。条件是真 Rust 表达式 |
| `{#each expr as pat, i (key)}` | **保留** | → `each_block`;`(key)` 编译 keyed reconcile(ADR-7);`{:else}` 空列表分支保留(廉价) |
| `{#key expr}` | **保留** | 表达式变则重建子树,`if_block` 变体,实现近乎免费 |
| `{#snippet}` / `{@render}` | **保留** | 组件 API 的基石(取代 slot);编译为闭包值,`{@render row(x)}` 即调用。组件标签体内的 `{#snippet}` 编译为对应闭包 prop |
| `{@const}` | **保留** | 块内局部 `let`,零成本 |
| `{@attach expr}` | **保留** | **命令式逃生舱**:`expr: impl Fn(NodeHandle)`,挂载时拿到节点句柄(focus/测量/自绘钩子),返回值实现 Drop 则卸载时清理。同时覆盖 `bind:this` 与 `use:` 的职责,三合一 |
| `{#await}` | **保留语法,推迟实现** | 桌面确实需要(文件/网络),但正确语义依赖 `resource()` 原语与取消/竞态设计(04 号报告 §6.5)。v0 parser 接受、报"未实现"诊断,格式不留破坏性变更 |
| `bind:value` 等 | **保留** | 白名单属性(value/checked/focused + 组件 `#[bind]` props);展开为 读 + 写 两条(04 号报告 §2.3);编译期拒绝只读信号 |
| `class:name={cond}` | **保留** | 与 style 块联动:切换组件样式表中的类(见 §4),编译为布尔槽位,翻转时打/摘 StyleClass 并重算节点样式 |
| `class="a b"` | **保留** | 静态类集合,纯数据 |
| `style:prop={expr}` / `style:prop="lit"` | **保留** | 直接绑定 `Style` 字段(`style:gap={n}`),动态样式的唯一通道;字面量形式落数据面 |
| 事件属性 `onclick={h}` | **保留** | 跟随 Svelte 5(不是 `on:` 指令)。修饰符(`\|once` 等)Svelte 5 已删,我们也不做。v0 事件集:click,后续 hover/focus/key/input 随 sv-ui 事件模型扩 |
| `transition:` `in:` `out:` `animate:` | **保留语法,推迟实现** | 桌面动画是差异化点,但依赖帧调度(ADR-6)。语法进 v0 spec,实现排在帧调度后。砍掉的话格式 v1 要破坏性扩展,不划算 |
| `{@html}` | **砍** | 场景树没有 HTML。富文本未来另设元素(`<rich>`),不复用这个口子 |
| `use:action` | **砍** | 与 `{@attach}` 职责重叠,Svelte 官方也在向 attachment 迁移(5.29+)。一个机制够了 |
| `<svelte:element this={...}>` | **砍** | 动态标签名对封闭元素集意义稀薄,需要时用 `{#if}` 分支 |
| `<svelte:window>` 等 | **砍,留变体** | HTML 宿主对象无对应物;但 `<sv:window title={...} min-width=...>` 是真实桌面需求(窗口属性绑定),列入 v1 议程,v0 不做 |
| `<svelte:boundary>` | **砍(v0)** | 错误边界值得要(panic 隔离),但依赖 effect 层的 catch_unwind 设计,推迟 |
| `{@debug}` | **保留** | 编译为 `inspect` 日志,几行代码的事 |
| 遗留 slot / `on:` / `export let` | **不进格式** | 我们没有历史包袱,只对齐 Svelte 5 runes 世代 |
| 属性展开 `{...props}` | **砍(v0)** | 强类型 props 下语义含糊(哪些字段?),需要 builder 级别的设计,v1 再议 |

**文本转义**:模板文本里字面 `{` 写作 `{'{'}`(一个 Rust char 表达式),无需 HTML entity
——插值本身就是转义机制,不加词法特例。

---

## 4. style 块:极简静态样式语言

### 4.1 设计目标与否决项

目标按优先级排:① 100% 静态数据(热重载数据面);② scoped 零成本(无运行时选择器匹配);
③ 与 `sv_ui::Style` 一一对应,不发明 CSS 没有的概念,也不搬 CSS 的历史债。

三个被否决的方案:

- **CSS 子集(带选择器/级联)**:选择器匹配要运行时引擎,级联要 specificity 规则——桌面
  场景树用户根本没要这些,Slint 也没做级联。否。
- **style 块里写 Rust 表达式**:任何表达式都把样式拖进代码面,改个颜色也要 rustc,热重载
  红利直接清零。动态样式已有 `style:` 指令承接。否。
- **不要 style 块,全部内联 Rust(`style:` + 常量)**:可行但把"设计师可调的静态外观"和
  "逻辑"搅在一起,且丢掉样式热重载最顺手的形态(改数值即时生效)。否。

### 4.2 语法与语义

```svelte
<style>
  @tokens { accent: #4a9eff; row-gap: 8; }

  .card         { direction: column; gap: @row-gap; padding: 16; bg: #202028; radius: 6; }
  .card:hover   { bg: #26262e; }
  .warn         { fg: @theme.warn; }        /* 引用全局主题 token */
  .done         { fg: #888; strikethrough: true; }
</style>
```

- **只有类规则**:`.name[:state] { prop: value; ... }`。无元素选择器、无后代组合器、无级联
  ——一个节点的最终样式 = `Style::default()` ⊕ 静态属性 ⊕ 按声明序叠加的激活类 patch ⊕
  `style:` 动态绑定(优先级最高)。全部是确定性的、编译期可算的合成,没有 specificity。
- **属性集封闭**,即 `Style` 字段(direction/gap/padding/bg/fg/font-size/width/height/
  radius…),未来随 taffy 扩(flex-grow/align/justify…)。未知属性 = 编译错误 + did-you-mean。
- **伪状态** `:hover :pressed :focused :disabled`:每个类编译成
  `StyleClass { base: StylePatch, hover: Option<StylePatch>, ... }`;交互态由 sv-ui 事件层
  置位后合成。v0 spec 收录、实现可只做 base(交互态随事件模型落地)。
- **token**:`@tokens` 声明组件局部常量;`@theme.*` 引用应用级主题表(运行时查表,支持
  深色模式切换)。token 引用仍是数据(名字索引),不破坏数据面。
- **scoped 语义**:类名编译为**组件样式表的索引**(u16),模板里 `class="card"` 在编译期
  解析成索引——类名根本不出现在运行时,跨组件不可能"选中",scoped 是免费的、绝对的。
  样式不穿透组件边界;父组件想影响子组件外观,走 props 或 theme token,不走样式泄漏。
  未引用的类、引用不存在的类,都是编译期诊断(对齐 Svelte 的 unused-css 警告)。

---

## 5. 热重载架构:编译器路线的最大红利

### 5.1 两条已验证路线(联网核实,2026-07)

**路线 A:Dioxus 0.7 —— 模板数据 diff + 二进制热补丁。**当前版 0.7.9(2026-07-12)。

- **模板级(毫秒,不碰 rustc)**:rsx parser 同时活在编译期与 devtools 里;`dx serve` 监听
  源码,用 `dioxus-rsx-hotreload`(`diff_rsx`/`collect_from_file`)对新旧 rsx 做 diff,经
  WebSocket 把新模板推给运行中的 app。**免重编译的边界**(官方文档,已核实):元素结构
  增删改、字符串型属性、格式串里**挪动已有**表达式、单 token 字面量 props、if/for 体内的
  标记结构。**必须重编译**:上一次编译中不存在的新表达式/新变量、逻辑改动、组件签名、
  import。—— 关键机制含义:热更新的模板只能**引用旧二进制里已编译的表达式池**,这是
  一切"数据 diff 式热重载"的共同天花板。
- **代码级(Subsecond,亚秒,实验性)**:`dx serve --hotpatch`。机制(docs.rs subsecond
  0.7.9,已核实):函数调用经**跳表**间接化(`subsecond::call` / `HotFn`);外部编译器只重编
  变更部分、按运行中进程的地址链接(运行时上报 `main` 实际地址修正 ASLR 偏移),生成新
  `JumpTable` 推给 app,`apply_patch` 换表。**限制**(已核实):只追踪 tip crate(workspace
  依赖 crate 的改动不感知);statics 保留但析构不跑、改初始化器不生效;**结构体布局变更
  不支持**,框架须丢弃重建实例;**被补丁 crate 的 thread-local 会重置**;平台 Linux/macOS/
  Windows/iOS 模拟器/Android/WASM(iOS 真机因签名不支持)。Bevy 已集成(官方文档提及;
  0.7 发布公告另提 Iced)。

**路线 B:Slint —— 解释器。**当前版 1.17.1(2026-07-07)。`slint-interpreter` 在运行时直接
加载执行 `.slint`,零代码生成;LSP 的 live-preview、`slint-viewer`、Node/Python 绑定全骑在
它上面(已核实)。1.17 的 `slint-viewer --remote` 把"桌面改文件、真机看效果"跑通。
**全解释 = 一切皆热**(包括表达式和回调),代价是:表达式语言必须是自家可解释的小 DSL,
且要终身维护"解释器与编译器语义一致"。

**对我们的裁决**:我们的表达式是真 Rust——解释路线等于自研 Rust 解释器(miri 级工程),
**排除**;数据 diff 路线(A)与我们"生成数据而非类型"的既定方针(ADR-2)完全同构,而且
独立编译器让我们比 Dioxus 更干净:Dioxus 的模板数据 diff 是 dev 工具对宏调用的"体外再解析",
我们的模板数据是**正式编译产物本身**——dev/release 同构,不存在"CLI 里的 parser 和宏里的
parser 语义漂移"这类问题。Slint 留给我们的是**远程预览**思路(数据通道推到鸿蒙真机),
不是解释器。

### 5.2 svelte-rs 设计:数据面 / 代码面

编译器把一个 `.sv` 编成两面:

```
counter.sv ──sv-compiler──┬── 代码面(counter.rs,经 rustc)
                          │     · setup():script 体原样 + props 展开
                          │     · 表达式槽位闭包表(binders):文本/条件/each 源/事件/each 行、if 分支的 render fn
                          │     · Component impl(mount = setup + render)
                          └── 数据面(Template + StyleSheet,不经 rustc)
                                · 节点结构表(静态层级、静态文本、静态属性、类索引)
                                · 槽位引用表(第 n 个动态位 → 槽位 id)
                                · 样式表(StyleClass 数组 + token 引用)
```

数据结构骨架(release 下是 `static` 常量,dev 下从注册表加载、可整体替换):

```rust
pub struct Template {
    pub id: &'static str,            // "src/counter.sv#0"(文件 + 块序号,嵌套块各有子模板)
    pub roots: &'static [TNode],
    pub sig: &'static [SlotSig],     // 槽位签名:(kind, 源码 hash, 出现序)——热重载匹配依据
}
pub enum TNode {
    Elem { kind: ElemKind, classes: &'static [u16], attrs: &'static [(AttrId, Lit)],
           children: &'static [TNode] },
    Text(&'static str),
    DynText { slot: u16 },           // → binders[slot] 的 Fn() -> String
    Block   { slot: u16 },           // if/each/key/await 子块,闭包携带子模板引用
    Comp    { comp: u16, props: u16 }, // 组件注册表索引 + props 构造槽位
    Event   { .. }, StyleBind { .. }, ClassToggle { .. }, Attach { .. },
}
```

**codegen 强制拆 setup / render**(这是热重载的承重墙):

```rust
impl Component for Counter {
    type Props = CounterProps;
    fn mount(doc: &Doc, parent: ViewId, props: CounterProps) {
        let scope = setup(props);                 // ① script 体:signals、handlers、binders 表
        create_scope(|| render(&scope, doc, parent, tpl("src/counter.sv#0"))); // ② 数据驱动 stamp
    }
}
```

`render` 只做两件事:按 `Template` 数据批量建节点(stamp),按槽位表把 binders 里的闭包
接到 `bind_text` / `if_block` / `each_block` / `set_on_click` 上。**它自身不含任何用户表达式**
——所以给它换一份新 `Template` 数据重跑,不需要 rustc。

**热重载流程**:文件变 → `svc` 重跑 sv-compiler 前端(纯解析,毫秒级)→ 对比新旧 `sig`
(槽位签名序列):

- **签名集合是旧的子集(可少、可重排、可重复引用)** → 数据面热更新:序列化新
  Template/StyleSheet 推给运行中 app;app 在 UI 线程对该模板的每个活动实例:销毁 render
  子作用域(sv-reactive 的 effect 所有权树已支持,`{#if}` 分支销毁同款机制)→ 用新数据
  重跑 render → 重绘。**setup 作用域未动,script 里的 signals 原值保留**——计数器改模板,
  计数不清零。
- **出现新签名(新表达式/改了表达式源码)** → 走 rustc:触发 `cargo build`,后续可选接
  Subsecond 热补丁把"重启"降级为"亚秒补丁"。

**免 rustc / 需 rustc 清单**(对锚定示例):

| 改动 | 通道 |
|---|---|
| `+1` 改成 `Add`、加一个 `<text>hi</text>`、把 `{doubled}` 挪到另一行、删掉整个 `{#if}` 块 | 数据面,毫秒 |
| `gap: 8` → `12`、`.warn` 换色、加 `:hover` 规则 | 数据面(样式表),毫秒 |
| 复制一份 `{count}`、把 `{#if}` 块换个父容器 | 数据面(槽位重复/移动) |
| `count.get() > 10` → `> 5`、`onclick` 闭包体任何字符、script 任何字符、props 定义、`use` 新组件 | rustc(可叠 Subsecond) |
| 给**已 import** 的组件加一个新实例 `<Button .../>`(props 全字面量) | 数据面——组件引用走注册表索引,这是独立编译器优于 Dioxus 的一格 |

v1 优化方向:**字面量提升**——把模板表达式里的数字/字符串字面量抽成数据面槽位
(Dioxus 对字面量 props 做了同类事),让 `> 10` 改 `> 5` 也免 rustc。v0 不做,留接口。

**Subsecond 集成的三个关键注意**(由已核实的限制推导):

1. **"只补 tip crate"反而保护我们**:用户 app 是 tip crate,`sv-reactive`/`sv-ui` 是依赖
   crate——补丁不会重置依赖 crate 的 thread-local,我们的 thread-local 响应式 runtime 与
   场景树安然无恙;要是 runtime 也被补,thread-local 重置会直接炸。framework 代码自身的
   热补丁(我们开发框架时)享受不到,接受。
2. 包裹点:事件分发循环与组件 mount 调用处包 `subsecond::call`;补丁后按 Dioxus/Bevy 惯例
   丢弃 UI 重建(setup+render 全重跑),状态靠 signal 快照恢复(v1)或接受重置(v0)。
3. **OHOS 不在支持平台列表**(签名/动态加载策略未验证,04 号报告已标注)——鸿蒙的开发
   体验靠**远程预览**补:数据面本来就走序列化通道,把本地 WebSocket 换成推到真机的
   WebSocket 就是 `slint-viewer --remote` 的同构物,模板/样式改动在真机毫秒生效,Rust
   改动回退到重部署。

### 5.3 热重载 MVP 步骤清单

1. **抽编译器核心**:把 `sv-macro` 的 parse/ir/codegen 抽成独立库 crate `sv-compiler`
   (无 proc-macro 依赖);IR 增补 Snippet/Key/ClassToggle/StyleProp/Attach/Component 节点、
   样式表、带稳定 id + 源码 hash 的表达式槽位。
2. **Template 数据结构落地**(`sv-ui` 或新 `sv-template`):`TNode` 树 + `SlotSig` + serde
   (dev 用 JSON 便于人查,通道用 postcard);实现 `stamp(doc, parent, tpl, binders)`;
   `if_block/each_block` 改造成槽位可驱动。
3. **codegen 拆 setup/render**:release 走 `static TEMPLATE`,dev 走
   `HotRegistry::get(id)`(`#[cfg(sv_hot)]` 切换);mount 时把实例登记进按 template id 索引的
   weak 列表。
4. **`svc dev`**:`notify` watcher → 重跑前端 → `sig` 比对 → 子集则序列化推送(本地
   WebSocket/named pipe),否则触发 `cargo build` 并在终端/覆盖层提示"需要重编译的原因 +
   位置"。
5. **运行时 HotClient**:收到新数据 → UI 线程调度:逐实例销毁 render 子作用域 → 新数据
   重跑 render → bump 版本重绘。样式表推送同通道,只需替换样式表 + 对打了类的节点重算
   `Style`。
6. **验收 demo**(counter + todo):改文本/加节点/挪插值/调样式/加已 import 组件实例 →
   <100ms 生效且 signal 状态保留;改表达式 → 正确回退 rustc 并给出精准提示。快照测试:
   同一 `.sv` 的 数据面输出 在仅改代码面时逐字节不变(防误伤)。
7. **M+1**:Subsecond 评估集成(包裹点 + 补丁后全量重建策略);signal 快照/恢复
   (`#[hot(preserve)]` 标注,serde)让 rustc 路径也尽量保状态。
8. **M+2**:远程预览通道(WebSocket 到鸿蒙/另一台桌面真机),对标 `slint-viewer --remote`。
9. **持续**:热重载语义测试矩阵(每类模板改动 × 状态保留断言)进 CI——这是 Dioxus 靠
   issue 反馈慢慢补的地方,我们从第一天当作契约。

---

## 6. 编译产物形态

**人类可读:是,且是硬要求。**理由:① 调试器/panic 回溯落在生成码上,可读性直接决定排障
体验;② "编译器生成了什么"是这类框架的信任基础(Svelte 的 playground 把编译输出当教学
工具);③ 审计/安全评审需要。做法:prettyplease 排版 + 每段前注释源位置
(`// ── counter.sv:3-9 <script> ──`)+ 文件头"generated, do not edit";`svc expand foo.sv`
即看(对标 cargo-expand)。script 体逐行原样拷贝,**同时输出行映射表**
(`counter.sv.map.json`:生成行 ↔ 源行),`svc check/build` 包一层 `cargo --message-format=json`
把 rustc 诊断重写回 `.sv` 坐标——这就是 04 号报告说的"跨文件 span 工程税",用行映射 +
诊断重写付掉,不追求 proc-macro 级的表达式内 span(rust-analyzer 内联诊断到不了 `.sv`,
诚实标注为路线代价)。

**放哪 & 怎么触发**:`sv-build`(build.rs)扫 `src/**/*.sv` → OUT_DIR 生成镜像模块树,用户
crate 一行 `sv::include_components!();` 挂载——保证**裸 `cargo build` 可用**(CI/新人零门槛),
r-a 会索引 OUT_DIR 生成码,组件 API(Props/mount)在 Rust 侧有补全跳转(04 号报告已核实
r-a 行为)。`svc` CLI 是叠加物(dev/热重载/fmt/诊断映射),不是必需品。不把生成码 checked-in
(diff 噪音 + 手改风险)。

**签名约定**:`counter.sv` → `mod counter`,导出 `pub struct Counter;`(单元类型)+
`pub struct CounterProps`(builder,含默认值/`#[bind]`)+
`impl Component for Counter { type Props; fn mount(&Doc, ViewId, Props) }`。trait 统一三件事:
手写 Rust 组件与 `.sv` 组件互操作、热重载组件注册表(名字 → 构造器)、未来 `view!` 宏产物
同型。

**`.sv` import `.sv`:就是 `use`。**生成模块树镜像目录结构后,`use crate::ui::button::Button;`
天然工作;模板里 `<Button/>` 按 script 作用域解析(同 Svelte:import 即可用)。不发明
`import "./button.sv"` 语法——发明它意味着自建路径解析/重导出/可见性半套模块系统,而 Rust
的 `use` 全都有。副作用:`.sv` 文件必须位于 crate 源码树内并被 sv-build 扫到,可接受。

---

## 7. 格式演进与生态

- **语法版本化**:双层。`[package.metadata.sv] edition = "v0"`(crate 级,cargo 惯例)+
  `<sv:options edition="v0"/>` 按文件覆写(渐进迁移用)。编译器支持当前与前一 edition,
  配 `svc migrate`(对标 svelte-migrate 的 5.0 自动迁移)。v0 期间明确"不承诺稳定";
  1.0 冻结 = §8 spec + 热重载语义矩阵。
- **fmt**:`svc fmt` 三段式——script 喂 rustfmt 子进程(stdin,保注释;prettyplease 丢注释,
  只配生成码不配用户码);模板用自研 printer(属性换行/块缩进规则抄 Svelte 官方 prettier
  plugin 的成熟决策);style 块规则平凡。printer 挂在 sv-compiler 的 AST 上,与编译共享
  parser——fmt 是编译器核心的第一个"白送"前端。
- **编辑器高亮**:`tree-sitter-sv` = 改造 tree-sitter-svelte(结构同构),injection 从 JS/TS
  换成 Rust——script 与 `{expr}` 内直接得到 Rust 语法高亮,几乎是配置量级的工作。
- **LSP(诚实的代价核算)**:`.sv` 内的 Rust 表达式,rust-analyzer 看不见——这是 `.sv`
  路线相对 `view!` 的最大税(04 号报告的核心论据,此处不翻案)。路线图:
  ① 近期:tree-sitter 高亮 + `svc check` 诊断映射(保存时出错误,及格线);
  ② 远期:`sv-language-server` 走 **Volar 式虚拟文档**——为每个 `.sv` 维护生成 Rust 的
  内存版本,自己作为 LSP 中间层把补全/hover/定义请求经行映射转发给挂在生成码上的
  rust-analyzer,再把结果映射回来。先例充分:svelte-language-server 对 TS 做的正是这个,
  volar.js 已把该模式框架化;但对 r-a 做代理无现成轮子,列为**未验证的最大工程风险**。
  ③ 对冲:双前端并存(见下),重 IDE 的用户用 `view!`,`.sv` 不必独扛。
- **与 `view!` 宏共享编译器核心(同一 IR)**:

  ```
  .sv 文件 ──parse_sv(自研 lexer,Svelte 语法)──┐
                                                 ├─→ 模板 IR ──┬─→ codegen_rust(setup/render)
  view! tokens ──parse_view(现 sv-macro,保留)──┘   (共享)   ├─→ codegen_data(Template/StyleSheet)
                                                               ├─→ fmt printer
                                                               └─→ 诊断/查询(未来 LSP)
  ```

  两个前端**语法必然不同**(`{#if}` 进不了 Rust tokenizer,`view!` 里用 Rust 原生
  `if/for`——ADR-2 已定),但**语义严格同构**:同一 IR、同一 codegen、同一运行时原语。
  `view!` 产物同样走"数据 + 槽位闭包"形态,于是**宏路线也白得模板热重载**(Dioxus 已证明
  宏内模板可 diff 热推);`svc migrate --to-sv / --to-macro` 的双向转换因 IR 无损而可行。
  这就是"格式演进不是赌注"的结构保障:哪条前端赢了用户,核心资产都不作废。

---

## 8. v0 规格骨架(EBNF 级)

### 8.1 词法要点

- **Rust 表达式切取**:`{` 之后做括号平衡扫描(`{} () []`),正确跳过 Rust 字符串/char/
  注释字面量,取到匹配 `}` 为止,再交 `syn::parse_str::<Expr>` 出 AST(Svelte 对 JS 用
  acorn 同理)。pattern/type 同法(`parse_str::<Pat>/<Type>`)。
- **文本**:非 `<`、`{` 的字符序列;`{` 一律开表达式/块/标签,字面 `{` 写 `{'{'}`。
  空白折叠:行内连续空白折为一格,纯空白文本节点丢弃(场景树无 white-space 语义)。
- **错误恢复**:parser 永不 panic,残缺输入出"尽量完整"的 AST + 诊断列表(fmt/LSP/热重载
  前端共用此性质;继承 04 号报告 §3.2 的要求)。

### 8.2 文法

```ebnf
File         ::= ( Script | Style | Options | Node )*
                 (* Script、Style、Options 至多各一;其余顶层 Node 依序构成模板 *)

Script       ::= '<script>' RustStmts '</script>'
Style        ::= '<style>' Sheet '</style>'
Options      ::= '<sv:options' Attribute* '/>'

(* ── 模板 ── *)
Node         ::= Element | ComponentUse | TextRun | Mustache | Block | Tag | Comment
Comment      ::= '<!--' text '-->'

Element      ::= '<' elem-name Member* ( '/>' | '>' Node* '</' elem-name '>' )
                 (* elem-name ∈ 封闭元素集:view | text | button | …(随 sv-ui 扩) *)
ComponentUse ::= '<' CompPath Member* ( '/>' | '>' ( SnippetBlock | Node )* '</' CompPath '>' )
CompPath     ::= UpperIdent ( '::' Ident )*      (* 须在 script 作用域可解析 *)
                 (* 组件体内:裸 Node* 编译为默认 children snippet;SnippetBlock 编译为命名闭包 prop *)

Member       ::= Attribute | Directive | AttachTag
Attribute    ::= attr-name ( '=' Value )?        (* 省略值 ≡ ={true} *)
Value        ::= '{' RustExpr '}'
               | '"' ( attr-text | '{' RustExpr '}' )* '"'   (* 插值字符串 *)
               | number | ident                              (* 无引号单 token 字面量 *)
Directive    ::= 'bind:'  bind-name  '=' '{' RustExpr '}'
               | 'class:' class-name ( '=' '{' RustExpr '}' )?   (* 省略 ≡ 同名变量 *)
               | 'style:' prop-name  '=' Value
               | ( 'transition:' | 'in:' | 'out:' | 'animate:' ) Ident
                 ( '=' '{' RustExpr '}' )?                   (* v0:仅语法保留 *)
AttachTag    ::= '{@attach' RustExpr '}'

TextRun      ::= text
Mustache     ::= '{' RustExpr '}'

Block        ::= IfBlock | EachBlock | KeyBlock | AwaitBlock | SnippetBlock
IfBlock      ::= '{#if' RustExpr '}' Node*
                 ( '{:else' 'if' RustExpr '}' Node* )*
                 ( '{:else}' Node* )?
                 '{/if}'
EachBlock    ::= '{#each' RustExpr 'as' RustPat ( ',' Ident )? ( '(' RustExpr ')' )? '}'
                 Node* ( '{:else}' Node* )? '{/each}'
KeyBlock     ::= '{#key' RustExpr '}' Node* '{/key}'
AwaitBlock   ::= '{#await' RustExpr '}' Node*
                 ( '{:then' RustPat? '}' Node* )? ( '{:catch' RustPat? '}' Node* )?
                 '{/await}'                                  (* v0:仅语法保留 *)
SnippetBlock ::= '{#snippet' Ident '(' ( SnipParam ( ',' SnipParam )* )? ')' '}'
                 Node* '{/snippet}'
SnipParam    ::= RustPat ':' RustType
Tag          ::= '{@render' RustExpr '}'
               | '{@const' RustPat '=' RustExpr '}'
               | '{@debug' Ident ( ',' Ident )* '}'

(* ── style ── *)
Sheet        ::= ( TokenSet | ClassRule )*
TokenSet     ::= '@tokens' '{' ( Ident ':' PropValue ';' )* '}'
ClassRule    ::= '.' Ident Pseudo? '{' ( Decl )* '}'
Pseudo       ::= ':hover' | ':pressed' | ':focused' | ':disabled'
Decl         ::= prop-name ':' PropValue ';'
PropValue    ::= number | color | ident | bool
               | '@' Ident ( '.' Ident )*        (* @token / @theme.name *)
color        ::= '#' hex3 | '#' hex6 | '#' hex8

(* ── script 内编译器识别的宏 ── *)
PropsMacro   ::= 'sv::props!' '{' ( PropDecl ( ',' PropDecl )* ','? )? '}'
PropDecl     ::= OuterAttr* Ident ':' RustType ( '=' RustExpr )?
                 (* OuterAttr ⊇ #[bind] ;script 其余内容 = 任意合法 Rust 语句 *)
```

### 8.3 语义要点(spec 附则)

1. script 块整体必须通过 `syn` 解析;`sv::props!` 至多一次且在顶层。
2. `{expr}` 文本槽位经 `SvDisplay` 求值(`T: Display` 或 `Signal<T: Display>` 自动追踪)。
3. `bind:` 目标必须是可写信号(`Signal<T>` / `#[bind]` prop);指向 `Derived` 为编译错误。
4. `class="…"`/`class:` 引用的类必须在本文件 style 块声明;未使用的类产生警告。
5. 每个 `{#…}` 块与组件体 snippet 编成独立子模板(独立 id、独立槽位空间)——热重载的
   替换单元。
6. 槽位签名 = (kind, 表达式源码 hash, 同 hash 出现序);数据面热更新要求新签名集合 ⊆ 旧
   集合(引用可重排、可重复、可缺省)。

---

## 9. 结论与建议

1. **先做 §5.3 的 1–3 步**(编译器核心抽库 + Template 数据化 + setup/render 拆分):这三步
   同时服务 `.sv` 前端、`view!` 宏、热重载三件事,是无悔投入;`.sv` parser(第 4 步起)在
   IR 稳定后再上,风险被隔离。
2. `.sv` 路线的**卖点排序**:热重载天花板(数据面毫秒级 + 状态保留)> 100% Svelte 语法
   (迁移心智零折扣、裸文本/指令不受 Rust 词法绑架)> 设计师可读性;**代价排序**:
   模板内 r-a 缺席(Volar 式 LSP 未验证)> 诊断映射工程税 > 自研 lexer/printer 维护面。
   双前端共享 IR 是对冲这笔账的结构性答案,应作为 ADR 固化。
3. 样式语言坚持"静态数据 + 封闭属性集 + 零选择器",把一切动态性推给 `style:` 指令——
   这条纪律是热重载数据面的边界,破一次(比如允许 style 块写表达式)就再也守不住。
4. 热重载语义(哪些改动免 rustc、状态如何保留)从第一天写成测试矩阵进 CI,当作对用户的
   契约而不是尽力而为的 dev 糖。

**未决问题**:① Volar 式 LSP 代理 rust-analyzer 的可行性 spike(最大风险项);
② 槽位 hash 匹配在宏调用/格式化差异下的稳定性(是否需要 token 级规范化);③ `{#await}`
配套的 `resource()` 取消/竞态语义;④ Subsecond 在 OHOS 的替代方案只剩远程预览,真机
IME/输入在预览通道下如何联调;⑤ 热重载时 `{#each}` 行内局部状态被重建是否可接受
(svelte-hmr 默认也重置局部状态,可作为辩护,但桌面表单场景更敏感);⑥ 样式跨组件
定制(token 之外)是否需要受控口子(如组件显式导出可覆写类)。

---

## 10. 来源

- Dioxus 0.7 热重载文档(模板热重载能力边界、devtools 内 parser):https://dioxuslabs.com/learn/0.7/essentials/ui/hotreload/
- Dioxus 0.7.0 发布公告 / 热补丁 PR:https://github.com/DioxusLabs/dioxus/releases/tag/v0.7.0 · https://github.com/DioxusLabs/dioxus/pull/3797
- subsecond 0.7.9 docs(跳表/HotFn/apply_patch/ASLR/限制/平台/集成方):https://docs.rs/subsecond/latest/subsecond/
- dioxus-rsx-hotreload 0.7.9(diff_rsx/collect_from_file API):https://docs.rs/dioxus-rsx-hotreload/latest/dioxus_rsx_hotreload/
- Subsecond tip-crate 限制 issue:https://github.com/DioxusLabs/dioxus/issues/4160
- Slint LSP / live-preview(slint-interpreter 驱动):https://github.com/slint-ui/slint/blob/master/tools/lsp/README.md
- Slint 1.17(remote viewer)与 1.17.1 当前版:https://slint.dev/blog/slint-1.17-released · https://github.com/slint-ui/slint/releases
- Svelte 5 文档(模板语法全清单:blocks/tags/directives/runes/特殊元素):https://svelte.dev/docs/svelte
- Svelte 现状(2026-07 仍为 5.x,无 Svelte 6;async/await 相关 patch 持续):https://svelte.dev/blog/whats-new-in-svelte-july-2026 · https://github.com/sveltejs/svelte/releases
- Vue SFC spec(块结构/src imports/custom blocks 对照):https://vuejs.org/api/sfc-spec.html
- svelte-language-server / Volar(虚拟文档 LSP 先例):https://github.com/sveltejs/language-tools · https://volarjs.dev/
- 本仓库:`docs/research/04-compiler-strategy.md`(路线对比与"生成数据"方针)、`crates/sv-ui/src/lib.rs`(绑定原语)、`crates/sv-macro/src/ir.rs`(现有 IR,共享核心的种子)

**仅基于训练数据、未联网核实的点**:svelte-hmr 默认重置组件局部状态的行为细节;Svelte
编译器 parse→analyze→transform 三段架构与 prettier-plugin-svelte 的排版决策;volar.js 的
虚拟文档机制细节;`{@attach}` 于 Svelte 5.29 引入的具体版本号;Iced 集成 Subsecond(0.7
发布公告提及,本次核实到的官方文档仅列 Dioxus/Bevy)。以上均为低风险稳定事实,引用前建议抽查。
