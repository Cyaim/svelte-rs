# 未了结问题登记(截至 2026-07-24)

> 这一批工作横跨增量布局、ADR-2 ③、`sv check`、以及三种动画格式。
> 每条线各自的 README / 计划文档里都写了缺口,但**散着放就等于没写** ——
> 下一个人不会先读七份文档再动手。这里把它们收在一处,按"会不会咬人"排序。
>
> 规矩:**已知但没做**的写在这里;**不知道**的也写在这里并标明。
> 一条缺口从这里消失,只能是因为它被做掉了或被明确判为不做,不能因为被遗忘。

## ✅ 2026-07-23 复核轮已修复

- **增量 Measure(计划步骤 3 的安全子集)**(此前"未实现",本轮实现):一帧里若
  **只有 `Measure` 变更**(结构没动)且布局树留着,不再整棵重扔 —— 只 `set_style` +
  `set_node_context`(都标脏,taffy 最稳那层)让 taffy 重算脏子树。**§3.4 那五条
  taffy 陷阱一条不碰**(前提就是结构没变,从不 add_child/remove/reparent)。
  差分 fuzz(增量 vs 全量逐帧对拍)+ 定点测试(证明路径被走、树被复用、坐标逐个
  相同)双守。仍未做:结构变更的增量(那才是陷阱区,继续全量)、walk 优化(步骤 4)。
- **`.svelte` 语言服务器 `sv-lsp`(LSP MVP)**(此前"未实现",本轮实现):打开/改动
  `.svelte` → `compile` → `publishDiagnostics` 波浪线。零外部依赖(手写 `Content-Length`
  分帧 + JSON-RPC,协议解析复用 `sv_compiler::check::json`)。纯函数 `Server::handle`
  有单测,stdio 端到端冒烟过。仍未做:补全/跳转/hover(要符号表)。
- **PAG 差分帧重放 + WebP 解码(全链打通)**(此前"未实现",本轮实现):
  `sv_pag::replay_frame`(**仍零依赖**,解码器注入回调)从最近关键帧逐帧覆盖脏矩形
  还原整帧;`sv_shell::register_pag` 进 Frames 注册表 → 场景树。**WebP 解码已接上**:
  网络恢复后加了 `image-webp`(纯 Rust,MIT/Apache)到 sv-shell,
  `register_pag_webp` 用它解码;端到端测试**用真编码的 WebP 字节**(image-webp 编码
  →解码→重放→注册)跑通,不是假解码器。仍缺:**真实 `.pag` 素材验证**(仓库仍无
  真文件,固件是手工构造 + 真 WebP 块);容器解析本身仍未在 AE 导出的真 `.pag` 上验过。
- **Lottie 矢量档接入场景树**(此前"未实现",本轮实现):`sv-shell` 新增
  `sv-lottie` 依赖 + `register_vector`/`render_vector` + `PainterSink` 桥
  (`sv_lottie::PathSink` → `Painter` 同形动词转发);矢量动画节点现在每帧
  现算路径直发 `Painter`,不落位图。端到端记录型测试守住"发出填充路径 +
  裁剪栈平衡"。顺带确认"`PathCmd` 等公开但不可命名"的洞本 PR 已通过
  `paint` 的 re-export 补上(sv-lottie `path.rs` 的旧注释已随之更新)。
- **🔴 `sv-vap` largesize box 溢出 panic**(复核新发现,非本表原有):`find_vapc`
  对 size==1 的 64 位 largesize 做 `p + size`,debug 溢出 panic、release 环绕后
  越界切片 panic,打破"任何输入绝不 panic"承诺。已改 `checked_add` + 回归测试。
  同轮修 `VapConfig` 数值 `as u32` 截断(2^32+1→1),超 u32 一律报 `BadGeometry`/`BadRect`。
- **§5.2 三道穷尽解构闸门只装了一道**:`Style::eq` 与 `to_taffy` 补上无 `..` 的
  穷尽解构,给 `Style` 加字段而漏改 = 编译错误(此前只有 `layout_relevant` 有)。
- **`sv check` 包络档措辞**:`check.rs` 把"节点级近似"改为"行级近似",与
  `sourcemap.rs` 设计注释一致(节点栈未做,只能定位到行)。
- **`update_overlay_anchor` 埋雷注释**:注释原说"重走一遍就够"(暗示可降 Position),
  实际靠 `OverlayRegistry` 重建级承重,已改注释说明降级会静默丢锚点更新。
- **#7(下表)**:`CLAUDE.md` 的构建产物路径已订正为"默认 `./target`,重定向那行默认注释掉"。
- **文档横向同步**:CHANGELOG 补本轮(增量布局/`sv check`/三动画格式 + `bump` 必传
  破坏性变更);两语 `performance.md` 把已落地的帧调度/局部布局移出"尚未实现";
  DESIGN.md 的整帧基准数改以 membench README 为准(并订正 `bump` 点计数 34→42);
  docs/README 与根 README 的 ADR-1..10 / 调研 ×26 / 新 crate 与示例清单 / plans 入口。

## 手写组件能力面(不用 sv-arco 完成功能;调研 28 硬缺口的填补进度)

- **~~表单元素属性层 E0382/E0507~~** ✅ 2026-07-24(PR#62):`<input value={x} aria-label={x}>`、
  `value={x} oninput={…x…}` 等手写表单写法现可编译(八个元素级 move 闭包站点走
  `with_captured_plain`,@attach 另补 pre_call)。
- **~~`disabled` 属性(交互禁用)~~** ✅ 2026-07-24:`ViewNode.disabled` 位 +
  `set_disabled`/`is_disabled`;交互门在 `click_handler`/`pointer_down/up_handler`/
  `focusable` 统一收口(禁用节点拿不到回调、不可获焦,命中测试仍命中留给样式/tooltip);
  a11y `node.set_disabled()` + 不广告 Click/Focus 动作。codegen `disabled={表达式}`
  反应式 effect / 裸 `disabled` 恒 true。守卫:`disabled_gates_handlers_and_focus`
  (sv-ui)+ `disabled_attr_compiles`(sv-compiler)。
  - **⏳ Phase 2:`:disabled` 伪类样式**(未做):要把 disabled 态接进 `bind_style`
    重算(disabled 由 `disabled={expr}` 的 effect 改,非指针驱动的 `__hv` 类状态,
    重算闭包需跟随该 expr 的响应式)。当前 `<style>` 里写 `:disabled` 仍硬报错。
    禁用态**功能**已全,只差**视觉**(变灰);A2 表单件可先用条件类/内联样式绕行。
- **~~`oncontextmenu` 右键菜单~~** ✅ 2026-07-24:`ViewNode.on_context_menu:
  Fn(f32, f32)`(逻辑坐标)+ `set_on_context_menu`/`context_menu_handler`(禁用节点
  取不到,同点击门);shell `Pane::handle_window_event` 加 `MouseButton::Right` 命中
  派发;codegen `oncontextmenu={|x, y| …}`。用户在回调里按坐标开 overlay(Anchor::Point)。
  守卫:`context_menu_handler_carries_pos_and_disabled_gates`(sv-ui)+
  `oncontextmenu_compiles`(sv-compiler)。
- **仍缺(调研 28,sv-arco 补不了、须框架本体)**:`:disabled` 伪类样式(见上,Phase 2)、
  壳层 pointer-move(hover 跟随已有,move 事件回调未透出)/拖放/触屏/键盘滚动环、
  视觉字重/字体族/图标/grid/阴影、a11y 角色层、i18n locale。

## 🔴 会咬人的(动手前必须先处理)

| # | 问题 | 位置 | 为什么危险 |
|---|---|---|---|
| 1 | **`sv-pag` 从未在真实 `.pag` 上验证过**,一次都没有。固件全是按 libpag 源码手工构造 | `crates/sv-pag` | 整个 crate 的正确性依赖"我读对了规范"这一个假设。对照组:`sv-vap` 拿 10 个真素材跑过,当场发现两条我写错的断言 —— **手工构造的固件不会打脸,真文件会** |
| 2 | `sv-pag` 的 `BitmapSequence` 布局是**单源**(只有 libpag 的 C++ 那一份;libpag-lite 没有它) | 同上 | 它恰好是本 crate 最核心的结构,交叉印证缺位 |
| 3 | 位图序列帧档在真实素材里的**占比未知** | — | 它直接决定"c2 免 libpag"这条路线能覆盖多少素材。核实了"能读",没核实"设计师实际会不会这么导" |
| 4 | `sv-lottie` 只测过手写固件,**没跑过真实资产**(Tiger、Noto emoji 之类) | `crates/sv-lottie` | 同 #1。"渐变均值色好不好看""轨道遮罩会糊成什么样"目前全是按 velato 源码推断 |
| 5 | velato 在**合法** Lottie 上会在导入期 panic(**七处**:4 个 `todo!()` + 3 个 `unimplemented!()`,见 `lib.rs:154-168`);`catch_unwind` 只兜解析期 | `sv-lottie/src/lib.rs` | 根治要给上游提 PR。渲染期(`Renderer::append`)刻意不加 unwind 屏障(每帧热路径);velato 0.11 的两处渲染期 `unwrap`(`animated.rs:257`、`render.rs:306`)读下来均被上一行守住、判为不可达,但**这不是上游 API 承诺**,真炸只能靠调用方兜。裁剪栈污染由 `PainterSink::Drop` 补 `pop_clip` 覆盖 |

## ⚠️ 已知的不实/未查明

| # | 问题 | 位置 |
|---|---|---|
| 6 | **membench `deep` 两档与 `virtual 3000` 档比上一版慢 3–10%,没查明原因。** 已排除"留着布局树导致的内存压力"(关掉留树 `deep` 一点没变)。也可能是上一版基线本身偏乐观 —— 两批数字之间隔了十来个提交 | `examples/membench/README.md` |
| 7 | ~~`CLAUDE.md` 的构建产物路径与 `.cargo/config.toml`(注释掉的 `target-dir`)不符~~ **✅ 2026-07-23 已订正** | `CLAUDE.md` |
| 8 | `sv check` 的映射覆盖率 80.5%,**胶水代码(runes 改写产物)映射不回去**。这是设计边界不是 bug,但要防止有人当它是 bug 去"修" | `crates/sv-compiler/src/sourcemap.rs` |

## 各条线的欠账

### 增量布局(计划 `incremental-layout.md`)

- 步骤 3(增量 `mark_dirty`)未做:C 类变更仍是**整棵扔掉重建**。好处是 §3.4 那五条 taffy 陷阱一条都碰不到;代价是文本/结构变更没有增量。
- 步骤 4(walk 优化)未做:滚动帧的 0.66ms **全部**是 walk。
- 布局缓存是**单槽 thread_local**:两个窗口交替渲染会互相顶掉,退化成每帧全量。多窗口今天没支持,真做时要改成按 doc_id 分槽。
- `DirtyItem` 的 `from`/`to`、`InheritFontSize`、以及各条里的 `id`,**今天一个消费者都没有**(变异测试证实:去掉任一条,差分 fuzz 仍全绿)。它们是为步骤 3 预留的 —— 因为那些信息**记录时不抓就永远拿不到**。别拿它们的存在当"增量已经做了"的证据。

### 绘制端 —— **这是当前真正的瓶颈**

滚动帧整帧 12.45ms,其中布局只占 0.66ms。**剩下 12ms 全是绘制。**
`incremental-layout.md` §8.2 那句"如果只能排一件事,那件事是 shape 缓存"已经从推断变成有实测支撑的结论。脏矩形同理 —— 它是 lottie/PAG/VAP 的共同前置(动画一跑就整窗重绘)。

### ADR-2 ③(计划 `adr2-3-setup-render-split.md`)

- S1–S4 未做(要动 codegen)。已落地的只有 S5(热重载判据 + 槽位重映射)。
- **2026-07-23 内核合并(ADR-2 ①完成)后的新前提**:codegen 已是双前端共享
  的一份(`Cg`),③ 动工时的改造对象是共享内核而非 `.svelte` 专属 codegen;
  计划 §0.10 "view! 宏不跟 stamp"的预裁决届时需复议(否则 emit 建树词汇表
  的"唯一发射口"结构又会被拆开)。
- `tmpl.rs` 还欠 §9 表里的两条:`TNode::If`/`Key` 内联子节点数组、`Binder` 六变体。**刻意没提前加** —— 没有发射方的枚举变体是死代码。
- `remap_slots` **每次调用泄漏内存**(`TNode` 的 `binds`/`children` 是 `&'static`,改写产物只能新建 + leak)。量级是每次热重载几 KB,且只该出现在 dev 热通道。
- 热重载的**通道**(编译端产 sig、dev 端推送、运行端重放)整个没有。

### `sv check` / LSP(计划 `lsp-spike.md`)

- 真嵌套包络(节点栈)未做,region 粒度是"一个 parse 入口的**一行**"。计划 §3.2 点名"必须做",§723 又批准了第一版降级 —— 现在是后者。
- `build()` 仍 `panic!`,`sv check` 靠 scrape panic dump 兜住(**脆**,依赖 panic 文本格式)。计划里的 P-1(`cargo::error=`)没做,约 0.1 人周。
- **没在 VS Code 里真看过 Problems 面板**,只做了正则层验证。
- LSP server、VS Code 扩展都没做(按复核结论,应先用 tasks.json 跑两周再决定)。

### 动画:三种格式的现状

| | 解析 | 像素 | 进场景树 | 真实素材验证 |
|---|---|---|---|---|
| **VAP** | ✅ `sv-vap` | ✅(需外部 H.264 解码) | ✅ `examples/vap-gift` 端到端 | ✅ **10 个素材,含与 Python 参考逐字节对拍** |
| **PAG** | ✅ `sv-pag`(位图序列档) | ❌ 缺 WebP 解码 + 差分帧重放 | — | 🔴 **零** |
| **Lottie** | ✅ `sv-lottie` | ✅(自己发路径命令) | ✅ **2026-07-23 已接**(`register_vector` + `render_vector`) | ❌ 只有手写固件 |

共同欠账:

- ~~`<animation src>` 前端标签未做~~ **✅ 2026-07-23 `.svelte` 侧已做**:
  `<animation src="..." loop autoplay label="..." />` 叶子标签,建
  `ElementKind::Animation` 节点(sv-compiler:template/codegen/emit/style +
  `animation_compiles` 测试)。素材经壳侧 `register_vector`/`register_frames`
  接入(与 sv-ui/sv-shell 分层一致,模板层只建节点)。`view!` 宏按 ADR-2
  冻结策略不加(checkbox/textarea/overlay 同样只在 `.svelte`)。仍缺:构建期
  importer(把 `src` 转译+注册的胶水,与解码器决策同批)、play-loop 短路(§4.2)。
- **动画帧仍整窗重绘**。分级只让布局归零(`set_anim_frame` 是 Paint 级),绘制端没有脏矩形。ADR-6 里那段"别指望零功耗自动成立"依然成立。
- ~~`AnimSource::Vector` 恒返回 `None`~~ **✅ 2026-07-23 已接线**:壳侧新增
  `PainterSink`(sv_lottie `PathSink` → `Painter` 的同形动词转发)+ `render_vector`
  每帧经 velato 现算路径直发 `Painter`;裁剪成对、句柄失效静默不画,有端到端
  记录型测试守着(`vector_registers_and_renders_paths_into_the_painter`)。
- **没有解码器**:`sv-pag` 交出 WebP 字节、`sv-vap` 要 RGB24 输入,两者都把解码挡在外面。"引哪个解码器"是一次独立的重裁决,且与平台强相关。

### `draw_image` 的已知近似

- tiny-skia **无 mipmap**:缩小超过 2× 会走样,大图缩小需上层预缩放。
- vello 图集上限 8192,任一边超过就**静默不画**(上游私有常数,我们这侧看不见也留不了痕)。
- CPU 与 GPU **未做逐像素互相对拍**(两边各自对着同一份手算期望值断言)。
- `push_clip` 的 radius 在 CPU 端仍被忽略(与既有 `fill_rounded_rect` 同款近似)。
- 非整数缩放的双线性**没有逐像素断言**。

### `sv-vap` 自己的缺口

- **VAPX(融合动画)不支持**:`isVapx=1` 的素材带运行期动态元素(头像、昵称),解析成功并透出标志,但不处理那些元素。
- **只见过一种布局**(alpha 在右、半分辨率)。代码按配置矩形走、不假设方位,等分辨率那条路有测试,但**没有真实的"alpha 在下方"素材可验**。
- 音频没管(这些 mp4 带 AAC 礼物音效)。
- 手写 JSON 取值器假设值里**不含转义引号**。

### sv-arco 组件库(2026-07-24 起步,调研 26)

- **A0/A1 已落地**:sv-arco-tokens(色板 + global.less 转译,金样/同步/抽查
  三层测试)+ **A1 静态件七件**(Button/Tag/Badge/Divider/Alert/Typography/
  Link,行为测试 36 项 + arco-gallery 离屏 PNG 视觉验收)。
- **🔴 `if_block` 的包装节点参与布局**(A1 批新发现,内核级):`sv_ui::if_block`
  给每个 `{#if}` 建一个真实 View 容器(lib.rs `if_block`,默认 column/start)
  ——交叉轴拉伸与 flex 组合**穿不过它**:块内的"吃满宽"内容(如分割线)
  会塌成零宽,块内 flex-grow 的参考系也变成包装节点自己。Divider 被迫
  改成"恒渲染扁平结构 + 条件类清零"绕行(见组件注释)。根治方向:包装
  节点透传布局(display:contents 语义)或给 if/each 容器一套可配样式;
  动手前先评估对增量布局(布局树复用)的影响。
- **~~plain 变量进多个同级**块构造器**闭包会 move 冲突(E0382)~~ ✅ 2026-07-24
  已修(限块级)**:根因 = 预克隆 prelude 放在 `move` 闭包**体内**、`move` 按值
  捕获,`if_block`/`await_block`/`each_block`/`key_block`/`overlay_block` 的多个
  同级 move 闭包(分支 **与** cond/future/key/open 引导闭包、each list/row)争夺同
  一非 Copy plain 变量的所有权。修法 = 新增 `Cg::with_captured_plain`,给每个 move
  闭包在**闭包外**补捕获份 `{ let x=Clone::clone(&x); <move 闭包> }`(照搬 @render/
  snippet 已有形状),铺到全部**块级**引导+分支闭包。守卫:golden fixture
  `multiclosure`(结构 shape)+ **`examples/multiclosure-check`(**块级**借用检查,
  golden 只 parse 不 borrow-check,靠它兜块级 E0382 回退)**。Badge/Alert 的手工
  绕行(块内多分支引 String prop)现变冗余、可清。**注:emit::if_block 签名从
  `cond: expr` 改为 `cond_closure`**(前端负责建带捕获的驱动闭包)。
- **~~同一元素上 ≥2 个属性 move 闭包共享同一非 Copy plain 变量 → E0382~~** ✅ 2026-07-24
  已修:根因同块级(`emit_nodes` 只补**一份**节点级捕获,一个元素却发射多个 move
  闭包争抢它)。修法 = 给 emit_element 的各属性 move 闭包也走 `with_captured_plain`
  (散在多分支,逐个补捕获份):**effect 类**——`value` / 动态 `aria-label` /
  `checked` / `style:x` / `@attach`;**存储型处理器**——`onclick` / `oninput` /
  `onsubmit` / `onscroll` / `onpointerenter`/`onpointerleave`(非 hover 态)。处理器
  补捕获份还顺带消除了**发射顺序依赖**(`oninput` 写在 `value` 前也不再 E0382)。
  `@attach` 另有一坑:`self.expr(.., true)` 给它加了 `move`,该闭包在 FnMut effect
  体内**按值**吞 plain,每次调用都 move-out → E0507;故其 effect 体内再补一份
  **每次调用**预克隆(pre_call)。守卫:`examples/multiclosure-check` 与 golden
  fixture `multiclosure` 均已扩到元素级(input 的 value+aria+style:+oninput、
  checkbox 的 checked+aria、view 的 aria+onclick+@attach 同元素共享 String prop,
  build.rs 真编回退即翻红)。反应式 `$state`(Copy 信号)本就不受影响。
  - **窄残留(低发)**:`:hover`/`:active`/`:focus` 的样式**重算闭包**
    (`bind_style(move |s| …)`)与 `onfocus`/`onblur`/`onkeydown` 的**合成态**
    处理器(`__ue`/`__uf`/`key_handlers`)未走 `with_captured_plain`。触发条件苛刻
    (同一元素既有伪类样式态**又**在其重算/合成处理器里引同一非 Copy plain,
    且与另一 mover 同元素)。当前 `bind_style` 是每元素唯一且最后的 label 消费者,
    单闭包不自争;真正暴露需"伪类态 + 引 plain 的 onfocus/onkeydown + 另一 mover"
    三件套,A2 静态件未见。修法同款(合成态的 `let __ue = …` 前补捕获份),留待触发时补。
- **keyed `{#each}` 的 key 表达式引用非 Copy plain 变量 → E0373**(2026-07-24
  对抗评审查获,与上条同域但**机制不同**、暂未修):`{#each rows as it (key_using_plain)}`
  的 key 闭包是**非 move**(`|__item| {…}`),按引用借用 plain 变量,而
  `each_block_keyed` 要求 `key_of: Fn(&T)->K + 'static` → 借用函数局部无法 'static。
  修法:把 key 闭包改 `move` + 外层捕获份(但会给所有 keyed each 的 key 闭包加
  `move`、churn wide golden)。narrow(keyed each + key 引 plain,罕见),留后续。
- **keyed `{#each}` 的 key 表达式引用非 Copy plain 变量 → E0373**(2026-07-24
  对抗评审查获,与上条同域但**机制不同**、暂未修):`{#each rows as it (key_using_plain)}`
  的 key 闭包是**非 move**(`|__item| {…}`),按引用借用 plain 变量,而
  `each_block_keyed` 要求 `key_of: Fn(&T)->K + 'static` → 借用函数局部无法 'static。
  修法:把 key 闭包改 `move` + 外层捕获份(但会给所有 keyed each 的 key 闭包加
  `move`、churn wide golden)。narrow(keyed each + key 引 plain,罕见),留后续。
- **~~条件类上的 `:active`/`:focus` 被 codegen 静默丢弃~~ ✅ 2026-07-24 已修**
  (A1 批对抗评审查获,内核级):`sv-compiler/src/codegen.rs` 的条件类分支
  此前只把 `entry.hover` 收进 `hover_conds`,`active`/`focus` 变体没有对应
  向量,整块无声丢——Button/Link 用 `class:x={cond}` 承载全部变体,按压态
  因此从不生效(生成产物里 `__ac`/`set_on_pointer_down` 出现 0 次)。修法:
  加 `active_conds`/`focus_conds` 与 `hover_conds` 对称收集 + 在 active/focus
  block 里加条件臂。契约测试 `conditional_class_active_and_focus_variants_emit`
  (产物字符串断言 + 变异探针)守着;sv-arco Button/Link 补了 hover/active
  离屏行为测试。静态类路径本就正确,不受影响。
- **~~hover/active 视觉无自动化断言~~ ✅ 已补**:Button/Link 用
  `pointer_{enter,down,up,leave}_handler` 离屏直调,断言按压压过悬停、
  禁用态门控(hover 臂 `!disabled`)。
- **暗色模式未接**:tokens 的 `CSS_ROOT_DARK` 已生成,但 build.rs 只注入
  亮色块;换主题要等 `@media (prefers-color-scheme)`(C2)或组件加 mode
  prop 再议。**别拿"CSS_ROOT_DARK 存在"当暗色已支持的证据。**
- **focus-visible 缺**:arco 的键盘焦点环是 box-shadow(0 0 0 2px 色板-3),
  渲染动词 ⏳;`:focus` 伪类接线本身已通(上一条修的一部分),但没有可画
  焦点环的属性,Tab 落焦目前无视觉反馈,键盘可达性数据面(focusable/激活)
  是好的。
- **arco 的 1px 透明边框未补偿几何**(A1 批,minor):arco 各变体带 1px
  透明边框(border-box),Button 非 outline 变体与 Tag 未把这 1px 折进
  padding(Alert 折了),故同 label 的 outline 比 primary 宽 2px、Tag
  横向内缩少 1px/侧。亚感知级,视觉可忽略,未修:统一折算会被 CSS 子集的
  "padding 同时依赖变体与尺寸"卡住(padding 在尺寸类、边框在变体类,单类
  改不干净);彻底修等 box-sizing 可配或透明边框色支持。
- **组件跨 crate 无标签语法**:PropsRegistry 单构建目录扫描,`<Button>` 只
  在 sv-arco 自己的 components/ 内可用;对外交付 = Rust 函数 API。要给
  消费者 `.svelte` 标签体验,需要编译器支持外部组件注册表(未排期,与
  ADR-2 相关,动它之前先别承诺)。
- **variant/status/size 是字符串 prop,拼错静默失效**:落点分两种——默认
  形态由静态类承载的(Tag/Alert/Badge/Typography、Button 的 size)落默认;
  Button 的 variant/status 与 Link 的 status 默认外观由条件类携带,拼错落到
  **无变体裸基类**(透明底无字色),不是默认形态。Button 共 35 个条件类
  (32 变体×状态×disabled + 3 尺寸;sz-default 是静态类)。换枚举要么让
  `$props` 支持非 String 类型的字面量传参体验变好,要么生成侧校验——都没做。

### 内核合并(2026-07-23 对抗评审)遗留的存量小项

- **`idents_within` 把 `expr.field` 的字段名当变量使用**(`sv-compiler/src/codegen.rs`
  的 walk 不区分成员访问):`.svelte` 侧当行内有 `s.x` 且字段名 `x` 撞上普通变量名
  时会多发一行无意义的预克隆(无害但冗余)。修法:跳过"前一 token 是单个 `.`"的
  ident,**注意 `0..n` 的 `n` 前一 token 也是 `.`(range),须看再前一 token 是否
  也是 `.` 才能区分**;修复会让部分 golden 少一行死克隆,需随之重录——刻意没在
  内核合并批里做(那批的验收基线是 .svelte 产物逐字节不变)。宏路径不受影响
  (Tokens 形态已不进 plain 集合,契约测试钉着)。
- **暂不首发的 crate 未设 `publish = false`**(sv-lsp / sv-pag / sv-lottie / sv-vap):
  CHANGELOG 首发清单已写明六 crate 口径,但没有机器可查的约束;依赖序脚本或
  `cargo publish --workspace` 有误发风险。定夺后给四个 Cargo.toml 加
  `publish = false`(注意 publish-readiness CI job 按 `select(.publish != [])`
  过滤,加了会自动跳过它们的元数据检查——行为正确但覆盖面变化要知情)。

### R4 发布工程

- ~~改名(ADR-10 待裁决)~~ ✅ 已裁决并落地(`svelte-rs` + `.svelte`;
  标识符批 2026-07-23:`compile` 族 + `sv` 二进制)。
- `cargo-semver-checks` 需要已发布基线才有意义。
- ~~双前端内核合并~~ ✅ 2026-07-23 落地(公共 IR `sv_compiler::template` +
  共享 codegen,两前端只剩 parser;span 精度有契约测试守护)。API 冻结前置
  已全部出清,首发关键路径推进到依赖序发 crates.io。
- ~~**🔴 首发清单被依赖打破**~~ ✅ **2026-07-24 已修**(feature 化路线):
  `sv-lottie`/`sv-pag`/`image-webp` 在 sv-shell 改为**可选**(feature `lottie`/`pag`,
  默认关),velato 的上游 panic 面不再进默认依赖树;首发清单据实改为 **8 crate**
  (optional 依赖仍须发布,故 sv-lottie/sv-pag 随首发,但默认构建不拉它们);
  `publish = false` 已加到 **sv-lsp / sv-vap**(它们不在首发依赖闭包)。CI 补
  `lottie,pag` 特性 matrix + clippy 道保住那 5 个门控测试的覆盖。**仍未做**:
  依赖序 `cargo publish --dry-run` 全链演练(需真 registry 环境,CI 现只真打包
  叶子 sv-reactive)+ 版本跳 0.1.0。
- 商标粗查/GitHub org 查重仍未做(ADR-10 裁决时风险由维护者书面自担,
  DESIGN.md ADR-10 注)——是"知情承担"不是"已排除";首发公告前顺手项:
  README 首行残留 working name(`# svelte-rs (working name \`sv\`)`)应清。

## 方法论:这一轮反复出现的几类错

写在这里是因为它们**每一条都真的发生了**,而且不止一次:

1. **量尺进入被量对象。** 探针让 30000 个叶子共享两种文本串 → 缓存永远命中,量到的是"没有 measure 成本的布局",偏 3 倍;A/B 开关用 `env::var_os` → 18 万次查环境变量自己成了大头,量出"关掉优化反而慢 8 倍"。
2. **假绿的测试。** 差分 fuzz 第一版:树在留树阈值之下、随机 op 落默认分支、滚动打在非滚动容器上 —— 故意把分级改错也照样全绿。**新测试必须做变异验证**,不然它只是让人安心。
3. **恒真的断言。** 对 `map` 的结果断言长度守恒(由类型系统保证);对 `u32::MAX` 溢出断言返回 `None`(wrapping 实现也返回 `None`)。
4. **在一层缓存前面加一层缓存,会让后面那层的自适应失效。** 叶内 memo 把查询量砍掉四分之三,下游的容量棘轮就爬不上去,30k 档从 96ms 劣化到 365ms。
5. **基线塌方时量出来的收益,不能拿到基线修好之后用。** 计划预测叶内 memo 降 20%,实测慢 29% —— 因为它的对照表量于缓存正在颠簸时。
6. **"公开但不可命名"的 API 洞,crate 内的测试结构上看不见。** 连着出现两次(`PathCmd` 五个、`PixelImage` 一个),两次都是第一个外部消费者当场编译失败。守卫必须从外部 crate 真写一个 `impl`。
7. **子系统的比值不是用户体感。** 布局快 44 倍,整帧只快 1.63 倍,剩下全是绘制。
8. **真实素材的形态跨度比直觉大得多。** "alpha 应当同时有透明区与不透明区"这条看起来无害的断言,被全屏不透明的背景素材和几乎全透明的细长物体各打脸一次。
