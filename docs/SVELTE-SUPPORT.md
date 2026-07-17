# Svelte 5 语法/特性支持矩阵

> 生成:2026-07-17。Svelte 5 语法面对照 svelte.dev 官方文档逐项核实(runes 全清单、
> bind: 全家、class 对象/数组 5.16+、{@attach} 5.29+、函数绑定 5.9+、`<svelte:boundary>`
> 5.3+、await 表达式 5.36 实验、mount/unmount API、stores 地位、{#each} 全形态);
> "sv 现状"以本仓库代码/测试实证为准(判定依据见文末[验证方式](#验证方式))。
> 背景阅读:`docs/DESIGN.md`(ADR-2 修订版)、`docs/research/08-sv-runes-transform.md`、
> `docs/research/09-sv-sfc-format-hotreload.md`。
>
> ✅ 2026-07-17 复核完毕:写作时"施工中"的 12 项特性已全部落地并有测试,
> 终局状态见文末[落地复核](#落地复核2026-07-17)表——**该表覆盖正文状态列**,
> 冲突时以该表为准。TL;DR 统计已按落地后口径更新。

## TL;DR

**量化概括**(共 **77** 项特性,五档统计):

| 档位 | 数量 | 占比 | 一句话 |
|---|---|---|---|
| ✅ 已实现 | 43 | 56% | runes 全家(含 .raw/.snapshot/.by/.root/.tracking/.pending/.with、$props.id、writable $derived)+ 全部块语法(含 keyed each、{#await}、{#snippet}/{@render}/{@attach}/{@const}/{@debug}、注释)+ 事件(click/hover)/style:/class:/bind:(checked+组件 prop)+ 组件模型(props/children/snippet-props/默认值/回调)+ scoped `<style>` + 进场过渡 in:fade + context/mount/tick,均有测试与 showcase 演示 |
| 🚧 部分 | 3 | 4% | $state 深层响应(**设计性差异**:变量粒度,永久文档化)、$effect.pre(pre 阶段已实现,帧管线语义待 ADR-6)、$inspect.trace(重跑打点版,依赖图谱内省待 dev 工具期) |
| 📋 已设计未实现 | 0 | 0% | —— |
| ⏳ 推迟 | 11 | 14% | bind:value/group/尺寸/函数绑定(待输入控件与布局测量)、out:/animate:(待 INERT/FLIP)、键盘事件(待焦点链)、class 对象语法、{#await} shorthand、await 表达式、svelte:boundary、属性展开——每项都有明确前置条件 |
| ❌ 明确不做 | 20 | 26% | HTML 宿主域整域({@html}/svelte:window 系/媒体 bind)、遗留语法(slot/$$props/on: 长期)、被更好机制取代者(use:/bind:this→{@attach},stores→signal) |

> 数字为**五批落地后的终局口径**(逐项证据见文末各批次落地表,与正文行冲突时以落地表为准)。
> 计划内(❌ 之外)57 项中 ✅+🚧 已覆盖 46 项(81%);剩余 ⏳ 全部有明确的基建前置
> (输入控件/焦点链/INERT/FLIP/布局测量),不是设计空白。

关键读法:
- **可运行核心闭环已成立并扩展**:从 `.sv` 源文件到点击交互有两个端到端行为测试
  (`counter-sfc` 基础闭环;`todo-sfc` 覆盖组件+$props+{:else}+{@const}+{#key}+style:)。
- **计划覆盖面 74%**(❌ 之外的 57 项):桌面场景适用的 Svelte 5 语法基本全数在册,
  砍掉的 20 项集中在 HTML/DOM 宿主特有与遗留语法两类,不构成桌面能力缺口。
- **组件模型 v0 已通**(组件标签 + $props 必填/默认值/闭包 prop):剩余空白收敛到
  snippet/children 与 $bindable(08 §5 设计已闭合),仍是 M1 主体工程。

**心智兼容性定性评估**:目标是"模板语法层 100% Svelte 5、语义层按桌面裁剪"(09 §3)。
与 Svelte 心智一致的部分——runes"声明显式、使用隐式"、块语法、snippet 取代 slot、
事件属性取代 on: 指令、{@attach} 取代 use:。**刻意且文档化的语义差异**(迁移者须知):
① 响应粒度是**变量级**而非 Proxy 字段级(08 §2.5,over-notify 由 derived 相等剪枝兜底);
② `f(user)` 传对象是**快照**而非 Proxy 同行引用(差异最大点);③ effect **创建时同步首跑**
(Svelte 是微任务,ADR-1);④ 无隐式赋值响应的 Proxy 深改(`arr.push` 需走 update 改写);
⑤ `{@html}`/HTML 宿主对象无对应物。总体:写惯 Svelte 5 runes 世代的人,心智迁移成本
集中在"对象状态的传递与深改"一处,其余语法几乎逐字对应。

## 图例

| 档 | 含义 | 判定依据 |
|---|---|---|
| ✅ 已实现 | 代码 + 测试可实证 | 注明代码/测试位置 |
| 🚧 部分 | 核心可用但有明确缺口,或运行时原语已备、编译器未接线 | 注明已有/缺失两侧 |
| 📋 已设计未实现 | 08/09 报告有落地设计(规则表/EBNF/类型形态),代码无 | 引用报告章节 |
| ⏳ 推迟 | 有意向无设计,或设计判定"语法保留、实现推迟" | 注明理由与前置条件 |
| ❌ 明确不做 | 设计裁决不进格式 | 注明理由与替代物 |

代码位置速记:`compiler` = `crates/sv-compiler/src/`,`ui` = `crates/sv-ui/src/lib.rs`,
`reactive` = `crates/sv-reactive/src/lib.rs`,`macro` = `crates/sv-macro/src/`。

---

## A. Runes(18 项)

| 特性 | Svelte 5 语义 | 桌面适用性 | sv 现状 | 结论/计划 |
|---|---|---|---|---|
| `$state` | 声明深层响应式状态(对象/数组走 Proxy,字段级追踪) | 高(状态内核) | 🚧 `let x = $state(v)` → `state(v)` 源变换 + 读 `.get()`/写 `.set()`/复合赋值 `.update()` 已实现(`compiler/script.rs`,测试 `counter_compiles`);**缺**:无 Proxy 深响应(变量粒度,设计如此)、字段/索引赋值 `pos.x = 1` 未改写、语句位方法写 `xs.push(v)` 未改写、`+=` 的 RHS 预求值(08 §2.3 重入防护)未做、`$state` 预处理是字符串替换而非 token 级(字符串字面量会被误伤,`script.rs` 头注释自认) | 按 08 §2/§3 规则表补全写位改写 + 拒绝清单 sv0001–0013;粒度差异是 ADR-1 定案,`#[derive(Store)]` 字段投影列 v1 |
| `$state.raw` | 非深层响应,整体重赋值才触发 | 中 | ⏳ 无代码。08 §6.1 裁定"不进 v0":sv 无 Proxy,快照/整体赋值本来就是缺省语义,无差异可言 | 🔄 主线程施工中,语义定位待其结论复核 |
| `$state.snapshot` | 取 Proxy 状态的静态快照(脱 Proxy) | 低 | ❌ 不需要:sv 读 `x` 即 `.get()` 快照(要求 Clone),无 Proxy 即无"脱 Proxy"需求(08 §6.1) | 不做;文档对照表说明即可 |
| `$props` | 组件接收 props(对象解构 + 默认值) | 高(组件模型基石) | 📋 无代码(现编译产物是无参 `pub fn xxx(doc, parent)`)。08 §5.1 已定形态:`struct Props` 编译器指令 + `let Props {..} = $props()` 解构,prop 读端包 `PropValue::Static/Reactive` 保持响应 | 🔄 主线程施工中(组件+$props);M1 组件模型主体 |
| `$props.id` | 生成组件实例唯一 id(SSR 安全) | 低(无 SSR;无障碍 label 关联或有用) | ⏳ 无代码无设计 | 随组件模型 + AccessKit 接入(M2)评估 |
| `$bindable` | 声明可双向绑定的 prop | 高(表单/复合控件) | 📋 无代码。08 §5.2 已定形态:`count: $bindable(i32)` → prop 类型为 `Signal<i32>` 句柄直传,"bind: 只接受可写信号"由类型系统静态保证(强于 Svelte 运行时告警) | 随 $props 落地 |
| `$derived` | 表达式派生值,惰性 + 依赖追踪 | 高 | ✅ `$derived(expr)` → `derived(move \|\| expr')` 自动闭包化 + 读改写(`compiler/script.rs`,测试 `counter_compiles` 断言生成形态;端到端 `sfc_counter_behaves` 断言"双倍"联动);运行时惰性求值 + PartialEq 剪枝(`reactive`,17 测试) | 保持;派生表达式内写 state 的静态检查(sv0006)待补 |
| `$derived.by` | 多语句闭包形态的 derived | 高 | 📋 无代码(`$derived.by` 会在预处理后解析失败)。08 §2.4 已定:`by` 收闭包、本体只收表达式 | 🔄 主线程施工中 |
| writable `$derived`(5.25+) | 临时覆盖派生值(optimistic UI),依赖变化后回退 | 中 | ⏳ 无代码。08 §1.2 裁定 v0 拒绝写 derived(运行时已有写保护),列观察项 | 跟踪上游用法沉淀,不承诺 |
| `$effect` | 副作用,依赖变化重跑,返回值/内部可注册清理;微任务批量调度 | 高 | ✅ `$effect(\|\| ..)` → `effect(move \|\| ..)`(测试 `effect_and_fmt_macro_rewrite`;fmt 宏参数内读改写同测);清理用 `on_cleanup`(`reactive`,块销毁语义有 `if_block_inner_state_disposed` 测试)。**刻意差异**:创建时同步首跑、写后同步 flush(ADR-1) | 帧调度(ADR-6)落地后对齐"帧前 flush"批量语义 |
| `$effect.pre` | DOM 更新**前**运行的 effect | 中(滚动锚定类场景) | ⏳ 无代码。当前运行时无 pre/post 阶段之分(写后同步 flush),区分只有在帧管线(事件→batch→pre→render→layout→paint→user effects)里才有意义 | 挂 ADR-6 帧调度之后(08 §6.1) |
| `$effect.tracking` | 查询当前是否处于追踪上下文 | 低 | ⏳ 无代码,运行时无此查询 API | 需求出现再加(08 §6.1) |
| `$effect.root` | 手动生命周期的非追踪作用域 | 中(长生命周期后台绑定) | 🚧 运行时等价物已有:`create_root` + `RootHandle::dispose`(`reactive:524`,counter-sfc 测试实际使用);`$effect.root` rune 形态未进 script 变换 | rune 映射随拒绝清单一起补;或文档直接教 `create_root` |
| `$effect.pending`(async 实验配套) | 边界内待决 await 数量查询 | 中 | ⏳ 无代码;上游本身是实验特性 | 与 {#await}/resource() 议题(04 §6.5)一并设计 |
| `$inspect` | dev 模式追踪值变化并 console.log,prod 为 noop | 中(调试 DX) | ⏳ 无代码。08 §6.1 裁定"dev 工具期"再做 | 🔄 主线程施工中,状态待复核 |
| `$inspect(...).with` | 自定义 inspect 回调(如 debugger) | 中 | ⏳ 同上 | 随 $inspect |
| `$inspect.trace` | 追踪 effect/derived 重跑原因 | 中 | ⏳ 同上;运行时需暴露依赖图谱内省接口 | dev 工具期 |
| `$host` | custom element 的宿主元素访问 | 无(无 custom elements) | ❌ 无对应语境(08 §6.1) | 不做 |

**小计**:✅ 2 · 🚧 2 · 📋 3 · ⏳ 9 · ❌ 2

---

## B. 模板:块与标签(15 项)

| 特性 | Svelte 5 语义 | 桌面适用性 | sv 现状 | 结论/计划 |
|---|---|---|---|---|
| `{expr}` 插值 | 文本位表达式,依赖变化精准更新文本节点 | 高 | ✅ 静态/插值混排段落 → 全静态零绑定、含插值 `bind_text` 闭包(`compiler/codegen.rs::leaf_create`;测试 `counter_compiles` 断言两种形态;`ui` `text_update_is_fine_grained` 断言相同文本不重绘);表达式内 runes 读与 script 共用同一改写器 | 保持;09 的 `SvDisplay` trait 方案(`Signal<T: Display>` 直插)未接,当前靠读改写达成同效 |
| `{#if}/{:else if}/{:else}` | 条件块,分支销毁时块内状态一并销毁 | 高 | ✅ 全三段解析(`compiler/template.rs::parse_if`)→ 嵌套 `if_block`(`codegen.rs::emit_if`);运行时相等剪枝 + 分支销毁语义有 4 项测试(`ui`:`if_block_switches_and_disposes`/`if_block_inner_state_disposed`/`cond_flip_only_rebuilds_on_change`;端到端 `sfc_counter_behaves`) | 保持 |
| `{#each expr as pat, i}` 基本形态 | 列表渲染,含解构模式与 index | 高 | 🚧 解析 + codegen 已通(`template.rs::parse_each`,pat 走 `syn::Pat` 支持解构,行内遮蔽正确处理,测试 `each_block_compiles`/`shadowed_each_pattern_not_rewritten`);**缺**:运行时是整块重建原型(`ui::each_block` TODO 自认),非 Svelte 的按行复用 | keyed reconcile 是 ADR-7 定案方向:每项 `Signal<Item>`、原地 set、增删移 diff |
| `{#each ... (key)}` keyed | 按 key 复用/移动 DOM,行状态跟随 | 高 | 📋 无代码(`(key)` 现会被解析进 pattern 报错);ADR-7 + 09 EBNF 已收语法 | M1;seen-set vs LIS 待场景树搬移成本基准后定 |
| `{#each}` 的 `{:else}` | 空列表分支 | 高(空状态 UI) | 🚧 运行时原语已备:`ui::each_block_else`(签名含 empty 闭包);**模板 parser 未接**(`{:else}` 在 each 内会报"意外的块标记") | 🔄 主线程施工中(接线即完) |
| `{#each}` 省略 `as`(渲染 N 次) | 不绑定项,仅按长度渲染 | 低 | ⏳ 无代码;09 EBNF 要求 `as` | 边缘形态,需求出现再放开文法 |
| `{#key expr}` | 表达式变则销毁重建块(重置内部状态/重放动画) | 中 | 🚧 运行时原语已备:`ui::key_block`(注释明言与 Svelte {#key} 语义一致);**模板 parser 未接**(`{#key` 报"未知块类型") | 🔄 主线程施工中;09 评"if_block 变体,近乎免费" |
| `{#await}/{:then}/{:catch}` | Promise 三态渲染 | 高(文件/网络) | ⏳ 无代码。09 §3 裁定"保留语法、推迟实现":正确语义依赖 `resource()` 原语与取消/竞态设计(04 §6.5) | parser 先接受语法、报"未实现"诊断,格式不留破坏性变更;实现挂 async 议题 |
| `{#snippet name(params)}` | 可复用模板片段(取代 slot) | 高(组件组合基石) | 📋 无代码。08 §5.3 已定:编译为 `Rc<dyn Fn(&Doc, ViewId, ...)>` 闭包值,捕获全 Copy 句柄;09 EBNF 已收 | 随组件模型(M1) |
| `{@render snippet(args)}` | 调用 snippet | 高 | 📋 同上,codegen 处即调用闭包 | 随 {#snippet} |
| `{@const x = expr}` | 块内局部常量 | 中 | 📋 无代码;09 EBNF 已收("块内局部 let,零成本") | 🔄 主线程施工中 |
| `{@debug vars}` | 值变化时 log + debugger | 低-中 | 📋 无代码;09 裁定保留("编译为 inspect 日志,几行代码的事") | dev 工具期与 $inspect 一并 |
| `{@html expr}` | 注入原始 HTML | 无 | ❌ 场景树没有 HTML(09 §3);富文本未来另设元素(如 `<rich>`),不复用这个口子 | 不做 |
| `{@attach expr}` | 元素挂载时运行、响应依赖重跑的附件函数(5.29+,取代 use: 方向) | 高(命令式逃生舱:focus/测量/自绘) | 📋 无代码。09 §3 已定:`expr: impl Fn(NodeHandle)`,返回 Drop 即卸载清理,**一并覆盖 bind:this 与 use: 职责(三合一)** | M1 组件模型后;是自绘/测量类需求的官方出口 |
| 模板注释 `<!-- -->` | 注释(可携 svelte-ignore 指令) | 中 | 📋 09 EBNF 已收 `Comment`;**parser 未实现**(现状 `<!--` 会报"`<` 后应为标签名") | 补上(平凡);svelte-ignore 类指令随诊断系统 |

**小计**:✅ 2 · 🚧 3 · 📋 7 · ⏳ 2 · ❌ 1

---

## C. 指令与属性(19 项)

| 特性 | Svelte 5 语义 | 桌面适用性 | sv 现状 | 结论/计划 |
|---|---|---|---|---|
| 事件属性 `onclick={h}` | Svelte 5 事件即普通属性(非指令) | 高 | 📋 无代码——**现状实现的恰是遗留 `on:click` 形态**(见下行);09 §3 裁定跟随 Svelte 5 事件属性形态 | 🔄 主线程施工中(onclick);落地后 on: 移除 |
| 遗留 `on:click` 指令 | Svelte 4 事件指令(5 中弃用) | —(过渡形态) | 🚧 v0 现状唯一事件语法:`on:click={闭包}` → `set_on_click`,handler 自动 move(`codegen.rs:134`;`counter_compiles`、端到端点击测试);其余 `on:*` 明确报错 | 迁移到 onclick 后**移除**,不进正式格式(09"无历史包袱") |
| 其它事件(oninput/onkeydown/hover/focus 等) | 全量 DOM 事件面 | 高 | ⏳ 场景树只有 `on_click` 一种回调(`ui::ViewNode`);09:v0 事件集 = click | 随 sv-ui 事件模型扩(键盘焦点链/滚动在 M1 路线图) |
| 事件修饰符 `\|once` `\|preventDefault` 等 | Svelte 4 遗留,**5 已删**(改用包装函数) | 无 | ❌ 与 Svelte 5 同步不做(09 §3) | 不做 |
| `bind:value/checked` 等表单绑定 | 表单元素双向绑定(value/checked/group/files/indeterminate/select) | 高(表单是桌面刚需) | 📋 无代码,且场景树尚无输入类元素(仅 view/text/button)。09 §3 已定:白名单属性(value/checked/focused)展开为读+写两条,编译期拒绝只读信号 | 随 sv-ui 输入控件落地(M1 TodoMVC 逼出) |
| `bind:group` | radio 互斥 / checkbox 数组聚合 | 中 | ⏳ 无代码无设计(09 白名单未含) | 随表单控件集再定 |
| `bind:this` | 取 DOM 节点/组件实例引用 | 中 | ❌ 职责并入 `{@attach}`(09 §3"三合一"),不单独设指令 | 用 {@attach} 拿 NodeHandle |
| `bind:` 尺寸系(clientWidth/contentRect 等 8 个只读) | ResizeObserver 驱动的尺寸观察 | 高(布局联动) | ⏳ 无代码;09 白名单未含,且依赖布局系统提供测量回调 | taffy 布局(M1)落地后设计桌面对应物 |
| `bind:` HTML 专属(媒体系 currentTime/paused 等、contenteditable、details open、img natural 尺寸) | 宿主元素属性双向绑定 | 无-低(无对应元素) | ❌ 无媒体/contenteditable/details 元素;未来若设媒体元素另行设计,不搬 HTML 属性面 | 不做 |
| `bind:prop`(组件 prop,配 $bindable) | 父子双向绑定 | 高 | 📋 无代码。08 §5.2 已定:`bind:count={total}` 直传 `Signal` 句柄本体,零胶水,类型系统保证可写 | 随 $props/$bindable |
| 函数绑定 `bind:x={get, set}`(5.9+) | 绑定经自定义 get/set 变换 | 中 | ⏳ 无代码无设计 | bind: 基础落地后按需求评估 |
| `class:name={cond}` 指令 | 条件切换单个 class | 高 | 📋 无代码(`class` 属性现会报"未知样式键")。09 §3/§4 已定:类名编译为组件样式表索引(u16),翻转打/摘 StyleClass 重算样式 | 随 `<style>` 块(M1) |
| `class` 对象/数组语法(5.16+,clsx) | `class={{ cool, lame: !cool }}` / 数组组合 | 低-中 | ⏳ 无代码;09 只设计了静态 `class="a b"` 与 `class:` 切换——sv 的类是编译期样式表索引,不是运行时字符串,clsx 形态需另行映射 | class: 落地后评估是否有真实需求 |
| `style:prop={expr}` 指令 | 单条内联样式绑定(含 `\|important` 修饰) | 高(动态样式唯一通道,09 §4 纪律) | 🚧 运行时原语已备:`ui::bind_style_patch`(注释明言对应 style: 指令,单字段修补不重置其它);**parser/codegen 未接**。`\|important` 无级联语境,不适用 | 🔄 主线程施工中(接线);09 定位:一切动态样式走 style:,`<style>` 块保持纯静态 |
| `use:action` | 元素挂载时的 action(参数变不重跑) | —(被取代) | ❌ 被 `{@attach}` 取代(09 §3;Svelte 官方亦向 attachment 迁移),一个机制够了 | 不做 |
| `transition:` | 进出场双向过渡(可配 params/global) | 高(桌面动画是差异化点) | ⏳ 无代码。09 §3:**保留语法、推迟实现**——依赖帧调度(ADR-6)与动画系统;EBNF 已收语法 | 帧调度后立项;v0 parser 接受语法报"未实现" |
| `in:` / `out:` | 单向进/出场过渡 | 高 | ⏳ 同上 | 同上 |
| `animate:`(FLIP) | keyed each 重排动画 | 中-高 | ⏳ 同上,且前置依赖 keyed each(ADR-7) | keyed each + 帧调度之后 |
| 属性值形态(表达式/字符串/简写 `{name}`/布尔省略/插值字符串 `"a{b}c"`) | 属性的多种书写形态 | 高 | 🚧 已有:`name="str"` 与 `name={expr}`(`template.rs::parse_attr`);**缺**:简写 `{name}`、省略值 ≡ `={true}`、字符串内插值、无引号单 token——均在 09 EBNF | 按 EBNF 补全 parser(平凡) |
| 属性展开 `{...props}` | 对象展开为属性集 | 中 | ⏳ 09 §3:v0 砍——强类型 props 下语义含糊(哪些字段?),需 builder 级设计 | v1 再议 |

**小计**:✅ 0 · 🚧 3 · 📋 4 · ⏳ 8 · ❌ 4

---

## D. 特殊元素与遗留语法(11 项)

| 特性 | Svelte 5 语义 | 桌面适用性 | sv 现状 | 结论/计划 |
|---|---|---|---|---|
| `<svelte:window>` | window 事件/属性绑定(scrollY 等) | —(HTML 宿主形态) | ❌ HTML 宿主对象无对应物(09 §3) | 桌面变体 `<sv:window title={} min-width=...>` 是真实需求,列 v1 议程 |
| `<svelte:document>` | document 事件/属性 | 无 | ❌ 同上 | 不做 |
| `<svelte:body>` | body 事件 | 无 | ❌ 同上 | 不做 |
| `<svelte:head>` | 注入 `<head>`(title/meta) | 无(窗口标题另有归属) | ❌ 同上;窗口标题走 `<sv:window>` 变体 | 不做 |
| `<svelte:element this={tag}>` | 动态标签名元素 | 低 | ❌ 封闭元素集下意义稀薄(09 §3),需要时 `{#if}` 分支 | 不做 |
| `<svelte:boundary>`(5.3+) | 错误边界(failed/onerror)+ async pending 边界 | 高(panic 隔离对桌面长进程有价值) | ⏳ 无代码。09 §3:值得要,依赖 effect 层 catch_unwind 设计,v0 推迟 | 随 effect 层健壮性设计(panic 隔离)+ async 议题(pending) |
| `<svelte:options>` | 组件级编译选项 | 中 | 📋 09 已定 sv 变体 `<sv:options edition="..." name="..."/>`,EBNF 已收;`sfc.rs` 未实现 | 随 edition 版本化机制(09 §7) |
| `<svelte:self>`(遗留) | 递归引用自身(5 中不推荐) | — | ❌ Rust `use` 自身模块即可递归,无需专门元素 | 不做 |
| `<svelte:component this={C}>`(5 中已非必需) | 动态组件(5 中组件引用天然动态) | 中 | ❌ 强类型组件下"值即组件"需 trait 对象设计,当前用 `{#if}` 分支;Svelte 5 自己也标记为不再需要 | 动态组件需求出现时按 `Box<dyn Component>` 议,不复刻此元素 |
| `<slot>` / `slot="name"`(遗留) | Svelte 4 内容分发 | — | ❌ snippet/children 全面取代(09"只对齐 runes 世代") | 不做 |
| `$$props` / `$$restProps`(遗留) | 未声明 props 透传 | — | ❌ 同上;$props + 未来 spread 设计承接 | 不做 |

**小计**:✅ 0 · 🚧 0 · 📋 1 · ⏳ 1 · ❌ 9

---

## E. 组件模型与运行时(9 项)

| 特性 | Svelte 5 语义 | 桌面适用性 | sv 现状 | 结论/计划 |
|---|---|---|---|---|
| 组件标签 `<Foo prop={x} />` | 组件实例化 + props 传递 | 高 | 📋 无代码(元素集只有 view/text/button,`template.rs` 未知标签报错)。09 §6/EBNF 已定:`CompPath` 按 script 作用域解析,`.sv` 间 import 就是 Rust `use`,签名统一 `Component` trait | 🔄 主线程施工中(组件+$props);M1 主体 |
| `children` 隐式 snippet | 组件标签体自动成为 children prop | 高 | 📋 08 §5.3 已定:标签体非 snippet 内容自动包成缺省 `children`(`Rc<dyn Fn(&Doc, ViewId)>`) | 随组件模型 |
| snippet 作为 props(显式传递/组件体内 `{#snippet}`) | snippet 是一等值,可作 prop 传递 | 高 | 📋 08 §5.3/09 EBNF 已定:组件体内 `{#snippet}` 编译为命名闭包 prop,`Snippet`/`Snippet1<A>` 类型已给出 | 随组件模型 |
| 组件事件 = 回调 props | 5 中 createEventDispatcher 弃用,事件即 `onX` 回调 prop | 高 | 📋 08 §5.1 形态含 `on_change: Option<Callback<i32>>` 示例;与 props 同一机制,无独立事件系统 | 随 $props |
| `mount` / `unmount` 命令式 API | 手动实例化/卸载组件(hydrate/render 为 web/SSR 专属) | 高(应用入口) | 🚧 函数式等价物可用:编译产物 `pub fn xxx(doc, parent)` + `create_root`/`RootHandle::dispose`(counter-sfc 测试实际这么挂载)+ `sv_shell::run_app` 壳;**缺**:统一 `Component` trait(Props 结构 + mount,兼作热重载注册表,09 §6) | trait 化随组件模型;hydrate/render 无 SSR 语境不做 |
| `setContext` / `getContext` | 组件树层级注入依赖 | 高(主题/服务注入) | ⏳ 无代码无设计(reactive 无 context API) | M1 组件模型时一并设计(所有权树已有父子结构可挂) |
| stores(svelte/store,`$store` 前缀订阅) | 5 中未弃用但地位收缩:复杂异步流/手动订阅控制场景 | 低 | ❌ 不做:无历史包袱,共享状态 = 模块级 signal;跨线程走消息回 UI 线程写 signal(ADR-1);`$` 前缀另有 rune 语义 | 异步数据流场景由未来 `resource()` 原语承接 |
| await 表达式(5.36 实验,Svelte 6 转正) | script 顶层/`$derived`/模板内直接 await,配 boundary pending | 高(文件/网络 IO) | ⏳ 无代码。上游仍实验(需 `experimental.async` 旗标);Rust 侧对应物是 async/资源原语整章设计(04 §6.5) | 与 {#await}/resource()/boundary pending 作为同一 async 议题设计;记录上游动向 |
| 生命周期 `onMount`/`onDestroy`/`tick` | 挂载/销毁钩子与微任务 flush 等待 | 高 | 🚧 等价物:effect 创建即同步首跑(≈onMount)、`on_cleanup`(≈onDestroy,`reactive:508`,销毁语义有测试);**缺**:`tick` 等价物(当前写后同步 flush,无"等待 flush"语义;帧调度后需要 `flush_sync` 逃生舱,ADR-6) | 帧调度落地时补 tick/flush_sync;文档写明 effect 首跑时序差异 |

**小计**:✅ 0 · 🚧 2 · 📋 4 · ⏳ 2 · ❌ 1

---

## F. SFC 文件结构与样式(5 项)

| 特性 | Svelte 5 语义 | 桌面适用性 | sv 现状 | 结论/计划 |
|---|---|---|---|---|
| `<script>` 块 | 组件实例逻辑 | 高 | ✅ `sfc.rs::split` 切块 + 全量 runes 变换,7 测试 + 端到端;限制:script 须在文件顶部、内容含 `"</script>"` 字符串会截断(09 设计了词法级扫描解法,未实现) | 按 09 §1 升级为 Rust 词法扫描切块 |
| `<script module>` | 模块级一次性代码 | 低 | ❌ 09 §1:不设——Rust fn 体内本可声明 use/struct/fn/impl,跨组件共享写普通 .rs,少一个概念 | 不做 |
| `<style>` 块 + scoped CSS | 组件样式,默认 scoped | 高 | 📋 无代码(`sfc.rs` 不识别 `<style>`)。09 §4 完整设计:极简"类 + 属性"静态样式语言、类名编译为样式表索引(scoped 免费且绝对)、`@tokens`/`@theme`、伪状态;**现行替代**:`style="k:v"` 内联迷你样式 + 样式简写属性(`fg=`/`font-size=`)已实现(`compiler/style.rs`,9 个属性键,错误带定位) | M1;它同时是热重载数据面的承重墙(09 §5) |
| CSS `:global` | 逃逸 scoped 作用域 | 无 | ❌ 09 §4:永远 scoped,无全局样式概念;跨组件外观走 props 或 theme token,不走样式泄漏 | 不做 |
| `lang="ts"` / `src` 外链 / custom blocks(SFC 扩展面) | 预处理与外链 | 无 | ❌ script 只能是 Rust,外链/自定义块是生态税(09 §1) | 不做 |

**小计**:✅ 1 · 🚧 0 · 📋 1 · ⏳ 0 · ❌ 3

---

## G. sv 特有扩展(非 Svelte 语法,不计入统计)

| 特性 | 语义 | 现状 |
|---|---|---|
| `$sig(x)` 逃生舱 | 取 `Signal<T>` 句柄本体,显式 API 全开放(Svelte 无对应物也不需要) | 📋 08 §2.2 已定;无代码。**部分等价物已可用**:显式 `.get()/.set()/.update()/.with()/.get_untracked()/.with_untracked()` 方法调用的接收者不会被二次包装(`script.rs:300-309` 白名单放行)。🔄 主线程施工中 |
| `$untrack(expr)` | 脱追踪读(Svelte 有同名函数 `untrack`,非 rune) | 🚧 运行时 `untrack()` 已有(`reactive:493`);`$untrack` script 映射未实现(08 §2.2 规则表已列) |
| 内联 `style="k:v; ..."` 迷你样式语言 + 样式简写属性 | Svelte 无此形态;`<style>` 块落地前的临时静态样式通道 | ✅ `compiler/style.rs`(padding/gap/font-size/radius/width/height/direction/bg/fg),错误带 .sv 行列;`{表达式}` 形式的 style 明确报错留给 `style:` 指令 |
| `view!` 宏前端 | 同一语义的 proc-macro 路线(Rust 原生 if/for 语法,非 Svelte 语法) | ✅ `crates/sv-macro` 12 测试:view/text/button、if/else-if/else、for(含 index)、文本插值、`style(闭包)`、`on_click(闭包)`。ADR-2:与 .sv 前端共享内核是 M1 合并目标 |

---

## 验证方式

各档判定依据(供复核者按同一口径更新状态列):

1. **✅ 已实现**:同时满足 ①`crates/sv-compiler/src/` 中有对应 parser/codegen 路径,
   ②仓库内测试直接断言该行为(`sv-compiler/src/lib.rs` 单测、`crates/sv-ui/src/lib.rs`
   原语测试、`examples/counter-sfc/src/main.rs::sfc_counter_behaves` 端到端)。
   本文写作时未运行 `cargo test`(主线程正在改代码,避免干扰),"全绿"陈述转引自
   DESIGN.md §4;复核时以 `cargo test` 实跑为准。
2. **🚧 部分**:两种情形,行内已注明是哪种——
   a) 编译器路径存在但对照 08/09 规则表有明确缺口(如 $state 的写位改写子集);
   b) **sv-ui 运行时原语已存在、编译器未接线**(each `{:else}` ↔ `each_block_else`、
   `{#key}` ↔ `key_block`、`style:` ↔ `bind_style_patch`——这三个原语在
   `crates/sv-ui/src/lib.rs:380-435`,但 `template.rs`/`codegen.rs` 中无任何引用)。
3. **📋 已设计未实现**:代码全无(grep 不到 parser 分支/codegen 路径),但 08 或 09
   报告有可直接施工的设计(变换规则表、类型形态或 EBNF 文法),行内已引章节。
4. **⏳ 推迟**:代码全无,且报告明示"保留语法推迟实现"或未设计;行内注明前置条件
   (帧调度 ADR-6、keyed each ADR-7、async 议题、组件模型等)。
5. **❌ 明确不做**:09 §3 裁决表或 08 §6.1 的明文裁决;行内注明理由与替代物。

**Svelte 5 语法面核实方式**:svelte.dev 官方文档(2026-07-17 抓取)——runes 导航全清单
(含 $effect.pending/$inspect.trace)、bind: 全部目标枚举、class 对象/数组(5.16+)、
{@attach}(5.29+)、函数绑定(5.9+)、`<svelte:boundary>`(5.3+,failed/pending/onerror)、
await 表达式(5.36 实验、`experimental.async` 旗标、Svelte 6 转正计划)、
mount/unmount/hydrate/render、stores"未弃用但地位收缩"的原文口径、{#each} 全形态
(含省略 `as`)。

### 落地复核(2026-07-17)

写作时"施工中"的特性已全部落地,**本表为终局状态,覆盖正文表 A–G 的状态列**
(测试均在 `compiler` lib tests 与 `examples/todo-sfc` 行为测试):

| 特性 | 终局状态 | 实证 |
|---|---|---|
| each 的 `{:else}` | ✅ | `each_else_compiles`;`todo-sfc` 空状态往返 |
| `{#key expr}` | ✅ | `key_block_compiles`;`ui` `key_block_recreates_on_key_change`(相等剪枝) |
| `{@const name = expr}` | ✅ 编译成**块级 derived**(比 Svelte 的块内重算更细粒度;被 move 捕获的行内变量自动预克隆) | `const_becomes_block_derived`;`todo-sfc` 摘要行联动 |
| `style:字段={expr}` | ✅ 单字段响应式修补,与静态 style 属性叠加 | `style_directive_patches_field`;`todo-sfc` 勾选变灰 |
| 组件标签 + `$props` | ✅ v0:`$props { name: Type [= default] }` → 结构体 + 解构;标签编译成函数调用,必填校验/默认值内联/闭包 prop 均可;**快照语义**,响应式 prop 传 `$sig(x)` 句柄;children/snippet 仍 📋 | `props_callee_generates_struct`、`component_call_with_props_and_default`;`todo-sfc` |
| `$derived.by(\|\| {...})` | ✅ | `rune_variants` |
| `$state.raw(v)` | ✅ **别名**:sv 无 Proxy 深响应,缺省语义即 raw,两者等价(裁决:接受语法、零差异,迁移友好) | `rune_variants` |
| `$inspect(a, b)` | ✅ 基础版(effect + Debug 打印);`.with`/`.trace` ⏳ | `rune_variants`;`todo-sfc` 运行输出 |
| `$effect.pre` | 🚧 别名(接受语法;"渲染前"时序等帧调度 ADR-6 落地后区分) | `rune_variants` |
| `$sig(x)` 逃生舱 | ✅ 取裸句柄、内部零改写 | `rune_variants` |
| `onclick` 事件属性 | ✅ 与遗留 `on:click` 双形态并存(推荐 onclick) | `onclick_svelte5_attr` |
| `+=` RHS 预求值 | ✅ 修复(`count += count` 不再重入 panic,08 §2.3) | `compound_assign_rhs_preevaluated` |

### 第三批落地(2026-07-17,含综合演示 examples/showcase)

| 特性 | 终局状态 | 实证 |
|---|---|---|
| `{#snippet name(p: T)}` / `{@render}` | ✅ 编译成局部闭包 + 调用;**参数响应式**(以参数元组为 key 的 key_block 重建,粒度比 Svelte 粗一档、语义一致;参数需 Clone+PartialEq) | `snippet_and_render_compile`;showcase stat 面板 |
| 组件 children | ✅ 子内容编译成隐式 `children: sv_ui::Snippet` prop,callee `{@render children()}` | `component_children_snippet`;showcase Card |
| `$bindable(T)` + `bind:name={x}` | ✅ 类型展开 `Signal<T>`;callee 内按反应式变量参与 runes 改写;caller 传裸句柄零胶水 | `bindable_prop_two_way`;showcase Stepper |
| keyed `{#each ... (key)}` | ✅ `each_block_keyed`:同 key 行**零重建**复用(场景子树+行内状态保留),重排/增删精准;暂不支持索引与 {:else} | `keyed_each_compiles`;`ui` `keyed_each_preserves_row_state`;showcase 反转演示 |
| `<style>` 块 + `class` 属性 | ✅ `.类 { 封闭属性集 }`,编译进组件函数(天然 scoped,零运行时选择器);优先级 类 < style="" < style: 指令 | `style_block_classes`;showcase 两个同名 .btn 互不干扰 |
| `{@debug a, b}` | ✅ 依赖变化即 Debug 打印 | `debug_tag_compiles` |

### 第四/五批落地(2026-07-17,"把没实现的全部落地"专项)

运行时新增(sv-reactive/sv-ui,均有单测):writable `Derived::set/update`、两阶段 flush
+`effect_pre`、`is_tracking`、`unique_id`、`tick`、`provide_context`/`use_context`(owner 链,
穿 create_root)、`detached`(游离创建,线程级单例信号的逃生舱)、Checkbox 元素、
pointer enter/leave 回调、`mount`/`MountHandle`、`anim` 模块(opacity 通道+帧泵)、
`tasks` 模块(后台线程异步桥:spawn/cancel/pump/响应式 pending_count + await_block 重启语义)。

| 特性 | 终局状态 | 实证 |
|---|---|---|
| `$state.snapshot` / `$props.id` / `$effect.tracking` / `$effect.root` | ✅ | `rune_variants_batch4` |
| `$effect.pre` | ✅ pre 阶段真实现(同轮 flush 内先于普通 effect;帧管线语义待 ADR-6 深化,标 🚧) | reactive 单测 |
| `$effect.pending()` | ✅ 异步桥在途计数(响应式) | tasks 单测 |
| `$inspect(...).with` | ✅ 值元组回调;`.trace` 🚧 重跑打点版 | `rune_variants_batch4` |
| writable `$derived`(5.25 乐观 UI) | ✅ 覆盖+依赖变化自动回退 | `writable_derived_assignment`;reactive 单测 |
| `{#await}/{:then}/{:catch}` | ✅ 依赖变化重启、卸载/重启取消在途任务、Result 分派 | `await_block_compiles`;tasks 四个行为测试;showcase |
| `transition:fade` / `in:fade` | ✅ 进场淡入(组透明近似级联);`out:`/`animate:` ⏳(INERT/FLIP) | `transition_fade_compiles`;anim 单测;showcase |
| `bind:checked` + `<checkbox>` | ✅ 双向;`bind:value` 等 ⏳(待文本输入控件) | `bind_checked_two_way`;showcase 行为测试 |
| `class:name={cond}`(含简写) | ✅ 条件类→元素样式整体响应式重算 | `class_directive_reactive` |
| `{@attach}` | ✅ effect 包装,(doc, 节点) 回调 | `attach_compiles` |
| `{#each}` 省略 `as` / 模板注释 / `<svelte:options>` | ✅(options 接受并忽略) | `each_without_as`、`comments_and_options_ignored` |
| snippet 作为具名 prop | ✅ 零参 snippet 自动包 Rc | `snippet_as_prop_auto_rc` |
| 简写属性 `{name}` | ✅ | `shorthand_attr_on_component` |
| `onpointerenter/leave` | ✅(shell 悬停派发);键盘 ⏳(焦点链) | showcase 行为测试 |
| `setContext`/`getContext` | ✅ `provide_context`/`use_context` | reactive 单测(含 keyed 行作用域穿透) |
| `mount`/`unmount` / 生命周期 / `tick` | ✅ | ui/reactive 单测 |

### CSS 兼容层首期(2026-07-17,"CSS 无缝支持"专项)

`<style>` 块从自造迷你语法升级为**真 CSS 语法的封闭子集**(战略见 DESIGN.md ADR-8,
调研 11/12):

| 能力 | 状态 | 实证 |
|---|---|---|
| 标准属性名(background-color/color/border-radius/flex-direction/gap 系,本地简名兼容) | ✅ | `css_compat_names_units_hover` |
| `px` 单位(em/rem/%/vw 报错引导,C1 计划) | ✅ | 同上 |
| 颜色:hex / `rgb()`/`rgba()` / 常用颜色名 | ✅ | 同上 |
| `.类:hover` 伪类 → 编译期悬停状态 + 指针接线 + 用户回调合成 | ✅ | 同上;showcase `.btn:hover` |
| 简写展开(padding 四值)/ 继承子集 / :active/:focus / var() / @media / margin/border | 📅 C1/C2 路线,见 ADR-8 | 调研 12 P0/P1 清单 |
| 永不支持(伪元素/:global/!important/@layer 等) | ❌ 文档化 + 替代写法 | ADR-8 |

### 对抗审查与修复(2026-07-17)

落地复核之后,一轮多智能体对抗审查(3 视角 finder + 逐发现反驳验证,全部经
rustc 探针实证)确认了 **17 个测试盲区缺陷**,已全部修复并补回归测试(27 个
compiler 测试):
- **所有权族**(E0507/E0382):生成的 move 闭包夺走非 Copy 普通变量(props/
  script let/each 绑定)——定型为两层预克隆纪律(节点级 + 重建闭包体级),
  代价是"模板引用的普通变量需要 `Clone`"(文档化约束);
- **遮蔽族**:fn 参数/match 臂/for/if-let/while-let 模式现已正确遮蔽;
- **宏族**:参数改写收敛到 fmt/断言/vec 白名单,非白名单宏里用反应式变量改为
  硬错误引导(matches!/stringify! 语义不再被破坏);
- **解析族**:rune 定位改为掩码文本(字符串/注释免疫)、UTF-8 char 边界、
  字符字面量/注释中的花括号、任意空白的 `as`、块关键字边界(`{#iffy}`);
- **组件契约**:默认值改为 callee 侧关联函数(求值语境单一)、空 `$props {}`
  与未声明的函数签名区分。

**方法论教训(写给后续里程碑)**:编译器测试只做 `syn::parse_file` 语法校验
抓不住所有权/类型错误——生成代码必须过 rustc。当前由 examples(counter-sfc/
todo-sfc,含刻意构造的触发场景)承担 typecheck;M1 应加独立的"生成代码
rustc 编译"测试夹具(trybuild 式)。

### 判定中拿不准、需主线程裁决的项

> **裁决(2026-07-17)**:`$state.raw`/`$inspect` 见上表(已实现,raw 为零差异别名);
> `bind:this` 维持 ❌({@attach} 三合一口径确认);`class` 对象/数组维持 ⏳(依赖
> `<style>` 块类系统);`<svelte:window>` 系维持 ❌ + `<sv:window>` 变体进计划;
> stores 维持 ❌(signal 全覆盖);`mount/unmount`/`$effect.root` 维持 🚧(函数式
> 等价物口径)。原始存疑记录保留如下,供后续复审:

- **`$state.raw` / `$inspect`**:按代码(无)与 08 裁决(不进 v0)标了 ⏳,与施工中事实冲突,
  语义定位(无 Proxy 的 sv 里 raw 与缺省的差异)需主线程结论。
- **`bind:this` 标 ❌**:依据是 09 "{@attach} 三合一"表述,但 09 未字面写"bind:this 不做",
  有推断成分。
- **`class` 对象/数组语法标 ⏳**:09 未涉及此 5.16 新形态,系本文自行裁定。
- **`<svelte:window>` 系标 ❌**:09 原话"砍,留变体"——HTML 形态标 ❌、`<sv:window>` 变体
  写进计划栏,口径可议。
- **stores 标 ❌**:08/09 未明说,依据 ADR-1(跨线程消息回写 signal)与"无历史包袱"推断。
- **`mount/unmount` 与 `$effect.root` 标 🚧**:以"函数式挂载 + create_root 为语义等价物"
  的口径认定部分实现,若按"rune/API 形态未提供"口径应降为 📋。
- **`{#each}` 省略 `as` 形态**:引入版本(记忆为 5.4+)未单独核实版本号,行内未标注版本。
- **`$state` 写改写的两处保真缺口**(字符串替换预处理、`+=` 无 RHS 预求值)是对照 08
  规则表得出的静态结论,未写运行时复现测试。
