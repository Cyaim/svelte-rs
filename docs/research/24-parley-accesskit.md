# 24 · Parley 文本栈迁移 + AccessKit 无障碍接入:档 B 两大件落地设计

> 日期 2026-07-18。方法:先逐行核对 sv-shell(render/paint/font/vello_backend)与
> sv-ui/sv-compiler 的真实接缝,再联网核实 parley/fontique/accesskit 版本现状与
> 七个对标框架的实际选型(crates.io API + GitHub 源码级证据),给出类型级改动
> 清单、阶段化验收与人周估算。未能核实处显式标注。

## 0. 一句话结论

两件事共享同一前置——glyph run 载体扩成"带字体身份的 run"。Parley 迁移的真实
半径比 ADR-3b 的"只动 shaping 门面"多一圈:门面本身确实窄(render.rs 三个自由
函数),但**单字体假设已泄漏进 GlyphKey/Painter::glyph_run/字形缓存/vello
FontData 四处**,签名要一起动;AccessKit 则是纯增量接入(场景树→语义树纯函数
映射 + accesskit_winit 适配器),唯一硬依赖是焦点链(TreeUpdate.focus 每次必填,
与同批调研 20 强耦合)。合计约 10–15 人周,全部落在 M2,与调研 19 档 B 判决吻合。

## 1. 业界先例(联网核实,2026-07-18)

### 1.1 谁在用 Parley / AccessKit

| 框架 | 文本栈 | 无障碍 | 证据 |
|---|---|---|---|
| Blitz(Dioxus) | parley | accesskit(blitz-shell) | README |
| Masonry/Xilem | parley + fontique | accesskit | README |
| Floem(Lapce) | parley 0.7 + swash 光栅 + taffy | **无** accesskit 依赖 | Cargo.toml |
| Slint | 自研渲染器内部 | accesskit 0.24 + accesskit_winit 0.33(winit 后端,`accessibility` feature) | Cargo.toml |
| egui | 自研 epaint | AccessKit(PR #2294,2022-12 合入) | PR |
| iced / COSMIC | cosmic-text | 上游未合入;System76 fork 内置 iced_accessibility(`a11y` feature) | libcosmic Cargo.toml |
| Zed GPUI | **不用 parley**:core-text(mac)/windows crate(DirectWrite 推断)/font-kit fork | 未查证 | gpui Cargo.toml |

要点:**parley 排版 + swash 光栅是 Floem 已走通的组合**,与我们现状(swash 光栅
已就位)同构,迁移只换排版半层。Zed 反例证明平台原生文本栈可行,但三平台三份
实现,与"自研面收敛到编译器+响应式+组件运行时"的维护面策略(调研 19 §4)相悖,
不采纳。

### 1.2 parley 版本现状:比调研 05 记录的 0.3 已跑出八个版本

crates.io 实测:**parley / fontique 0.11.0(2026-06-26)**、harfrust 0.12.0、
swash 0.2.10、accesskit 0.24.1、accesskit_winit 0.33.2。关键变更(GitHub
releases;发布年份以 crates.io 为准,GitHub 页面摘要有年份歧义):

- 0.6:shaping 从 swash 换 **HarfRust**,剥离 kurbo/peniko 依赖;
- 0.7:修大段落布局非线性耗时;
- **0.8:字形坐标系翻转(breaking)**、text-indent;
- 0.9:inline/floating box 与 exclusions(文字环绕);
- 0.10:复杂文种 feature 化;0.11:换行定制、样式 From 转换。

判断:一年 5 个 breaking minor,**API 未稳**——调研 05 的"TextEngine 门面隔离"
从建议升级为强制项;锁 minor 版本,季度集中升级。

### 1.3 PlainEditor:可以直接喂 TextInput

parley::editing(docs.rs 0.11)= `PlainEditor`(持文本+选区+IME 组合态,单一
样式)+ `PlainEditorDriver`(短生命周期写入包装)。读侧方法已逐个核实:
`raw_compose`/`is_composing`(IME preedit)、`cursor_geometry`、
**`ime_cursor_area`(正好对接 winit `set_ime_cursor_area` 做候选窗定位)**、
`selection_geometry(_with)`、`selected_text`、`text`/`raw_text`/`set_text`、
`layout`/`refresh_layout`、`try_accessibility`(accesskit feature,选区/文本
属性直通语义树)。写侧动词(插入/删除/光标移动/compose 提交)在 Driver 上,
本次**未逐一核实方法名**。判决:同批调研 21 的 TextInput **不必自研编辑内核**,
value/选区/IME 状态机整体外包 PlainEditor。

### 1.4 AccessKit 与 accesskit_winit 的用法形态

- accesskit 0.24.1:`TreeUpdate = { nodes: Vec<(NodeId, Node)>, tree:
  Option<Tree>, focus: NodeId, tree_id }`。**nodes 支持增量**(只提交新增/变更
  节点);tree 仅初次与全局信息变化时必填;**focus 每次必填**(无焦点填 root)。
  平台 adapter:Windows UIA / macOS NSAccessibility / Unix AT-SPI / Android,
  官方称"大致功能对齐"。
- accesskit_winit 0.33.2:`Adapter::with_event_loop_proxy / with_direct_handlers
  / with_mixed_handlers` 三种构造;每个 winit 事件先过 `process_event(window,
  &event)`;回给宿主三个事件:`InitialTreeRequested`(此刻才需要建树)/
  `ActionRequested` / `AccessibilityDeactivated`。
- egui PR #2294 的实现形态(AccessKit 作者 mwcampbell 亲写,Google Fonts 资助,
  2022-12-04 合入):**懒激活**——AT 首次请求前零成本,请求时先回占位树、下帧
  回真树;widget 映射复用既有 WidgetInfo;AT 动作经 ActionHandler 回派;初版仅
  Windows。懒激活 + 变更时推送是业界标准姿势,直接照抄。

## 2. 现状核对:"只动 shaping 门面"需要修正

逐行核对结果:render.rs 侧成立——shaping 只集中在 `shape_text` / `measure_text`
/ `line_metrics` 三个自由函数(render.rs:71–85, 303–330),半径确实窄。但**载体
类型不成立**,单字体假设泄漏在四处:

| 位置 | 证据 | 迁移动作 |
|---|---|---|
| paint.rs `GlyphKey { id, px_bits }` | 注释自认"单字体前提下唯一决定一张位图" | 加 `font_key: u64`(fallback 后同帧多字体) |
| paint.rs `Painter::glyph_run(&[GlyphPos], Color)` | trait 文档:"字体是全局单字体" | 签名加字体参数:`glyph_run(&FontHandle, &[GlyphPos], Color)` |
| paint.rs glyph_cache `rasterize` | 硬编码 `crate::font::ui_font()` | 按 FontHandle 光栅 |
| vello_backend `VelloPainter { font: FontData }` | 构造期一次性 `ui_font_data()` 建单字体,`draw_glyphs(&self.font)` | 每 run 由其 Blob 建/缓存 FontData |
| font.rs `CANDIDATES` | 三平台硬编码路径探测,选中即全局唯一 | 整文件退役,换 fontique Collection |

好消息:Painter/GlyphPos/GlyphKey 是 **sv-shell 私有词汇**,不外露到 sv-ui 与
编译器(CLAUDE.md"绑定原语签名"约束不触发);RecordingPainter 金样与 GPU/CPU
parity 测试正是为这次迁移准备的对拍工具(调研 14 迁移八步在自家后院重演一次)。

## 3. 方案 A:Parley 迁移

### 3.1 TextEngine 门面(新文件 sv-shell/src/text.rs)

- thread-local 持 `fontique::Collection`(系统后端:DirectWrite/CoreText/
  fontconfig,mmap 懒加载)与 `parley::LayoutContext`(复用 scratch 分配)。
- API 三个动词,对应替换现有三函数:
  `measure(text, px, max_width: Option<f32>) -> (w, h)`;
  `layout(text, px, max_width) -> TextLayout`(包 parley Layout,迭代
  `lines() → PositionedLayoutItem::GlyphRun`,产出 `(FontHandle, Vec<GlyphPos>)`
  序列);行度量取自 Layout 而非手写 ascent 累加。
- `FontHandle = { blob: Blob<u8>, index: u32, key: u64 }`:CPU 端由 blob+index 建
  swash FontRef 光栅(Floem 同款组合;swash 0.2.10 在维护);GPU 端由同一 blob 建
  peniko::FontData(零拷贝,与现 `ui_font_data` 思路一致)。
- render.rs 改动:`measure()`/`place()`/`paint_tree` 的 `font: &FontRef` 参数链
  整条删除(layout_tree 的 `ui_font()` 调用点消失);Text/Button 分支改调
  TextEngine;`hit_click_target`、布局缓存(版本键控)、继承解析全部不动。
- locale 注入:Query 带 `Language`(zh-Hans)+ `Script`,避免"日式汉字"消歧
  问题(调研 05 §3.2);默认族 `GenericFamily::SansSerif`。

### 3.2 fallback / emoji / BiDi / 富文本 / 换行

- **fallback 链**:现状单字体,微软雅黑覆盖外(emoji、生僻字、藏文等)直接
  .notdef 方框;fontique Query 按 script/locale 走系统 fallback,混排逐 run 换
  字体——这正是 glyph run 载体必须带字体身份的原因。
- **BiDi**:parley Layout 内建 bidi-reordering,免费获得。
- **富文本/多字号 run**:场景树模型是"每 Text 节点单样式"(font_size/fg 继承
  解析已有);v1 保持单 style 布局。parley RangedBuilder 支持一段文本多 span,
  但那要求跨子节点的 inline 流合并排版,与现行"每 Text 节点独立盒"冲突——
  留到 taffy 期裁决(见 §6 开放问题)。
- **换行**:`layout(max_width)` 一行就通;难点在约束从哪来——现布局是
  measure-then-place、无可用宽度下传,v1 仅对**显式 width 节点**换行,完整
  换行随 M1 taffy 的 measure 闭包(把 available width 喂给 TextEngine)。

### 3.3 PlainEditor → TextInput(衔接同批调研 21)

- **分层裁决:parley 类型不进 sv-ui**(sv-ui 是双前端编译目标,依赖面必须干净)。
  sv-ui 侧:`ElementKind::TextInput` + `ViewNode.text` 复用为 value +
  `on_input: Option<Rc<dyn Fn(&str)>>`/`on_keydown` 回调槽 + focusable 位
  (调研 20);新增 `bind_value` 绑定原语(仿 bind_text + bind:checked 双向形态)。
  ElementKind 加变体连带:render.rs measure/paint 两处 match、`dump()`、
  sv-macro/sv-compiler 标签表(CLAUDE.md:两前端同步改)。
- sv-shell 侧:`HashMap<ViewId, PlainEditor>` 编辑器池;焦点节点的
  KeyboardInput/Ime 事件驱动 PlainEditorDriver;提交时取 `text()` 调 on_input →
  用户 signal → bind_value effect 写回 node——用 parley `Generation` 比对防回声
  循环。渲染:焦点 TextInput 由编辑器的 layout 出 glyph run(含 preedit 下划线
  区),`cursor_geometry`/`selection_geometry` 画光标与选区。
- winit 接线(sv-shell/src/lib.rs `window_event` 新分支):`KeyboardInput`、
  `Ime::{Enabled, Preedit, Commit, Disabled}`;焦点进入 TextInput 时
  `set_ime_allowed(true)`,`ime_cursor_area()` → `set_ime_cursor_area`(候选窗
  跟随)。
- sv-compiler 侧:template.rs `Tag` 加 TextInput(`<input>`);codegen.rs 属性
  分派现状是硬白名单——`onclick|on:click` 之外的 `on:` 一律报"v0 只支持
  on:click"(codegen.rs:934),`bind:` 仅 checked(:817)——扩:`bind:value`
  (走 `__b_sig` 双向形态)、`on:input`/`on:keydown`/`on:focus`/`on:blur`;
  错误提示文案同步收窄。

## 4. 方案 B:AccessKit 接入

### 4.1 树映射(新文件 sv-shell/src/a11y.rs,纯函数)

`fn build_tree_update(doc: &Doc, placed: &[Placed], focus: ViewId, scale: f32)
-> TreeUpdate`,与 RecordingPainter 同哲学:**零窗口零平台即可金样测试**。

- **NodeId**:ViewId(slotmap key)`KeyData::as_ffi() → u64` 直转,含世代号,
  节点删除后 id 不复用,天然满足 accesskit 稳定性要求。
- **role/属性来源**:View→GenericContainer、Text→Label、Button→Button、
  Checkbox→CheckBox、TextInput→TextInput;name 取 `node.text`(可被新增的
  `Doc::set_accessible_label`(.sv 侧 `aria-label` 属性)覆盖);Checkbox 勾选
  → toggled;TextInput 值 → value,后续经 PlainEditor `try_accessibility` 直通
  选区/文本属性(parley accesskit feature 的存在意义)。
- **bounds**:来自 `Placed.rect`(命中测试同源,天然一致);逻辑/物理坐标与
  adapter 期望的对齐**未实证**,P4 首个验证项。
- **父子结构**:Placed 是平铺画序,树结构回 `DocumentInner.nodes` 读
  parent/children——`Doc::read` 现成。
- **virtual_list**:只实例化视口槽位 → 语义树天然只有几十节点,朗读跟随滚动;
  完整"列表共 N 项"语义(ScrollView role + 集合属性)列档 B 打磨。

### 4.2 winit 集成与推送策略(sv-shell/src/lib.rs)

- **UserEvent 改造**:现 `EventLoopProxy<()>` 仅作 tasks 唤醒 → 改
  `enum UserEvent { Wake, Access(accesskit_winit::Event) }`,
  `EventLoop::with_user_event::<UserEvent>()`;`resumed()` 建
  `Adapter::with_event_loop_proxy(&window, proxy)`;`window_event` 首行
  `adapter.process_event(&window, &event)`。
- **懒激活 + 变更时推送**(egui 同款):`InitialTreeRequested` → 建全量树、置
  active;此后复用 `paint()` 已有的 `last_frame_key` 版本比对——版本变更帧顺手
  `adapter.update_if_active(|| build_tree_update(..))`。**不做每帧推送**:静止帧
  短路已是架构基调,a11y 跟随同一节拍。v1 全量 TreeUpdate(协议合法,节点数小);
  增量(Doc 侧脏节点集)列档 B 打磨。
- **动作回派**:`ActionRequested` 按 NodeId 反查 ViewId,点击类动作走
  `click_handler`,焦点类动作走焦点链 API(调研 20);具体 Action 枚举变体名
  (Click/Default 的更名史)**未核实**,实现时以 docs.rs 为准。
- **焦点联动**:TreeUpdate.focus 每次必填 → **焦点链必须先行或同批**;未落地
  期可 focus=root 做"只读朗读"降级(调研 05 对 OHOS 的同款策略)。

### 4.3 平台完成度

桌面三平台 adapter 现成(0.24.1);鸿蒙无 adapter 且路线图未提——自研
accesskit-ohos 桥(ArkUI_AccessibilityProvider 回调模型与 TreeUpdate 语义同构,
但系统侧是**拉取式**、AccessKit 是推送式),**2–4 人周出原型**(引调研 05
§5.2 估算,含真机 ScreenReader 验证的上修风险)。

## 5. 分步落地

| 阶段 | 内容 | 验收(测试名) | 人周 | 置信 |
|---|---|---|---|---|
| P0 载体扩宽 | GlyphKey+font_key、glyph_run 加 FontHandle、vello per-run FontData;**行为不变** | 全量测试绿;`recording_painter_golden` 含字体身份重放稳定 | 1 | 高 |
| P1 Parley 接管 | TextEngine 门面、fontique fallback、font.rs 退役 | `mixed_cjk_emoji_no_notdef`、`kerning_changes_linear_advance`、`vello_offscreen_parity` 维持 0.5–2.0 | 2–3 | 中 |
| P2 换行 | 显式宽度节点 max_width 换行、多行高度进布局 | `text_wraps_at_explicit_width`、`wrap_height_grows_layout` | 1–2 | 中(与 M1 taffy 交错) |
| P3 TextInput+IME | 编辑器池、Keyboard/Ime 分支、set_ime_allowed/cursor_area、bind:value / on:keydown 编译面(双前端) | `textinput_type_signal_roundtrip`(无头)、`sfc_bind_value_compiles`;中文 IME 三平台手测矩阵 | 3–4 | 低(IME 长尾) |
| P4 a11y 树映射 | a11y.rs 纯函数 + sv-ui 语义字段(aria-label/focusable) | `a11y_roles_names_bounds_golden`(无头) | 1 | 高 |
| P5 adapter 接线 | UserEvent 改造、懒激活、动作回派、焦点联动 | `a11y_action_click_dispatch`(无头);NVDA/VoiceOver/Orca 朗读冒烟 | 1–2 | 中 |
| P6 档 B 打磨 | 增量 TreeUpdate、列表/滚动语义、PlainEditor 文本属性直通 | `a11y_update_only_dirty_nodes` | 1–2 | 中 |

合计 10–15 人周;P2 依赖 M1 taffy 进度、P3/P5 依赖调研 20 焦点链,非纯串行。

## 6. 风险与开放问题

1. **parley 0.x 破坏性节奏**(一年 5 个 breaking minor,0.8 坐标系翻转)——门面
   +锁版本硬性缓解;金样测试在 P1 全翻新一次,属一次性成本。
2. **shaping 变贵**:线性 advance 求和 → 真 shaping(HarfRust)。virtual_list
   视口行数小、静止帧短路仍在,预期可承受;但 100k 全量档(调研 17/18 基线)
   须复测,超预算则加 per-node Layout 缓存(text+px+width 键)。
3. **IME 平台长尾**(Wayland text-input 差异、候选窗定位、OHOS 软键盘避让)——
   调研 05 风险 #3 原样成立,P3 置信度低的主因。
4. **富文本 inline 流**与"每 Text 节点独立盒"模型冲突:span 合并进单 Layout
   还是保持独立盒,taffy 期裁决;错裁会返工 P2。
5. **AccessKit bounds 坐标空间**(逻辑/物理/transform)与各平台 AT 实测行为
   未实证;egui 的 2026 现行实现是否仍是 PR #2294 形态未复核。
6. **focus 必填**导致 a11y 与焦点链强耦合,排期上焦点链(调研 20)必须先行。
7. **OHOS 桥的拉取式阻抗**:2–4 人周是"纯适配器"假设,真机可能上修(调研 05
   风险 #5)。
8. 未核实清单:PlainEditorDriver 写侧方法名;accesskit Action 变体名;Zed
   Windows 端 DirectWrite(由 windows crate 依赖推断);parley 各版本发布年份
   (GitHub 摘要与 crates.io 歧义,取 crates.io);swash 光栅被 Glifo 替代的
   时间表(Linebender 方向,未见承诺)。

## 7. 结论:最小可商用切片

- **档 A(内部工具)**:P0–P3 必做——fallback 链让混排不再出方框、显式宽度
  换行、TextInput 中文输入可用(IME 可粗糙但必须有,调研 19 九项交集第 2 项)。
  **AccessKit 可为零**——egui 先例:商用(Rerun)两年后才接 AccessKit。
  即文本侧 7–10 人周。
- **档 B(单桌面平台可商用)**:+P4–P6 全量,叠加三平台屏幕阅读器冒烟
  (NVDA/VoiceOver/Orca)、IME 三平台手测矩阵、焦点链完整(Tab/快捷键);
  政企/消费级采标场景无障碍会升格为硬门槛(调研 19 §1)。
- **档 C(四平台)**:+accesskit-ohos 桥 2–4 人周与 OHOS IME 真机验证
  (调研 03/05)。

## 出处

- crates.io 版本(2026-07-18 实查 API):https://crates.io/crates/parley ·
  https://crates.io/crates/fontique · https://crates.io/crates/accesskit ·
  https://crates.io/crates/accesskit_winit · https://crates.io/crates/swash ·
  https://crates.io/crates/harfrust
- parley releases(0.6–0.11 变更):https://github.com/linebender/parley/releases
- parley 文档 / PlainEditor:https://docs.rs/parley/latest/parley/ ·
  https://docs.rs/parley/latest/parley/editing/struct.PlainEditor.html
- fontique 文档:https://docs.rs/fontique/latest/fontique/
- accesskit TreeUpdate:https://docs.rs/accesskit/latest/accesskit/struct.TreeUpdate.html
- accesskit_winit:https://docs.rs/accesskit_winit/latest/accesskit_winit/
- egui AccessKit 集成:https://github.com/emilk/egui/pull/2294
- Blitz 技术栈:https://github.com/DioxusLabs/blitz
- Masonry/Xilem 技术栈:https://github.com/linebender/xilem
- Floem 依赖:https://github.com/lapce/floem/blob/main/Cargo.toml
- Slint winit 后端依赖:https://github.com/slint-ui/slint/blob/master/internal/backends/winit/Cargo.toml
- libcosmic(iced fork + a11y):https://github.com/pop-os/libcosmic/blob/master/Cargo.toml
- Zed GPUI 依赖:https://github.com/zed-industries/zed/blob/main/crates/gpui/Cargo.toml
