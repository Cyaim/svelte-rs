# 20 · 键盘事件通道 + 焦点链 + 快捷键:一切输入交互的地基

> 日期 2026-07-18。方法:先通读 sv-shell/sv-ui/sv-compiler 相关源码定位缺口逐行取证,
> 再对 egui/iced/floem/Masonry/Slint 的焦点与快捷键实现逐一联网核实(docs.rs /
> docs.slint.dev,URL 见文末),据此给出对准本仓库真实类型与签名的分步落地方案。
> 本文是调研 19"最短商用路径"第 1 步的施工图。

---

## 0. TL;DR(结论先行)

1. **焦点模型裁决:单一焦点点(`Doc` 持 `focused: Option<ViewId>`)+ 节点级
   `focusable` 位 + 树序 Tab 遍历,不做数值 tabindex。** 这是 egui(注册序)、
   floem(`keyboard_navigable` ≈ `tabindex="0"`)、Masonry(`accepts_focus`)、
   Slint(树序)四家交集;数值 tabindex 无一家 Rust 框架实现。
2. **键盘路由裁决:焦点节点起步、沿父链冒泡、`stop_propagation()` 截断**(Masonry
   "target the focused widget, then bubble to each parent" 同款),再按序落入三级
   默认层:Tab 导航 → Enter/Space 激活 → 全局快捷键。调研 19 点名的"无冒泡"
   欠账从键盘通道开始还。
3. **快捷键裁决:注册表 + 消费语义**(egui `consume_shortcut` 教训:匹配即消费、
   只触发一次、精确修饰键匹配防 `Ctrl+S` 吃掉 `Ctrl+Shift+S`);注册挂 sv-reactive
   作用域,`on_cleanup` 自动注销——组件卸载快捷键随之消失,免费。
4. **`.sv` 语法裁决:只走 Svelte 5 事件属性形态 `onkeydown={|e| ...}`**,不新增
   `on:keydown`(SVELTE-SUPPORT.md 已裁决 `on:` 是待移除的遗留形态,新事件不进);
   `:focus` 伪类复用 `:hover` 的"每元素状态 signal + 回调接线"既有模式。
5. **Painter 不需要新动词**(默认焦点环用现有 `stroke`/`PaintCmd::StrokeRect`
   外扩 2px 画出)。**体量:全切片约 4.5–6 人周**(置信度中高;不含 IME/文本
   输入——那是最短路径第 2 步,但本切片把它的挂点留好)。

---

## 1. 现状与缺口(逐条代码证据)

| 缺口 | 证据(均为本仓库现行代码) |
|---|---|
| 事件循环零键盘 | `crates/sv-shell/src/lib.rs` `window_event`(311–364 行)只有 `CloseRequested/RedrawRequested/Resized/ScaleFactorChanged/CursorMoved/MouseInput(Left)` 六个分支;无 `KeyboardInput`、无 `ModifiersChanged`、无 `Ime` |
| 场景树无焦点概念 | `crates/sv-ui/src/lib.rs` `ViewNode`(189–203 行)回调只有 `on_click` + 四个 `on_pointer_*`;无 `focusable` 位、无 `on_key`;`DocumentInner` 无 `focused` |
| 无冒泡 | `hit_click_target`(`render.rs` 433–439 行)反序扫 `Placed` 取最上层单点直达,调研 19 已点名 |
| `:focus` 被显式拒绝 | `sv-compiler/src/style.rs` 219–226 行:伪类只认 hover/active,报错文案自认"`:focus/:disabled` 随焦点链 C2";CSS-SUPPORT.md 第 60 行同 |
| 编译器拒收键盘事件 | `sv-compiler/src/codegen.rs` 922–930 行:`on*` 只认 onclick/onpointerenter/onpointerleave,其余报"键盘事件待焦点链" |
| 宏前端同窄 | `sv-macro/src/codegen.rs:160` 仅 `AttrKind::OnClick` 一个事件分支 |
| 事件命名已有裁决 | SVELTE-SUPPORT.md 117–119 行:Svelte 5 事件属性(`onclick`)是正式形态,`on:click` 落地后移除——**新键盘事件只能以属性形态进** |

可依赖的既有资产:`Placed` 绘制序 = 树 DFS 序(Tab 序免费);`:hover/:active` 的
"状态 signal + 接线"codegen 模式(659–763 行)可整体复制给 `:focus`;
`click_handler` 的"clone 出回调再调、不持树借用"惯例(429 行)与 `set_checked`
的相等剪枝模式(377–389 行)分别是回调取用与 `focus()` 的写法模板。

---

## 2. 业界先例(联网核实,2026-07)

**winit 0.30.13(事件源,已核实字段)**:`WindowEvent::KeyboardInput { event: KeyEvent, is_synthetic, .. }`,
`KeyEvent { physical_key: PhysicalKey, logical_key: Key, text: Option<SmolStr>, location, state: ElementState, repeat: bool }`;
修饰键单独走 `WindowEvent::ModifiersChanged(Modifiers)`(应用自存状态);扩展 trait
`KeyEventExtModifierSupplement` 供 `key_without_modifiers()`(快捷键显示名)。
注意 `is_synthetic`:X11 窗口获焦时合成按下事件,不过滤会误触发。

**egui 0.35(immediate 阵营)**:焦点是 `Memory` 的单一 `focused() -> Option<Id>`;
控件 `interested_in_focus()` 注册、`request_focus()/surrender_focus()` 移交;
Tab/Shift+Tab 按注册序(= 帧内构建序)顺移,`EventFilter` 决定 Tab/方向键/Esc
是否被控件自身吃掉(文本框吞 Tab 的场景)。快捷键:
`InputState::consume_shortcut(KeyboardShortcut{modifiers, key})` **匹配即从事件队列
删除、只触发一次**;文档明示"先匹配更具体组合"(`Cmd-Shift-S` 先于 `Cmd-S`),
因 `matches_logically` 允许多余修饰键——我们用精确匹配避开此坑。

**iced 0.14(retained 阵营,COSMIC 底座)**:`keyboard::Event::KeyPressed { key, modified_key, physical_key, location, modifiers, text, repeat } / KeyReleased / ModifiersChanged`
——modifiers 随事件带上,免处理端自存。焦点是**树上操作**:
`advanced::widget::operation::focusable` 提供 `Focusable` trait 与
`focus(id)/unfocus/find_focused/focus_next/focus_previous/count`,按树序遍历——
"焦点遍历是一次树上 Operation"的形态与我们的 `Doc` 方法最同构。

**floem 0.2(细粒度响应式,与我们架构最近)**:`Decorators::keyboard_navigable()`
文档原话"Similar to setting `tabindex="0"` in html"——**opt-in 布尔位,无数值序**;
`on_key_down/on_key_up` 要求先 `keyboard_navigable()`;`request_focus()/clear_focus()`;
传播用 `on_event_stop/on_event_cont` 显式控制。

**Masonry 0.4(Linebender/Xilem 底座)**:`Widget::accepts_focus()`("If true,
pressing Tab can focus this widget";值在控件创建时缓存不可变)、
`accepts_text_input()`("focusing this widget will start an IME session"——**焦点位
就是 IME 挂点**,直接印证第 2 步的衔接设计)、`on_text_event()` 文档原话
"Text events will target the focused widget, then bubble to each parent"——冒泡先例。

**Slint(声明式 DSL)**:专门的 `FocusScope` 元素:`key-pressed/key-released` 回调
返回 `EventResult`(accept/reject = 消费语义)、`capture-key-pressed`(**捕获阶段**,
先于焦点元素)、`focus-on-tab-navigation`/`focus-on-click` 开关、`forward-focus`
转发焦点给子域;Tab 序跟随元素树。捕获阶段 v0 不做,路由分段设计让它日后可插。

**共识提炼**:① 单一焦点点 + opt-in 焦点位 + 树/构建序遍历,五家全同;② 键盘
事件从焦点节点出发,靠"返回消费标记"或"冒泡+截断"传播;③ 快捷键是焦点链
未消费后的独立层,消费语义防双触发;④ `accepts_text_input` 类的位把焦点系统和
IME 缝在一起。**未能核实**:COSMIC(libcosmic)应用级快捷键注册表的具体实现与
Blitz 的焦点实现(检索额度耗尽,不引为据)。

---

## 3. 方案设计(对准本仓库代码)

### 3.1 sv-ui:焦点内核与键盘事件类型

新增自有事件类型(**不依赖 winit**——ADR-4 窄窗口 trait 要求事件类型归自己,鸿蒙 XComponent 端要能喂同一类型):

```rust
// sv-ui(新增,~80 行)
pub enum Key { Char(char), Enter, Tab, Escape, Backspace, Delete, Space,
    ArrowUp/Down/Left/Right, Home, End, PageUp/Down, F(u8) } // v0 ~20 值,Char 兜底
bitflags Mods { CTRL, SHIFT, ALT, META }
pub struct KeyEvent { pub key: Key, pub text: Option<String>, pub mods: Mods,
    pub repeat: bool, stop: Cell<bool>, default_prevented: Cell<bool> }
    // 方法:e.stop_propagation() / e.prevent_default(),DOM 心智
```

`ViewNode` 加字段(现 ≤320B 预算,新增约 40B,`memory_probe` 护栏同步调):
`focusable: bool`、`on_key: Option<Rc<dyn Fn(&KeyEvent)>>`、
`on_focus_change: Option<Rc<dyn Fn(bool)>>`,并预留 `accepts_text: bool`(本切片
恒 false,第 2 步文本输入用它触发 `set_ime_allowed`,Masonry 同款)。
`create_button/create_checkbox` 默认 `focusable = true`,View/Text 默认 false;
`set_focusable(id, bool)` 供编译器与 TextInput 用。

`DocumentInner` 加 `focused: Option<ViewId>`,`Doc` 加方法(走 `set_checked`
相等剪枝 + bump 模板;回调 clone 出借用外再调):

- `focus(id)`/`blur()`/`focused()`:变更时先旧节点 `on_focus_change(false)`、
  再新节点 `(true)`、bump 一次(焦点环重绘由版本号驱动);
- `focus_next()`/`focus_prev()`:从 `focused` 起按**树 DFS 序**(与 `Placed`
  绘制序同构)找下一个 `focusable`,环绕;隐藏分支天然不在树上(`if_block`
  物理移除子树,本架构送的便宜——无需 display:none 过滤);
- `key_handler(id)`/`focus_change_handler(id)`:与 `click_handler` 同款取用器;
- **`remove()` 补一刀**(331–347 行):被删子树含 `focused` 时清焦点并回调——
  否则 `if_block` 重建留下悬空焦点,这是最容易翻车的边界。

**焦点刻意不做成 signal**:它是树状态(与 `checked` 同类),经版本号驱动重绘;
全局 `Signal<Option<ViewId>>` 会让所有带 `:focus` 规则的元素订阅同一信号、任一
焦点变化全体重算,违背细粒度原则。`:focus` 视觉走 per-element
`__fc: Signal<bool>` + `on_focus_change` 接线(3.4),与 `__hv` 完全同构。

### 3.2 sv-shell:事件循环接入与路由

`App` 加字段 `mods: ModifiersState`;`window_event` 加两个分支:

```rust
WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
WindowEvent::KeyboardInput { event, is_synthetic: false, .. }
    if event.state == ElementState::Pressed => self.dispatch_key(map_key(&event, self.mods)),
```

`map_key`:`logical_key` 的 `Key::Named(NamedKey::Tab) → sv::Key::Tab` 等 ~20 条 +
`Key::Character → Key::Char` + `text`/`repeat` 透传;v0 只派发 keydown(keyup
留给拖拽/游戏后议);`is_synthetic` 直接丢弃(X11 坑)。

`dispatch_key` 四段路由(每段仅在前段未消费时进入):① **冒泡段**——从
`doc.focused()` 沿 `parent` 链逐节点取 `key_handler` 调用,`stop_propagation()`
置位即停,调研 19 点名的"无冒泡"从这里破局;② **导航段**
(`!default_prevented()`)——Tab→`focus_next()`、Shift+Tab→`focus_prev()`、
Esc→`blur()`;③ **激活段**——Enter/Space 且焦点是 Button/Checkbox → 调
`click_handler`,按钮免费获得键盘可达性;④ **快捷键段**——查注册表(3.3)。
鼠标侧一行接线:`MouseInput(Pressed)` 命中节点若 `focusable` 则 `doc.focus(id)`
(Slint `focus-on-click` 同款);空白区点击不清焦点(桌面惯例)。

**焦点环视觉**:`paint_tree`(共享遍历器,双后端免费)绘制完节点后,若
`id == doc.focused()` 统一画默认环:`stroke` 外扩 2px、宽 2px、accent 定色。
**Painter trait 零改动**(`StrokeRect` 现成,`RecordingPainter` 金样自动覆盖);
自定义 `:focus` 样式落地后默认环与之并存(浏览器 outline 心智),
`outline:none` 式关闭留 C2。

### 3.3 sv-ui::shortcuts:快捷键注册表

```rust
pub struct Shortcut { pub mods: Mods, pub key: Key }   // 精确匹配,无子集歧义
pub fn register_shortcut(sc: Shortcut, f: impl Fn() + 'static)  // 挂当前响应式作用域
```

thread-local `HashMap<Shortcut, Vec<(u64, Rc<dyn Fn()>)>>`(与 sv-reactive runtime
同线程,零同步);**注册即在当前作用域 `on_cleanup` 埋注销**——组件/`if_block`
分支卸载,快捷键自动消失,这是响应式所有权树白送的生命周期管理。同键多注册
后进先出、调用即止(对话框压栈覆盖底层快捷键的正确语义);精确修饰键匹配
O(1) 查表,规避 egui "具体优先"的排序负担。`Shortcut::cmd_or_ctrl(key)` 构造器
注册时按平台展开(macOS→META,其余→CTRL),不引入运行时判断。

### 3.4 编译器双前端(sv-compiler + sv-macro)

**事件属性**(codegen.rs 事件循环 767 行起加 arm;559–566 行的属性过滤 `on`
前缀已放行,零改动):

- `onkeydown={|e| ...}` → `__doc.set_on_key(#el, #handler);` 并自动
  `__doc.set_focusable(#el, true);`(floem "on_key_down 要求 keyboard_navigable"
  的教训:不自动设位是新手第一坑);handler 经既有 `self.expr(e, scope, true)`
  走闭包 move 改写,签名 `Fn(&sv_ui::KeyEvent)`,消费用 `e.stop_propagation()`;
- `onfocus`/`onblur` → 合成进 `set_on_focus_change`(true/false 分叉);
  `autofocus` 布尔属性 → 建树末尾 `__doc.focus(#el);`;
- `on:keydown` **不新增**(SVELTE-SUPPORT 118 行:`on:` 不进正式格式),
  934 行 `on:` 报错指路 `onkeydown`;930 行"键盘事件待焦点链"拒绝分支删除。

**`:focus` 伪类**(style.rs + codegen.rs):219–226 行伪类 match 加 `"focus"`,
规则组加 `focus` 槽(hover/active 旁);codegen 659–763 行状态合成路径加 `__fc`
signal 与 `set_on_focus_change` 接线,优先级序:类 < style="" < 条件类 <
`:hover` < `:focus` < `:active` < `style:` 指令(LVHA 惯例);用户 `onfocus/onblur`
与 `:focus` 接线共存时走 683–727 行 `__ue/__ul` 同款合成,避免互相覆盖。
CSS-SUPPORT.md 第 60 行、SVELTE-SUPPORT.md 第 119/286 行状态翻绿。

**sv-macro 同步**(CLAUDE.md 约束:绑定原语变动必须同步宏前端):parse.rs
`AttrKind` 加 `OnKeyDown/OnFocus/OnBlur`,codegen.rs:160 旁各加一行转发;双前端
产物一致性由既有"同一编译目标"测试模式覆盖。

### 3.5 与 sv-reactive 的时序

键盘路径与点击路径**完全同构**,零新时序概念:handler 写 signal → effect 同步
flush(ADR-1 写后同步;ADR-6 未落地)→ 树精准变更 → bump → `on_mutate` →
`request_redraw`。两条纪律:① 回调必须 clone 出借用外再调(既有惯例);
② handler 内再调 `focus()` 仍安全——焦点不是 signal、不参与依赖图,不可能造出
响应式环。ADR-6 落地后 `dispatch_key` 整体挪进"事件段"(batch 写入),四段
路由顺序与语义不变——本设计对帧调度前后中立。

---

## 4. 分步落地(验收标准 + 人周,置信度标注)

| 步 | 内容 | 验收(测试名) | 估算 |
|---|---|---|---|
| 1 | sv-ui 焦点内核:Key/KeyEvent/Mods 类型、focusable 位(含 accepts_text 预留)、focused + focus/blur/next/prev、remove 清焦点、on_key/on_focus_change 存取器 | `focus_next_follows_tree_order`、`focus_wraps_and_skips_unfocusable`、`remove_focused_subtree_clears_focus`、`key_event_bubbles_until_consumed`、`focus_change_fires_blur_then_focus` | 1 人周(高) |
| 2 | sv-shell 接入:ModifiersChanged/KeyboardInput 分支、map_key、四段 dispatch_key、click 设焦、默认焦点环 | `offscreen_tab_enter_activates_button`(离屏合成 KeyEvent→dispatch→dump 断言,复用 `offscreen_click_roundtrip` 模式)、`shift_tab_reverses`、`synthetic_key_ignored`、`recording_painter_focus_ring_golden` | 1–1.5 人周(高) |
| 3 | 快捷键注册表 + dispatch 第 4 段 + `on_cleanup` 注销 | `shortcut_fires_when_unconsumed`、`focused_handler_consumes_shortcut`、`scope_dispose_unregisters_shortcut`、`same_key_last_registered_wins` | 0.5–1 人周(高) |
| 4 | 双前端语法:onkeydown/onfocus/onblur/autofocus、`:focus` 伪类接线、on:keydown 指路报错、sv-macro 同步 | `onkeydown_compiles`(对拍 set_on_key + set_focusable)、`focus_pseudo_wires_signal`、`autofocus_focuses_on_mount`、counter-sfc 端到端键盘加法测试 | 1–1.5 人周(中高,状态合成路径较绕) |
| 5 | 打磨:Esc/激活语义补全、SVELTE-SUPPORT/CSS-SUPPORT/DESIGN 文档翻绿、showcase 加键盘演示 | showcase 手测 + 文档 diff | 0.5 人周(高) |

合计 **4.5–6 人周**。步 1–3 无外部依赖可先行;步 4 依赖步 1 的类型定型。

---

## 5. 风险与开放问题(诚实清单)

1. **两套传播语义并存**:本切片给键盘冒泡,指针仍是单点直达(`hit_click_target`)
   ——M1 统一指针冒泡是一次 breaking(`set_on_click` 语义微变),须在 API 冻结前还清。
2. **virtual_list 焦点语义开放**:槽位复用意味着焦点跟槽不跟数据,滚动后"焦点行"
   内容已换——需要"focus follows data"上层约定(槽信号里带选中态),本切片不解。
3. **焦点环与裁剪**:环画在 border-box 外 2px,Painter 尚无 clip;滚动容器(最短
   路径第 3 步)落地后需重审出界绘制。
4. **Key 枚举裁剪面**:winit `NamedKey` 百余值,v0 收 ~20 + `Char` 兜底;漏配键
   静默失效,map_key 留 `Unidentified` 日志位。**repeat 与快捷键**:v0 快捷键
   忽略 repeat、冒泡段透传,业界无共识,留 flag 后议。
5. **多窗口(M2)**:焦点在 Doc 上天然 per-window,但 shortcuts 注册表 thread-local
   全局——多窗口需 per-Doc 化(注册表键加 doc identity,留了缝未实现)。
6. **AccessKit 对齐(M2)**:ViewId 世代键做 NodeId 稳定映射没问题;Tab 序与
   无障碍树序的一致性未验证。**`:focus-visible`**(键盘获焦才显环)是商用级细节,
   本切片统一显环,档 B 前补。
7. **IME 衔接假设未真机验证**:`accepts_text` 位 → `set_ime_allowed` 的挂法来自
   Masonry 文档语义,winit 三平台 + 鸿蒙的实际组合文本行为要在第 2 步探底。

---

## 6. 结论:最小可商用切片

**档 A(内部工具可用)= 步 1–3 + 步 4 最小面**(onkeydown 属性 + 默认焦点环,
`:focus` 伪类可缓):Tab/Shift+Tab 遍历、Enter/Space 激活、Esc 失焦、Ctrl 级
快捷键、键盘事件可达组件并冒泡,约 **3–4 人周**;"设置面板 + 表单"类工具的
键盘闭环成立,并为文本输入(第 2 步)备好 focused + accepts_text 挂点。

**档 B(商用)在此之上必须补齐**:`:focus`/`:focus-visible` 全接线与可关闭焦点环、
指针/键盘统一冒泡(breaking,须在 API 冻结前)、`cmd_or_ctrl` 跨平台修饰键、
快捷键与菜单(弹层体系)联动、AccessKit 焦点同步、keyup 通道、捕获段。这些不
单列人周——分别挂在弹层/无障碍/API 冻结三个既有里程碑上,本切片的四段路由与
per-element 信号接线是它们的公共地基。

---

## 出处(联网核实,2026-07-18)

- winit 0.30.13:https://docs.rs/winit/latest/winit/event/struct.KeyEvent.html ·
  https://docs.rs/winit/latest/winit/event/enum.WindowEvent.html
- egui 0.35:https://docs.rs/egui/latest/egui/struct.Memory.html ·
  https://docs.rs/egui/latest/egui/struct.InputState.html
- iced 0.14:https://docs.rs/iced/latest/iced/keyboard/enum.Event.html ·
  https://docs.rs/iced/latest/iced/advanced/widget/operation/focusable/index.html
- floem 0.2 `keyboard_navigable`/`on_key_down`/`request_focus`:
  https://docs.rs/floem/latest/floem/views/trait.Decorators.html
- Masonry 0.4 `accepts_focus`/`accepts_text_input`/文本事件冒泡:
  https://docs.rs/masonry/latest/masonry/core/trait.Widget.html
- Slint `FocusScope`(key-pressed/EventResult/capture/forward-focus/Tab 序):
  https://docs.slint.dev/latest/docs/slint/reference/keyboard-input/focusscope/
- 未能核实项见 §2 末:COSMIC 快捷键注册表实现、Blitz 焦点实现。
