//! 单行文本输入的编辑内核(调研 21,R1 第二步)。
//!
//! 纯模型、零字体依赖:光标/选区/预编辑全部按**字节偏移**存储(恒在 UTF-8
//! char 边界),几何换算(光标 x 坐标、点击定位)在渲染壳。EditOp 动词对齐
//! Parley `PlainEditor` / cosmic-text `Edit`,M2 迁移时接口 1:1 映射。
//!
//! 编辑态放节点内(`Option<Box<InputState>>`)不进 Signal:移光标是控件私有
//! 交互态,进 Signal 会把每次移动都变成可见响应式事件污染依赖图
//! (egui/iced 同款归属;与 `checked` 的"节点为渲染真源"先例一致)。

use std::cell::RefCell;
use std::rc::Rc;

use crate::{Doc, ElementKind, Key, KeyEvent, ViewId};

/// 值回调([`InputState::on_input`] / [`InputState::on_submit`])
pub type ValueHandler = Rc<dyn Fn(&str)>;

/// 输入框私有编辑态(回调也收在这里,控制 `ViewNode` 大小预算)
pub struct InputState {
    /// 光标字节偏移(恒在 char 边界)
    pub cursor: usize,
    /// 选区锚点;`== cursor` 即无选区
    pub anchor: usize,
    /// IME 组合中文本 + 组合区间(winit Preedit 原样;None = 无组合)
    pub preedit: Option<(String, Option<(usize, usize)>)>,
    pub placeholder: String,
    /// 单行横向滚动(光标跟随;渲染壳每帧计算,此处为持久化载体)
    pub scroll_x: f32,
    /// 值变化回调(每次编辑后带新值)
    pub on_input: Option<ValueHandler>,
    /// Enter 提交回调(带当前值)
    pub on_submit: Option<ValueHandler>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            cursor: 0,
            anchor: 0,
            preedit: None,
            placeholder: String::new(),
            scroll_x: 0.0,
            on_input: None,
            on_submit: None,
        }
    }
}

/// 光标移动目标(词级移动列档 B,unicode-segmentation)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Caret {
    Left,
    Right,
    Home,
    End,
}

/// 编辑操作集(纯模型词汇,对齐 cosmic-text `Edit` 动作分类裁剪)
#[derive(Clone, PartialEq, Debug)]
pub enum EditOp {
    /// 插入(有选区先删选区;IME Commit 也走这里)
    InsertStr(String),
    DeleteBackward,
    DeleteForward,
    /// bool = 是否扩选(Shift)
    Move(Caret, bool),
    SelectAll,
}

/// IME 事件(sv-ui 自有类型,ADR-4:winit 类型不上浮;鸿蒙端喂同一类型)
#[derive(Clone, PartialEq, Debug)]
pub enum ImeEvent {
    Enabled,
    /// 组合中文本 + 字节区间光标(None 隐藏;空串 = 清除)
    Preedit(String, Option<(usize, usize)>),
    /// 上屏(winit 保证其前发一个空 Preedit)
    Commit(String),
    Disabled,
}

fn prev_boundary(s: &str, i: usize) -> usize {
    s[..i].char_indices().next_back().map_or(0, |(j, _)| j)
}

fn next_boundary(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(i, |c| i + c.len_utf8())
}

/// 应用一个编辑操作:改 text/cursor/anchor 并 bump;值变化时调 `on_input`
pub fn apply_edit(doc: &Doc, id: ViewId, op: EditOp) {
    let (changed, on_input, new_value) = doc.with_inner_mut(|inner| {
        let Some(n) = inner.nodes.get_mut(id) else {
            return (false, None, String::new());
        };
        let Some(input) = n.input.as_deref_mut() else {
            return (false, None, String::new());
        };
        let before_text = n.text.clone();
        let (before_c, before_a) = (input.cursor, input.anchor);
        let (sel_lo, sel_hi) = (
            input.cursor.min(input.anchor),
            input.cursor.max(input.anchor),
        );
        match op {
            EditOp::InsertStr(s) => {
                n.text.replace_range(sel_lo..sel_hi, &s);
                input.cursor = sel_lo + s.len();
                input.anchor = input.cursor;
            }
            EditOp::DeleteBackward => {
                if sel_lo != sel_hi {
                    n.text.replace_range(sel_lo..sel_hi, "");
                    input.cursor = sel_lo;
                } else if sel_lo > 0 {
                    let from = prev_boundary(&n.text, sel_lo);
                    n.text.replace_range(from..sel_lo, "");
                    input.cursor = from;
                }
                input.anchor = input.cursor;
            }
            EditOp::DeleteForward => {
                if sel_lo != sel_hi {
                    n.text.replace_range(sel_lo..sel_hi, "");
                    input.cursor = sel_lo;
                } else if sel_hi < n.text.len() {
                    let to = next_boundary(&n.text, sel_hi);
                    n.text.replace_range(sel_hi..to, "");
                    input.cursor = sel_hi;
                }
                input.anchor = input.cursor;
            }
            EditOp::Move(caret, extend) => {
                input.cursor = match caret {
                    // 有选区且不扩选:左右先折叠到选区端点(桌面惯例)
                    Caret::Left if !extend && sel_lo != sel_hi => sel_lo,
                    Caret::Right if !extend && sel_lo != sel_hi => sel_hi,
                    Caret::Left => prev_boundary(&n.text, input.cursor),
                    Caret::Right => next_boundary(&n.text, input.cursor),
                    Caret::Home => 0,
                    Caret::End => n.text.len(),
                };
                if !extend {
                    input.anchor = input.cursor;
                }
            }
            EditOp::SelectAll => {
                input.anchor = 0;
                input.cursor = n.text.len();
            }
        }
        let text_changed = n.text != before_text;
        let changed = text_changed || input.cursor != before_c || input.anchor != before_a;
        let cb = if text_changed {
            input.on_input.clone()
        } else {
            None
        };
        (changed, cb, n.text.clone())
    });
    if let Some(cb) = on_input {
        cb(&new_value);
    }
    if changed {
        doc.bump();
    }
}

/// IME 事件处理(纯函数,离屏可测;容忍乱序/重复:空 Preedit 幂等)
pub fn handle_ime(doc: &Doc, id: ViewId, ev: ImeEvent) {
    match ev {
        ImeEvent::Enabled => {}
        ImeEvent::Preedit(s, range) => {
            let new = if s.is_empty() { None } else { Some((s, range)) };
            let changed = doc.with_inner_mut(|inner| {
                let Some(input) = inner.nodes.get_mut(id).and_then(|n| n.input.as_deref_mut())
                else {
                    return false;
                };
                if input.preedit == new {
                    return false;
                }
                input.preedit = new;
                true
            });
            if changed {
                doc.bump();
            }
        }
        ImeEvent::Commit(s) => {
            // winit 保证 Commit 前发空 Preedit;这里再防御性清一次
            let cleared = doc.with_inner_mut(|inner| {
                let Some(input) = inner.nodes.get_mut(id).and_then(|n| n.input.as_deref_mut())
                else {
                    return false;
                };
                input.preedit.take().is_some()
            });
            let _ = cleared;
            apply_edit(doc, id, EditOp::InsertStr(s));
        }
        ImeEvent::Disabled => {
            let changed = doc.with_inner_mut(|inner| {
                let Some(input) = inner.nodes.get_mut(id).and_then(|n| n.input.as_deref_mut())
                else {
                    return false;
                };
                input.preedit.take().is_some()
            });
            if changed {
                doc.bump();
            }
        }
    }
}

/// 绘制层显示串:`value[..cursor] + 预编辑 + value[cursor..]`。
/// 返回 (显示串, 光标字节偏移, 预编辑区间)。`ViewNode.text` 不含半成品组合
/// 文本(Parley `text()`/`raw_text()` 同款区分),渲染壳与 IME 光标区域上报
/// 共用这一个函数,保证"画的"与"报的"一致
pub fn display_text(value: &str, input: &InputState) -> (String, usize, Option<(usize, usize)>) {
    match &input.preedit {
        Some((pe, pe_cursor)) => {
            let mut d = String::with_capacity(value.len() + pe.len());
            d.push_str(&value[..input.cursor]);
            d.push_str(pe);
            d.push_str(&value[input.cursor..]);
            let local = pe_cursor.map(|(_, e)| e).unwrap_or(pe.len());
            (
                d,
                input.cursor + local.min(pe.len()),
                Some((input.cursor, input.cursor + pe.len())),
            )
        }
        None => (value.to_string(), input.cursor, None),
    }
}

// ---------------------------------------------------------------------------
// 剪贴板 provider(平台实现在渲染壳注册:桌面 arboard;测试注入假实现)
// ---------------------------------------------------------------------------

pub trait Clipboard {
    fn get_text(&mut self) -> Option<String>;
    fn set_text(&mut self, text: &str);
}

thread_local! {
    static CLIPBOARD: RefCell<Option<Box<dyn Clipboard>>> = const { RefCell::new(None) };
}

/// 注册剪贴板实现(渲染壳启动时调;测试注入假剪贴板)
pub fn set_clipboard(c: impl Clipboard + 'static) {
    CLIPBOARD.with(|cb| *cb.borrow_mut() = Some(Box::new(c)));
}

pub fn clipboard_get() -> Option<String> {
    CLIPBOARD.with(|cb| cb.borrow_mut().as_mut().and_then(|c| c.get_text()))
}

pub fn clipboard_set(text: &str) {
    CLIPBOARD.with(|cb| {
        if let Some(c) = cb.borrow_mut().as_mut() {
            c.set_text(text);
        }
    });
}

// ---------------------------------------------------------------------------
// 键 → 编辑操作路由(dispatch_key 的编辑段从这里进)
// ---------------------------------------------------------------------------

/// 焦点输入框的键盘处理。返回是否消费(Tab/Esc 恒不消费——放行给导航段)
pub(crate) fn route_editing_key(doc: &Doc, id: ViewId, e: &KeyEvent) -> bool {
    debug_assert!(doc.read(|inner| {
        inner
            .nodes
            .get(id)
            .is_some_and(|n| n.kind == ElementKind::TextInput)
    }));
    let ctrl_or_meta = e.mods.ctrl || e.mods.meta;
    match e.key {
        // 快捷键族:Ctrl/Cmd + A/C/X/V(其余组合放行给全局快捷键段)
        Key::Char(c) if ctrl_or_meta => match c.to_ascii_lowercase() {
            'a' => {
                apply_edit(doc, id, EditOp::SelectAll);
                true
            }
            'c' => {
                if let Some(sel) = doc.selected_text(id)
                    && !sel.is_empty()
                {
                    clipboard_set(&sel);
                }
                true
            }
            'x' => {
                if let Some(sel) = doc.selected_text(id)
                    && !sel.is_empty()
                {
                    clipboard_set(&sel);
                    apply_edit(doc, id, EditOp::InsertStr(String::new()));
                }
                true
            }
            'v' => {
                if let Some(text) = clipboard_get() {
                    // 单行控件:粘贴内容里的换行替换为空格
                    let text = text.replace(['\r', '\n'], " ");
                    apply_edit(doc, id, EditOp::InsertStr(text));
                }
                true
            }
            _ => false,
        },
        Key::Char(_) if !e.mods.alt => {
            // 优先用平台产生的文本(带 Shift 大小写/键盘布局),兜底键面字符
            let s = match (&e.text, e.key) {
                (Some(t), _) => t.clone(),
                (None, Key::Char(c)) => c.to_string(),
                _ => unreachable!(),
            };
            apply_edit(doc, id, EditOp::InsertStr(s));
            true
        }
        Key::Space if !ctrl_or_meta && !e.mods.alt => {
            apply_edit(doc, id, EditOp::InsertStr(" ".into()));
            true
        }
        Key::Backspace => {
            apply_edit(doc, id, EditOp::DeleteBackward);
            true
        }
        Key::Delete => {
            apply_edit(doc, id, EditOp::DeleteForward);
            true
        }
        Key::ArrowLeft => {
            apply_edit(doc, id, EditOp::Move(Caret::Left, e.mods.shift));
            true
        }
        Key::ArrowRight => {
            apply_edit(doc, id, EditOp::Move(Caret::Right, e.mods.shift));
            true
        }
        Key::Home => {
            apply_edit(doc, id, EditOp::Move(Caret::Home, e.mods.shift));
            true
        }
        Key::End => {
            apply_edit(doc, id, EditOp::Move(Caret::End, e.mods.shift));
            true
        }
        Key::Enter => {
            let (cb, value) = doc.read(|inner| {
                let n = inner.nodes.get(id);
                (
                    n.and_then(|n| n.input.as_deref())
                        .and_then(|i| i.on_submit.clone()),
                    n.map(|n| n.text.clone()).unwrap_or_default(),
                )
            });
            if let Some(cb) = cb {
                cb(&value);
            }
            true
        }
        // Tab/Esc/方向上下/翻页等:不消费,放行给导航段与快捷键段
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::{Mods, dispatch_key};

    fn input_doc() -> (Doc, ViewId) {
        let doc = Doc::new();
        let input = doc.create_text_input();
        doc.append(doc.root(), input);
        (doc, input)
    }

    fn caret(doc: &Doc, id: ViewId) -> (usize, usize) {
        doc.read(|inner| {
            let i = inner.nodes[id].input.as_deref().unwrap();
            (i.cursor, i.anchor)
        })
    }

    #[test]
    fn edit_ops_utf8_boundaries() {
        let (doc, id) = input_doc();
        apply_edit(&doc, id, EditOp::InsertStr("你好a".into()));
        assert_eq!(doc.input_value(id).unwrap(), "你好a");
        assert_eq!(caret(&doc, id), (7, 7), "光标应在末尾(2×3B 中文 + 1B)");
        // 退格一次删一整个 char(a → 好 → 你)
        apply_edit(&doc, id, EditOp::DeleteBackward);
        assert_eq!(doc.input_value(id).unwrap(), "你好");
        apply_edit(&doc, id, EditOp::DeleteBackward);
        assert_eq!(doc.input_value(id).unwrap(), "你");
        // 左移到 0,DeleteForward 删"你"
        apply_edit(&doc, id, EditOp::Move(Caret::Left, false));
        assert_eq!(caret(&doc, id), (0, 0));
        apply_edit(&doc, id, EditOp::DeleteForward);
        assert_eq!(doc.input_value(id).unwrap(), "");
        // 空串上的操作幂等不 panic
        apply_edit(&doc, id, EditOp::DeleteBackward);
        apply_edit(&doc, id, EditOp::DeleteForward);
        assert_eq!(caret(&doc, id), (0, 0));
    }

    #[test]
    fn selection_replace_and_collapse() {
        let (doc, id) = input_doc();
        apply_edit(&doc, id, EditOp::InsertStr("甲乙丙丁".into()));
        // Home,Shift+Right×2 选中"甲乙"
        apply_edit(&doc, id, EditOp::Move(Caret::Home, false));
        apply_edit(&doc, id, EditOp::Move(Caret::Right, true));
        apply_edit(&doc, id, EditOp::Move(Caret::Right, true));
        assert_eq!(doc.selected_text(id).unwrap(), "甲乙");
        // 有选区时插入 = 替换
        apply_edit(&doc, id, EditOp::InsertStr("X".into()));
        assert_eq!(doc.input_value(id).unwrap(), "X丙丁");
        // 全选后退格清空
        apply_edit(&doc, id, EditOp::SelectAll);
        assert_eq!(doc.selected_text(id).unwrap(), "X丙丁");
        apply_edit(&doc, id, EditOp::DeleteBackward);
        assert_eq!(doc.input_value(id).unwrap(), "");
        // 无扩选左右移动折叠选区到端点
        apply_edit(&doc, id, EditOp::InsertStr("ab".into()));
        apply_edit(&doc, id, EditOp::SelectAll);
        apply_edit(&doc, id, EditOp::Move(Caret::Left, false));
        assert_eq!(caret(&doc, id), (0, 0), "左移应折叠到选区起点");
    }

    #[test]
    fn ime_preedit_commit_sequence() {
        let (doc, id) = input_doc();
        let inputs: Rc<RefCell<Vec<String>>> = Default::default();
        let log = inputs.clone();
        doc.set_on_input(id, move |v| log.borrow_mut().push(v.to_string()));

        // 组合期:nihao → 预编辑可见但 value 不含半成品
        handle_ime(&doc, id, ImeEvent::Enabled);
        handle_ime(&doc, id, ImeEvent::Preedit("nihao".into(), Some((5, 5))));
        assert_eq!(doc.input_value(id).unwrap(), "", "预编辑不进 value");
        assert!(inputs.borrow().is_empty(), "组合中不应触发 on_input");
        // winit 在 Commit 前发空 Preedit,随后上屏
        handle_ime(&doc, id, ImeEvent::Preedit(String::new(), None));
        handle_ime(&doc, id, ImeEvent::Commit("你好".into()));
        assert_eq!(doc.input_value(id).unwrap(), "你好");
        assert_eq!(*inputs.borrow(), vec!["你好"], "Commit 应触发一次 on_input");
        assert_eq!(caret(&doc, id), (6, 6));
        // 空 Preedit 幂等:重复清除不 bump
        let v = doc.version();
        handle_ime(&doc, id, ImeEvent::Preedit(String::new(), None));
        assert_eq!(doc.version(), v);
        // Disabled 清残留预编辑
        handle_ime(&doc, id, ImeEvent::Preedit("ma".into(), None));
        handle_ime(&doc, id, ImeEvent::Disabled);
        let has_preedit =
            doc.read(|inner| inner.nodes[id].input.as_deref().unwrap().preedit.is_some());
        assert!(!has_preedit);
    }

    #[test]
    fn set_input_value_prunes_and_clears_preedit() {
        let (doc, id) = input_doc();
        apply_edit(&doc, id, EditOp::InsertStr("你好世界".into()));
        handle_ime(&doc, id, ImeEvent::Preedit("zu".into(), None));
        // 外部响应式写入:清预编辑、光标钳制到新值内 char 边界
        doc.set_input_value(id, "短");
        let (cursor, preedit) = doc.read(|inner| {
            let i = inner.nodes[id].input.as_deref().unwrap();
            (i.cursor, i.preedit.clone())
        });
        assert!(preedit.is_none(), "外部写入应清预编辑(风险 5 裁决)");
        assert_eq!(cursor, 3, "光标应钳制到新值末尾的 char 边界");
        // 相等剪枝
        let v = doc.version();
        doc.set_input_value(id, "短");
        assert_eq!(doc.version(), v);
    }

    /// 假剪贴板注入,Ctrl+C/X/V/A 全链路走 dispatch_key(离屏)
    #[test]
    fn clipboard_shortcuts_offscreen() {
        struct Fake(Rc<RefCell<String>>);
        impl Clipboard for Fake {
            fn get_text(&mut self) -> Option<String> {
                Some(self.0.borrow().clone())
            }
            fn set_text(&mut self, text: &str) {
                *self.0.borrow_mut() = text.to_string();
            }
        }
        let store: Rc<RefCell<String>> = Default::default();
        set_clipboard(Fake(store.clone()));

        let (doc, id) = input_doc();
        doc.focus(id);
        apply_edit(&doc, id, EditOp::InsertStr("秘密文本".into()));
        // Ctrl+A 全选 → Ctrl+C 复制
        dispatch_key(&doc, &KeyEvent::new(Key::Char('a'), Mods::CTRL));
        dispatch_key(&doc, &KeyEvent::new(Key::Char('c'), Mods::CTRL));
        assert_eq!(*store.borrow(), "秘密文本");
        // Ctrl+X 剪切:值清空,剪贴板保留
        dispatch_key(&doc, &KeyEvent::new(Key::Char('x'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "");
        // Ctrl+V 粘贴两次(多行折为空格由 route 层处理)
        *store.borrow_mut() = "A\nB".into();
        dispatch_key(&doc, &KeyEvent::new(Key::Char('v'), Mods::CTRL));
        dispatch_key(&doc, &KeyEvent::new(Key::Char('v'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "A BA B");
    }

    /// 键入走 dispatch_key 编辑段;Enter 提交;Tab 仍导航(不被吞)
    #[test]
    fn typing_submit_and_tab_navigation() {
        let (doc, id) = input_doc();
        let btn = doc.create_button("确定");
        doc.append(doc.root(), btn);
        let submitted: Rc<RefCell<Vec<String>>> = Default::default();
        let log = submitted.clone();
        doc.set_on_submit(id, move |v| log.borrow_mut().push(v.to_string()));

        doc.focus(id);
        // 平台文本优先(Shift 大小写),键面字符兜底
        dispatch_key(
            &doc,
            &KeyEvent::new(Key::Char('h'), Mods::NONE).with_text("h"),
        );
        dispatch_key(
            &doc,
            &KeyEvent::new(Key::Char('i'), Mods::SHIFT).with_text("I"),
        );
        dispatch_key(&doc, &KeyEvent::new(Key::Space, Mods::NONE));
        dispatch_key(&doc, &KeyEvent::new(Key::Char('!'), Mods::NONE));
        assert_eq!(doc.input_value(id).unwrap(), "hI !");
        // Enter 提交当前值
        dispatch_key(&doc, &KeyEvent::new(Key::Enter, Mods::NONE));
        assert_eq!(*submitted.borrow(), vec!["hI !"]);
        // Tab 不被编辑段吞:焦点移到按钮
        dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE));
        assert_eq!(doc.focused(), Some(btn), "Tab 应离开输入框继续导航");
        // Esc 失焦同样放行
        dispatch_key(&doc, &KeyEvent::new(Key::Escape, Mods::NONE));
        assert_eq!(doc.focused(), None);
    }
}
