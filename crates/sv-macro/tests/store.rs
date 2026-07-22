//! `#[derive(Store)]` 端到端:字段级信号的粒度承诺必须可观测
//! ——"改一个字段,只叫醒读这个字段的 effect"。

use std::cell::RefCell;
use std::rc::Rc;

use sv_macro::Store;
use sv_reactive::{create_root, effect};

#[derive(Store, Clone, PartialEq, Debug)]
struct Settings {
    theme: String,
    volume: f32,
    muted: bool,
}

fn sample() -> Settings {
    Settings {
        theme: "dark".into(),
        volume: 0.8,
        muted: false,
    }
}

#[test]
fn field_level_signals_wake_only_their_readers() {
    let (_, _scope) = create_root(|| {
        let s = sample().into_store();
        let theme_runs = Rc::new(RefCell::new(0));
        let volume_runs = Rc::new(RefCell::new(0));

        let t = theme_runs.clone();
        effect(move || {
            s.theme.get();
            *t.borrow_mut() += 1;
        });
        let v = volume_runs.clone();
        effect(move || {
            s.volume.get();
            *v.borrow_mut() += 1;
        });
        assert_eq!((*theme_runs.borrow(), *volume_runs.borrow()), (1, 1));

        // 改 volume:读 theme 的 effect **不该**醒 —— 整个 derive 就为这一行
        s.volume.set(0.5);
        assert_eq!(
            (*theme_runs.borrow(), *volume_runs.borrow()),
            (1, 2),
            "字段级信号:改 volume 不该叫醒 theme 的读者"
        );

        s.theme.set("light".into());
        assert_eq!((*theme_runs.borrow(), *volume_runs.borrow()), (2, 2));
    });
}

#[test]
fn snapshot_roundtrips_and_apply_prunes_unchanged_fields() {
    let (_, _scope) = create_root(|| {
        let s = sample().into_store();
        assert_eq!(s.snapshot(), sample(), "快照应还原整值");

        let theme_runs = Rc::new(RefCell::new(0));
        let t = theme_runs.clone();
        effect(move || {
            s.theme.get();
            *t.borrow_mut() += 1;
        });
        let muted_runs = Rc::new(RefCell::new(0));
        let m = muted_runs.clone();
        effect(move || {
            s.muted.get();
            *m.borrow_mut() += 1;
        });

        // 整体写回:只有 muted 变了 → 只有 muted 的读者被叫醒
        s.apply(Settings {
            muted: true,
            ..sample()
        });
        assert_eq!(*theme_runs.borrow(), 1, "apply 不该写没变的字段");
        assert_eq!(*muted_runs.borrow(), 2);
        assert!(s.muted.get());

        // 完全相同的整值:一个字段都不写
        s.apply(s.snapshot());
        assert_eq!((*theme_runs.borrow(), *muted_runs.borrow()), (1, 2));
    });
}

/// store 句柄是 `Copy`(与 Signal 一致),可以随手塞进闭包
#[test]
fn store_handle_is_copy() {
    let (_, _scope) = create_root(|| {
        let s = sample().into_store();
        let f = move || s.volume.get();
        let g = move || s.theme.get();
        assert_eq!(f(), 0.8);
        assert_eq!(g(), "dark");
        assert_eq!(s.snapshot().volume, 0.8);
    });
}
