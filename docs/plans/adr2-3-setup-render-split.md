# ADR-2 ③ codegen 拆 setup/render — 落地方案

> 状态:**方案**(未实现)。2026-07-22 写于 `feat/roadmap-r3-r4-push`。
> 依据:`docs/research/09-sv-sfc-format-hotreload.md` §5.2/§5.3、DESIGN.md ADR-2 修订版、
> 以及本次对 `crates/sv-compiler/src/{codegen,emit,script,style}.rs` 与
> `crates/sv-ui/src/lib.rs`、`crates/sv-reactive/src/lib.rs` 的通读 + 三组实测。
>
> 前置:ADR-2 ①(共享发射口 `sv_compiler::emit`)**已落地**;
> ②(Template 数据化,`crates/sv-ui/src/tmpl.rs`)**已落地**(commit `7966785`,807 行)。
> 本文初稿写于 ② 之前,§9 曾整表标"未核实";**2026-07-22 对抗性复核已按真实
> `tmpl.rs` 逐条核对完毕**,结论见 §9 与文末「复核记录」。
>
> ⚠ **读之前先读文末的「## 复核记录」**:复核推翻了本文四处结论(§3.3 的两个数字、
> §5.3 表格第二行、§6 S3 的等价性判据),并发现一处**编译不过的硬冲突**(§0.11)。
> 正文里已用【复核修正】标出。

---

## 0 结论先行(裁决清单)

1. **不生成 `Scope` 类型。**调研 09 写的 `setup(props) -> Scope` 在 Rust 里落不了地:
   script 块允许声明闭包(`let add = |_| ...`)、`fn`/`struct`/`impl` item(09 §2 明文允许),
   闭包类型不可命名,item 根本不是字段。生成泛型 `Scope<F1..Fn>` 又会把类型参数灌进
   `render` 签名并沿组件调用链传染 —— 那正是 DESIGN.md §6 风险 4("坚持生成数据而非类型")
   要避免的东西。**真正可落地的形态是:同一个函数体内分三相,`binders` 表是唯一物化的接缝。**
   "Scope" 不是类型,是那张表里的闭包共同捕获的那份环境。
2. **`mount = setup + stamp(tpl, binders)`**,其中 `stamp` 是 **sv-ui 里的运行时函数,不是生成代码**。
   于是 09 说的"render 自身不含任何用户表达式"变成结构事实而非纪律。
3. **对外函数签名一个字不动**:`pub fn counter(doc: &Doc, parent: ViewId[, props: XxxProps])`。
   `sv_shell::run_app`/`render_to_png` 直接吃这个函数指针(见 `examples/counter-sfc/src/main.rs`),
   examples 与 sv-shell 零改动。拆分是**函数体内部**的事。
4. **Binder 六变体**(§2.2),其中 `Sub` 一个变体吃掉 each/snippet/带绑定的块 ——
   闭包管绑定与生命周期,子模板管结构,于是 `{#each}` **行内的标记结构也是数据面**。
5. **修正调研 09 §8.3 第 5 条**:不是"每个 `{#…}` 块都编成独立子模板",而是
   **只有引入新词法绑定的块**(`{#each}` 的模式、`{#snippet}` 参数、`{:then/:catch}` 绑定、
   块内 `{@const}`)才开独立槽位空间;`{#if}`/`{#key}` 共用父模板槽位空间。
   收益:if 分支里的插值可以自由移进移出、分支结构随便改,全部免 rustc。
6. **修正调研 09 §8.3 第 6 条**:槽位签名的匹配键是 `(kind, 规范化 token 流 hash)`,
   **"出现序"不进匹配键**(只留作诊断)。带上出现序会让"复制一份 `{count}`"变成新签名 →
   需要 rustc,而 09 §5.2 的表格自己承诺那一格是数据面。
7. **下调调研 09 §5.2 表格的最后一行**:"给已 import 的组件加一个新实例(props 全字面量)→ 数据面"
   在 v0 **做不到**,组件实例是 `Opaque` binder。要拿到那一格必须先做"字面量提升 + 组件注册表 +
   props 数据化构造",代价与收益不成比例,列 v1。
8. **代价认账**:生成代码可读性会**实打实变差**(结构从"照着模板念的建树代码"变成一坨
   `static` 字面量 + 一张闭包表,panic 回溯落到 `stamp` 里)。这与 ADR-2 记的止血手段
   "生成代码可读"直接冲突,只能减轻不能消除,需要维护者认账 —— 见 §7.1 与 §8。
9. **不可逆的那一步是 S3**(stamp 接管结构)。S1/S2 是可回滚的纯重构,S3 之后生成代码形态
   彻底换了,回退等于回滚 S3+S2。
10. **③ 只做 `.svelte` 前端,`view!` 宏不跟。**宏路径改模板本来就要 rustc,拿不到热重载红利,
    却要照付装箱税与可读性税。emit 词汇表继续共享(ADR-2 ① 的成果不动)。
    【复核修正】最后半句是自欺:S3 之后 `.svelte` 侧不再发射 `create`/`append`/`update_style`/
    `rebuild_closure`,emit.rs 的建树词汇表**只剩 `view!` 一个消费者**,ADR-2 ① 的
    "唯一发射口"名存实亡。这不是反对 ③ 的理由,但必须记在代价栏里(§7.5)。
11. 【复核新增,硬约束】**`stamp` 不能收 `&[Binder]`,必须收 `Rc<[Binder]>`(或等价的
    owned 句柄)。**只要 `TNode::If`/`Sub` 的分支体由 stamp 自己解释(裁决 5、附录 A
    第 2/3 行),分支构建闭包就要满足 `sv_ui::if_block` / `each_block*` 的
    `impl Fn(&Doc, ViewId) + 'static` 约束(sv-ui lib.rs:1240/1268/1313),
    而借用的 binders 切片进不了 `'static` 闭包 —— **本次复核用最小复现实测到
    rustc `E0521: borrowed data escapes outside of function`**。
    连带:§9 要求 4 的签名要改,每个 if/each 块多一次 `Rc::clone`(两个计数器字,
    不额外分配),`binders![]` 要产 `Rc<[Binder]>`。②(tmpl.rs:359)今天的
    `binders: &[Binder]` 之所以够用,正是因为它把整块交给了一个自带 `Rc` 的
    `Wire` 闭包 —— 附录 A 要推翻的恰恰是这条,推翻就得付这笔账。

---

## 1 现状核实(读到的,带位置)

- `crates/sv-compiler/src/codegen.rs:38` `generate()` 产出**一个大函数**:
  `pub fn <name>(doc, parent[, props])`,函数体 = `props_destructure` + `script_stmts` + `body`
  (codegen.rs:118–124)。script 语句与建树代码确实混在同一个函数体里。
- 生成产物实样(`C:/cargo-target/.../out/counter.rs`,144 行):`state/derived/effect` 三行之后
  直接是 `create_view / append / update_style / bind_text / set_on_click / if_block` 的流水账,
  嵌套靠 `{ let __parent = #el; ... }`(codegen.rs:1270)。
- **预克隆**两层(codegen.rs 文件头注释 + `preclones()` codegen.rs:176):节点级
  `{ let x = Clone::clone(&x); ... }`(codegen.rs:301)与重建闭包体级(codegen.rs:1791)。
  `todo_item.rs` 里能直接看到四层嵌套的 `Clone::clone(&label)`。
- `emit.rs` 是唯一发射口(301 行,含形状测试 `emitted_shapes_are_stable`),约定:
  作用域内有 owned 的 `__doc` 与 `__parent`,重建闭包参数是 `&Doc`、体内首行 clone。
- `style.rs` 产出的是 **TokenStream setter**(`ClassStyle.base/hover/active/focus`,style.rs:24–29;
  `parse_style()` style.rs:377)。**数据化必须先改这里**,否则静态样式进不了数据面。
- `emit_element` 的属性面很宽:两个属性循环合计 **约 20 个具名分支**(`onclick`/`bind:value`/
  `bind:checked`/`bind:scrolly`/`@attach`/`aria-label`/`rows`/`placeholder`/… codegen.rs:869–1239)
  **+ 7 个前缀族**(`class:`/`style:`/`bind:`/`transition:`/`in:`/`out:`/`on:`;【复核修正】
  初稿写 6 个,漏了 `bind:` 的通配分支 codegen.rs:1153;`out:` 目前是"已推迟"的报错分支
  codegen.rs:1193)。这个数字决定了"数据面 vs Wire binder"的边界要画在哪
  (§8 的自我质疑指标 2)。
- `emit_element` **本体 788 行**(codegen.rs:489–1276),占 codegen.rs 的 43%;
  `emit_component` 195 行(1277–1471)、`emit_overlay` 约 130 行(1472–)。
  **S3 要重排的就是这 1100 行**,这是估算 §6 的分母。
- 【复核新增,S3 最大的未解结构问题】**一个元素上的静态样式与全部动态样式合成在
  同一个 `bind_style` 闭包里**(codegen.rs:823–838):闭包体依次是
  `静态 setters → class: 条件类 arms → :focus/:hover/:active 块 → style: 指令`,
  **后写覆盖先写**;而 `sv_ui::bind_style`(lib.rs:1226)每次重跑都
  `Style::default()` 全量重设 —— 伪类退出时那些声明是靠"重设"消失的,不是靠回滚。
  于是"静态样式进数据面、动态样式留 binder"**不是对现有语义的分解**:一旦元素有
  任意一条动态样式,静态部分就不能单独搬进 `TNode::Elem.style`,否则 stamp 先
  `update_style` 写的静态值会被随后的 `bind_style` 第一次运行整个抹掉。
  同一个块里还声明了元素局部的 `__hv`/`__ac`/`__fc` 三个 `state`,被样式闭包与
  指针/焦点接线闭包**共同捕获**。见 §7.6。
- 运行时侧:`Signal`/`Derived` 是 `Copy`(sv-reactive lib.rs:811/910);
  effect 存 `Rc<RefCell<dyn FnMut()>>`(lib.rs:69);Doc 的回调槽全是 `Rc<dyn Fn…>`
  (sv-ui lib.rs:341–351);`create_root` 挂在**当前 owner** 之下(lib.rs:751),
  `use_context` 沿 owner 链上溯(lib.rs:716)——所以拆分后 context 天然可达(§2.4)。

---

## 2 拆成什么形状

### 2.1 三相,不是两个函数

```rust
pub fn counter(doc: &::sv_ui::Doc, parent: ::sv_ui::ViewId) {
    let __doc: ::sv_ui::Doc = doc.clone();
    let __parent = parent;

    // ── 相 1 setup:props 解构 + script 体(原样,行号保真)────────────────
    let count  = ::sv_reactive::state(0i32);
    let double = ::sv_reactive::derived(move || count.get() * 2);
    ::sv_reactive::effect(move || { println!("count 变为 {}", count.get()); });

    // ── 相 2 binders:模板里所有用户表达式,扁平一张表 ────────────────────
    let __b: ::sv_ui::tmpl::Binders = ::sv_ui::tmpl::binders![
        /* 0 */ Text(move || { let mut __s = String::new();
                    __s.push_str("Count: "); __s.push_str(&(count.get()).to_string());
                    __s.push_str(" · 双倍 = "); __s.push_str(&(double.get()).to_string()); __s }),
        /* 1 */ Wire(move |__doc, __el| __doc.set_on_click(__el, move || count.update(|__v| *__v += 1))),
        /* 2 */ Wire(move |__doc, __el| __doc.set_on_click(__el, move || count.update(|__v| *__v -= 1))),
        /* 3 */ Wire(move |__doc, __el| __doc.set_on_click(__el, move || count.set(0))),
        /* 4 */ Cond(move || count.get() > 5),
        /* 5 */ Cond(move || count.get() < 0),
    ];

    // ── 相 3 render:一行,数据驱动 ──────────────────────────────────────
    ::sv_ui::tmpl::stamp(&__doc, __parent, &TPL_COUNTER, &__b);
}

// 数据面(release 是 static;dev 走 HotRegistry::get("src/Counter.svelte#0"))
static TPL_COUNTER: ::sv_ui::tmpl::Template = /* …TNode 树 + StyleDecl + SlotSig… */;
```

**为什么不是两个函数。**如果写成 `fn setup() -> CounterScope`,`CounterScope` 得装下
`count`(可以)、`double`(可以)、以及 script 里任何 `let f = |x| …`(**不可能**,闭包类型不可名)
和任何 `fn`/`struct`/`impl` item(**不是字段**)。改用泛型 `CounterScope<F1, F2, …>` 就把
类型参数写进了 `render` 的签名,组件互调时沿调用链传染,并且直接违反
DESIGN.md §6 风险 4 的编译时间纪律。三相方案里 setup 的"作用域"就是函数体本身,
**binders 表把它 Rc 化地持有** —— 热重载重放 render 时不用重跑 setup,正是因为
那张表活着。这是本方案对调研 09 的第一处实质性偏离。

**热重载重放的落点。**重放需要三样:活着的 binders 表、一个能整体清掉的场景子树、
一个能整体销毁的响应式子作用域。

```rust
// 【复核修正】① Template 的字段在 ② 里全是 `&'static`(tmpl.rs:59–65)、
// Template 本身 Copy;热重载推来的模板由 ② 的既定裁决用 Box::leak 泄漏成 'static
// (tmpl.rs 模块头裁决 1),所以这里是 `&'static Template` 而不是 `Cow`/`Rc`,
// 初稿的 `.into()` 是凭空想象的。② 的 §9 要求 7 因此判**已满足(以另一种方式)**。
// ② `binders: &[Binder]`,而 §0.11 已证明 If/Sub 落地后必须是 Rc<[Binder]>。
#[cfg(sv_hot)]
{
    const ID: &str = "src/Counter.svelte#0";
    let __tpl: &'static ::sv_ui::tmpl::Template =
        ::sv_ui::tmpl::hot::get(ID).unwrap_or(&TPL_COUNTER);
    let __c = __doc.create_view();                 // 可清空的容器
    __doc.append(__parent, __c);
    let (_, __root) = ::sv_reactive::create_root(|| stamp(&__doc, __c, __tpl, &__b));
    ::sv_ui::tmpl::hot::register(ID, &__doc, __c, __root, __b.clone());
}
#[cfg(not(sv_hot))]
stamp(&__doc, __parent, &TPL_COUNTER, &__b);       // release:不多一个节点
```

【复核新增,会当场撞墙】`sv_hot` 是自定义 cfg。仓库 CI 跑
`cargo clippy --workspace --all-targets -- -D warnings`(.github/workflows/ci.yml:217),
而 rustc 1.80 起未声明的 cfg 会触发 `unexpected_cfgs` —— 它是 **rustc lint,不是 clippy lint**,
生成代码头上那串 `#[allow(unused_variables, …, clippy::all)]`(codegen.rs:110 附近)
**盖不住它**。全仓当前没有任何 `check-cfg` 声明(已 grep 核实)。
所以 S3 必须同时改 `sv_compiler::build()`:让每个用户 crate 的 build.rs 打印
`cargo::rustc-check-cfg=cfg(sv_hot)`,并在生成代码的 allow 列表里补 `unexpected_cfgs`。
少了这一步,**每个用户项目**一开 `-D warnings` 就红。

**裁决:容器 + 子 root 只在 dev 加。**理由是实测的:全量档已经吃紧
(【复核修正】本机复现值是 30k 节点 `build_ms=10–12`、`frame_avg=118ms`,
**不是初稿写的 18 / 202**;详见 §3.3 与复核记录),
不为 dev 便利给 release 的每个组件实例加一个布局节点。代价是 dev/release 结构差一层 view,
必须配一条纪律:**布局金样测试两种 cfg 各跑一遍**。
`create_root` 挂当前 owner(已核实 sv-reactive:751),所以子 root 不切断 context 链。

【复核质疑】"两种 cfg 各跑一遍"只是**检测**分歧,不是**消除**分歧:那个容器是一个真
flex item,dev 下每个组件实例的布局都与 release 不同 —— 设计师/开发者对着一个
"结构不一样的 app"调样式,正是热重载要消灭的那类错觉。更便宜的替代:**不加容器,
让热注册表记住这次 stamp 在 parent 下占用的子节点区间**,重放时删这段区间再重建。
需要 sv-ui 补一个 `remove_children_range(parent, range)`(今天只有
`clear_children`,lib.rs 内部大量在用),约 20 行 + 一条测试,
换掉一条"dev 与 release 结构不同"的永久纪律,**建议改采这条**。
唯一的前提是同一 parent 下多次 stamp 的区间不交叉 —— 单线程、同步建树,天然成立。

### 2.2 Binder 六变体

```rust
pub enum Binder {
    Text (Rc<dyn Fn() -> String>),                       // 文本插值
    Cond (Rc<dyn Fn() -> bool>),                         // {#if} 条件、class:x={cond}
    Patch(Rc<dyn Fn(&mut Style)>),                       // style:prop={expr}
    Wire (Rc<dyn Fn(&Doc, ViewId)>),                     // 事件/bind:/@attach/aria-label…
    Sub  (Rc<dyn Fn(&Doc, ViewId, &Template)>),          // {#each}/{#snippet}/带绑定的块
    Opaque(Rc<dyn Fn(&Doc, ViewId)>),                    // v0 兜底:组件实例/{#await}/<overlay>
}
```

三条设计要点:

- **`Sub` 是关键。**它接收子模板**作为参数**(由 `stamp` 从父模板数据里取出来传下去),
  闭包只负责"绑定 + 生命周期"(建 `Signal<T>` 行、解构模式、调 `each_block_keyed`),
  结构留在数据面。于是"改 `{#each}` 行内的标记"免 rustc —— 这一格 Dioxus 也有
  (09 §5.1 "if/for 体内的标记结构"),而我们靠类型而不是靠约定拿到。
- **`Wire` 是长尾的泄压阀。**§1 数到的 20 个具名属性分支不必全部搬进 `TNode` 枚举;
  搬进去的只有"stamp 必须解释才能建树"的东西(元素种类、静态文本、静态样式、类索引),
  其余一律是"在这个节点上跑一次任意接线代码"。**数据面因此记得住 Wire 的位置**
  (把 onclick 从按钮 A 挪到按钮 B 是数据面),但记不住它是什么(改 handler 体要 rustc)。
- **`Opaque` 是诚实的欠条。**组件实例的 props 构造是任意 Rust 表达式,数据面表达不了;
  `{#await}`/`<overlay>` 的形参也超出 v0 数据面。它们整块进代码面,**其子树结构也跟着进**
  (这是 `Opaque` 与 `Sub` 的区别,也是把它们分成两个变体的唯一理由:
  `Opaque` 的存在量是"还欠多少数据化"的直接读数,见 §8 指标 2)。

`Rc<dyn Fn>` **不实现 `Fn`**(本次实测:rustc E0277;`Box<dyn Fn>` 实现),
所以 `stamp` 消费 binder 时必须包一层:`let f = b.clone(); bind_text(doc, id, move || f())`。
这一层不额外分配(闭包被 effect 的那个 `Rc` 一起装走),开销见 §3.3。

### 2.3 TNode 侧对 ② 的要求(接口,不是既成事实)

```rust
enum TNode {
    Elem { kind: ElemKind, text: TextSpec, style: &[StyleDecl],
           classes: &[ClassRef], wires: &[SlotId], patches: &[SlotId], children: &[TNode] },
    If   { cond: SlotId, then: &[TNode], els: &[TNode] },      // 共用父槽位空间
    Key  { key:  SlotId, children: &[TNode] },                 // 同上
    Sub  { slot: SlotId, tpl: &Template },                     // 独立槽位空间
    Opaque { slot: SlotId },                                   // 结构不进数据面
}
enum TextSpec { Static(&str), Dyn(SlotId) }
```

`wires` 的**数组顺序 = 今天生成代码里语句的先后顺序**。今天有若干处顺序是有意义的:
`bind_style` 必须先于 `set_on_pointer_enter` 的合成接线(codegen.rs:822–839)、
`set_multiline` 在属性循环之后(codegen.rs:1258)、`bind:scrolly` 延到末尾(codegen.rs:1261)、
`autofocus` 最后(codegen.rs:1264)。拆完之后这个顺序**在 code review 里看不见了**
(它变成一个 `&[SlotId]` 字面量),所以必须有金样测试钉死(§6 S3)。

### 2.4 props / `$bindable` / snippet / context 怎么过缝

| 东西 | 过缝方式 | 是否要改现有约定 |
|---|---|---|
| 组件 props | 在**相 1** 解构(`let XxxProps{..} = props;`,codegen.rs:90 那行原样保留),binder 闭包从这里克隆 | 否 |
| `$bindable` | 类型是 `Signal<T>`,**Copy**(sv-reactive:811),binder 闭包直接捕获,写回父组件照旧 | 否 |
| `{@const}` | 落在**相 2 的构造块**里当普通 `let`:`let __b = { let summary = derived(..); binders![..] };` 顺序即声明序 | 否。且**约束反而放松了** —— 今天 `{@const}` 只对后续兄弟可见(codegen.rs:238),拆完之后它的值已被闭包捕获,数据面把引用它的槽位挪到它"前面"也没问题 |
| `{#snippet}` | 相 2 里 `let name = { let 预克隆…; move |doc, parent, a: T| stamp(doc, parent, TPL_SNIP_k, &binders_k(a)) };` | 否(仍是闭包);作为 prop 传出去时照旧包 `Rc` → `sv_ui::Snippet` |
| `{@render name(args)}` | 一个 `Sub`(有参)或 `Opaque`(参数需要 key 比对时,codegen.rs:363 的 key_block 语义保留) | 否 |
| 组件实例 `<Card …>` | 一个 `Opaque`:闭包里构造 `CardProps{..}` 并调 `card(&doc, parent, props)` | 否,**组件 ABI 一个字不动** |
| context | `provide_context` 在相 1 执行,binder 里的 effect 由 stamp 在同一 owner 链上建 → `use_context` 可达;dev 的子 root 也不断链(create_root 挂当前 owner) | 否,但要一条测试钉死(`context_crosses_setup_render_split`) |

**用户永远不需要手写任何新类型**:暴露面仍然只有 `XxxProps`(今天就有)与 `sv_ui::Snippet`。
`Binders`/`Template`/`SlotId` 全部由 codegen 生成或由 sv-ui 提供。

---

## 3 闭包捕获的现实

### 3.1 signal 免费,普通变量还是要 Clone

`Signal<T>`/`Derived<T>` 是 `Copy + !Send`(ADR-1 的设计目的就是"随意塞闭包"),
**拆成一张表之后一行代码都不用改**:六个 binder 各捕获一份 `count` 的 Copy。

普通变量(props 解构名、script 普通 `let`、each 模式绑定)仍需 `Clone` ——
这是今天就有的文档化约束(codegen.rs 文件头),拆分不改变它,只改变**克隆写在哪**:

- **今天**:节点级 + 重建闭包级两层预克隆,按树形嵌套。`todo_item.rs`(158 行)里
  `Clone::clone(&label)` 出现 **5 次**,最深处嵌在 4 层块里(实测 grep)。
- **拆完**:每个 binder 构造点各克隆一次,**扁平**。
  `Text(  { let label = label.clone(); move || … } )`。
  原值一直活在函数体里(相 2 结束后才 drop),不存在 use-after-move。

净效果:**克隆次数不变**(仍是"每个引用点一次"),**嵌套消失**。这是拆分少数几处
让生成代码更好读的地方。

### 3.2 两层预克隆的纪律必须原样保留

`Sub`/`Opaque` 的闭包是 `Fn`,会被**反复调用**(每行一次、每次重建一次)。
所以今天 codegen.rs:278–279(snippet 的 `pre_capture`/`pre_call`)、
codegen.rs:1736/1763(each 的 `outer_pre`)、codegen.rs:1791(rebuild_closure)
那套"定义处克隆一次、调用处每次再克隆"的两层纪律**一个字不能少**,只是搬到
binder 构造点上。`preclones()`(codegen.rs:176)可以原样复用,输入从"节点 TokenStream"
换成"binder 体 TokenStream"。

### 3.3 装箱代价:实测

下表**已经过第二次独立复现**(2026-07-22,同机、release、CPU 后端)。
标【复现】= 两次测量一致;标【推翻】= 复核测到的与初稿不同,以复核值为准。

| 项 | 数字 | 口径 |
|---|---|---|
| 每模板的 binder 数(counter.svelte) | **6**(1 Text + 3 Wire + 2 Cond) | 按 §2.1 的骨架数 |
| 每模板的 binder 数(TodoItem.svelte) | **8**(3 文本 + 2 点击 + 2 样式修补 + 1 if 条件) | 数 `todo_item.rs` 的绑定调用 |
| 构造一张 8 槽位的表 | **≈60–64 ns/槽位**(3000×8 = 1.43–1.52 ms)【复现】 | `%TEMP%` 微基准,release,三轮 |
| 经 Rc 多一跳的调用开销(**空体**) | **直接 0.86–1.96 ns vs 经 Rc 4.91–5.95 ns**【推翻】 | 初稿写"2.06 vs 1.69,判为 0"**是错的**:空体下这一跳是 **+3～5 ns、相对 3–6×** |
| 经 Rc 多一跳的调用开销(**真实体**:读 signal + `format!`) | 直接 51.0–51.7 ns vs 经 Rc 51.4–56.8 ns → **+0～6 ns,落在噪声里** | 同上;**这才是支持"可忽略"的证据** |
| 建树基线 | 3001 节点 `build_ms=0–1`;15001 节点 `=4`;30001 节点 `=10–12`(≈**0.35 µs/节点**)【推翻】 | `membench --scene rows`,初稿的 18 ms / 0.6 µs 复现不出来 |
| 帧基线(参考) | `--frames 3`(默认):rows 3k `p99=21.0/21.2/23.3`;`--frames 30`:15k `p99=75.3`、30k `p99=145.4`;`--frames 60`:3k `p99=18.06/18.26` | 见下方对"p99 不能当闸"的**推翻** |

**分配次数的增量是 +1/槽位,不是 +2。**今天一个 `bind_text` 已经要为 effect 分配一个
`Rc<RefCell<dyn FnMut()>>`(sv-reactive:69);拆完之后是 binder 的 `Rc` **加**effect 的 `Rc`。
`Rc<dyn Fn>` 与 `Box<dyn Fn>` 都是**一次**分配(Rc 只多两个计数器字),
所以"用 Rc 换取数据面可以重复引用同一槽位"这件事是**白拿的**,没有必要退回 Box。
(`Rc<dyn Fn>` 不实现 `Fn`、`Box<dyn Fn>` 实现 —— 【复现】rustc E0277,`Rc` 那行报错、
`Box` 那行编过。§2.2 末尾那条成立。)

**【推翻】"3k 行 × 8 槽位 ≈ 建树翻倍"是算错了口径。**初稿拿"3000 张表 ≈1.6 ms"去比
"3000 **节点** 的建树基线 1–2 ms"——但 3000 个 8 槽位的 `.svelte` 组件实例根本不是 3000 个节点。
`membench` 的 rows 场景每行 5 个节点(`build_scene` 里 `rows = controls / 5`),
而一个 8 槽位的真实组件(照 TodoItem 数)约 6–8 个节点。所以对照系是
**3000 行 ≈ 21000 节点 ≈ 7 ms 建树基线**,+1.5 ms = **+21%**,不是翻倍。
结论方向不变(确实是最坏面、确实压着闸线),但"接近翻倍"这个说法**高估了约 4 倍**,
不该拿它去 review 里吓人。

**【推翻】"p99 跑间抖动 ~30%,不适合当闸"——抖动是 `--frames` 太小的产物,不是 p99 的性质。**
`membench` 默认 `--frames 3`,而 p99 的取法是 `sorted[(len*0.99) as usize]`(main.rs:207)
—— 3 个样本时它**就是三帧里的最大值**,根本不是百分位。复核用 `--frames 60` 跑两次:
`p99 = 18.06 / 18.26`,**跨跑抖动 ~1%**;CI 的 bench job 本来就用 `--frames 30`
(ci.yml:271)。所以"p99 天生 flaky"是错的,**只要采够帧,p99 完全可以当闸**。
选 `build_ms` 而不是 `p99` 的正当理由只有一条:**这项工作花的是建树时间,不是帧时间**,
拿帧时间当闸是量错了东西。理由要换,结论可以留。

> **验收闸(S2 起)**:membench 增开 `--scene sfc-rows`(行是 `.svelte` 组件),
> 卡 `build_ms` 相对拆分前 **≤ +20%**。
> 【复核修正,否则这道闸不成立】`build_ms` 今天是 `built.as_millis()`(main.rs:214),
> **整数毫秒**;3k 行档的基线只有 0–1,拿它卡 ±20% 是量不出来的。S2 必须同时:
> (a) 把输出改成 `build_us`(微秒,只增字段不改字段 —— membench 文件头有这条纪律),
> 或 (b) 把 `sfc-rows` 的规模顶到 `build_ms ≥ 20` 的档位(照 0.35 µs/节点,约 6 万节点)。
> 建议 (a),(b) 会把一次 CI 跑拉长到分钟级。
> ADR-9 的虚拟化档不受影响:槽位数与**视口**成正比,不与逻辑条目数成正比。

**【复核新增】还有一笔初稿没算的建树开销:永不进入的分支也要付钱。**
裁决 5 让 `{#if}` 与父模板共用槽位空间,于是 `{:else}` 分支里那些插值的 binder
**在相 2 无条件构造**,不管这个分支这辈子会不会渲染。今天不是这样:分支体在
`rebuild_closure` 里(codegen.rs:1791),连同它的 `preclones` 一起,只有真的进分支才跑。
拆完之后:(a) 未走的分支照付 `Rc` 分配;(b) 更要命的是 **`preclones` 变成无条件执行**
—— `{#if}{:else}<text>{big_vec_prop}</text>{/if}` 会在挂载时克隆一份 `big_vec_prop`
并由那个 `Rc` 闭包**永久持有**。这是一条真实的性能/内存回归路径,
用户看不见也解释不了。S2 的验收里要加 `unused_branch_binder_does_not_clone`
(或明确认账:文档写清"if 分支里引用的普通变量会在挂载时克隆")。

---

## 4 与 ② 的接缝:槽位编号不变量

这是整套方案里**最容易静默出错**的地方:数据面说"这里是 slot 3",binders 表第 3 项
必须正好是那个表达式。错位不会 panic,只会让界面显示错东西 —— 最坏的那种 bug。

### 4.1 一次遍历,同源产出(结构性保证)

codegen 侧引入一个只能由分配器构造的不透明 id:

```rust
pub(crate) struct SlotId(u16);            // 无 pub 字段、无 From<u16>

struct SlotTable {
    binders: Vec<TokenStream>,            // 下标 = SlotId,进相 2
    sigs:    Vec<SlotSig>,                // 与 binders 同序,进数据面
}
impl SlotTable {
    fn push(&mut self, kind: SlotKind, sig_src: &syn::Expr, binder: TokenStream) -> SlotId {
        let id = SlotId(self.binders.len() as u16);
        self.sigs.push(SlotSig { kind, hash: norm_hash(sig_src) });
        self.binders.push(binder);
        id
    }
}
```

`TNode::DynText { slot: SlotId }` 只接受 `SlotId`,而 `SlotId` **只能**从 `push()` 拿到。
于是"数据面引用了一个不存在 / 不对应的槽位"在 Rust 类型层面就写不出来 ——
**这是不变量的第一道也是最强的一道闸,它不是测试,是类型**。
两份产物来自同一次 IR 遍历、同一个调用,天然不会漂移。

### 4.2 三道校验(纵深防御)

1. **codegen 单测** `slot_ids_are_dense_and_referenced`:遍历生成的 TNode 树收集被引用的
   slot 集合,断言 `== 0..sigs.len()`(既无空洞也无越界)。空洞意味着分配了 binder 却没人用
   —— 那是一个真 bug(表达式被静默丢弃)。
2. **生成代码里的编译期断言**:`const _: () = assert!(TPL_COUNTER.sig.len() == 6);`
   ——`6` 是 codegen 直接写死的字面量,与 `binders![]` 的元素个数同源。表长度对不上,
   **用户的 `cargo build` 当场红**,不用等运行。
3. **`stamp` 的运行时校验**:release 走 `debug_assert_eq!(tpl.sig[i].kind, b[i].kind())`;
   **dev 从热通道加载的数据面必须走非 debug 的硬校验**并"拒绝加载 + 报错",不能 panic ——
   热重载数据是外部输入,崩掉正在调试的 app 是最坏处理(与 ADR/R4 去 panic 的口径一致)。

### 4.3 子模板的槽位空间与 id

- **开子模板的唯一条件:该块引入了新的词法绑定。**即 `{#each}` 的模式与索引名、
  `{#snippet}` 的参数、`{:then v}/{:catch e}` 的绑定、以及**块内出现的 `{@const}`**。
  其余(`{#if}`/`{#key}`/纯结构嵌套)共用父模板槽位空间。
  —— `{@const}` 那一条不能漏:槽位空间拉平会把它的可见性从"块内"扩到"块后的所有兄弟",
  是个悄悄的语义放宽。
- **子模板 id 必须内容派生,不能用遍历序号。**若按 09 的 `"src/counter.svelte#0"` 递增编号,
  在文件中间插入一个新块会让**后面所有子模板换 id** → 全部判成"新模板" → 全量 rustc,
  热重载在最常见的编辑动作上直接失效。
  定案:`子模板 id = 父 id + "/" + 该块控制表达式的 SlotSig.hash + "#" + 同 hash 出现序`。
  于是"在 `{#each todos}` 上面加一个 `<text>`"不动它的 id。
  验收:`subtemplate_id_stable_under_sibling_insert`。

---

## 5 热重载真正需要什么:SlotSig 与判据算法

### 5.1 SlotSig 的定义(两处修正 09)

```rust
struct SlotSig { kind: SlotKind, hash: u64 }   // 出现序只进诊断,不进匹配键
```

- **hash 的输入是"runes 改写之后"的表达式的规范化 token 流**,不是 `.svelte` 源码串。
  改写后才是真正编进 binder 的东西;而且这样能抓到"script 把 `count` 从 `$state` 改成普通变量"
  ——源码串一个字没变,改写结果变了,必须走 rustc。
  规范化用 `syn::Expr → to_token_stream().to_string()`,**本次实测**它吞掉空白差异
  (`count.get()  >  5` ≡ `count.get() > 5`)、吞掉块注释与行注释、吞掉 `.` 两侧的空格,
  并且区分 `> 5` / `> 6`。09 §9 把"hash 在格式化差异下是否稳定"列为未决问题 ②,
  这条实测把它关掉。
  【复核修正,别把这条吹过头】复现验证同时测到**两处它不规范化**的地方:
  `(count.get()) > 5` ≠ `count.get() > 5`(多余括号不消);`5` / `5u32` / `0x5` 三者互不相等。
  两者都**失败在安全侧**(多走一次 rustc,不会错绑),但"格式化差异"这个说法要收窄成
  "**空白与注释**差异",因为一个会去冗余括号的 formatter 就能让全表 hash 失配。
  另外:`SlotSig.hash` 的算法**必须钉死**(FNV-1a / 固定种子的 FxHash 之类,
  不引新依赖)。`std::collections::hash_map::DefaultHasher` 的文档明说算法可能随
  Rust 版本变 —— 它一变,dev 通道里"旧二进制的 hash"与"新前端算的 hash"全体失配,
  症状是热重载莫名其妙全量重编,且没人查得出来。
  还有一条得写进协议:**u64 hash 无回退比较,碰撞 = 静默绑错闭包**。
  §5.2 明确允许多对一映射,等于把碰撞后果从"漏匹配"升级成"错匹配"。
  概率极低(1e4 槽位量级 ~1e-12),但 `SlotSig` 应当同时留一份**规范化字符串**
  在 dev 侧(release 编掉)做最终校验,否则这是唯一一类"界面显示错东西且不 panic"的残余风险。
- **出现序不进匹配键**。09 §8.3 第 6 条写的是 `(kind, 表达式源码 hash, **同 hash 出现序**)`;
  但同一模板的同一表达式两次出现在词法上完全等价(共用同一个函数体作用域),
  可以映射到同一个 binder。带上"同 hash 出现序"会让 09 §5.2 表格自己承诺的
  "复制一份 `{count}` → 数据面(槽位重复/移动)"变成需要 rustc,自相矛盾。
  **判据:多对一映射合法。**
  【复核修正,别把这条当成对 09 的"实质偏离"】09 §5.2 的**正文**写的是
  "签名集合是旧的子集(**可少、可重排、可重复引用**)",与本文完全一致;
  本文推翻的只是 09 §8.3 第 6 条那个**结构体注释**。09 是自相矛盾,
  本文站在了 09 自己两处表述里更靠谱的那一边 —— 这是**消歧**,不是新裁决。
  §0.6 的措辞要照此降级。

### 5.2 判据算法

```text
verdict(T_old: Map<TplId, Template>, T_new: Map<TplId, Template>) -> Verdict
  remap := {}
  for (id, tn) in T_new:
      if id ∉ T_old:                              return NeedsRustc(NewTemplate, id 的源位置)
      to := T_old[id]
      # 建旧表的签名索引:sig -> 第一个下标(多对一映射的落点)
      idx := { s.sig -> min(下标) | s ∈ to.sig }
      for each slot s 被 tn 的 TNode 树引用:
          need := tn.sig[s]
          if need ∉ idx:                           return NeedsRustc(NewExpr, need 的源位置)
          remap[(id, s)] := idx[need]
  # T_old 里多出来的模板/槽位一律无所谓(实例会被销毁 / binder 闲置)
  return DataOnly(remap)
```

**最容易做错的一步是 `remap`。**热通道推过去的数据面是**新编译的**,槽位编号是新编的;
运行中的 app 手里只有**旧的** binders 表。所以推送前必须把新数据里的每个 `SlotId`
**改写成旧表的下标**,推的是"重映射后的数据面"。Dioxus 也是卡在这一步。
验收:`hot_remap_to_old_slot_ids`(构造"新增一个静态节点导致后面槽位整体后移"的用例,
断言重映射后旧 binder 仍接在正确的位置上)。

### 5.3 免 rustc / 需 rustc(对 `examples/counter-sfc/src/Counter.svelte` 逐条)

| 改动 | 判据 | 通道 |
|---|---|---|
| `"+1"` 改成 `"Add"`;加一个 `<text>hi</text>`;删掉整个 `{#if}` 块 | 只动 TNode/静态串,不引入新 sig | **数据面** |
| `padding:24` → `12`;**该元素上没有任何动态样式时**改 `.warn` 的颜色 | StyleDecl 是纯数据 | **数据面**(前提:style.rs 已改产值,§9) |
| ~~加一条 `:hover` 规则~~ | 【复核推翻】见下 | **rustc** |
| **元素上已有 `class:`/`style:`/伪类中任意一种**时改它的静态样式 | 【复核推翻】静态部分此时不在数据面 | **rustc** |
| 把 `{double}` 挪到另一行、复制一份 `{count}`、把 `{#if}` 换个父容器 | sig 集合是旧集合的子集(多对一合法) | **数据面** |
| 在 `{#if}` 分支里加 `<text>`、在 `{#each}` 行内加节点 | if 共用父槽位空间;each 行结构在子模板里(`Sub`) | **数据面** |
| `count > 5` → `> 3`;任一 `on:click` 闭包体改一个字符;script 任何改动;`$props` 改签名 | hash miss / 代码面 | **rustc** |
| **新增**一个 `{#if}`、`{#each}`、`<Card/>` 实例 | 引入新 Cond/Sub/Opaque 签名 | **rustc**(09 表格最后一行的"加已 import 组件实例"在 v0 拿不到,见 §0.7) |

**【复核推翻】"加一条 `:hover` 规则 → 数据面"是错的,而且错得不小。**
读 codegen.rs:812–838:`has_hover` 一旦为真,codegen 就会额外生成
`let __hv = ::sv_reactive::state(false);` + `set_on_pointer_enter/leave` 的合成接线,
并把 hover 块塞进那个**唯一的 `bind_style` 闭包**。也就是说,给一个类加 `:hover`
不是"往样式表里加一行数据",而是**给元素长出一个新的响应式状态 + 两个新事件接线 +
把该元素的整份样式从数据面搬进代码面**。这必然引入新槽位签名 → **必须 rustc**。
同理,元素只要已经有 `class:`/`style:`/`:hover`/`:active`/`:focus` 中任意一种,
它的静态样式今天就活在那个闭包里(§1 末尾),连"改 padding"也不是数据面。

这一格是 09 §5.2 表格里**收益最高**的一行(设计侧调参),也是替代形态 B 的全部卖点,
所以必须正面解决而不是含糊过去。三条出路,按代价排:

1. **数据面保留类索引 + 运行时样式表**(= 09 原设计的 `classes: &[u16]` + StyleClass 数组;
   ② 的附录 A 把它记成了"欠条")。伪类状态由 sv-ui 统一管(它已经有
   `set_on_pointer_enter`/`set_focusable`/`on_focus_change` 的完整接线,
   focus.rs/lib.rs 都在用),元素上只留"我引用了哪几个类 + 我要不要 hover 态"。
   这样加 `:hover` 才真是改数据。**代价:sv-ui 多一层样式合成与查表,
   且这是 R2/R3 已冻结的样式路径上的改动。**
2. **`bind_style` 改成不重设、伪类退出时按声明回滚**。会让"整体重算"的语义没了,
   与今天所有依赖它的路径(`class:` 条件类、`style:` 指令)全部对账,风险最高。
3. **认账**:静态样式只在"该元素完全没有动态样式"时进数据面,并把 §5.3 表格改成
   现在这样。收益缩到"纯静态元素的调参",替代形态 B 的估值要跟着下调。

**S3 必须先在这三条里选一条,不能带着 §5.3 原表格开工。**本复核倾向 1(它同时救活
替代形态 B),但没有实测数据说那层查表在 3k 行档上要多少钱,**未核实**。

**v1 才做的字面量提升**(把 `> 5` 里的 `5` 抽成数据槽位)不在 ③ 范围,但 `SlotSig`
预留 `kind` 字段足够表达,不需要为它改格式。

---

## 6 分步落地

每步能独立合入、独立验收。人周为**全职单人**估算,置信度标在每步。

### S0 对齐 ②(0.5 人周,置信度高)—— **已完成,见附录 A + 复核记录**
读主进程落地的 `crates/sv-ui/src/tmpl.rs`,把本文 §2.2/§2.3/§4 的假设逐条核对,
**差异写回本文附录而不是去改 tmpl.rs**。产出:本文 §9 的清单变成"已核实/需协商"两栏。
验收:无代码改动。

### S1 槽位分配器(1–1.5 人周,置信度高)—— **纯重构,生成代码逐字节不变**
在 codegen 内引入 `SlotId`/`SlotTable`,把现有每个表达式发射点改成经 `push()` 分配,
但 binder token **仍原地内联展开**(不建表、不改形状)。
验收:`slot_ids_are_dense_and_referenced`、**逐字节金样**、
既有编译器测试(`sv-compiler/src/lib.rs` 里 **58 个 `#[test]`**,已核实计数)与宏测试零改动。
**可回滚。**

【复核修正】金样**已经存在了**,不用再建:`crates/sv-compiler/tests/golden.rs`(134 行,
2 个测试)+ `tests/fixtures/{wide,child,parent}.svelte` 与对应 `.rs.expected`
(commit `1a07b06`)。fixture 是 `wide.svelte`(铺开语法面)与 `child.svelte`+`parent.svelte`(组件面),
**不是**本文初稿写的 counter/todo/todo_item/showcase,验收里的名字要照实改。
它逐字节比对且刻意不宽容空白,S1 拿它当网正合适。
**但要认清它的有效期:S3 会把三份 `.rs.expected` 全部作废**——生成代码形态整个换了,
金样必须重写,而"重写后的金样对不对"没有独立证据。
换句话说:**这张网在最需要它的那一步自动失效**,S3 的等价性只能靠下面那条,
所以下面那条必须够硬。

### S2 binders 表(2–3 人周,置信度中)
生成 `let __b: Binders = binders![…]`,把 **Text / Cond / Patch / Wire** 四类改成经表消费;
**结构仍由生成代码建**(`create_view`/`append` 还在生成代码里),只是绑定从表里取。
验收:`binders_table_shape_is_stable`(emit 侧形状测试,与 `emitted_shapes_are_stable` 同款)、
`plain_var_cloned_once_per_binder`、四个 example 的端到端行为测试零改动
(`sfc_counter_behaves`/`sfc_counter_keyboard_roundtrip`/todo/overlay-demo)、
新增 membench `--scene sfc-rows` 的 `build_ms` 基线入库。**可回滚。**

### S3 stamp 接管结构(2.5–4 人周,置信度中低)—— **不可逆**
codegen 产出 `static TPL_x: Template`;建树从生成代码搬进 `sv_ui::tmpl::stamp`。
**前置**:style.rs 改成产 `StyleDecl` 值(§9),否则静态样式进不了数据面。
验收:
- ~~`stamp_builds_identical_tree`:`doc.dump()` 逐字符相同 —— 最强的等价性证据~~
  **【复核推翻,这是本方案里最危险的一句话】**读 `Doc::dump()`(sv-ui lib.rs:1156–1188):
  它只输出**元素种类 + 文本 + checkbox 勾选态 + 树形层级 + overlay 分节**。
  它**不含**:任何样式字段(gap/padding/bg/fg/font_size/…)、任何回调槽
  (on_click / on_key / on_focus_change / on_pointer_*)、`focusable` 位、aria-label、
  滚动偏移、TextInput 的 placeholder / multiline / rows、transition、`@attach`。
  也就是说 §2.3 自己列的**四处顺序敏感点(`bind_style` 先于指针接线、`set_multiline`
  在属性循环后、`bind:scrolly` 末尾、`autofocus` 最后)在 `dump()` 里一个都看不见**。
  丢一个 onclick、样式整体错、焦点位没设 —— 全部照样通过。
  顺带:② 现有的 `stamp_matches_imperative`(tmpl.rs:460)自称"语义逐字等价",
  也有同一个问题,它的强度被高估了。
  **改成**:S3 的第一件事是给 sv-ui 加一个测试用的**全量快照**
  (`Doc::dump_full()`:样式的 `StyleDecl::snapshot` + 各回调槽的"有/无" + focusable +
  aria + input 参数),再拿它做拆分前后逐字符对拍。
  这是一笔 §6 原估算里**没有的**工作量(约 0.5 人周 + 一次 sv-ui 改动),
  而且它不在 `docs/plans/` 里,要跟主进程排期。
- `wire_order_matches_statement_order_golden`:钉死 §2.3 列的四处顺序敏感点
  (在有了 `dump_full` 之前,这条是**唯一**能看见顺序的验收);
- 离屏 PNG 金样(counter / showcase / settings)不变 —— 这条覆盖了样式,
  但只覆盖"看得见的那部分",事件与焦点它管不着;
- `cargo build --timings` 或 wall clock 记录 showcase 的编译时间,前后对比入库。

### S4 子模板与块(2.5–4 人周,置信度中低)
`Sub`/`Opaque` 落地:`{#each}`(含 keyed 的 `Signal<T>` 行,ADR-7 语义一个字不能改)、
`{#snippet}`/`{@render}`、组件实例、`{#await}`、`<overlay>`;子模板 id 内容派生。
验收:`subtemplate_id_stable_under_sibling_insert`、`each_row_template_is_data`、
`if_branch_shares_parent_slot_space`、`const_in_block_opens_subtemplate`、
`keyed_each_compiles` 等既有测试零改动、`context_crosses_setup_render_split`。

### S5 sig 与判据(1–2 人周,置信度中高)—— **纯函数,不需要运行时**
产出 `SlotSig` 进数据面 + `sv_compiler::hot::verdict(old, new) -> Verdict` 纯函数 + 重映射。
验收:`hot_verdict_matrix`(§5.3 表格逐行一个用例,这就是 09 §5.3 第 9 条说的
"从第一天当契约"的测试矩阵)、`hot_remap_to_old_slot_ids`、`sig_hash_ignores_formatting`、
`sig_hash_follows_runes_rewrite`。

> **S5 之后 ③ 就完成了。**`svc dev` 的 watcher / 通道 / HotClient(09 §5.3 步 4–5)
> 是独立工程,不在 ③ 范围。

**合计 ≈ 9–14.5 人周,置信度中(±40%)。**参照系:调研 24 对 Parley+AccessKit 估 10–15 人周、
实际落地基本吻合 —— 但那是**有上游可抄的迁移**,③ 是自研机制,更容易低估;S3/S4 的成本
大头不在写代码,在**证明等价**。

**【复核上调】9–14.5 人周低估了,建议改成 13–20 人周,置信度中低(±50%)。**
不是拍脑袋,是把复核发现的、原估算里**根本没有条目**的活儿加进去:

| 追加项 | 出处 | 估 |
|---|---|---|
| `Doc::dump_full()` 全量快照 + 拿它重做等价性对拍 | 上面 S3 那条 | +0.5–1 人周(且要改 sv-ui,跨进程排期) |
| 静态/动态样式合成的解法(§5.3 三条出路选一) | §1 末尾、§5.3 | +1–3 人周(选出路 1 就是上限) |
| `stamp` 签名改 `Rc<[Binder]>` 并贯穿 If/Sub/each | §0.11 | +0.5 人周 |
| `sv_hot` 的 `check-cfg` + 生成代码 allow(不做则用户项目 CI 全红) | §2.1 | +0.2 人周 |
| 三份 `.rs.expected` 金样重写 + 逐段人工 review | S1/S3 | +0.5 人周(**纯人工**,压不下去) |
| `build_us` 口径改造 + `sfc-rows` 场景 | §3.3 验收闸 | +0.3 人周 |
| 未走分支的 preclone 回归(测出来 or 认账写文档) | §3.3 | +0.2 人周 |

另有一条结构性的低估源:`emit_element` 本体 788 行 + `emit_component` 195 行 +
`emit_overlay` ~130 行,S3/S4 要动的是这 1100 行里的**几乎每一处**,
而它们各自都挂着既有测试与既有语义(`out:` 的推迟报错、textarea 的 rows、
overlay 的 anchor/close 枚举…)。原估的 S3 2.5–4 + S4 2.5–4 人周对应到这个体量,
等于每人周消化 150+ 行高密度 codegen 并证明等价 —— 参照 ADR-2 ① 的 `emit.rs`
只有 301 行就单独成了一次提交,这个速率不现实。

---

## 7 风险与代价

### 7.1 生成代码可读性:**明确变差,收不回来**

今天的 `counter.rs`(144 行)是照着 `.svelte` 念的流水账,逐行能对上模板;panic/断点落在
"那个 `bind_text` 调用"上。拆完之后:结构去了一坨 `static` 字面量,代码里只剩一张闭包表,
**panic 回溯落在 `sv_ui::tmpl::stamp` 里**,而 stamp 对所有组件都长一个样。
这与 ADR-2 记录的三层止血的第二层("生成代码可读 + 锚点注释")直接冲突。

能做的只有减轻:
1. Template 字面量每个 TNode 前带源位置注释(`// Counter.svelte:12:3 <text>`)——
   prettyplease 会保留,今天的文件头注释机制(codegen.rs:132)现成;
2. 数据面里带 `slot → 源位置` 表,dev 下 stamp 出错时打印模板位置(release 编掉);
3. 保留 `svc expand --inline` 输出**今天这种**内联形态供人查 —— 但那等于维护两套 codegen
   的一半,**不建议**,写在这里只是说明它是个选项。

**结论:这笔账要维护者认。**如果"生成代码可读"被判为不可让渡(它是 `.svelte` 前端 IDE
体验的第一层止血,DESIGN.md §6 风险 1),那么 ③ 就该停在 S2(见 §8 替代形态 A)。

### 7.2 编译时间:**方向上应改善,但未实测**
模板结构从"3k 行需要类型检查的建树代码"变成"`static` 常量字面量",正是 DESIGN.md §6
风险 4 说的"生成数据而非类型"。但 **`static` 里的大常量对 rustc 也不是白送的**
(常量求值 + 单态化 + 链接),没有实测数据前不许把"编译更快"写进宣传语。
S3 的验收里已经放了 `--timings` 这一项。

### 7.3 运行时开销:**+1 分配/槽位(复现 ≈60–64 ns),调用开销在真实 binder 体下落进噪声**
详见 §3.3。最坏面是列表(3k 行 × 8 槽位 ≈ +1.5 ms 建树,对照 ≈21000 节点的
≈7 ms 基线 = **+21%**,不是初稿写的"接近翻倍"),已配 `sfc-rows` 闸
(但闸本身要先把 `build_ms` 改成 `build_us`,见 §3.3)。
**另有一项初稿漏算:未进入的 `{#if}` 分支也要付 binder 分配与 preclone 的钱**,见 §3.3 末尾。

### 7.4 调试体验
断点从"用户模板对应的生成行"退到"stamp 内部"。缓解同 §7.1。
**新增的一类失败模式**是数据/表错位(§4 三道闸)。

### 7.5 新增的维护面
加一个模板特性从"改一处 `quote!`"变成"改 parser/IR + 改 TNode 枚举 + 改 stamp 解释"。
这条会随特性数量线性放大 —— §8 指标 1 就是盯它的。
【复核补】还要加上:ADR-2 ① 的 `emit.rs` 建树词汇表(`create`/`append`/`rebuild_closure`/
`if_block`/`each_block`)S3 之后**只剩 `view!` 宏一个消费者**,"双前端共享内核"退化成
"一个前端用、另一个前端绕开"。要么接受 emit.rs 缩水成宏专用,要么让 `view!` 也走 stamp
(§0.10 已否决,理由是宏拿不到热重载红利)。**这是 ③ 与 ADR-2 ① 之间的真实张力,
不是可以用一句"词汇表继续共享"糊过去的。**

### 7.6 【复核新增】样式合成:S3 的真前置,比 style.rs 改产值更硬
§1 末尾与 §5.3 已经展开:今天一个元素的静态样式、条件类、伪类、`style:` 指令
**合成在同一个 `bind_style` 闭包**里,靠"每次全量重设 + 后写覆盖先写"实现语义。
把静态那部分单独搬进 `TNode::Elem.style` 会被随后的 `bind_style` 抹掉。
`sv_ui::bind_style_patch`(lib.rs:1510)是**不重设**的那一半,但它没法表达
"hover 退出时把 hover 声明去掉"。
**结论:S3 的前置不止"style.rs 产 `StyleDecl` 值"(§9 末),还有一次样式合成模型的重设计。**
初稿把它整个漏掉了,这是本方案里技术风险最高、也最可能拖垮 S3 排期的一处。

---

## 8 这个方案可能是错的

### 三个月后拆错了,最先暴露的症状

1. **加一个属性要改三处、跨两个 crate。**今天加一个 `.svelte` 属性,diff 集中在
   `codegen.rs` 的一个 match 臂(约 20 行)。拆完之后若它要同时改 `TNode` 枚举 +
   `stamp` 的解释分支 + codegen,**diff 超过 60 行且横跨 sv-compiler/sv-ui**,
   就是数据面边界画得太靠里了。**观察指标:下一个新属性的 diff 行数与跨 crate 数。**
   (对照组现成:调研 26 的 sv-arco 要 `fill_path` + SVG 转译,是一次真实的扩张压力测试。)
2. **`stamp` 长成迷你解释器。**TNode 变体单调增长就是在把 codegen 的 match 搬进运行时,
   而那正是 09 §5.1 判"解释器路线死路"的东西的一半。
   **观察指标:`TNode` 变体数 > 20,或 `Opaque` binder 的占比不降反升。**
   前者说明该留在 `Wire` 的东西被硬塞进了数据面;后者说明数据化根本没推进,
   只是白付了装箱与可读性的税。
3. **热重载迟迟没落地。**③ 的**全部**代价都是替热重载付的。若 S5 完成后三个月内
   `svc dev`(09 §5.3 步 4–5)还没开工 —— 大概率是被 IDE/LSP(DESIGN.md §6 风险 1)
   或人力挤掉 —— 那这些代价就是纯亏损,应认真考虑退回 S2 形态。

### 更简单的替代形态(按性价比排)

- **B. 只把 `<style>` 块数据化。**改颜色/间距是热重载**最高频**的使用场景(设计侧调参),
  而它只需要:style.rs 产 `StyleSheet` 值 + 运行时替换样式表 + 对打了类的节点重算 `Style`。
  模板结构照旧走生成代码,`setup/render` 一点不拆。~~**估 1–2 人周**~~,拿到 09 §5.2 表格里
  第二行的全部收益,零可读性代价。
  **【复核修正】1–2 人周低估了,实际是 2–4 人周。**理由见 §7.6:元素只要有
  `class:`/`style:`/伪类中任意一种,它的静态样式今天就在那个 `bind_style` 闭包里,
  "替换样式表 + 重算 Style"没有作用点。B 要落地就得先做那次样式合成模型重设计,
  而那正是 S3 的硬前置 —— B 的便宜是假象,它和 ③ 共享同一块最难啃的骨头。
- **B′(复核新增,这才是真正的 20% 力气 / 80% 收益)。只把 `:root` 变量做成运行时 token。**
  style.rs 今天已经有 `:root { --x: v }` + `var(--x)`,但它是**编译期文本替换**
  (`substitute_vars`,style.rs:385–410:找到 `var(`、按名字换成字面串,然后才解析)。
  把它改成:token 表进 sv-ui,`var(--x)` 不再产字面量而产一次 `theme.get(TOKEN_X)` 读取。
  **不需要任何新机制** —— 这些 setter 本来就活在 `bind_style` 的 effect 闭包里
  (codegen.rs:823),在里面读一个 signal,精准更新自己就发生了,零 diff、零 stamp、
  零可读性代价、金样只动被 token 化的那几行。
  于是"改主色/改间距/改圆角"当场生效,**而且是在 release 构建里也生效**
  (顺带白拿一个换肤/暗色主题能力,DESIGN.md 里还没有这一项)。
  代价:token 必须有类型(v0 限成"颜色"与"长度"两类,由落点属性推定),
  且只有写成 token 的值才热;写死的字面量不热 —— 但设计侧调参本来就该走 token。
  **估 0.5–1.5 人周,置信度中**(未核实:`substitute_vars` 的调用点是否只有一处、
  token 落进 `Edges`/`Color` 之外的字段时的类型推定有多少特例)。
  **如果只能做一件事,做 B′,不是 B。**
- **A. 停在 S2:只收 binders 表,不做 Template 数据、不上 stamp。**
  拿到:槽位编号不变量、扁平化的预克隆、S5 的判据(判据只依赖 sig,不依赖数据面落地)。
  丢掉:改模板**结构**仍要 rustc。可读性代价小得多(建树代码还在)。
  **这是最有性价比的中途站,也是 S2 之后的天然停车位。**
- **C. 不拆,等 Subsecond。**让亚秒热补丁承担一切。09 §5.1 已核实的限制:
  只补 tip crate、结构体布局变更不支持、被补 crate 的 thread-local 会重置、**OHOS 不支持**。
  它能替代"改表达式"那一半,**替代不了**"改结构/改样式"那一半(那需要重跑 mount,状态就没了)。
- **B + C 组合可能拿到 80% 的收益、20% 的成本。**这是本方案**最可能被推翻的地方**,
  写在这里而不是藏起来:如果维护者读完 §7.1 觉得"生成代码可读"不能让,
  正确的决定是 B + C,而不是硬上 ③。
  **【复核修正】这个组合要改成 B′ + C。**B 与 ③ 共用同一块难啃的骨头(§7.6),
  它不便宜;B′ 才是真正独立、真正便宜、且**不与 ③ 冲突**的那一块
  —— 就算将来上 ③,token 化的样式值照样有用。
  复核的排序:**B′(0.5–1.5 人周,零风险)> C(等上游)> A(停在 S2)> B > 完整 ③**。

### 还有一处可能是错的

**`Sub` 变体接收 `&Template` 参数**这个设计,让"行结构是数据"成立,但也意味着
`stamp` 与 binder 之间有一个**双向**依赖(stamp 调 binder,binder 回调 stamp)。
若某天要把 stamp 换成"批量建树 + 一次性写入 Doc"的优化形态(ADR-9 后续阶梯里的
增量场景编码会想要这个),这个回调结构会挡路。届时的出路是让 `Sub` 返回一个
"待建列表"而不是自己建 —— 那是一次不小的重写。**已知,未解决。**

---

## 9 对 ②(`sv-ui/src/tmpl.rs`)的接口要求清单

按"③ 需要 ② 提供什么"列。初稿写作时 `tmpl.rs` 尚不存在,整表标"未核实";
**2026-07-22 复核已按真实实现(commit `7966785`,`crates/sv-ui/src/tmpl.rs` 807 行)
逐条核对**,状态栏已更新。

| # | 要求 | 为什么 | 状态(复核后) |
|---|---|---|---|
| 1 | `Binder` 至少能表达 §2.2 的六语义(名字可以不同) | 少了 `Sub` 则 each 行结构进不了数据面;少了 `Wire` 则 20 个属性分支要全搬进 TNode | **部分**:② 是 `Text/Style/Click/Wire` 四变体(tmpl.rs:315–324),`Cond`/`Sub`/`Opaque` 需补;`Click` 建议按附录 A 保留 |
| 2 | 槽位 id 是**不透明类型**,不是裸 `u16` | §4.1 的类型级不变量 | **不满足且不该强求**:② 数据面存 `u16`(`Bind::Text(u16)` 等)。附录 A 的裁决("codegen 侧不透明 / 数据面裸 u16")成立,理由是 `static` 里放 newtype 要导出构造函数 |
| 3 | `Template` 带 `sig: &[SlotSig]`,`SlotSig = (kind, u64 hash)` | §5 判据的全部输入 | **已满足**(tmpl.rs:59–92),字段名与本文一致 |
| 4 | `stamp(&Doc, ViewId, &Template, &Binders)`,可**重复调用**且不持有状态 | 热重载重放 = 清子树 + 再 stamp 一次 | **✅ 已改**(2026-07-22):签名换成 `stamp(&Doc, ViewId, &Template, Rc<[Binder]>)`。提前改是为了不在 S2/S3 落地时回头改所有调用方 |
| 5 | `TNode::If/Key` **内联子节点数组**(不是独立 Template) | §4.3 的槽位空间规则,直接决定热重载能力面 | **要改**:② 只有 `TNode::Block{slot}`(tmpl.rs:110)。改动代价见 §0.11 |
| 6 | `StyleDecl` 是**值**,能表达 `Style` 的全部字段 + 类的 base/hover/active/focus 四态 | 静态样式与伪类进数据面(§5.3 第二行) | **一半**:`StyleDecl` 覆盖 `Style` 全部 26 个字段且用解构防漏(tmpl.rs:184–305,做得比本文要求好);**但四态与类索引都没有**——伪类今天在 codegen 侧展开进 `bind_style` 闭包,见 §7.6 |
| 7 | dev 下 `Template` 可以不是 `'static` | 热注册表里的模板不是 static | **以另一种方式满足**:② 明文裁决用 `Box::leak` 泄漏成 `'static`(tmpl.rs 模块头裁决 1),换 `Copy` + 零分支的热路径。本文 §2.1 的代码骨架已按此改正 |
| 8(复核新增) | `Template::hot_swappable_with` 要能表达 §5.2 的**子集 + 多对一重映射** | 判据是 ③ 的最终产物 | **✅ 已满足**(2026-07-22,= S5 的主体):新增 `Template::hot_swap_verdict` 返回 `Verdict::{DataOnly{remap}, NeedsRustc{..}}`,实现子集 / 重排 / 多对一;`hot_swappable_with` 降级为它的一个 bit。新增 `remap_slots` 做槽位改写(Dioxus 卡住的那一步),配 `hot_remap_to_old_slot_ids` 端到端测试。**复核对附录 A 那句"判据算法已有可运行实现"的指控成立**——旧实现是逐位全等,而且它那条测试断言的方向是**错的**(fixture `D` 其实是少了一个槽位、注释却写成"新增插值",于是把合法的子集判成了要 rustc),已一并改正 |

**③ 反过来欠 ② 之外的一件事**:`crates/sv-compiler/src/style.rs` 今天产 `TokenStream` setter
(style.rs:24–29、377),必须改成产 `StyleDecl` 值。这是 S3 的硬前置,
估 **1–1.5 人周**,可以与 S1/S2 并行,**也可以作为替代形态 B 单独交付**。

---

## 10 未核实 / 明确不做

- ~~`tmpl.rs` 的真实形状(§9 整表)~~ —— **已核实**,见 §9 与复核记录。
- 拆分后的**编译时间**方向(§7.2),没有实测不下结论。
- `binders![]` 是否真需要一个宏(也可以是 `vec![Binder::Text(Rc::new(..)), ..]`);
  写成宏只是为了让生成代码短一点,**未验证**它对 rustc 的开销是正是负。
  【复核补】§0.11 定了要产 `Rc<[Binder]>`,宏形态至少省掉每处 `Rc::from(vec![…])` 的样板。
- membench 的 `--scene sfc-rows` **尚不存在**,§3.3 的"3k 行 × 8 槽位 ≈ +1.5 ms / +21%"是
  用微基准的 60–64 ns/槽位与 rows 场景 build_ms 外推的**估算**,不是端到端实测。
- 本文的帧数字全部来自**本机、CPU 后端、2026-07-22 的 `target/release/membench.exe`**,
  **不能用来判断有没有回归**,只能当量级参照。
  【复核修正】初稿的"跑间抖动 ~30%"是 `--frames 3` 的产物(默认值),不是 p99 的性质;
  `--frames 60` 下抖动约 1%。另外初稿的 30k 档数字(18 ms / 202 ms)复现不出来,
  本机复现值是 10–12 ms / 118 ms —— **测量条件没记下来的数字不该进方案**。
- 【复核新增,未解决】`{#if}` 分支体内出现 `{@const}` 时,§4.3(要开子模板)与
  §2.3(`If` 的分支是内联数组)冲突,交点没有定义。S4 前必须补这条 spec。
- 【复核新增,未核实】替代形态 B′(运行时 token)的 0.5–1.5 人周估算;
  §5.3 出路 1(类索引 + 运行时样式表)在 3k 行档上的查表代价。
- 明确不做:Rust 表达式解释器(09 §5.1 已判死路);`view!` 宏跟进 stamp(§0.10);
  组件实例数据化 / 字面量提升(v1)。

---

## 附录 A:S0 对齐结果(2026-07-22,由 ② 的实现者填写)

② 已落地(commit `7966785`,`crates/sv-ui/src/tmpl.rs`)。逐条核对本文 §2.2/§2.3/§4
的假设与既成实现,**分歧一律以本文为准**——理由见每行末列。

| 本文的要求 | ② 的现状 | 裁决 |
|---|---|---|
| `Binder` 六变体 | 四变体 `Text/Style/Click/Wire` | **改**:`Style`→`Patch`(改名对齐);补 `Cond`/`Sub`/`Opaque`。`Click` **保留**——点击是绝对高频且签名 `Fn()` 比 `Fn(&Doc,ViewId)` 少一层,而"不再为别的具体事件加变体"这条纪律不变 |
| `TNode::If{cond, then, els}` 共用父槽位空间 | 统一为 `Block{slot}`,整块交闭包 | **改,且这是本次对齐里最有价值的一条**:我的初版把 if 的**分支结构**也推到代码面,于是"改 if 分支里的标记"要 rustc;本文的形状让它免 rustc。免重编译边界的宽窄直接决定热重载的体感 |
| `TNode::Sub{slot, tpl}`,子模板**作为参数**传给闭包 | `Block{slot}`,子模板由闭包自己引用 | **改**。同上:each 行内的标记结构本该在数据面。闭包只管绑定与生命周期 |
| `TNode::Opaque{slot}` | 无 | **补**。它不只是兜底,更是**可度量的欠条**:`Opaque` 的出现次数就是"还欠多少数据化"的直接读数 |
| `TextSpec::{Static, Dyn}` 互斥 | `label: &str` 与 `Bind::Text` 并存,靠 codegen 自觉 | **改**成互斥枚举。能用类型排除的非法状态就不要靠约定 |
| `classes: &[ClassRef]` | 无——样式在编译期已展开成 `StyleDecl` | **暂不补,但记为欠条**。展开成 decl 之后"改一条类定义"要重编译所有用到它的元素;保留类索引 + 运行时样式表才能让改类定义也免 rustc。代价是运行时多一层查表,**留到热重载通道真的落地时再定**(那时才有数据说它值不值) |
| `SlotId` 不透明、只能由分配器构造 | 数据面直接存 `u16` | **两边都要**:codegen 侧用 `SlotId`(本文 §4.1 的类型级闸门),数据面落到 `static` 时仍是 `u16`——`static` 里放不透明 newtype 会逼着导出构造函数,那反而把闸门开了 |

### ② 里应当保留、本文没提到的三件

1. **`StyleDecl::snapshot()` 用解构写**:`Style` 新增字段会编译期报错。本文 §9 只说
   "style.rs 要改成产 `StyleDecl` 值",没说怎么防"新样式键悄悄没法数据化"——
   这条洞只有用户才发现得了,必须有编译期闸门。
2. ~~**`hot_swappable_with()`**:本文 §5.2 的判据算法在 ② 里已有可运行实现与测试。~~
   **【复核推翻】**② 的实现是 `self.id == next.id && self.sig == next.sig`(tmpl.rs:73–75)
   —— 两个 `&[SlotSig]` 切片**逐位全等**。它与 §5.2 的判据(新签名集合 ⊆ 旧集合、
   允许重排/重复/缺省、并产出 remap)**不是一回事,也不是它的弱化版**:
   ② 判不了"删掉一个插值"(sig 短了 → 不能热换)、判不了"把 `{count}` 挪到另一行"
   (只要挪动改变了槽位序就不等)、更没有 remap。它对应的测试
   `hot_swap_judged_by_slot_signature`(tmpl.rs:715)钉的正是这个强判据。
   S5 要**重写函数 + 重写测试**,不是"接上已有实现"。这条误判会让 S5 的
   1–2 人周估算失真,也会让人以为热重载能力面已经到位。
3. **槽位对不上时跳过而不是 panic**:本文 §4.2 第 3 条与之一致(dev 热通道走硬校验 +
   拒绝加载,release 走 `debug_assert`),② 已按此实现。

### 结论

**S0 完成**:分歧七条,六条改 ②、一条(类索引)记欠条。`tmpl.rs` 的调整不与 S1 冲突
——S1 是 codegen 内的纯重构,不碰数据面类型。建议顺序仍按 S1 → S2 → S3,
其中 `tmpl.rs` 的形状调整并入 S3 的前置(与 style.rs 数据化同批),
理由:**在没有消费者之前调整数据面类型,既没法验证也没法回归**——
② 现有的契约测试(`stamp_matches_imperative` 等 7 条)是照着当前形状写的,
改形状就要重写它们,而重写的正确性只有等 codegen 真的发射数据面时才验得了。

【复核补】"类索引记欠条"这条要重新掂量。附录 A 说"留到热重载通道真的落地时再定",
但 §5.3 与替代形态 B 的**全部收益**都压在这条上(§7.6):没有类索引与运行时样式表,
"改一条类定义"就不是数据面。把它记成欠条 = 把 09 §5.2 表格第二行整行记成欠条,
这个代价没有在附录 A 里被说出来。**建议改成:要么按 §5.3 出路 1 做掉,要么直接采
复核提的 B′(运行时 token)—— 后者用 1/3 的力气拿到那一行里最高频的那一半。**

---

## 复核记录(2026-07-22,对抗性复核,独立于初稿作者)

立场:默认这份方案是错的,尽力证伪。下面每条要么给源码位置、要么给本次实测命令与数字;
没做到的一律写"未核实"。**复核只改了本文件,没碰 crates/ / examples/ / .github/ / 其它 docs/。**

### A. 复现/核实过、**成立**的部分

初稿的「现状核实」这一节质量很高,抽查的行号**全部对得上**,没有编造:

- `codegen.rs:38 generate()`、`:176 preclones()`、`:238 {@const} 逐节点推进作用域`、
  `:278–279 snippet 的 pre_capture/pre_call`、`:301 节点级预克隆`、`:363 {@render} 的 key_block`、
  `:1258 set_multiline`、`:1261 bind:scrolly`、`:1264 autofocus`、`:1270 嵌套 { let __parent = }`、
  `:1736/1763 each 的 outer_pre`、`:1791 rebuild_closure` —— 逐条 `sed` 核对,全中。
- `style.rs:24–29 ClassStyle{base,hover,active,focus}: TokenStream`、`:377 parse_style` —— 对。
  "style.rs 今天产 TokenStream setter,数据化必须先改这里"成立。
- sv-reactive `:69 Effect{f: Rc<RefCell<dyn FnMut()>>}`、`:716 use_context`、`:751 create_root`、
  `:811 impl Copy for Signal`、`:910 impl Copy for Derived` —— 全中。
  "create_root 挂当前 owner → 子 root 不切断 context 链"成立。
- sv-ui `:341–351` 回调槽全是 `Rc<dyn Fn…>` —— 对。
- 生成产物:`counter.rs` **144 行**、`todo_item.rs` **158 行**、其中
  `Clone::clone(&label)` 出现 **5 次** —— 全中(`showcase.rs` 是 377 行)。
- `Rc<dyn Fn>` 不实现 `Fn` / `Box<dyn Fn>` 实现 —— **复现**:`%TEMP%` 里两行同时写,
  `Box` 那行编过、`Rc` 那行 `E0277: the trait Fn() is not implemented for Rc<dyn Fn() -> String>`。
- binder 表构造 **≈60–64 ns/槽位**(3000×8 = 1.43–1.52 ms,三轮)—— 复现初稿的 66 ns / 1.58 ms,
  **在噪声内一致**。
- `syn::Expr → to_token_stream().to_string()` 吞空白、吞块注释与行注释、区分 `>5`/`>6`
  —— 复现。09 §9 未决问题 ② 确实可以关掉(但见 B.5 的收窄)。
- 3001 节点 `build_ms=0–1` —— 复现。
- "对外签名一个字不动"—— `run_app(title, build: impl FnOnce(&Doc, ViewId) + 'static)`
  (sv-shell:894)、`render_to_png(build: impl FnOnce(&Doc, ViewId), …)`(:941),
  `examples/counter-sfc/src/main.rs` 直接传函数项。成立。

### B. **推翻**的部分(按严重度)

1. **`stamp(&Doc, ViewId, &Template, &[Binder])` 与"stamp 自己解释 If/Sub"编译不过。**
   `if_block`/`each_block*`(sv-ui lib.rs:1240/1268/1313)的分支闭包是
   `impl Fn(&Doc, ViewId) + 'static`,借用的 binders 切片进不去。
   `%TEMP%` 最小复现给出 **`E0521: borrowed data escapes outside of function`**。
   必须换成 `Rc<[Binder]>`。→ 已写进 §0.11、§9 要求 4。
   **这条是硬的:初稿的裁决 5 与它自己的接口要求 4 互斥,不改就开不了工。**
2. **`doc.dump()` 不能当等价性判据。**`dump()`(sv-ui lib.rs:1156–1188)只输出
   种类 + 文本 + checked + 层级 + overlay 分节;样式、全部回调槽、focusable、aria、
   input 参数、滚动位一概不含。初稿称它"最强的等价性证据"是**错的**,而 §2.3 自己
   列的四处顺序敏感点在它眼里全都隐形。→ 已在 S3 验收里换成"先加 `dump_full` 再对拍",
   并把这笔工作量加进估算。②`stamp_matches_imperative` 的自我评价同样偏高。
3. **§5.3 表格第二行"加一条 `:hover` 规则 → 数据面"是错的。**codegen.rs:812–838:
   `has_hover` 为真会额外生成 `let __hv = state(false)` 与两条指针接线,并把静态样式、
   条件类、伪类、`style:` 指令**全部合成进同一个 `bind_style` 闭包**(该闭包每次全量
   `Style::default()` 重设,靠"后写覆盖"实现语义)。所以静态样式**无法**在有任何动态
   样式时单独进数据面。→ 已改表格、新增 §7.6、给出三条出路。**这是本方案最大的技术坑。**
4. **附录 A 说"§5.2 的判据算法在 ② 里已有可运行实现与测试"—— 不成立。**
   ② 的 `hot_swappable_with` 是 `self.sig == next.sig` 逐位全等(tmpl.rs:73–75),
   不是子集判据,没有 remap,连"删一个插值"都判不能热换。S5 是重写不是接线。
5. **`p99 跑间抖动 ~30%,不适合当闸`—— 抖动是 `--frames 3` 的产物。**
   membench 默认 `--frames 3`,而 p99 取 `sorted[(len*0.99) as usize]`(main.rs:207),
   3 个样本时它**就是最大值**。复核用 `--frames 60` 跑两次得 `p99 = 18.06 / 18.26`
   (~1% 抖动);CI 本来就用 `--frames 30`(ci.yml:271)。
   结论(用 build_ms 当闸)可以留,但理由必须换成"这项工作花的是建树时间"。
6. **30k 档的建树/帧数字对不上。**初稿 `build_ms=18`、`frame_avg=202ms`;
   本次同机同二进制复现 **`build_ms=10–12`、`frame_avg=118ms`**(另测 15001 节点 `=4`),
   即 ≈0.35 µs/节点而不是 0.6。差 40–70%。初稿没说清那次是在什么负载下测的
   (并行编译?),**这类数字必须带测量条件,否则不能进方案**。
7. **"3k 行 × 8 槽位 ≈ 建树翻倍"高估约 4 倍。**拿"3000 张表"比"3000 **节点**"是错口径:
   membench 的 rows 场景 `rows = controls / 5`(main.rs:266),一行 5 个节点;
   3000 个 8 槽位组件 ≈ 21000 节点 ≈ 7 ms 基线,+1.5 ms = **+21%**。
8. **`build_ms` 是整数毫秒(`built.as_millis()`,main.rs:214)**,3k 档基线 0–1,
   拿它卡 "≤ +20%" 量不出来。→ 要么改 `build_us`,要么把 `sfc-rows` 顶到 6 万节点档。
9. **"经 Rc 多一跳 = 0" 的证据是错的,结论侥幸对。**初稿的 2.06 vs 1.69 ns 复现不出来:
   空体下是 **0.86–1.96 ns vs 4.91–5.95 ns(3–6×)**。但换成真实 binder 体
   (读 signal + `format!`):**51.0–51.7 ns vs 51.4–56.8 ns,落在噪声里**。
   结论保留,证据换掉。
10. **前缀族是 7 个不是 6 个**(漏了 `bind:` 的通配分支 codegen.rs:1153);
    `out:` 是"已推迟"的报错分支(:1193)。`sv-compiler/src/lib.rs` 的测试是
    **58 个 `#[test]`**(初稿"50 余项",可以)。

### C. **致命遗漏**(初稿完全没提、跑起来会撞墙的)

1. **`#[cfg(sv_hot)]` 会让每个用户项目的 `-D warnings` 变红。**rustc 1.80 起
   `unexpected_cfgs` 对未声明 cfg 报警,它是 rustc lint,生成代码头上的
   `#[allow(…, clippy::all)]` 盖不住;全仓 grep 无任何 `check-cfg` 声明;
   CI 跑 `cargo clippy --workspace --all-targets -- -D warnings`(ci.yml:217)。
   → `sv_compiler::build()` 必须打 `cargo::rustc-check-cfg=cfg(sv_hot)`。已写进 §2.1。
2. **未进入的 `{#if}` 分支,它的 binder 与 preclone 变成无条件执行。**
   今天分支体在 `rebuild_closure`(codegen.rs:1791)里,不进分支不跑;
   裁决 5 把分支槽位拉平到父表之后,`{:else}` 里引用的普通变量会在**挂载时克隆**
   并被那个 `Rc` 闭包**永久持有**。这是一条真实的性能/内存回归,用户既看不见也解释不了。
   已写进 §3.3,并加了验收项。
3. **ADR-2 ① 会被架空。**S3 之后 emit.rs 的建树词汇表只剩 `view!` 一个消费者。
   初稿写"emit 词汇表继续共享(ADR-2 ① 的成果不动)"是自欺。已写进 §0.10 与 §7.5。
4. **dev 专属容器 = dev 与 release 布局不同。**初稿的缓解措施("两种 cfg 各跑一遍金样")
   只是检测分歧,不是消除。复核给了替代:热注册表记住本次 stamp 在 parent 下占用的
   **子节点区间**,重放时删区间 —— sv-ui 补一个 `remove_children_range` 约 20 行,
   换掉一条永久纪律。已写进 §2.1,**建议改采**。
5. **`SlotSig.hash` 的算法没钉死。**若用 `DefaultHasher`,标准库文档明说算法可能随
   Rust 版本变 → 换个工具链热重载就全量重编,且查不出原因。另:u64 无回退比较,
   §5.2 又允许多对一,碰撞后果从"漏匹配"升级为"**静默绑错闭包**"——
   dev 侧应保留规范化字符串做最终校验。已写进 §5.1。
6. **规范化不吞冗余括号、不归一字面量后缀**(`(x) > 5` ≠ `x > 5`;`5`/`5u32`/`0x5` 互不等)。
   都失败在安全侧,但"关掉 09 未决问题 ②"要收窄成"关掉**空白与注释**那一半"。已写进 §5.1。
7. **`{#if}` 分支里出现 `{@const}` 时怎么办没写。**§4.3 说带 `{@const}` 的块要开子模板,
   §2.3 又说 `If` 的分支是**内联数组**。两条规则在"if 分支里有 `{@const}`"这个交点上
   没有定义。**未解决**,S4 必须先补这条 spec(最省事的答案是:该分支整体降级成一个 `Sub`)。

### D. 工作量

初稿 9–14.5 人周 → 复核改为 **13–20 人周,置信度中低(±50%)**,逐项加账见 §6 末尾。
主要低估源:样式合成模型重设计(初稿完全没有这一项)、`dump_full` 等价性设施、
金样重写的**人工** review、以及 `emit_element` 788 行 / `emit_component` 195 行 /
`emit_overlay` ~130 行这个真实体量对应的消化速率。

### E. 更简单的替代:**B′ —— 运行时 token**

初稿的替代形态 B("只把 `<style>` 块数据化,1–2 人周")**站不住**:它和 ③ 共用
§7.6 那块最难啃的骨头。复核提出的 B′ 才是真正的 20%/80%:
style.rs 今天的 `:root { --x: v }` + `var(--x)` 是**编译期文本替换**
(`substitute_vars`,style.rs:385–410);把它改成运行时 token 查表,
因为这些 setter 本来就活在 `bind_style` 的 effect 闭包里,读一个 signal
就自动获得精准更新 —— **不需要 stamp、不需要数据面、不需要 dev 通道、
不需要 cfg 分叉、生成代码可读性零损失、release 里也生效(白拿换肤/暗色主题)**。
估 0.5–1.5 人周。详见 §8。
**复核的最终排序:B′ > C(等 Subsecond)> A(停在 S2)> B > 完整 ③。**

### F. 复核**无法**验证的部分

- **编译时间方向**(§7.2)。没跑 `--timings`:S3 的产物形态还不存在,
  今天能测的只有"现状",测了也没有对照组。初稿把它列为未实测,这个态度是对的。
- **B′ 的估算 0.5–1.5 人周**。只读了 `substitute_vars` 与它的两个调用点
  (style.rs:341/372),没有清点 token 落进 `Edges`/`Color` 之外字段时的类型推定特例数。
- **§5.3 出路 1(类索引 + 运行时样式表)在 3k 行档上多花多少钱**。需要先有实现才测得了。
- **`sfc-rows` 场景的真实数字**。该场景不存在;§3.3 的 +21% 是拿 60 ns/槽位微基准
  与 rows 场景的 build_ms 外推的,**不是端到端实测**。
- **热重载 remap 的正确性**。`hot_remap_to_old_slot_ids` 只是一个测试名,
  算法本身(§5.2 伪码)没有可运行实现,复核也没有写一份来验。
- **`{#await}` / `<overlay>` / 组件实例落 `Opaque` 之后,它们的子树结构进代码面
  对热重载体感的实际影响**。这是 v0 的主要能力缺口,只能等真用起来才知道疼不疼。
- 复核的全部性能数字来自**同一台本机、CPU 后端、release、2026-07-22**,
  与初稿口径相同但数值不同(见 B.6)。**跨机不可比,不能拿来判回归。**
