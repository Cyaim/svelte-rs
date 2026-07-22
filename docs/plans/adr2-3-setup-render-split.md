# ADR-2 ③ codegen 拆 setup/render — 落地方案

> 状态:**方案**(未实现)。2026-07-22 写于 `feat/roadmap-r3-r4-push`。
> 依据:`docs/research/09-sv-sfc-format-hotreload.md` §5.2/§5.3、DESIGN.md ADR-2 修订版、
> 以及本次对 `crates/sv-compiler/src/{codegen,emit,script,style}.rs` 与
> `crates/sv-ui/src/lib.rs`、`crates/sv-reactive/src/lib.rs` 的通读 + 三组实测。
>
> 前置:ADR-2 ①(共享发射口 `sv_compiler::emit`)**已落地**;
> ②(Template 数据化,`crates/sv-ui/src/tmpl.rs`)**主进程正在实现,本文写作时该文件尚不存在**
> ——本文对 ② 的所有引用都是**接口要求**而非事实,见 §9;落地第一步就是拿真实的 `tmpl.rs`
> 逐条核对本文假设。

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
10. **③ 只做 `.sv` 前端,`view!` 宏不跟。**宏路径改模板本来就要 rustc,拿不到热重载红利,
    却要照付装箱税与可读性税。emit 词汇表继续共享(ADR-2 ① 的成果不动)。

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
  **+ 6 个前缀族**(`class:`/`style:`/`transition:`/`in:`/`out:`/`on:`)。这个数字决定了
  "数据面 vs Wire binder"的边界要画在哪(§8 的自我质疑指标 2)。
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

// 数据面(release 是 static;dev 走 HotRegistry::get("src/Counter.sv#0"))
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
#[cfg(sv_hot)]
{
    const ID: &str = "src/Counter.sv#0";
    let __tpl = ::sv_ui::tmpl::hot::get(ID).unwrap_or_else(|| TPL_COUNTER.into());
    let __c = __doc.create_view();                 // 可清空的容器
    __doc.append(__parent, __c);
    let __b = ::std::rc::Rc::new(__b);
    let (_, __root) = ::sv_reactive::create_root(|| stamp(&__doc, __c, &__tpl, &__b));
    ::sv_ui::tmpl::hot::register(ID, &__doc, __c, __root, __b);
}
#[cfg(not(sv_hot))]
stamp(&__doc, __parent, &TPL_COUNTER, &__b);       // release:不多一个节点
```

**裁决:容器 + 子 root 只在 dev 加。**理由是实测的:全量档已经吃紧
(本机 30k 节点 `build_ms=18`、`frame_avg=202ms`;DESIGN.md R2 记的 taffy ~45ms + measure ~70ms),
不为 dev 便利给 release 的每个组件实例加一个布局节点。代价是 dev/release 结构差一层 view,
必须配一条纪律:**布局金样测试两种 cfg 各跑一遍**。
`create_root` 挂当前 owner(已核实 sv-reactive:751),所以子 root 不切断 context 链。

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

| 项 | 数字 | 口径 |
|---|---|---|
| 每模板的 binder 数(counter.sv) | **6**(1 Text + 3 Wire + 2 Cond) | 按 §2.1 的骨架数 |
| 每模板的 binder 数(TodoItem.sv) | **8**(3 文本 + 2 点击 + 2 样式修补 + 1 if 条件) | 数 `todo_item.rs` 的绑定调用 |
| 构造一张 8 槽位的表 | **≈66 ns/槽位**(3000×8 = 1.58 ms) | 本次实测,`%TEMP%` 微基准,release |
| 经 Rc 多一跳的调用开销 | **2.06 ns vs 1.69 ns**(直接 vs 经 Rc) | 本次实测,**在噪声内,判为 0** |
| 建树基线 | 3001 节点 `build_ms=1–2`;30001 节点 `build_ms=18`(≈**0.6 µs/节点**) | 本机实测 `membench --scene rows`,CPU 后端 |
| 帧基线(参考) | rows 3k `p99=19.4/24.0/25.6 ms`(三次);churn 3k `p99=22.66`;virtual 100k `p99=8.64` | 同上;**跑间抖动 ~30%,p99 不适合当这项工作的闸** |

**分配次数的增量是 +1/槽位,不是 +2。**今天一个 `bind_text` 已经要为 effect 分配一个
`Rc<RefCell<dyn FnMut()>>`(sv-reactive:69);拆完之后是 binder 的 `Rc` **加**effect 的 `Rc`。
`Rc<dyn Fn>` 与 `Box<dyn Fn>` 都是**一次**分配(Rc 只多两个计数器字),
所以"用 Rc 换取数据面可以重复引用同一槽位"这件事是**白拿的**,没有必要退回 Box。

**最坏情况要盯住的是列表。**3000 行、每行一个 8 槽位的 `.sv` 组件 = 3000 张表
≈ **1.6 ms** 额外建树,而 3000 节点的建树基线只有 1–2 ms —— **建树时间可能接近翻倍**。
放到整帧里看是 +1.6 ms / 200 ms(30k 档)≈ 1%,但"建树翻倍"这个说法本身就足以让人
在 review 里叫停,所以必须先量后做:

> **验收闸(S2 起)**:membench 增开 `--scene sfc-rows`(3k 行,行是 `.sv` 组件),
> 卡 `build_ms` 相对拆分前 **≤ +20%**。不要用 `p99_ms` 当闸 —— 本次实测同一提交跑三次
> p99 在 19.4–25.6 ms 之间,那道闸只会 flaky(与 DESIGN.md 里 CI 帧预算"门槛故意宽"
> 是同一条经验)。
> ADR-9 的虚拟化档不受影响:槽位数与**视口**成正比,不与逻辑条目数成正比。

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
- **子模板 id 必须内容派生,不能用遍历序号。**若按 09 的 `"src/counter.sv#0"` 递增编号,
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

- **hash 的输入是"runes 改写之后"的表达式的规范化 token 流**,不是 `.sv` 源码串。
  改写后才是真正编进 binder 的东西;而且这样能抓到"script 把 `count` 从 `$state` 改成普通变量"
  ——源码串一个字没变,改写结果变了,必须走 rustc。
  规范化用 `syn::Expr → to_token_stream().to_string()`,**本次实测**它吞掉空白差异
  (`count.get()  >  5` ≡ `count.get() > 5`)、吞掉普通注释、吞掉 `.` 两侧的空格,
  并且区分 `> 5` / `> 6`。09 §9 把"hash 在格式化差异下是否稳定"列为未决问题 ②,
  这条实测把它关掉。
- **出现序不进匹配键**。09 §8.3 第 6 条写的是 `(kind, hash, 出现序)`,但同一模板的
  同一表达式两次出现在词法上完全等价(共用同一个函数体作用域),可以映射到同一个 binder。
  带上出现序会让 09 §5.2 表格自己承诺的"复制一份 `{count}` → 数据面"变成需要 rustc,
  自相矛盾。**判据:多对一映射合法。**

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

### 5.3 免 rustc / 需 rustc(对 `examples/counter-sfc/src/Counter.sv` 逐条)

| 改动 | 判据 | 通道 |
|---|---|---|
| `"+1"` 改成 `"Add"`;加一个 `<text>hi</text>`;删掉整个 `{#if}` 块 | 只动 TNode/静态串,不引入新 sig | **数据面** |
| `padding:24` → `12`;`.warn` 换色;加一条 `:hover` 规则 | StyleDecl / StyleClass 是纯数据 | **数据面**(前提:style.rs 已改产值,§9) |
| 把 `{double}` 挪到另一行、复制一份 `{count}`、把 `{#if}` 换个父容器 | sig 集合是旧集合的子集(多对一合法) | **数据面** |
| 在 `{#if}` 分支里加 `<text>`、在 `{#each}` 行内加节点 | if 共用父槽位空间;each 行结构在子模板里(`Sub`) | **数据面** |
| `count > 5` → `> 3`;任一 `on:click` 闭包体改一个字符;script 任何改动;`$props` 改签名 | hash miss / 代码面 | **rustc** |
| **新增**一个 `{#if}`、`{#each}`、`<Card/>` 实例 | 引入新 Cond/Sub/Opaque 签名 | **rustc**(09 表格最后一行的"加已 import 组件实例"在 v0 拿不到,见 §0.7) |

**v1 才做的字面量提升**(把 `> 5` 里的 `5` 抽成数据槽位)不在 ③ 范围,但 `SlotSig`
预留 `kind` 字段足够表达,不需要为它改格式。

---

## 6 分步落地

每步能独立合入、独立验收。人周为**全职单人**估算,置信度标在每步。

### S0 对齐 ②(0.5 人周,置信度高)
读主进程落地的 `crates/sv-ui/src/tmpl.rs`,把本文 §2.2/§2.3/§4 的假设逐条核对,
**差异写回本文附录而不是去改 tmpl.rs**。产出:本文 §9 的清单变成"已核实/需协商"两栏。
验收:无代码改动。

### S1 槽位分配器(1–1.5 人周,置信度高)—— **纯重构,生成代码逐字节不变**
在 codegen 内引入 `SlotId`/`SlotTable`,把现有每个表达式发射点改成经 `push()` 分配,
但 binder token **仍原地内联展开**(不建表、不改形状)。
验收:`slot_ids_are_dense_and_referenced`、`codegen_output_unchanged_golden`
(把 counter/todo/todo_item/showcase 四份现产物做成金样,断言逐字节不变)、
既有编译器测试(`sv-compiler/src/lib.rs` 里 50 余项)与宏测试零改动。**可回滚。**

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
- `stamp_builds_identical_tree`:同一份 `.sv`,拆分前后 `doc.dump()` **逐字符相同**
  ——这是最强的等价性证据,比逐个断言可靠;
- `wire_order_matches_statement_order_golden`:钉死 §2.3 列的四处顺序敏感点;
- 离屏 PNG 金样(counter / showcase / settings)不变;
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

---

## 7 风险与代价

### 7.1 生成代码可读性:**明确变差,收不回来**

今天的 `counter.rs`(144 行)是照着 `.sv` 念的流水账,逐行能对上模板;panic/断点落在
"那个 `bind_text` 调用"上。拆完之后:结构去了一坨 `static` 字面量,代码里只剩一张闭包表,
**panic 回溯落在 `sv_ui::tmpl::stamp` 里**,而 stamp 对所有组件都长一个样。
这与 ADR-2 记录的三层止血的第二层("生成代码可读 + 锚点注释")直接冲突。

能做的只有减轻:
1. Template 字面量每个 TNode 前带源位置注释(`// Counter.sv:12:3 <text>`)——
   prettyplease 会保留,今天的文件头注释机制(codegen.rs:132)现成;
2. 数据面里带 `slot → 源位置` 表,dev 下 stamp 出错时打印模板位置(release 编掉);
3. 保留 `svc expand --inline` 输出**今天这种**内联形态供人查 —— 但那等于维护两套 codegen
   的一半,**不建议**,写在这里只是说明它是个选项。

**结论:这笔账要维护者认。**如果"生成代码可读"被判为不可让渡(它是 `.sv` 前端 IDE
体验的第一层止血,DESIGN.md §6 风险 1),那么 ③ 就该停在 S2(见 §8 替代形态 A)。

### 7.2 编译时间:**方向上应改善,但未实测**
模板结构从"3k 行需要类型检查的建树代码"变成"`static` 常量字面量",正是 DESIGN.md §6
风险 4 说的"生成数据而非类型"。但 **`static` 里的大常量对 rustc 也不是白送的**
(常量求值 + 单态化 + 链接),没有实测数据前不许把"编译更快"写进宣传语。
S3 的验收里已经放了 `--timings` 这一项。

### 7.3 运行时开销:**+1 分配/槽位(实测 ≈66 ns),调用开销 0**
详见 §3.3。最坏面是列表(3k 行 × 8 槽位 ≈ +1.6 ms 建树),已配 `sfc-rows` 闸。

### 7.4 调试体验
断点从"用户模板对应的生成行"退到"stamp 内部"。缓解同 §7.1。
**新增的一类失败模式**是数据/表错位(§4 三道闸)。

### 7.5 新增的维护面
加一个模板特性从"改一处 `quote!`"变成"改 parser/IR + 改 TNode 枚举 + 改 stamp 解释"。
这条会随特性数量线性放大 —— §8 指标 1 就是盯它的。

---

## 8 这个方案可能是错的

### 三个月后拆错了,最先暴露的症状

1. **加一个属性要改三处、跨两个 crate。**今天加一个 `.sv` 属性,diff 集中在
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
  模板结构照旧走生成代码,`setup/render` 一点不拆。**估 1–2 人周**,拿到 09 §5.2 表格里
  第二行的全部收益,零可读性代价。
  **如果只能做一件事,做这个。**它也是 ③ 的天然前置(S3 反正要 style.rs 改产值)。
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

### 还有一处可能是错的

**`Sub` 变体接收 `&Template` 参数**这个设计,让"行结构是数据"成立,但也意味着
`stamp` 与 binder 之间有一个**双向**依赖(stamp 调 binder,binder 回调 stamp)。
若某天要把 stamp 换成"批量建树 + 一次性写入 Doc"的优化形态(ADR-9 后续阶梯里的
增量场景编码会想要这个),这个回调结构会挡路。届时的出路是让 `Sub` 返回一个
"待建列表"而不是自己建 —— 那是一次不小的重写。**已知,未解决。**

---

## 9 对 ②(`sv-ui/src/tmpl.rs`)的接口要求清单

按"③ 需要 ② 提供什么"列;写作时 `tmpl.rs` 尚不存在,**全部标记为未核实**,
S0 的任务就是把这张表变成"已核实/需协商"。

| # | 要求 | 为什么 | 状态 |
|---|---|---|---|
| 1 | `Binder` 至少能表达 §2.2 的六语义(名字可以不同) | 少了 `Sub` 则 each 行结构进不了数据面;少了 `Wire` 则 20 个属性分支要全搬进 TNode | 未核实 |
| 2 | 槽位 id 是**不透明类型**,不是裸 `u16` | §4.1 的类型级不变量 | 未核实 |
| 3 | `Template` 带 `sig: &[SlotSig]`,`SlotSig = (kind, u64 hash)` | §5 判据的全部输入 | 未核实 |
| 4 | `stamp(&Doc, ViewId, &Template, &Binders)`,可**重复调用**且不持有状态 | 热重载重放 = 清子树 + 再 stamp 一次 | 未核实 |
| 5 | `TNode::If/Key` **内联子节点数组**(不是独立 Template) | §4.3 的槽位空间规则,直接决定热重载能力面 | 未核实 |
| 6 | `StyleDecl` 是**值**,能表达 `Style` 的全部字段 + 类的 base/hover/active/focus 四态 | 静态样式与伪类进数据面(§5.3 第二行) | 未核实 |
| 7 | dev 下 `Template` 可以不是 `'static`(`Rc<Template>` 或带生命周期) | 热注册表里的模板不是 static | 未核实 |

**③ 反过来欠 ② 之外的一件事**:`crates/sv-compiler/src/style.rs` 今天产 `TokenStream` setter
(style.rs:24–29、377),必须改成产 `StyleDecl` 值。这是 S3 的硬前置,
估 **1–1.5 人周**,可以与 S1/S2 并行,**也可以作为替代形态 B 单独交付**。

---

## 10 未核实 / 明确不做

- `tmpl.rs` 的真实形状(§9 整表)。
- 拆分后的**编译时间**方向(§7.2),没有实测不下结论。
- `binders![]` 是否真需要一个宏(也可以是 `vec![Binder::Text(Rc::new(..)), ..]`);
  写成宏只是为了让生成代码短一点,**未验证**它对 rustc 的开销是正是负。
- membench 的 `--scene sfc-rows` **尚不存在**,§3.3 的"3k 行 × 8 槽位 ≈ +1.6 ms"是
  用微基准的 66 ns/槽位外推的**估算**,不是端到端实测。
- 本文的帧数字全部来自**本机、CPU 后端、2026-07-22 的 `target/release/membench.exe`**,
  与 DESIGN.md 记录的 19 ms / 5.4 ms 口径接近但不相同(跑间抖动 ~30%),
  **不能用来判断有没有回归**,只能当量级参照。
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
2. **`hot_swappable_with()`**:本文 §5.2 的判据算法在 ② 里已有可运行实现与测试。
3. **槽位对不上时跳过而不是 panic**:本文 §4.2 第 3 条与之一致(dev 热通道走硬校验 +
   拒绝加载,release 走 `debug_assert`),② 已按此实现。

### 结论

**S0 完成**:分歧七条,六条改 ②、一条(类索引)记欠条。`tmpl.rs` 的调整不与 S1 冲突
——S1 是 codegen 内的纯重构,不碰数据面类型。建议顺序仍按 S1 → S2 → S3,
其中 `tmpl.rs` 的形状调整并入 S3 的前置(与 style.rs 数据化同批),
理由:**在没有消费者之前调整数据面类型,既没法验证也没法回归**——
② 现有的契约测试(`stamp_matches_imperative` 等 7 条)是照着当前形状写的,
改形状就要重写它们,而重写的正确性只有等 codegen 真的发射数据面时才验得了。
