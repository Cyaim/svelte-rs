//! 单行文本输入的编辑内核(调研 21,R1 第二步;R3-P3 补词/撤销/选区)。
//!
//! 纯模型、零字体依赖:光标/选区/预编辑全部按**字节偏移**存储(恒在 UTF-8
//! char 边界),几何换算(光标 x 坐标、点击定位)在渲染壳。
//!
//! **R3-P3 裁决**(修订调研 24 §3.3 的"PlainEditor 外包编辑内核"):渲染壳
//! 改用 Parley 的 `Cursor`/`Selection` **只取几何**,本模块仍是编辑唯一真源。
//! 于是不存在"编辑器池 vs 场景树"双真源,也不需要 `Generation` 回声抑制,
//! 而 kerning/fallback/BiDi 下的光标落点照样正确——PlainEditor 的收益拿到了,
//! 它的架构代价没付。
//!
//! 编辑态放节点内(`Option<Box<InputState>>`)不进 Signal:移光标是控件私有
//! 交互态,进 Signal 会把每次移动都变成可见响应式事件污染依赖图
//! (egui/iced 同款归属;与 `checked` 的"节点为渲染真源"先例一致)。

use std::cell::RefCell;
use std::rc::Rc;

use crate::{Doc, ElementKind, Key, KeyEvent, ViewId};

/// 值回调([`InputState::on_input`] / [`InputState::on_submit`])
pub type ValueHandler = Rc<dyn Fn(&str)>;

/// 撤销栈一格:整值快照。单行输入的值天然短(URL/表单字段量级),
/// 快照比 diff 简单得多且绝不会算错;多行编辑器再谈增量
#[derive(Clone, PartialEq, Debug)]
pub struct UndoEntry {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}

/// 上一次编辑的种类(撤销合并判据:连续打字合成一格,删除自成一格)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LastEdit {
    None,
    Insert,
    Delete,
}

/// 撤销栈深度上限(超出丢最旧一格)
const UNDO_CAP: usize = 128;

/// 输入框私有编辑态(回调也收在这里,控制 `ViewNode` 大小预算)
pub struct InputState {
    /// 光标字节偏移(恒在 char 边界)
    pub cursor: usize,
    /// 选区锚点;`== cursor` 即无选区
    pub anchor: usize,
    /// IME 组合中文本 + 组合区间(winit Preedit 原样;None = 无组合)
    pub preedit: Option<(String, Option<(usize, usize)>)>,
    pub placeholder: String,
    /// 多行模式(`<textarea>`):Enter 换行而不是提交,粘贴保留换行,
    /// 文本按内容宽折行。几何相关的动作(上下行移动)由渲染壳处理 ——
    /// 本模块只认字节,视觉行是排版的产物
    pub multiline: bool,
    /// 多行模式的可见行数(布局高度 = rows × 行高)
    pub rows: u16,
    /// 单行横向滚动(光标跟随;渲染壳每帧计算,此处为持久化载体)
    pub scroll_x: f32,
    /// 值变化回调(每次编辑后带新值)
    pub on_input: Option<ValueHandler>,
    /// Enter 提交回调(带当前值)
    pub on_submit: Option<ValueHandler>,
    /// 撤销/重做栈(外部写入 `set_input_value` 会清空,与浏览器 input 一致)
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    last_edit: LastEdit,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            cursor: 0,
            anchor: 0,
            preedit: None,
            placeholder: String::new(),
            multiline: false,
            rows: 3,
            scroll_x: 0.0,
            on_input: None,
            on_submit: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: LastEdit::None,
        }
    }
}

impl InputState {
    /// 外部写入后清空历史(浏览器 input 同款:程序化赋值不进撤销栈)
    pub(crate) fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.last_edit = LastEdit::None;
    }

    /// 编辑前记一格。`kind` 相同且是连打字符时合并进上一格(不新开)
    fn push_undo(&mut self, kind: LastEdit, text: &str, coalesce: bool) {
        self.redo.clear();
        if coalesce && self.last_edit == kind && !self.undo.is_empty() {
            self.last_edit = kind;
            return;
        }
        if self.undo.len() >= UNDO_CAP {
            self.undo.remove(0);
        }
        self.undo.push(UndoEntry {
            text: text.to_string(),
            cursor: self.cursor,
            anchor: self.anchor,
        });
        // 不可合并的编辑(空格、整段粘贴、删词)**同时封口下一格**:
        // 否则空格后的连打会并进空格那一格,撤销一次退两个词
        self.last_edit = if coalesce { kind } else { LastEdit::None };
    }
}

/// 光标移动目标
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Caret {
    Left,
    Right,
    /// 词首(Ctrl/⌥ + ←)
    WordLeft,
    /// 下一词首(Ctrl/⌥ + →)
    WordRight,
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
    /// 删到词首/词尾(Ctrl+Backspace / Ctrl+Delete)
    DeleteWordBackward,
    DeleteWordForward,
    /// bool = 是否扩选(Shift)
    Move(Caret, bool),
    /// 定光标到字节偏移(点击/拖选;bool = 扩选)
    MoveTo(usize, bool),
    /// 选中字节区间(双击选词、三击全选走这里)
    SelectRange(usize, usize),
    SelectAll,
    Undo,
    Redo,
}

// ---------------------------------------------------------------------------
// 词边界(调研 21 步 6)
//
// **不引 UAX #29 分词表**:sv-ui 是双前端的编译目标,依赖面必须干净;而且
// 上游 icu_segmenter 在没有 cjdict 数据时对中文本就退化成逐字断——所以这里
// 的"表意文字逐字成词"与真分词器的实际可得行为一致。规则三类:
// 空白 / 表意文字(逐字) / 其余按"字母数字"与"标点"两类各自成串。
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Space,
    Ideograph,
    Word,
    Punct,
}

/// 表意文字与假名:逐字成词(中日韩无词间空格)
fn is_ideograph(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF      // 平假名/片假名
        | 0x3400..=0x4DBF    // CJK 扩展 A
        | 0x4E00..=0x9FFF    // CJK 基本区
        | 0xF900..=0xFAFF    // 兼容表意
        | 0xAC00..=0xD7AF    // 谚文音节
        | 0x20000..=0x3FFFF  // CJK 扩展 B+
    )
}

fn class_of(c: char) -> Class {
    if c.is_whitespace() {
        Class::Space
    } else if is_ideograph(c) {
        Class::Ideograph
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// 下一词首(桌面惯例:吃掉当前词,再跳过随后的空白)
pub fn next_word_boundary(s: &str, i: usize) -> usize {
    let i = snap(s, i);
    let mut it = s[i..].char_indices().peekable();
    let mut at = i;
    if let Some(&(_, c)) = it.peek() {
        let k = class_of(c);
        if k == Class::Ideograph {
            at += c.len_utf8();
            it.next();
        } else if k != Class::Space {
            while let Some(&(off, c)) = it.peek() {
                if class_of(c) != k {
                    break;
                }
                at = i + off + c.len_utf8();
                it.next();
            }
        }
    }
    // 跳过词后空白
    for (off, c) in s[at..].char_indices() {
        if !c.is_whitespace() {
            return at + off;
        }
    }
    s.len()
}

/// 上一词首(先跳过左侧空白,再吃掉整个词)
pub fn prev_word_boundary(s: &str, i: usize) -> usize {
    let mut at = snap(s, i);
    // 跳过左侧空白
    while at > 0 {
        let p = prev_boundary(s, at);
        if !s[p..at].chars().next().is_some_and(char::is_whitespace) {
            break;
        }
        at = p;
    }
    if at == 0 {
        return 0;
    }
    let p = prev_boundary(s, at);
    let k = class_of(s[p..at].chars().next().unwrap());
    if k == Class::Ideograph {
        return p;
    }
    at = p;
    while at > 0 {
        let p = prev_boundary(s, at);
        if class_of(s[p..at].chars().next().unwrap()) != k {
            break;
        }
        at = p;
    }
    at
}

/// `byte` 处的词区间(双击选词;落在空白上则选中整段空白)
pub fn word_range_at(s: &str, byte: usize) -> (usize, usize) {
    if s.is_empty() {
        return (0, 0);
    }
    let byte = snap(s, byte);
    // 词尾光标:按左侧字符判词
    let at = if byte == s.len() {
        prev_boundary(s, byte)
    } else {
        byte
    };
    let Some(c) = s[at..].chars().next() else {
        return (s.len(), s.len());
    };
    let k = class_of(c);
    if k == Class::Ideograph {
        return (at, at + c.len_utf8());
    }
    let mut lo = at;
    while lo > 0 {
        let p = prev_boundary(s, lo);
        if class_of(s[p..lo].chars().next().unwrap()) != k {
            break;
        }
        lo = p;
    }
    let mut hi = at;
    for (off, c) in s[at..].char_indices() {
        if class_of(c) != k {
            hi = at + off;
            return (lo, hi);
        }
        hi = at + off + c.len_utf8();
    }
    (lo, hi)
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

/// 钳到串内并落到 char 边界(外部给的字节偏移一律先过这里)
fn snap(s: &str, i: usize) -> usize {
    let mut i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
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
                // 连打非空白字符合并成一格撤销;空格/整段粘贴各自成格
                let coalesce = sel_lo == sel_hi
                    && s.chars().count() == 1
                    && !s.starts_with(char::is_whitespace);
                input.push_undo(LastEdit::Insert, &before_text, coalesce);
                n.text.replace_range(sel_lo..sel_hi, &s);
                input.cursor = sel_lo + s.len();
                input.anchor = input.cursor;
            }
            EditOp::DeleteBackward | EditOp::DeleteForward => {
                let backward = op == EditOp::DeleteBackward;
                let range = if sel_lo != sel_hi {
                    Some(sel_lo..sel_hi)
                } else if backward && sel_lo > 0 {
                    Some(prev_boundary(&n.text, sel_lo)..sel_lo)
                } else if !backward && sel_hi < n.text.len() {
                    Some(sel_hi..next_boundary(&n.text, sel_hi))
                } else {
                    None
                };
                if let Some(r) = range {
                    input.push_undo(LastEdit::Delete, &before_text, true);
                    input.cursor = r.start;
                    n.text.replace_range(r, "");
                }
                input.anchor = input.cursor;
            }
            EditOp::DeleteWordBackward | EditOp::DeleteWordForward => {
                let backward = op == EditOp::DeleteWordBackward;
                let range = if sel_lo != sel_hi {
                    sel_lo..sel_hi
                } else if backward {
                    prev_word_boundary(&n.text, sel_lo)..sel_lo
                } else {
                    sel_hi..next_word_boundary(&n.text, sel_hi)
                };
                if !range.is_empty() {
                    // 词删除不与连打合并:一次 Ctrl+Backspace 一格
                    input.push_undo(LastEdit::None, &before_text, false);
                    input.cursor = range.start;
                    n.text.replace_range(range, "");
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
                    Caret::WordLeft => prev_word_boundary(&n.text, input.cursor),
                    Caret::WordRight => next_word_boundary(&n.text, input.cursor),
                    // 多行:Home/End 走**硬行**(换行符之间)。视觉折行的
                    // 行首行尾要几何才知道,那是渲染壳的活儿,模型层不猜
                    Caret::Home if input.multiline => {
                        n.text[..input.cursor].rfind('\n').map_or(0, |i| i + 1)
                    }
                    Caret::End if input.multiline => n.text[input.cursor..]
                        .find('\n')
                        .map_or(n.text.len(), |i| input.cursor + i),
                    Caret::Home => 0,
                    Caret::End => n.text.len(),
                };
                if !extend {
                    input.anchor = input.cursor;
                }
                input.last_edit = LastEdit::None;
            }
            EditOp::MoveTo(byte, extend) => {
                input.cursor = snap(&n.text, byte);
                if !extend {
                    input.anchor = input.cursor;
                }
                input.last_edit = LastEdit::None;
            }
            EditOp::SelectRange(lo, hi) => {
                input.anchor = snap(&n.text, lo);
                input.cursor = snap(&n.text, hi);
                input.last_edit = LastEdit::None;
            }
            EditOp::SelectAll => {
                input.anchor = 0;
                input.cursor = n.text.len();
                input.last_edit = LastEdit::None;
            }
            EditOp::Undo | EditOp::Redo => {
                let (from, to) = if op == EditOp::Undo {
                    (&mut input.undo, &mut input.redo)
                } else {
                    (&mut input.redo, &mut input.undo)
                };
                if let Some(entry) = from.pop() {
                    to.push(UndoEntry {
                        text: before_text.clone(),
                        cursor: before_c,
                        anchor: before_a,
                    });
                    n.text = entry.text;
                    input.cursor = snap(&n.text, entry.cursor);
                    input.anchor = snap(&n.text, entry.anchor);
                }
                input.last_edit = LastEdit::None;
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
        // **打字是纯绘制帧**:输入框的测量恒为 200×行高×rows,与内容无关。
        // 这是分级表里收益最大的一条 —— 每个按键以前都在重排整棵树。
        // 【做 auto-size input 时这一条要改成 Measure】
        doc.bump(crate::dirty::DirtyItem::Paint);
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
                // 预编辑串画在输入框里,不改输入框尺寸
                doc.bump(crate::dirty::DirtyItem::Paint);
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
                doc.bump(crate::dirty::DirtyItem::Paint);
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
fn is_multiline(doc: &Doc, id: ViewId) -> bool {
    doc.read(|inner| {
        inner
            .nodes
            .get(id)
            .and_then(|n| n.input.as_deref())
            .is_some_and(|i| i.multiline)
    })
}

pub(crate) fn route_editing_key(doc: &Doc, id: ViewId, e: &KeyEvent) -> bool {
    debug_assert!(doc.read(|inner| {
        inner
            .nodes
            .get(id)
            .is_some_and(|n| n.kind == ElementKind::TextInput)
    }));
    let ctrl_or_meta = e.mods.ctrl || e.mods.meta;
    // 词跳修饰键:Win/Linux 是 Ctrl,macOS 惯例是 ⌥(Alt)
    let word_mod = ctrl_or_meta || e.mods.alt;
    match e.key {
        // 快捷键族:Ctrl/Cmd + A/C/X/V/Z/Y(其余组合放行给全局快捷键段)
        Key::Char(c) if ctrl_or_meta => match c.to_ascii_lowercase() {
            'a' => {
                apply_edit(doc, id, EditOp::SelectAll);
                true
            }
            // Ctrl+Z 撤销,Ctrl+Shift+Z / Ctrl+Y 重做(两套惯例都收)
            'z' => {
                let op = if e.mods.shift {
                    EditOp::Redo
                } else {
                    EditOp::Undo
                };
                apply_edit(doc, id, op);
                true
            }
            'y' => {
                apply_edit(doc, id, EditOp::Redo);
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
                    let text = if is_multiline(doc, id) {
                        // 多行:保留换行,只统一 CRLF
                        text.replace("\r\n", "\n").replace('\r', "\n")
                    } else {
                        // 单行控件:粘贴内容里的换行替换为空格
                        text.replace(['\r', '\n'], " ")
                    };
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
            let op = if word_mod {
                EditOp::DeleteWordBackward
            } else {
                EditOp::DeleteBackward
            };
            apply_edit(doc, id, op);
            true
        }
        Key::Delete => {
            let op = if word_mod {
                EditOp::DeleteWordForward
            } else {
                EditOp::DeleteForward
            };
            apply_edit(doc, id, op);
            true
        }
        Key::ArrowLeft => {
            let caret = if word_mod {
                Caret::WordLeft
            } else {
                Caret::Left
            };
            apply_edit(doc, id, EditOp::Move(caret, e.mods.shift));
            true
        }
        Key::ArrowRight => {
            let caret = if word_mod {
                Caret::WordRight
            } else {
                Caret::Right
            };
            apply_edit(doc, id, EditOp::Move(caret, e.mods.shift));
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
        // 多行:Enter 换行(与浏览器 textarea 一致);提交交给按钮或快捷键
        Key::Enter if is_multiline(doc, id) => {
            apply_edit(doc, id, EditOp::InsertStr("\n".into()));
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

    /// 词边界规则:空白/表意文字逐字/字母数字串/标点串四类
    #[test]
    fn word_boundaries_by_char_class() {
        let s = "foo bar_1, 你好 世界";
        // 下一词首:吃掉当前词 + 随后空白
        assert_eq!(next_word_boundary(s, 0), 4, "foo → bar_1");
        assert_eq!(next_word_boundary(s, 1), 4, "词中间也跳到下一词首");
        assert_eq!(next_word_boundary(s, 4), 9, "bar_1 → 标点 ','");
        assert_eq!(next_word_boundary(s, 9), 11, "',' 后跳过空白到 '你'");
        assert_eq!(next_word_boundary(s, 11), 14, "表意文字逐字:你 → 好");
        assert_eq!(next_word_boundary(s, s.len()), s.len(), "串尾幂等");
        // 上一词首:先跳空白再吃整词
        assert_eq!(
            prev_word_boundary(s, s.len()),
            s.len() - 3,
            "回到 '界'(逐字)"
        );
        assert_eq!(prev_word_boundary(s, 4), 0);
        assert_eq!(prev_word_boundary(s, 3), 0);
        assert_eq!(prev_word_boundary(s, 0), 0, "串首幂等");
        // 双击选词
        assert_eq!(word_range_at(s, 5), (4, 9), "bar_1 整词");
        assert_eq!(&s[11..14], "你");
        assert_eq!(word_range_at(s, 12), (11, 14), "落在多字节中间也回到该字");
        assert_eq!(word_range_at("", 0), (0, 0));
    }

    /// Ctrl+←/→ 词跳、Ctrl+Backspace 删词(全走 dispatch_key)
    #[test]
    fn word_motion_and_delete_via_keys() {
        let (doc, id) = input_doc();
        doc.focus(id);
        apply_edit(&doc, id, EditOp::InsertStr("hello brave world".into()));
        dispatch_key(&doc, &KeyEvent::new(Key::ArrowLeft, Mods::CTRL));
        assert_eq!(caret(&doc, id).0, 12, "Ctrl+← 应停在 'world' 词首");
        dispatch_key(&doc, &KeyEvent::new(Key::ArrowLeft, Mods::CTRL));
        assert_eq!(caret(&doc, id).0, 6, "再一次到 'brave'");
        // Shift 扩选一个词
        let mut shift_ctrl = Mods::CTRL;
        shift_ctrl.shift = true;
        dispatch_key(&doc, &KeyEvent::new(Key::ArrowRight, shift_ctrl));
        assert_eq!(doc.selected_text(id).unwrap(), "brave ");
        // Ctrl+Backspace:有选区先删选区
        dispatch_key(&doc, &KeyEvent::new(Key::Backspace, Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "hello world");
        // 无选区时删到词首
        apply_edit(&doc, id, EditOp::Move(Caret::End, false));
        dispatch_key(&doc, &KeyEvent::new(Key::Backspace, Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "hello ");
        // Ctrl+Delete 往后删词
        apply_edit(&doc, id, EditOp::Move(Caret::Home, false));
        dispatch_key(&doc, &KeyEvent::new(Key::Delete, Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "");
    }

    /// 撤销/重做:连打合并成一格、删除自成一格、外部写入清历史
    #[test]
    fn undo_redo_stack() {
        let (doc, id) = input_doc();
        doc.focus(id);
        for c in "abc".chars() {
            dispatch_key(&doc, &KeyEvent::new(Key::Char(c), Mods::NONE));
        }
        dispatch_key(&doc, &KeyEvent::new(Key::Space, Mods::NONE));
        for c in "de".chars() {
            dispatch_key(&doc, &KeyEvent::new(Key::Char(c), Mods::NONE));
        }
        assert_eq!(doc.input_value(id).unwrap(), "abc de");
        // 一次撤销回退整段连打 "de"(空格自成一格)
        dispatch_key(&doc, &KeyEvent::new(Key::Char('z'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "abc ");
        dispatch_key(&doc, &KeyEvent::new(Key::Char('z'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "abc");
        dispatch_key(&doc, &KeyEvent::new(Key::Char('z'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "");
        // 栈空后再撤销幂等
        dispatch_key(&doc, &KeyEvent::new(Key::Char('z'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "");
        // 重做(Ctrl+Y 与 Ctrl+Shift+Z 两套惯例)
        dispatch_key(&doc, &KeyEvent::new(Key::Char('y'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "abc");
        let mut shift_ctrl = Mods::CTRL;
        shift_ctrl.shift = true;
        dispatch_key(&doc, &KeyEvent::new(Key::Char('z'), shift_ctrl));
        assert_eq!(doc.input_value(id).unwrap(), "abc ");
        // 新编辑清空重做栈
        dispatch_key(&doc, &KeyEvent::new(Key::Char('X'), Mods::NONE));
        dispatch_key(&doc, &KeyEvent::new(Key::Char('y'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "abc X", "重做栈应已作废");
        // 撤销要带回光标/选区位置
        apply_edit(&doc, id, EditOp::SelectAll);
        apply_edit(&doc, id, EditOp::DeleteBackward);
        dispatch_key(&doc, &KeyEvent::new(Key::Char('z'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "abc X");
        assert_eq!(doc.selected_text(id).unwrap(), "abc X", "应恢复原选区");
        // 外部写入清历史
        doc.set_input_value(id, "外部");
        dispatch_key(&doc, &KeyEvent::new(Key::Char('z'), Mods::CTRL));
        assert_eq!(
            doc.input_value(id).unwrap(),
            "外部",
            "程序化赋值不该被 Ctrl+Z 回滚"
        );
    }

    /// 多行模式:Enter 换行、粘贴保留换行、Home/End 走硬行
    #[test]
    fn multiline_enter_paste_and_line_home_end() {
        struct Fake(&'static str);
        impl Clipboard for Fake {
            fn get_text(&mut self) -> Option<String> {
                Some(self.0.to_string())
            }
            fn set_text(&mut self, _: &str) {}
        }
        let (doc, id) = input_doc();
        let submitted: Rc<RefCell<Vec<String>>> = Default::default();
        let log = submitted.clone();
        doc.set_on_submit(id, move |v| log.borrow_mut().push(v.to_string()));
        doc.focus(id);

        // 单行:Enter 提交
        dispatch_key(&doc, &KeyEvent::new(Key::Char('a'), Mods::NONE));
        dispatch_key(&doc, &KeyEvent::new(Key::Enter, Mods::NONE));
        assert_eq!(*submitted.borrow(), vec!["a"], "单行 Enter 应提交");

        // 切多行:Enter 换行,不再提交
        doc.set_multiline(id, true, 4);
        dispatch_key(&doc, &KeyEvent::new(Key::Enter, Mods::NONE));
        dispatch_key(&doc, &KeyEvent::new(Key::Char('b'), Mods::NONE));
        assert_eq!(
            doc.input_value(id).unwrap(),
            "a
b"
        );
        assert_eq!(submitted.borrow().len(), 1, "多行 Enter 不该提交");

        // Home/End 走硬行(不是整串首尾)
        apply_edit(&doc, id, EditOp::Move(Caret::Home, false));
        assert_eq!(caret(&doc, id).0, 2, "Home 应到本行行首(\n 之后)");
        apply_edit(&doc, id, EditOp::Move(Caret::End, false));
        assert_eq!(caret(&doc, id).0, 3, "End 应到本行行尾");
        // 第一行的 End 停在换行符前
        apply_edit(&doc, id, EditOp::MoveTo(0, false));
        apply_edit(&doc, id, EditOp::Move(Caret::End, false));
        assert_eq!(caret(&doc, id).0, 1);

        // 粘贴:多行保留换行(单行折成空格)
        set_clipboard(Fake(
            "x
y",
        ));
        apply_edit(&doc, id, EditOp::Move(Caret::End, false));
        dispatch_key(&doc, &KeyEvent::new(Key::Char('v'), Mods::CTRL));
        assert_eq!(
            doc.input_value(id).unwrap(),
            "ax
y
b",
            "CRLF 应统一成 \n"
        );
        doc.set_multiline(id, false, 1);
        apply_edit(&doc, id, EditOp::SelectAll);
        dispatch_key(&doc, &KeyEvent::new(Key::Char('v'), Mods::CTRL));
        assert_eq!(doc.input_value(id).unwrap(), "x y", "单行应把换行折成空格");
    }

    /// 双击选词 / 三击全选 / 拖选(渲染壳调的 Doc 面)
    #[test]
    fn pointer_selection_api() {
        let (doc, id) = input_doc();
        apply_edit(&doc, id, EditOp::InsertStr("hello 世界 x".into()));
        doc.select_word_at(id, 2);
        assert_eq!(doc.selected_text(id).unwrap(), "hello");
        doc.select_word_at(id, 6);
        assert_eq!(doc.selected_text(id).unwrap(), "世", "表意文字逐字选");
        doc.select_range(id, 0, doc.input_value(id).unwrap().len());
        assert_eq!(doc.selected_text(id).unwrap(), "hello 世界 x");
        // 拖选:按下定锚,移动扩选
        doc.set_caret(id, 0, false);
        doc.set_caret(id, 5, true);
        assert_eq!(doc.selected_text(id).unwrap(), "hello");
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
