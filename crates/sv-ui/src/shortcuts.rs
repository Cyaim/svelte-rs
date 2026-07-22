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

    /// 注册挂的是**当前响应式作用域**,所以 effect 重跑会先注销上一轮:
    /// 同一个 effect 反复注册不堆积,触发的永远是最新那版闭包。
    /// 防的退化:忘了在重跑路径上注销——分支切换后按一次快捷键,
    /// 跑的是捕获了旧状态的上一版闭包(最难查的一类"幽灵回调")
    #[test]
    fn effect_rerun_reregisters_without_stacking() {
        let n0 = debug_count();
        let dep = sv_reactive::state(0);
        let log: Rc<RefCell<Vec<i32>>> = Default::default();
        let l = log.clone();
        let sc = Shortcut::new(Mods::CTRL, Key::Char('r'));
        let (_, scope) = create_root(move || {
            sv_reactive::effect(move || {
                let v = dep.get();
                let l = l.clone();
                register_shortcut(sc, move || l.borrow_mut().push(v));
            });
        });
        assert_eq!(debug_count(), n0 + 1);
        dep.set(1);
        assert_eq!(debug_count(), n0 + 1, "effect 重跑不该堆积注册");
        assert!(dispatch_shortcut(sc));
        assert_eq!(*log.borrow(), vec![1], "触发的应是最新一版闭包");
        scope.dispose();
        assert_eq!(debug_count(), n0);
    }

    /// 回调里再碰注册表(打开对话框顺手注册自己的快捷键、或转派另一个键)
    /// 必须安全:dispatch 先把 `Rc` clone 出借用再调。
    /// 防的退化:直接在 `borrow()` 里调回调 —— 上述场景当场 BorrowMutError
    #[test]
    fn callback_may_touch_registry_reentrantly() {
        let n0 = debug_count();
        let outer = Shortcut::new(Mods::CTRL, Key::Char('o'));
        let inner = Shortcut::new(Mods::CTRL, Key::Char('i'));
        let esc = Shortcut::new(Mods::NONE, Key::Escape);
        let log: Rc<RefCell<Vec<&'static str>>> = Default::default();
        let dialog: Rc<RefCell<Option<sv_reactive::RootHandle>>> = Default::default();
        let (l, d) = (log.clone(), dialog.clone());
        let (_, scope) = create_root(move || {
            let li = l.clone();
            register_shortcut(inner, move || li.borrow_mut().push("内层"));
            register_shortcut(outer, move || {
                l.borrow_mut().push("外层");
                // 模拟"回调里弹对话框":新作用域里注册自己的快捷键
                let (_, s) = create_root(|| register_shortcut(esc, || {}));
                *d.borrow_mut() = Some(s);
                dispatch_shortcut(inner); // 回调里再派发别的快捷键
            });
        });
        assert!(dispatch_shortcut(outer));
        assert_eq!(*log.borrow(), vec!["外层", "内层"]);
        assert_eq!(debug_count(), n0 + 3);
        dialog.borrow_mut().take().unwrap().dispose();
        scope.dispose();
        assert_eq!(debug_count(), n0);
    }

    /// 同一作用域重复注册同一个键:两条都在表里,派发只走最后一条,
    /// 作用域销毁要把**两条一起**摘掉。防的退化:注销时只 retain 掉一条
    /// (或按键整条删),开关反复的组件会残留幽灵注册
    #[test]
    fn duplicate_registration_in_same_scope_unwinds_completely() {
        let n0 = debug_count();
        let sc = Shortcut::new(Mods::ALT, Key::Char('d'));
        let log: Rc<RefCell<Vec<&'static str>>> = Default::default();
        let (l1, l2) = (log.clone(), log.clone());
        let (_, scope) = create_root(move || {
            register_shortcut(sc, move || l1.borrow_mut().push("先"));
            register_shortcut(sc, move || l2.borrow_mut().push("后"));
        });
        assert_eq!(debug_count(), n0 + 2, "两条注册都该在表里");
        assert!(dispatch_shortcut(sc));
        assert_eq!(*log.borrow(), vec!["后"], "只调最后注册的一条");
        scope.dispose();
        assert_eq!(debug_count(), n0, "销毁要摘掉同键的全部注册");
        assert!(!dispatch_shortcut(sc));
    }

    /// 精确匹配是**双向**的:注册无修饰的 F5,按 Ctrl+F5 不该触发;
    /// 注册 Ctrl+W,按 Cmd+W 或裸 W 都不该触发。
    /// 防的退化:改成"包含即匹配"的宽松比较(egui matches_logically 的坑),
    /// 那样 Ctrl+Shift+S 会被 Ctrl+S 的注册者吃掉
    #[test]
    fn modifier_match_is_exact_in_both_directions() {
        let hits = Rc::new(std::cell::Cell::new(0));
        let (h1, h2) = (hits.clone(), hits.clone());
        let f5 = Shortcut::new(Mods::NONE, Key::F(5));
        let ctrl_w = Shortcut::new(Mods::CTRL, Key::Char('w'));
        let (_, scope) = create_root(move || {
            register_shortcut(f5, move || h1.set(h1.get() + 1));
            register_shortcut(ctrl_w, move || h2.set(h2.get() + 1));
        });
        assert!(
            !dispatch_shortcut(Shortcut::new(Mods::CTRL, Key::F(5))),
            "多带修饰键不该匹配无修饰的注册"
        );
        assert!(
            !dispatch_shortcut(Shortcut::new(Mods::META, Key::Char('w'))),
            "Cmd 与 Ctrl 不是一回事"
        );
        assert!(
            !dispatch_shortcut(Shortcut::new(Mods::NONE, Key::Char('w'))),
            "少带修饰键同样不匹配"
        );
        assert_eq!(hits.get(), 0);
        assert!(dispatch_shortcut(f5));
        assert!(dispatch_shortcut(ctrl_w));
        assert_eq!(hits.get(), 2);
        scope.dispose();
    }
}
