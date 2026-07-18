//! 键盘事件类型 + 焦点路由(调研 20,R1 输入地基)。
//!
//! 事件类型归 sv-ui 自有(ADR-4:窗口层是窄 trait,鸿蒙 XComponent 端要能喂
//! 同一类型),渲染壳负责把平台事件(winit 等)映射成 [`KeyEvent`] 再交给
//! [`dispatch_key`] 四段路由:冒泡 → 导航 → 激活 → 快捷键。

use std::cell::Cell;

use crate::{Doc, ElementKind, input, shortcuts};

/// 键(v0 裁剪面:~20 个具名键 + `Char` 兜底;漏配键由渲染壳丢弃)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    /// 可打印字符(空格单列 [`Key::Space`])
    Char(char),
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Space,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Home,
    End,
    PageUp,
    PageDown,
    /// 功能键 F1–F12
    F(u8),
}

/// 修饰键组合(精确匹配语义:快捷键查表不允许多余修饰键)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        shift: false,
        alt: false,
        meta: false,
    };
    pub const CTRL: Mods = Mods {
        ctrl: true,
        ..Mods::NONE
    };
    pub const SHIFT: Mods = Mods {
        shift: true,
        ..Mods::NONE
    };
    pub const ALT: Mods = Mods {
        alt: true,
        ..Mods::NONE
    };
    pub const META: Mods = Mods {
        meta: true,
        ..Mods::NONE
    };
}

/// 键盘事件(DOM 心智:`stop_propagation` 截断冒泡,`prevent_default`
/// 取消默认层——Tab 导航 / Enter 激活 / 快捷键)
pub struct KeyEvent {
    pub key: Key,
    /// 本次按键产生的文本(为 IME/文本输入预留;快捷键路径不读它)
    pub text: Option<String>,
    pub mods: Mods,
    pub repeat: bool,
    stop: Cell<bool>,
    default_prevented: Cell<bool>,
}

impl KeyEvent {
    pub fn new(key: Key, mods: Mods) -> Self {
        Self {
            key,
            text: None,
            mods,
            repeat: false,
            stop: Cell::new(false),
            default_prevented: Cell::new(false),
        }
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    pub fn with_repeat(mut self, repeat: bool) -> Self {
        self.repeat = repeat;
        self
    }

    /// 截断冒泡:父链上更外层的 `on_key` 不再收到本事件
    pub fn stop_propagation(&self) {
        self.stop.set(true);
    }

    pub fn propagation_stopped(&self) -> bool {
        self.stop.get()
    }

    /// 取消默认层:Tab 导航、Enter/Space 激活、快捷键全部跳过
    /// (文本框吞 Tab 的场景用它)
    pub fn prevent_default(&self) {
        self.default_prevented.set(true);
    }

    pub fn default_prevented(&self) -> bool {
        self.default_prevented.get()
    }
}

/// 键盘事件四段路由(调研 20 §3.2;返回是否有段消费了事件)。
///
/// ① 冒泡段:从焦点节点沿父链逐个调 `on_key`,`stop_propagation()` 截断;
/// ② 导航段:Tab → 下一个 / Shift+Tab → 上一个 / Esc → 失焦;
/// ③ 激活段:Enter/Space 且焦点是 Button/Checkbox → 调点击回调
///    (按钮免费获得键盘可达性);
/// ④ 快捷键段:精确查注册表,后注册者优先、调用即止(repeat 忽略)。
///
/// ②–④ 仅在 `prevent_default()` 未置位时进入。放在 sv-ui 而非渲染壳:
/// 路由是纯树逻辑,离屏可测,且未来鸿蒙壳复用同一实现。
pub fn dispatch_key(doc: &Doc, e: &KeyEvent) -> bool {
    // ① 冒泡段(handler clone 出借用外再调,既有惯例)
    let mut handled = false;
    let mut cur = doc.focused();
    while let Some(id) = cur {
        if let Some(h) = doc.key_handler(id) {
            handled = true;
            h(e);
        }
        if e.propagation_stopped() {
            break;
        }
        cur = doc.parent(id);
    }
    if e.default_prevented() {
        return true;
    }

    // ①.5 编辑段:焦点是 TextInput → 键翻译成 EditOp / 剪贴板 / Enter 提交
    // (Tab/Esc 不消费,放行给导航段——文本框保留键盘可达性)
    if let Some(id) = doc.focused()
        && doc.read(|inner| {
            inner
                .nodes
                .get(id)
                .is_some_and(|n| n.kind == ElementKind::TextInput)
        })
        && input::route_editing_key(doc, id, e)
    {
        return true;
    }

    // ② 导航段
    match e.key {
        Key::Tab if e.mods == Mods::NONE => {
            doc.focus_next();
            return true;
        }
        Key::Tab if e.mods == Mods::SHIFT => {
            doc.focus_prev();
            return true;
        }
        // 菜单方向键(调研 25 O4):焦点在 Popup 弹层内时,上下键即焦点
        // 移动——菜单/下拉免费获得方向键导航(TextInput 的方向键已被
        // 编辑段先消费,不冲突)
        Key::ArrowDown
            if e.mods == Mods::NONE
                && doc.focused().is_some_and(|f| {
                    doc.overlay_layer_of(f) == Some(crate::OverlayLayer::Popup)
                }) =>
        {
            doc.focus_next();
            return true;
        }
        Key::ArrowUp
            if e.mods == Mods::NONE
                && doc.focused().is_some_and(|f| {
                    doc.overlay_layer_of(f) == Some(crate::OverlayLayer::Popup)
                }) =>
        {
            doc.focus_prev();
            return true;
        }
        Key::Escape if e.mods == Mods::NONE => {
            // 弹层优先(调研 25 O2:LIFO,嵌套弹层逐层关);其次失焦
            if doc.dismiss_topmost_overlay() {
                return true;
            }
            if doc.focused().is_some() {
                doc.blur();
                return true;
            }
        }
        _ => {}
    }

    // ③ 激活段
    if matches!(e.key, Key::Enter | Key::Space)
        && e.mods == Mods::NONE
        && let Some(id) = doc.focused()
        && doc.read(|inner| {
            inner
                .nodes
                .get(id)
                .is_some_and(|n| matches!(n.kind, ElementKind::Button | ElementKind::Checkbox))
        })
        && let Some(h) = doc.click_handler(id)
    {
        h();
        return true;
    }

    // ④ 快捷键段(v0:repeat 不触发快捷键,业界无共识,留 flag 后议)
    if !e.repeat
        && shortcuts::dispatch_shortcut(shortcuts::Shortcut {
            mods: e.mods,
            key: e.key,
        })
    {
        return true;
    }

    handled
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;
    use crate::Doc;

    /// root
    /// ├── view(不可焦)
    /// │   ├── btn_a
    /// │   └── btn_b
    /// └── btn_c
    fn three_buttons() -> (Doc, crate::ViewId, crate::ViewId, crate::ViewId) {
        let doc = Doc::new();
        let group = doc.create_view();
        doc.append(doc.root(), group);
        let a = doc.create_button("A");
        doc.append(group, a);
        let b = doc.create_button("B");
        doc.append(group, b);
        let c = doc.create_button("C");
        doc.append(doc.root(), c);
        (doc, a, b, c)
    }

    #[test]
    fn focus_next_follows_tree_order() {
        let (doc, a, b, c) = three_buttons();
        assert_eq!(doc.focused(), None);
        doc.focus_next();
        assert_eq!(doc.focused(), Some(a), "无焦点时 Tab 落到树序第一个");
        doc.focus_next();
        assert_eq!(doc.focused(), Some(b));
        doc.focus_next();
        assert_eq!(doc.focused(), Some(c));
    }

    #[test]
    fn focus_wraps_and_skips_unfocusable() {
        let (doc, a, _b, c) = three_buttons();
        doc.focus(c);
        doc.focus_next();
        assert_eq!(doc.focused(), Some(a), "尾部 Tab 应环绕回第一个");
        doc.focus_prev();
        assert_eq!(doc.focused(), Some(c), "头部 Shift+Tab 应环绕到最后");
        // View/Text 不可焦,遍历只在三个按钮间走
        for _ in 0..6 {
            doc.focus_next();
        }
        assert_eq!(doc.focused(), Some(c), "6 次 Tab 转两圈仍应停在按钮上");
    }

    #[test]
    fn focus_change_fires_blur_then_focus() {
        let (doc, a, b, _c) = three_buttons();
        let log: Rc<RefCell<Vec<String>>> = Default::default();
        let l = log.clone();
        doc.set_on_focus_change(a, move |f| l.borrow_mut().push(format!("a:{f}")));
        let l = log.clone();
        doc.set_on_focus_change(b, move |f| l.borrow_mut().push(format!("b:{f}")));
        doc.focus(a);
        doc.focus(b);
        assert_eq!(
            *log.borrow(),
            vec!["a:true", "a:false", "b:true"],
            "移焦顺序应为:旧失焦在前,新获焦在后"
        );
        // 相等剪枝:重复 focus 不再回调
        let v = doc.version();
        doc.focus(b);
        assert_eq!(doc.version(), v);
        assert_eq!(log.borrow().len(), 3);
        doc.blur();
        assert_eq!(log.borrow().last().unwrap(), "b:false");
        assert_eq!(doc.focused(), None);
    }

    #[test]
    fn remove_focused_subtree_clears_focus() {
        let doc = Doc::new();
        let group = doc.create_view();
        doc.append(doc.root(), group);
        let btn = doc.create_button("X");
        doc.append(group, btn);
        let blurred = Rc::new(RefCell::new(false));
        let bl = blurred.clone();
        doc.set_on_focus_change(btn, move |f| {
            if !f {
                *bl.borrow_mut() = true;
            }
        });
        doc.focus(btn);
        doc.remove(group); // 删的是祖先容器,焦点在其子树内
        assert_eq!(doc.focused(), None, "被删子树含焦点时应清焦点");
        assert!(*blurred.borrow(), "清焦点应触发失焦回调");
        // 焦点在子树外:remove 不动焦点
        let a = doc.create_button("A");
        doc.append(doc.root(), a);
        let other = doc.create_view();
        doc.append(doc.root(), other);
        doc.focus(a);
        doc.remove(other);
        assert_eq!(doc.focused(), Some(a));
    }

    #[test]
    fn key_event_bubbles_until_consumed() {
        let doc = Doc::new();
        let outer = doc.create_view();
        doc.append(doc.root(), outer);
        let inner = doc.create_view();
        doc.append(outer, inner);
        let btn = doc.create_button("X");
        doc.append(inner, btn);

        let log: Rc<RefCell<Vec<&'static str>>> = Default::default();
        let l = log.clone();
        doc.set_on_key(btn, move |_| l.borrow_mut().push("btn"));
        let l = log.clone();
        doc.set_on_key(inner, move |e| {
            l.borrow_mut().push("inner");
            e.stop_propagation();
        });
        let l = log.clone();
        doc.set_on_key(outer, move |_| l.borrow_mut().push("outer"));

        doc.focus(btn);
        dispatch_key(&doc, &KeyEvent::new(Key::Char('x'), Mods::NONE));
        assert_eq!(
            *log.borrow(),
            vec!["btn", "inner"],
            "stop_propagation 应截断冒泡,outer 不应收到"
        );
    }

    #[test]
    fn tab_enter_escape_default_layer() {
        let (doc, a, b, _c) = three_buttons();
        let clicks = Rc::new(RefCell::new(0));
        let cl = clicks.clone();
        doc.set_on_click(b, move || *cl.borrow_mut() += 1);

        // Tab 两次到 b,Enter 激活,Space 再激活,Esc 失焦
        assert!(dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE)));
        assert_eq!(doc.focused(), Some(a));
        assert!(dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE)));
        assert_eq!(doc.focused(), Some(b));
        assert!(dispatch_key(&doc, &KeyEvent::new(Key::Enter, Mods::NONE)));
        assert!(dispatch_key(&doc, &KeyEvent::new(Key::Space, Mods::NONE)));
        assert_eq!(*clicks.borrow(), 2, "Enter/Space 都应激活焦点按钮");
        assert!(dispatch_key(&doc, &KeyEvent::new(Key::Escape, Mods::NONE)));
        assert_eq!(doc.focused(), None, "Esc 应失焦");
        // Shift+Tab 反向
        dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::SHIFT));
        assert_eq!(doc.focused(), doc.focusables().last().copied());
    }

    #[test]
    fn prevent_default_skips_navigation() {
        let (doc, a, _b, _c) = three_buttons();
        doc.set_on_key(a, |e| {
            if e.key == Key::Tab {
                e.prevent_default(); // 文本框吞 Tab 的场景
            }
        });
        doc.focus(a);
        assert!(dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE)));
        assert_eq!(doc.focused(), Some(a), "prevent_default 后 Tab 不应移焦");
    }
}
