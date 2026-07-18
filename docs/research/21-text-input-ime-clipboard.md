# 21 · 文本输入 + IME + 剪贴板:最大不确定段的落地方案

> 日期 2026-07-18。方法:①业界五框架(winit 协议层 / iced / egui / COSMIC / Slint /
> Masonry-Parley)的 IME 实现路径联网核实,出处均附 URL;②对准本仓库真实代码
> (sv-ui lib.rs / sv-shell lib.rs+paint.rs+render.rs / sv-compiler codegen.rs)给出
> 改哪个文件、动哪个类型的施工级设计;③分步清单带验收测试名与人周估算。
> 定位:回应调研 19 的判决——"自绘文本栈是所有先例中最贵或被直接外包的一段",
> 按"最短商用路径"第 1–2 步(键盘通道 → 文本输入+剪贴板+IME)探底。

## 0. 一句话

**单行 TextInput 不需要新渲染动词**(光标/选区/预编辑下划线全是矩形,现有
`fill_rounded_rect` 就够;唯一新增 `push_clip/pop_clip` 同时是滚动体系的前置)——
真正的成本在**协议编排**(winit IME 四事件 × 焦点 × 光标区域上报)与**平台长尾**
(iced 合并主 PR 后还追了 3 个修复 PR)。总量粗估 6.5–10.5 人周到档 A 水位。

## 1. 业界先例(联网核实)

**winit 协议(所有人的共同地基)**:`Ime` 事件四态——`Enabled` /
`Preedit(String, Option<(usize, usize)>)`(组合中文本 + 字节区间光标,`None` 隐藏
光标、空串表示清除)/ `Commit(String)`(上屏;**winit 会在其前发一个空 Preedit**)/
`Disabled`。窗口须先 `set_ime_allowed(true)` 才收 Ime 事件;预编辑期间窗口
**收不到 KeyboardInput**;候选窗定位靠 `set_ime_cursor_area(position, size)`
(X11 据社区反馈仅位置生效)。<https://docs.rs/winit/latest/winit/event/enum.Ime.html>

**iced 的教训(与我们同构:winit + 自绘,参考价值最高)**:社区 2020 年就有 IME
尝试(PR #686 定位候选窗、#1474 basic IME,均未走通),正式支持拖到
**PR #2777(2025-02-03 合并,0.14 里程碑)**——维护者 hecrj 大幅重构后落地
"over-the-spot"预编辑:预编辑文本直接画在输入框内,组合区间用前景/背景反色标示;
widget 通过 `InputMethod::Open` 带选区**主动声明**接受 IME(可 opt-out)。
合并后仍追修:#2785(IME 激活时占位符没消失)、#2790(预编辑窗字号未跟随 widget)、
#2793(候选窗初始位置错)。教训:**主链路只是及格线,字号/占位符/初始定位这类
"第二层细节"各是一个独立 bug**。<https://github.com/iced-rs/iced/pull/2777>

**COSMIC(iced fork)**:libcosmic 自研 text_input 控件(cosmic-text 驱动,含光标
句柄/剪切复制粘贴菜单),但客户端 IME 直到 COSMIC 1.0(2025-12,调研 19)发布时
仍未闭环——官方 Epoch 2/3 路线图把 **"Adds IME and fcitx support" 挂在
"libcosmic Iced rebase"**(即回迁上游 0.14 的 IME)名下。反向印证:**fork 自补
IME 比等上游还贵,大厂也选择等**。<https://system76.com/blog/post/cosmic-epoch-2-and-3-roadmap>
<https://github.com/pop-os/libcosmic>

**egui**:`TextEditState` 持 `ime_enabled`/`ime_cursor_range`,每帧把 widget 与
光标 rect 写进 output 供 egui-winit 上报(`allow_ime`/`ime_rect_px`);同样有长尾
——#4896 修焦点得失时的 IME 状态、#5198 在 Linux 上"重新启用"IME(曾因崩溃禁用)。
<https://github.com/emilk/egui/issues/248> <https://github.com/emilk/egui/pull/5198>

**Slint**:CHANGELOG 1.3.0(2023-11-10)"Added initial support for input methods
with pre-edit text"——立项三年后才有 preedit,且叫 "initial"。
<https://github.com/slint-ui/slint/blob/master/CHANGELOG.md>

**Masonry / Parley(我们 M2 的既定归宿)**:Masonry 有 TextInput/TextArea,文本栈
即 Parley;Parley `PlainEditor` 原生带 IME 模型——`raw_compose()`(预编辑区间)、
`is_composing()`、`ime_cursor_area()`(上报矩形)、`cursor_geometry()` /
`selection_geometry()`(光标/选区几何)。**这意味着本方案 v0 自写的编辑模型在
M2 有一条现成的迁移路**:接口对齐 PlainEditor 的动词即可。
<https://docs.rs/parley/latest/parley/editing/struct.PlainEditor.html>
<https://docs.rs/masonry/latest/masonry/>

**cosmic-text(备选参照)**:`Edit` trait 的动作集(`action()`/`insert_at`/
`delete_range`/`copy_selection`/`delete_selection`/`set_cursor`/`set_selection`)
是成熟的编辑操作分类,v0 的 EditOp 词汇表照此裁剪。
<https://docs.rs/cosmic-text/latest/cosmic_text/struct.Editor.html>

## 2. 方案设计(对准本仓库代码)

### 2.1 元素与状态归属

`sv-ui/src/lib.rs:178` 的 `ElementKind` 加 **`TextInput`(单行先行)**。状态归属
沿用 Checkbox 先例(节点 `checked` 字段为渲染真源,Signal 经 bind:checked 接线,
codegen.rs:817–854):**value 复用 `ViewNode.text`;编辑态放节点内新字段**——
光标/选区/预编辑是"控件私有交互态",放 Signal 会把每次移光标都变成用户可见的
响应式事件,污染依赖图(egui/iced 也都把它作为 widget 内部 state)。

```rust
// sv-ui:ViewNode 增一个字段(Option<Box<>> 控制大小,memory_probe 预算 ViewNode≤320B)
pub input: Option<Box<InputState>>,

pub struct InputState {
    pub cursor: usize,            // 字节偏移,恒在 char 边界
    pub anchor: usize,            // 选区锚点;== cursor 即无选区
    pub preedit: Option<(String, Option<(usize, usize)>)>, // winit Preedit 原样
    pub placeholder: String,
    pub scroll_x: f32,            // 单行横向滚动(光标跟随)
}
```

回调按既有模式加三个:`on_input: Option<Rc<dyn Fn(&str)>>`(值变化)、
`on_submit`(Enter)、`on_keydown: Option<Rc<dyn Fn(&KeyStroke)>>`(sv-ui 定义
平台无关的 `KeyStroke`,**不能**让 winit 类型上浮进 sv-ui——ADR-4 窄窗口 trait
的纪律)。焦点放 `DocumentInner` 级:`focused: Option<ViewId>` +
`Doc::set_focus/focused()`,这同时是 M1 焦点链/Tab 导航的落脚点。

### 2.2 编辑操作集(纯模型,零字体依赖)

```rust
pub enum EditOp {
    InsertStr(String),            // 有选区先删选区;含 IME Commit
    DeleteBackward, DeleteForward, // 无选区按 char 边界退/删
    Move(Caret, bool),            // Caret: Left|Right|Home|End;bool = 是否扩选
    SelectAll,
}
pub fn apply_edit(doc: &Doc, id: ViewId, op: EditOp);  // 改 text/cursor/anchor 并 bump
```

字节偏移移动用 `str::char_indices` 保证 UTF-8 边界(中文删除/移动一次一整字)。
词级移动/双击选词需要 unicode-segmentation,列档 B。`apply_edit` 改值后调
`on_input`;`Doc::set_input_value`(bind:value 写端)相等剪枝(同 `set_text`)
并将 cursor 钳制到新值末尾——防 effect↔回调回声。

### 2.3 Painter:v0 不加绘制动词,只加裁剪

光标 = 1.5px 竖矩形;选区 = 文本后景矩形;预编辑下划线 = 2px 横矩形;组合区间
高亮 = 反色矩形(iced 同款)——全部是现有 `fill_rounded_rect`(paint.rs:85)。
新增一对动词(**同时是滚动体系的前置**,调研 19 P0 第 3 项):

```rust
fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32);   // Painter trait
fn pop_clip(&mut self);
```

TinySkiaPainter 用 `tiny_skia::Mask` 实现(fill_path 第 5 参今天传 None,
paint.rs:319);Vello 端 1:1 映射 `push_layer/pop_layer`;RecordingPainter 加
`PaintCmd::PushClip/PopClip` 进金样。长文本溢出框体靠 clip + `scroll_x` 平移。

### 2.4 sv-shell:布局、命中与事件循环分支

- **measure**(render.rs:107 的 match):`TextInput` 分支——宽 = `style.width`
  或默认 200 逻辑 px,高 = line_h + padding(不随内容变宽,与业界一致)。
- **光标几何双函数**(render.rs,与 `shape_text` 同一 advance 逻辑,保证画/点一致):
  `caret_x(font, text, px, byte_idx) -> f32` 与
  `caret_index_at(font, text, px, x) -> usize`(命中取最近 char 边界)。
- **窗口事件**(lib.rs:311 `window_event` 现仅鼠标左键+悬停,加三类分支):
  1. `MouseInput(Pressed)`:命中 `TextInput` → `doc.set_focus(id)` +
     `window.set_ime_allowed(true)` + `caret_index_at` 定光标;命中其它/空白 →
     失焦 + `set_ime_allowed(false)`;按住拖动(`CursorMoved` + pressed)扩选。
  2. `KeyboardInput`:有焦点输入框且非组合期 → 具名键映射 EditOp
     (Backspace/Delete/方向/Home/End/Enter→on_submit);`KeyEvent.text` →
     `InsertStr`;Ctrl+A/C/X/V → SelectAll/剪贴板。无焦点输入框 → 走未来焦点链。
  3. `WindowEvent::Ime(..)`:见 §2.5。
- **光标区域上报**:`paint()` 布局后,若有焦点输入框,把光标矩形(逻辑→物理 px)
  经 `window.set_ime_cursor_area(position, size)` 上报——**这就是 CJK 候选窗
  跟随光标的全部机制**;光标一动 bump 版本 → 重绘 → 重报,与
  `layout_tree_cached`(render.rs:235 版本键控)天然一致。

### 2.5 IME 全流程与组合文本渲染

```
点击输入框 → set_ime_allowed(true)
  Ime::Enabled     → 无操作(状态标记)
  Ime::Preedit(s, Some((b,e))) → doc.set_preedit(id, s, (b,e)) → bump → 重绘
  Ime::Preedit("", None)       → 清预编辑(winit 在 Commit 前必发)
  Ime::Commit(s)   → apply_edit(InsertStr(s))(预编辑已被上一事件清掉)
  Ime::Disabled    → 清预编辑
失焦/点击别处 → set_ime_allowed(false)
```

渲染:显示串 = `value[..cursor] + preedit + value[cursor..]`(**仅绘制层拼接**,
`ViewNode.text` 不含预编辑,bind:value 因此不会把半成品组合文本泄给应用——
Parley `text()`/`raw_text()` 同款区分)。预编辑整段 2px 下划线;winit 给的
`(b,e)` 区间反色高亮 + 光标画在 `e` 处。over-the-spot,不做候选窗内嵌
(那是输入法自己的窗口,我们只负责 cursor_area)。

### 2.6 剪贴板:arboard,壳层持有

选型 **arboard**(1Password 维护,MIT/Apache 双许可,Win/macOS/X11 原生支持,
Wayland 走可选 `wl-clipboard-rs` feature;文本+图片):
<https://docs.rs/arboard/latest/arboard/>。放 **sv-shell**(平台物,不进 sv-ui):
`App` 懒建 `arboard::Clipboard` 实例,Ctrl+C/X/V 在 KeyboardInput 分支直接调;
另暴露 `sv_shell::clipboard_text()/set_clipboard_text()` 供业务按钮用。离屏/测试
路径注入 `Rc<RefCell<String>>` 假剪贴板,使快捷键链路可离屏断言。

### 2.7 sv-compiler:`<input>` + bind:value(SVELTE-SUPPORT C 节 ⏳ 那条)

- **template.rs:290** tag 表加 `"input" => Tag::Input`(报错提示同步加 input)。
- **codegen.rs::emit_element**:`Tag::Input => create_input()`;属性:
  `placeholder="..."`(静态)、`value={expr}`(单向 effect→`set_input_value`)、
  **`bind:value={x}`**(完全复刻 bind:checked 的双向模板,codegen.rs:817:
  effect(sig→`set_input_value`) + `set_on_input(el, move |s| sig.set(s.into()))`;
  873 行 "v0 的元素绑定支持 bind:checked" 报错分支放行 value)。
- **事件属性白名单**(codegen.rs:920 现拒绝一切非 click/pointer 的 on*):放行
  `oninput`/`onsubmit`/`onkeydown`(→ `set_on_input/set_on_submit/set_on_keydown`)。
- **双前端纪律**(CLAUDE.md):sv-macro codegen 同步加 `input(...)` 节点与测试。
- 文档:SVELTE-SUPPORT.md `bind:value` 行 ⏳→✅;CSS-SUPPORT `:focus` 伪类挂
  焦点链后(C2 既定)。virtual_list 槽内 input:焦点绑 ViewId、槽复用换数据时
  焦点留在槽上——文档化为已知行为(开放问题见 §5)。

## 3. 测试策略:离屏测什么,手测什么

**离屏可自动化(占 ~90% 逻辑)**:
- sv-ui 纯模型:EditOp 全操作 × CJK 多字节边界、选区替换、钳制(无字体依赖);
- IME 事件序列:把 Ime 处理抽成 `handle_ime(&Doc, ViewId, Ime)` 纯函数,喂
  `Preedit("nihao")→Preedit("")→Commit("你好")` 断言 value/preedit/cursor;
- RecordingPainter 金样:焦点框命令流 = bg → PushClip → 选区矩形 → Glyphs →
  光标矩形 → 下划线 → PopClip;
- 光标几何:`caret_x`/`caret_index_at` 互逆、单调、`set_ime_cursor_area` 参数
  等于光标物理矩形;`render_to_png` 出图人查(现有 CI 已跑真字体)。
- bind:value 端到端:examples 行为测试(仿 `bind_checked_two_way`)。

**必须手测(OS 会话无法合成)**:真输入法全流程——Windows 微软拼音、macOS 拼音、
Linux fcitx5/ibus(X11+Wayland 各一)——验证:候选窗贴光标、选字上屏、预编辑
下划线、Esc 取消、跨应用复制粘贴、按键重复。做成 `examples/input-demo` +
5 项勾选清单入库(先例警示:iced/egui 的 IME 修复全部来自真机反馈)。

## 4. 分步落地

| # | 内容 | 验收(测试名) | 人周 | 置信 |
|---|---|---|---|---|
| 1 | 键盘通道+焦点:`Doc.focused`、KeyboardInput 分支、KeyStroke 类型、onkeydown | `focus_click_and_blur`、`keystroke_routes_to_focused` | 1–1.5 | 高 |
| 2 | TextInput 元素+编辑模型+渲染:ElementKind/InputState/EditOp、measure 分支、caret 双函数、push_clip、placeholder | `edit_ops_utf8_boundaries`、`caret_geometry_roundtrip`、`input_paint_golden`(RecordingPainter) | 2–3 | 中高 |
| 3 | IME:set_ime_allowed、handle_ime、预编辑渲染、set_ime_cursor_area | `ime_preedit_commit_sequence`、`ime_cursor_area_tracks_caret` + 三平台手测清单 | 1–2 | 中(平台长尾) |
| 4 | 剪贴板:arboard + Ctrl-C/X/V + 假剪贴板注入 | `clipboard_shortcuts_offscreen` + 跨应用手测 | 0.5–1 | 高 |
| 5 | 编译器:`<input>`/bind:value/oninput/onsubmit/onkeydown,双前端同步,文档翻绿 | `input_bind_value_two_way`、`input_events_compile`;todo-sfc 加输入框 | 1 | 高 |
| 6 | 打磨:拖拽选择、光标跟随滚动、双击选词(unicode-segmentation)、键重复 | `drag_selection`、`scroll_follows_caret` | 1–2 | 中 |

合计 **6.5–10.5 人周**(补上调研 19 "文本输入无估算"的空档)。1→2→(3‖4)→5→6,
3 与 4 可并行。

## 5. 风险与开放问题

1. **平台长尾是本议题的本质风险**:iced 合并后 3 个修复 PR、egui Linux 曾整体
   禁用 IME、winit X11 cursor_area 仅位置——预算里 §4-3 的"中"置信就是为此;
   Wayland 端还依赖合成器 text-input 协议实现质量(COSMIC/GNOME/KDE 各异)。
2. **winit 抑制 KeyboardInput 的范围各平台不完全一致**(macOS 死键也走 Preedit);
   handle_ime 必须容忍乱序/重复事件(空 Preedit 幂等)。
3. **ViewNode 大小预算**:`Option<Box<InputState>>` + 3 个回调 ≈ +32B,逼近
   320B 上限(lib.rs:1245 memory_probe)——若超,把三个回调并入 InputState。
4. **virtual_list × 焦点**:槽复用后"焦点跟槽不跟数据",滚动时体验存疑;正解
   可能是 each_block_keyed 行内 input,或焦点带 key——留到滚动体系落地后裁决。
5. **set_input_value 与组合中状态的竞争**:响应式写入恰逢用户组合中——v0 裁决
   为"外部写入清预编辑",记录在案等真实反馈。
6. **M2 Parley 迁移税**:v0 编辑模型的动词已对齐 PlainEditor(compose/
   ime_cursor_area/cursor_geometry),但 scroll_x/字节偏移语义能否零改动映射,
   迁移时需一个对拍 spike;密码框、undo/redo、多行 TextArea 均未在本方案内。
7. arboard 在 X11 下内容随进程退出消失(无剪贴板管理器时)——业界通病,
   文档化即可,但需在手测清单里验证口径。

## 6. 结论:最小可商用切片

**档 A(内部工具)= 步骤 1–5**:单行 `<input>` + bind:value/oninput/onsubmit +
光标/选区/全选 + Ctrl-C/X/V + 三平台 IME 手测清单通过(预编辑内联下划线即可,
无词级操作、无拖拽也及格)。这补齐调研 19 九项交集的第 1/2/3 项,约 **5.5–8.5
人周**,是档 A 的最长杆。
**档 B(商用)= 档 A + 步骤 6 + M2 项**:Parley/PlainEditor 迁移(字体 fallback/
emoji/BiDi 光标)、拖拽+双击选词+undo/redo、多行 TextArea、AccessKit 文本语义、
:focus 视觉、密码框、四输入法(含鸿蒙,ADR-5 已判"真机尽早验证")手测矩阵入 CI
清单。业界坐标:做到档 A ≈ iced 2025-02(#2777 合并日)的水位;Slint 从
"initial pre-edit"到成熟花了两年多个版本——档 B 按 1–2 季持续打磨预估,置信低。

## 出处(业界核实)

- winit Ime 协议:<https://docs.rs/winit/latest/winit/event/enum.Ime.html>
- iced:<https://github.com/iced-rs/iced/pull/2777>(主 PR)· #2785/#2790/#2793(追修)· #686/#1474(早期未果)· <https://github.com/iced-rs/iced/releases/tag/0.14.0>
- COSMIC:<https://system76.com/blog/post/cosmic-epoch-2-and-3-roadmap>("Adds IME and fcitx support")· <https://github.com/pop-os/libcosmic>
- egui:<https://github.com/emilk/egui/issues/248> · PR #4896/#5198 · <https://deepwiki.com/emilk/egui/4.5-text-input-and-editing>(二手汇总,API 名未逐行核对源码)
- Slint:<https://github.com/slint-ui/slint/blob/master/CHANGELOG.md>(1.3.0)
- Parley/Masonry:<https://docs.rs/parley/latest/parley/editing/struct.PlainEditor.html> · <https://docs.rs/masonry/latest/masonry/>
- cosmic-text:<https://docs.rs/cosmic-text/latest/cosmic_text/struct.Editor.html>
- arboard:<https://docs.rs/arboard/latest/arboard/>
