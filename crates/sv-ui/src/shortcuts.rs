//! 全局快捷键注册表(调研 20 §3.3)。
//!
//! 精确匹配(`Ctrl+S` 不会吃掉 `Ctrl+Shift+S`,规避 egui `matches_logically`
//! 的"具体优先"排序负担);注册挂当前响应式作用域,`on_cleanup` 自动注销——
//! 组件/`if_block` 分支卸载,快捷键随之消失。同键多注册后进先出、调用即止
//! (对话框压栈覆盖底层快捷键的正确语义)。
//!
//! 已知边界(调研 20 §5):注册表 thread-local 全局,多窗口(M2)需 per-Doc 化。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use sv_reactive::on_cleanup;

use crate::{Key, Mods};

/// 快捷键 = 精确修饰键组合 + 键(`Hash` 查表 O(1))
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Shortcut {
    pub mods: Mods,
    pub key: Key,
}

impl Shortcut {
    pub const fn new(mods: Mods, key: Key) -> Self {
        Self { mods, key }
    }

    /// 平台惯例修饰键:macOS → Cmd(META),其余 → Ctrl。
    /// 注册时展开,不引入运行时判断
    pub fn cmd_or_ctrl(key: Key) -> Self {
        #[cfg(target_os = "macos")]
        return Self {
            mods: Mods::META,
            key,
        };
        #[cfg(not(target_os = "macos"))]
        Self {
            mods: Mods::CTRL,
            key,
        }
    }
}

type Registry = HashMap<Shortcut, Vec<(u64, Rc<dyn Fn()>)>>;

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(HashMap::new());
    static NEXT_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// 注册快捷键,挂当前响应式作用域(effect/root),作用域销毁自动注销。
/// 作用域外调用会注册成功但永不注销(sv-reactive 会在 debug 下告警)
pub fn register_shortcut(sc: Shortcut, f: impl Fn() + 'static) {
    let id = NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    REGISTRY.with(|r| {
        r.borrow_mut().entry(sc).or_default().push((id, Rc::new(f)));
    });
    on_cleanup(move || {
        REGISTRY.with(|r| {
            let mut reg = r.borrow_mut();
            if let Some(v) = reg.get_mut(&sc) {
                v.retain(|(i, _)| *i != id);
                if v.is_empty() {
                    reg.remove(&sc);
                }
            }
        });
    });
}

/// 派发:精确匹配,后注册者优先、只调一个。返回是否触发
/// (回调 clone 出借用外再调:回调里可以再注册/注销快捷键)
pub fn dispatch_shortcut(sc: Shortcut) -> bool {
    let cb = REGISTRY.with(|r| {
        r.borrow()
            .get(&sc)
            .and_then(|v| v.last())
            .map(|(_, f)| f.clone())
    });
    match cb {
        Some(cb) => {
            cb();
            true
        }
        None => false,
    }
}

/// 测试辅助:当前注册条目总数
#[cfg(test)]
pub(crate) fn debug_count() -> usize {
    REGISTRY.with(|r| r.borrow().values().map(Vec::len).sum())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use sv_reactive::create_root;

    use super::*;
    use crate::{Doc, KeyEvent, dispatch_key};

    #[test]
    fn shortcut_fires_when_unconsumed() {
        let fired = Rc::new(RefCell::new(0));
        let f = fired.clone();
        let (_, scope) = create_root(move || {
            register_shortcut(Shortcut::new(Mods::CTRL, Key::Char('s')), move || {
                *f.borrow_mut() += 1;
            });
        });
        let doc = Doc::new();
        // 无焦点、无 handler:事件落到快捷键段
        assert!(dispatch_key(
            &doc,
            &KeyEvent::new(Key::Char('s'), Mods::CTRL)
        ));
        assert_eq!(*fired.borrow(), 1);
        // 精确匹配:多余修饰键不触发(Ctrl+Shift+S ≠ Ctrl+S)
        let mods = Mods {
            shift: true,
            ..Mods::CTRL
        };
        assert!(!dispatch_key(&doc, &KeyEvent::new(Key::Char('s'), mods)));
        assert_eq!(*fired.borrow(), 1);
        // repeat 不触发快捷键
        assert!(!dispatch_key(
            &doc,
            &KeyEvent::new(Key::Char('s'), Mods::CTRL).with_repeat(true)
        ));
        assert_eq!(*fired.borrow(), 1);
        scope.dispose();
    }

    #[test]
    fn focused_handler_consumes_shortcut() {
        let fired = Rc::new(RefCell::new(false));
        let f = fired.clone();
        let (_, scope) = create_root(move || {
            register_shortcut(Shortcut::new(Mods::CTRL, Key::Char('s')), move || {
                *f.borrow_mut() = true;
            });
        });
        let doc = Doc::new();
        let btn = doc.create_button("保存");
        doc.append(doc.root(), btn);
        doc.set_on_key(btn, |e| e.prevent_default()); // 焦点 handler 消费一切
        doc.focus(btn);
        assert!(dispatch_key(
            &doc,
            &KeyEvent::new(Key::Char('s'), Mods::CTRL)
        ));
        assert!(!*fired.borrow(), "prevent_default 后快捷键段不应触发");
        scope.dispose();
    }

    #[test]
    fn scope_dispose_unregisters_shortcut() {
        let n0 = debug_count();
        let (_, scope) = create_root(|| {
            register_shortcut(Shortcut::new(Mods::CTRL, Key::Char('k')), || {});
            register_shortcut(Shortcut::new(Mods::NONE, Key::F(5)), || {});
        });
        assert_eq!(debug_count(), n0 + 2);
        scope.dispose();
        assert_eq!(debug_count(), n0, "作用域销毁应自动注销全部快捷键");
        assert!(!dispatch_shortcut(Shortcut::new(
            Mods::CTRL,
            Key::Char('k')
        )));
    }

    #[test]
    fn same_key_last_registered_wins() {
        let log: Rc<RefCell<Vec<&'static str>>> = Default::default();
        let sc = Shortcut::cmd_or_ctrl(Key::Char('w'));
        let l = log.clone();
        let (_, base) = create_root(move || {
            register_shortcut(sc, move || l.borrow_mut().push("底层"));
        });
        let l = log.clone();
        // 对话框压栈:后注册者覆盖,且只触发一个
        let (_, dialog) = create_root(move || {
            register_shortcut(sc, move || l.borrow_mut().push("对话框"));
        });
        assert!(dispatch_shortcut(sc));
        assert_eq!(*log.borrow(), vec!["对话框"], "后进先出、调用即止");
        dialog.dispose(); // 对话框关闭,底层快捷键恢复
        assert!(dispatch_shortcut(sc));
        assert_eq!(*log.borrow(), vec!["对话框", "底层"]);
        base.dispose();
    }
}
