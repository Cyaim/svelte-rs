//! # sv-reactive
//!
//! Svelte 5 runes 风格的细粒度响应式内核(原型)。
//!
//! 对应关系:
//! - `$state`   → [`state`] / [`Signal`]
//! - `$derived` → [`derived`] / [`Derived`](惰性求值 + 值相等剪枝)
//! - `$effect`  → [`effect`](自动追踪依赖、重跑前自动清理子作用域)
//!
//! ## 模型
//!
//! 所有响应式节点存放在 **thread-local** 的 `Runtime` arena(slotmap)里,
//! [`Signal`]/[`Derived`] 只是 `Copy` 的世代句柄,可以随意塞进闭包——这是在
//! Rust 借用检查下做响应式图的标准解法(Leptos/Sycamore 同款)。
//!
//! 调度采用 push-pull 三态脏标记(`Clean`/`Check`/`Dirty`,同 Svelte 5 /
//! reactively):写入 signal 时只做标记(push),effect 统一在 flush 里跑,
//! derived 被读到时才真正重算(pull),菱形依赖不会产生 glitch 或重复执行。
//!
//! ## 约束
//!
//! - **单线程**:句柄不可跨线程(`!Send`)。UI 场景下其他线程通过消息回主线程改状态。
//! - derived 计算过程中禁止写 state(等价于 Svelte 的 `state_unsafe_mutation` 错误)。
//! - `with` 回调执行期间对**同一个**节点的重入读取会 panic(读其它节点没问题)。

use std::any::Any;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;

use slotmap::{SlotMap, new_key_type};

new_key_type! {
    struct NodeId;
}

const MAX_FLUSH_PASSES: usize = 1000;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Dirtiness {
    Clean,
    /// 上游 derived 可能变了,需要 pull 确认
    Check,
    /// 确定需要重算/重跑
    Dirty,
}

enum NodeKind {
    Signal,
    Derived {
        f: Rc<dyn Fn() -> Box<dyn Any>>,
        eq: fn(&dyn Any, &dyn Any) -> bool,
    },
    Effect {
        f: Rc<RefCell<dyn FnMut()>>,
    },
    /// 纯所有权作用域(create_root),只负责统一销毁
    Root,
}

struct Node {
    kind: NodeKind,
    /// Signal / Derived 的当前值;Effect / Root 恒为 None
    value: Option<Box<dyn Any>>,
    state: Dirtiness,
    /// 我依赖谁(Derived/Effect)
    sources: Vec<NodeId>,
    /// 谁依赖我(Signal/Derived)
    subscribers: Vec<NodeId>,
    /// 运行期间创建的子节点,重跑/销毁时级联清理
    children: Vec<NodeId>,
    cleanups: Vec<Box<dyn FnOnce()>>,
}

#[derive(Default)]
struct Runtime {
    nodes: SlotMap<NodeId, Node>,
    /// 当前正在运行、需要收集依赖的 Derived/Effect
    observer: Option<NodeId>,
    /// 当前所有权作用域,新节点挂到它名下
    owner: Option<NodeId>,
    queue: Vec<NodeId>,
    batch_depth: usize,
    flushing: bool,
}

thread_local! {
    static RT: RefCell<Runtime> = RefCell::new(Runtime::default());
}

// ---------------------------------------------------------------------------
// 内部机制
// ---------------------------------------------------------------------------

fn create_node(
    rtc: &RefCell<Runtime>,
    kind: NodeKind,
    value: Option<Box<dyn Any>>,
    state: Dirtiness,
) -> NodeId {
    let mut rt = rtc.borrow_mut();
    let owner = rt.owner;
    let id = rt.nodes.insert(Node {
        kind,
        value,
        state,
        sources: Vec::new(),
        subscribers: Vec::new(),
        children: Vec::new(),
        cleanups: Vec::new(),
    });
    if let Some(o) = owner
        && let Some(n) = rt.nodes.get_mut(o)
    {
        n.children.push(id);
    }
    id
}

/// 把 `id` 登记为当前 observer 的依赖
fn track(rtc: &RefCell<Runtime>, id: NodeId) {
    let mut rt = rtc.borrow_mut();
    let Some(obs) = rt.observer else { return };
    if obs == id {
        return;
    }
    match rt.nodes.get_mut(id) {
        Some(n) => {
            if !n.subscribers.contains(&obs) {
                n.subscribers.push(obs);
            }
        }
        None => return,
    }
    if let Some(n) = rt.nodes.get_mut(obs)
        && !n.sources.contains(&id)
    {
        n.sources.push(id);
    }
}

/// push 阶段:向下游传播脏标记。直接订阅者标 `level`,更下游标 `Check`
fn mark(rt: &mut Runtime, id: NodeId, level: Dirtiness) {
    let (was_clean, is_effect) = {
        let Some(node) = rt.nodes.get_mut(id) else {
            return;
        };
        if node.state >= level {
            return;
        }
        let was_clean = node.state == Dirtiness::Clean;
        node.state = level;
        (was_clean, matches!(node.kind, NodeKind::Effect { .. }))
    };
    if is_effect {
        if was_clean {
            rt.queue.push(id);
        }
    } else if was_clean {
        let subs = rt.nodes[id].subscribers.clone();
        for s in subs {
            mark(rt, s, Dirtiness::Check);
        }
    }
}

/// Signal 写入后的通知入口
fn notify(rtc: &RefCell<Runtime>, id: NodeId) {
    {
        let mut rt = rtc.borrow_mut();
        let subs = match rt.nodes.get(id) {
            Some(n) => n.subscribers.clone(),
            None => return,
        };
        for s in subs {
            mark(&mut rt, s, Dirtiness::Dirty);
        }
    }
    maybe_flush(rtc);
}

fn assert_writable(rtc: &RefCell<Runtime>) {
    let rt = rtc.borrow();
    if let Some(obs) = rt.observer
        && let Some(n) = rt.nodes.get(obs)
        && matches!(n.kind, NodeKind::Derived { .. })
    {
        drop(rt);
        panic!("sv-reactive: 不允许在 derived 计算过程中写入 state(对应 Svelte 的 state_unsafe_mutation)");
    }
}

fn maybe_flush(rtc: &RefCell<Runtime>) {
    let should = {
        let rt = rtc.borrow();
        rt.batch_depth == 0 && !rt.flushing
    };
    if should {
        flush(rtc);
    }
}

fn flush(rtc: &RefCell<Runtime>) {
    struct Unflag<'a>(&'a RefCell<Runtime>);
    impl Drop for Unflag<'_> {
        fn drop(&mut self) {
            self.0.borrow_mut().flushing = false;
        }
    }
    rtc.borrow_mut().flushing = true;
    let _g = Unflag(rtc);

    let mut passes = 0usize;
    loop {
        let batch = std::mem::take(&mut rtc.borrow_mut().queue);
        if batch.is_empty() {
            break;
        }
        passes += 1;
        assert!(
            passes <= MAX_FLUSH_PASSES,
            "sv-reactive: effect 更新超过 {MAX_FLUSH_PASSES} 轮仍未收敛,疑似在 effect 里循环写入 state"
        );
        for id in batch {
            update_if_necessary(rtc, id);
        }
    }
}

/// pull 阶段:确认 `id`(Derived/Effect)是否真的需要重算,需要则执行
fn update_if_necessary(rtc: &RefCell<Runtime>, id: NodeId) {
    let state = match rtc.borrow().nodes.get(id) {
        None => return,
        Some(n) => n.state,
    };
    if state == Dirtiness::Check {
        // 逐个把上游 derived 拉到最新;若其中某个真的变了,会把我标成 Dirty
        let sources = rtc.borrow().nodes[id].sources.clone();
        for s in sources {
            let src_is_derived = {
                let rt = rtc.borrow();
                matches!(rt.nodes.get(s).map(|n| &n.kind), Some(NodeKind::Derived { .. }))
            };
            if src_is_derived {
                update_if_necessary(rtc, s);
            }
            match rtc.borrow().nodes.get(id) {
                None => return,
                Some(n) if n.state == Dirtiness::Dirty => break,
                _ => {}
            }
        }
    }
    let state = match rtc.borrow().nodes.get(id) {
        None => return,
        Some(n) => n.state,
    };
    if state == Dirtiness::Dirty {
        run_node(rtc, id);
    } else if let Some(n) = rtc.borrow_mut().nodes.get_mut(id) {
        // 上游实际没变,虚惊一场
        n.state = Dirtiness::Clean;
    }
}

/// 重跑前清理:级联销毁子节点、执行 cleanup、退订旧依赖(节点本身保留)
fn cleanup_node(rtc: &RefCell<Runtime>, id: NodeId) {
    let (cleanups, children, sources) = {
        let mut rt = rtc.borrow_mut();
        let Some(n) = rt.nodes.get_mut(id) else {
            return;
        };
        (
            std::mem::take(&mut n.cleanups),
            std::mem::take(&mut n.children),
            std::mem::take(&mut n.sources),
        )
    };
    {
        let mut rt = rtc.borrow_mut();
        for s in sources {
            if let Some(sn) = rt.nodes.get_mut(s) {
                sn.subscribers.retain(|x| *x != id);
            }
        }
    }
    for c in children {
        dispose_node(rtc, c);
    }
    // 用户回调在 RefCell 未借用时执行
    for c in cleanups {
        c();
    }
}

fn dispose_node(rtc: &RefCell<Runtime>, id: NodeId) {
    cleanup_node(rtc, id);
    let mut rt = rtc.borrow_mut();
    if let Some(n) = rt.nodes.get(id) {
        let subs = n.subscribers.clone();
        for s in subs {
            if let Some(sn) = rt.nodes.get_mut(s) {
                sn.sources.retain(|x| *x != id);
            }
        }
    }
    rt.nodes.remove(id);
    rt.queue.retain(|x| *x != id);
}

/// 真正执行 Derived 重算 / Effect 重跑
fn run_node(rtc: &RefCell<Runtime>, id: NodeId) {
    cleanup_node(rtc, id);

    enum Job {
        Derived(Rc<dyn Fn() -> Box<dyn Any>>, fn(&dyn Any, &dyn Any) -> bool),
        Effect(Rc<RefCell<dyn FnMut()>>),
    }
    let job = {
        let mut rt = rtc.borrow_mut();
        match rt.nodes.get_mut(id) {
            None => None,
            Some(node) => {
                node.state = Dirtiness::Clean;
                match &node.kind {
                    NodeKind::Derived { f, eq } => Some(Job::Derived(f.clone(), *eq)),
                    NodeKind::Effect { f } => Some(Job::Effect(f.clone())),
                    _ => None,
                }
            }
        }
    };
    let Some(job) = job else { return };

    let (prev_obs, prev_owner) = {
        let mut rt = rtc.borrow_mut();
        (rt.observer.replace(id), rt.owner.replace(id))
    };
    struct Restore<'a> {
        rtc: &'a RefCell<Runtime>,
        obs: Option<NodeId>,
        owner: Option<NodeId>,
    }
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            let mut rt = self.rtc.borrow_mut();
            rt.observer = self.obs;
            rt.owner = self.owner;
        }
    }
    let _g = Restore { rtc, obs: prev_obs, owner: prev_owner };

    // 以下用户闭包均在 RefCell 未借用状态下执行
    match job {
        Job::Derived(f, eq) => {
            let new_value = f();
            let changed = {
                let mut rt = rtc.borrow_mut();
                match rt.nodes.get_mut(id) {
                    None => None,
                    Some(node) => {
                        let changed = match &node.value {
                            Some(old) => !eq(old.as_ref(), new_value.as_ref()),
                            None => true,
                        };
                        node.value = Some(new_value);
                        Some(changed)
                    }
                }
            };
            if changed == Some(true) {
                let mut rt = rtc.borrow_mut();
                let subs = match rt.nodes.get(id) {
                    Some(n) => n.subscribers.clone(),
                    None => Vec::new(),
                };
                for s in subs {
                    mark(&mut rt, s, Dirtiness::Dirty);
                }
            }
        }
        Job::Effect(f) => {
            let mut fb = f
                .try_borrow_mut()
                .expect("sv-reactive: effect 重入执行(effect 内同步触发了自身重跑)");
            fb();
        }
    }
}

fn any_eq<T: PartialEq + 'static>(a: &dyn Any, b: &dyn Any) -> bool {
    match (a.downcast_ref::<T>(), b.downcast_ref::<T>()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// 偷值读取:执行用户闭包时不持有 RefCell 借用,闭包里可以自由访问其它响应式值
fn with_value<T: 'static, R>(
    rtc: &RefCell<Runtime>,
    id: NodeId,
    what: &str,
    f: impl FnOnce(&T) -> R,
) -> R {
    let boxed = {
        let mut rt = rtc.borrow_mut();
        let node = rt
            .nodes
            .get_mut(id)
            .unwrap_or_else(|| panic!("sv-reactive: {what} 已随作用域销毁,不能再访问"));
        node.value
            .take()
            .unwrap_or_else(|| panic!("sv-reactive: 检测到对同一个 {what} 的重入读取"))
    };
    let r = f(boxed
        .downcast_ref::<T>()
        .expect("sv-reactive: 内部错误——值类型不匹配"));
    let mut rt = rtc.borrow_mut();
    if let Some(node) = rt.nodes.get_mut(id)
        && node.value.is_none()
    {
        node.value = Some(boxed);
    }
    r
}

// ---------------------------------------------------------------------------
// 公开 API
// ---------------------------------------------------------------------------

/// `$state`:创建一个响应式状态,返回 `Copy` 句柄
pub fn state<T: 'static>(value: T) -> Signal<T> {
    RT.with(|rtc| {
        let id = create_node(rtc, NodeKind::Signal, Some(Box::new(value)), Dirtiness::Clean);
        Signal { id, _t: PhantomData }
    })
}

/// `$derived`:惰性求值的派生值。重算后与旧值 `==` 相同时不惊动下游
pub fn derived<T: PartialEq + 'static>(f: impl Fn() -> T + 'static) -> Derived<T> {
    RT.with(|rtc| {
        let f: Rc<dyn Fn() -> Box<dyn Any>> = Rc::new(move || Box::new(f()) as Box<dyn Any>);
        let id = create_node(
            rtc,
            NodeKind::Derived { f, eq: any_eq::<T> },
            None,
            Dirtiness::Dirty,
        );
        Derived { id, _t: PhantomData }
    })
}

/// `$effect`:立即执行一次并自动追踪依赖,依赖变化后自动重跑。
/// 重跑前会销毁上次运行创建的子节点并执行 [`on_cleanup`] 注册的回调。
///
/// 与 Svelte 的差异:Svelte 把 effect 推迟到微任务,这里为桌面场景选择
/// **创建时同步首跑**;首跑视作一次原子刷新,期间写入的 state 在首跑结束后统一 flush。
pub fn effect(f: impl FnMut() + 'static) -> EffectHandle {
    RT.with(|rtc| {
        let f: Rc<RefCell<dyn FnMut()>> = Rc::new(RefCell::new(f));
        let id = create_node(rtc, NodeKind::Effect { f }, None, Dirtiness::Clean);
        let was_flushing = {
            let mut rt = rtc.borrow_mut();
            std::mem::replace(&mut rt.flushing, true)
        };
        run_node(rtc, id);
        rtc.borrow_mut().flushing = was_flushing;
        maybe_flush(rtc);
        EffectHandle { id }
    })
}

/// 批量写入:回调内的所有 set 只在回调结束后触发一轮 effect
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    RT.with(|rtc| {
        rtc.borrow_mut().batch_depth += 1;
        struct G<'a>(&'a RefCell<Runtime>);
        impl Drop for G<'_> {
            fn drop(&mut self) {
                self.0.borrow_mut().batch_depth -= 1;
            }
        }
        let r = {
            let _g = G(rtc);
            f()
        };
        maybe_flush(rtc);
        r
    })
}

/// `untrack`:回调内的读取不建立依赖
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    RT.with(|rtc| {
        let prev = rtc.borrow_mut().observer.take();
        struct G<'a>(&'a RefCell<Runtime>, Option<NodeId>);
        impl Drop for G<'_> {
            fn drop(&mut self) {
                self.0.borrow_mut().observer = self.1;
            }
        }
        let _g = G(rtc, prev);
        f()
    })
}

/// 在当前作用域(effect/root)注册清理回调,重跑或销毁前执行
pub fn on_cleanup(f: impl FnOnce() + 'static) {
    RT.with(|rtc| {
        let mut rt = rtc.borrow_mut();
        if let Some(o) = rt.owner
            && let Some(n) = rt.nodes.get_mut(o)
        {
            n.cleanups.push(Box::new(f));
            return;
        }
        #[cfg(debug_assertions)]
        eprintln!("sv-reactive: on_cleanup 在响应式作用域外调用,永远不会执行");
    })
}

/// 创建一个所有权根作用域。回调内创建的所有节点都挂在这个根下,
/// 通过返回的 [`RootHandle::dispose`] 一次性销毁(组件卸载的基石)
pub fn create_root<R>(f: impl FnOnce() -> R) -> (R, RootHandle) {
    RT.with(|rtc| {
        let id = create_node(rtc, NodeKind::Root, None, Dirtiness::Clean);
        let prev = rtc.borrow_mut().owner.replace(id);
        struct G<'a>(&'a RefCell<Runtime>, Option<NodeId>);
        impl Drop for G<'_> {
            fn drop(&mut self) {
                self.0.borrow_mut().owner = self.1;
            }
        }
        let r = {
            let _g = G(rtc, prev);
            f()
        };
        (r, RootHandle { id })
    })
}

/// 当前线程 runtime 里的节点总数(测试/调试用)
#[doc(hidden)]
pub fn debug_node_count() -> usize {
    RT.with(|rtc| rtc.borrow().nodes.len())
}

/// `$state` 的句柄。`Copy`、`!Send`,可自由塞进闭包
pub struct Signal<T: 'static> {
    id: NodeId,
    _t: PhantomData<(fn() -> T, *const ())>,
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Signal<T> {}

// 句柄身份相等(不是值相等):让 Signal 能进集合、进 {#each} 的列表
impl<T> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T> Eq for Signal<T> {}
impl<T> std::hash::Hash for Signal<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static> Signal<T> {
    /// 读取(建立依赖)。需要 `T: Clone`;不想 clone 用 [`Signal::with`]
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    /// 借用读取(建立依赖),不 clone
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        RT.with(|rtc| {
            track(rtc, self.id);
            with_value(rtc, self.id, "signal", f)
        })
    }

    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        untrack(|| self.get())
    }

    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        untrack(|| self.with(f))
    }

    /// 写入并通知下游。注意:不做相等性检查,写同样的值也会触发
    pub fn set(&self, value: T) {
        RT.with(|rtc| {
            assert_writable(rtc);
            {
                let mut rt = rtc.borrow_mut();
                let node = rt
                    .nodes
                    .get_mut(self.id)
                    .expect("sv-reactive: signal 已随作用域销毁,不能再写入");
                node.value = Some(Box::new(value));
            }
            notify(rtc, self.id);
        })
    }

    /// 原地修改并通知下游
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        RT.with(|rtc| {
            assert_writable(rtc);
            let mut boxed = {
                let mut rt = rtc.borrow_mut();
                let node = rt
                    .nodes
                    .get_mut(self.id)
                    .expect("sv-reactive: signal 已随作用域销毁,不能再写入");
                node.value
                    .take()
                    .expect("sv-reactive: 检测到对同一个 signal 的重入访问")
            };
            f(boxed.downcast_mut::<T>().expect("sv-reactive: 内部错误——值类型不匹配"));
            {
                let mut rt = rtc.borrow_mut();
                if let Some(node) = rt.nodes.get_mut(self.id) {
                    node.value = Some(boxed);
                }
            }
            notify(rtc, self.id);
        })
    }
}

/// `$derived` 的句柄。`Copy`、`!Send`、只读
pub struct Derived<T: 'static> {
    id: NodeId,
    _t: PhantomData<(fn() -> T, *const ())>,
}

impl<T> Clone for Derived<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Derived<T> {}

impl<T> PartialEq for Derived<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T> Eq for Derived<T> {}
impl<T> std::hash::Hash for Derived<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static> Derived<T> {
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(T::clone)
    }

    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        RT.with(|rtc| {
            update_if_necessary(rtc, self.id); // 惰性:读到才算
            track(rtc, self.id);
            with_value(rtc, self.id, "derived", f)
        })
    }

    pub fn get_untracked(&self) -> T
    where
        T: Clone,
    {
        untrack(|| self.get())
    }
}

/// [`effect`] 返回的句柄。effect 的生命周期由所属作用域管理,
/// 该句柄仅用于提前手动销毁,丢弃句柄不影响 effect 运行
pub struct EffectHandle {
    id: NodeId,
}

impl EffectHandle {
    pub fn dispose(self) {
        RT.with(|rtc| dispose_node(rtc, self.id));
    }
}

/// [`create_root`] 返回的作用域句柄
pub struct RootHandle {
    id: NodeId,
}

impl RootHandle {
    pub fn dispose(self) {
        RT.with(|rtc| dispose_node(rtc, self.id));
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn signal_get_set() {
        let a = state(1);
        assert_eq!(a.get(), 1);
        a.set(2);
        assert_eq!(a.get(), 2);
        a.update(|v| *v += 10);
        assert_eq!(a.get(), 12);
        assert_eq!(a.with(|v| v * 2), 24);
    }

    #[test]
    fn effect_runs_and_reruns() {
        let count = state(0);
        let log: Rc<RefCell<Vec<i32>>> = Rc::default();
        let l = log.clone();
        effect(move || l.borrow_mut().push(count.get()));
        count.set(1);
        count.set(2);
        assert_eq!(*log.borrow(), vec![0, 1, 2]);
    }

    #[test]
    fn unrelated_signal_does_not_rerun() {
        let a = state(0);
        let b = state(0);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            a.get();
            *r.borrow_mut() += 1;
        });
        b.set(1);
        assert_eq!(*runs.borrow(), 1);
    }

    #[test]
    fn derived_is_lazy_and_cached() {
        let a = state(1);
        let computes = Rc::new(RefCell::new(0));
        let c = computes.clone();
        let d = derived(move || {
            *c.borrow_mut() += 1;
            a.get() * 2
        });
        assert_eq!(*computes.borrow(), 0, "未读取前不应计算");
        assert_eq!(d.get(), 2);
        assert_eq!(*computes.borrow(), 1);
        d.get();
        assert_eq!(*computes.borrow(), 1, "值未变,应走缓存");
        a.set(3);
        assert_eq!(*computes.borrow(), 1, "惰性:标脏但没人读就不算");
        assert_eq!(d.get(), 6);
        assert_eq!(*computes.borrow(), 2);
    }

    #[test]
    fn diamond_runs_effect_once() {
        let a = state(1);
        let b = derived(move || a.get() * 2);
        let c = derived(move || a.get() + 10);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            let _ = (b.get(), c.get());
            *r.borrow_mut() += 1;
        });
        assert_eq!(*runs.borrow(), 1);
        a.set(2);
        assert_eq!(*runs.borrow(), 2, "菱形依赖只应触发一次重跑");
    }

    #[test]
    fn derived_equality_cuts_propagation() {
        let a = state(1);
        let big = derived(move || a.get() > 10);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            big.get();
            *r.borrow_mut() += 1;
        });
        assert_eq!(*runs.borrow(), 1);
        a.set(5); // big 仍为 false
        assert_eq!(*runs.borrow(), 1, "derived 值未变,不应惊动下游");
        a.set(11);
        assert_eq!(*runs.borrow(), 2);
    }

    #[test]
    fn batch_coalesces() {
        let a = state(1);
        let b = state(2);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            let _ = a.get() + b.get();
            *r.borrow_mut() += 1;
        });
        batch(|| {
            a.set(10);
            b.set(20);
        });
        assert_eq!(*runs.borrow(), 2, "batch 内两次写入只应触发一轮");
    }

    #[test]
    fn untrack_does_not_subscribe() {
        let a = state(1);
        let b = state(2);
        let log: Rc<RefCell<Vec<i32>>> = Rc::default();
        let l = log.clone();
        effect(move || {
            let s = a.get() + untrack(|| b.get());
            l.borrow_mut().push(s);
        });
        b.set(100);
        assert_eq!(log.borrow().len(), 1, "untrack 读取不应建立依赖");
        a.set(5);
        assert_eq!(*log.borrow().last().unwrap(), 105, "重跑时应读到 b 的新值");
    }

    #[test]
    fn dynamic_dependencies() {
        let flag = state(true);
        let a = state(0);
        let b = state(0);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            if flag.get() {
                a.get();
            } else {
                b.get();
            }
            *r.borrow_mut() += 1;
        });
        b.set(1);
        assert_eq!(*runs.borrow(), 1, "分支未读 b,不应触发");
        flag.set(false);
        assert_eq!(*runs.borrow(), 2);
        a.set(1);
        assert_eq!(*runs.borrow(), 2, "换分支后旧依赖 a 应被退订");
        b.set(2);
        assert_eq!(*runs.borrow(), 3);
    }

    #[test]
    fn nested_effect_disposed_on_parent_rerun() {
        let outer_dep = state(0);
        let inner_dep = state(0);
        let inner_runs = Rc::new(RefCell::new(0));
        let ir = inner_runs.clone();
        effect(move || {
            outer_dep.get();
            let ir = ir.clone();
            effect(move || {
                inner_dep.get();
                *ir.borrow_mut() += 1;
            });
        });
        assert_eq!(*inner_runs.borrow(), 1);
        inner_dep.set(1);
        assert_eq!(*inner_runs.borrow(), 2);
        outer_dep.set(1); // 旧内层销毁,新内层创建并首跑
        assert_eq!(*inner_runs.borrow(), 3);
        inner_dep.set(2); // 只应有一个内层存活
        assert_eq!(*inner_runs.borrow(), 4, "旧内层 effect 未被销毁");
    }

    #[test]
    fn cleanup_runs_before_rerun() {
        let a = state(0);
        let cleanups = Rc::new(RefCell::new(0));
        let c = cleanups.clone();
        effect(move || {
            a.get();
            let c = c.clone();
            on_cleanup(move || *c.borrow_mut() += 1);
        });
        assert_eq!(*cleanups.borrow(), 0);
        a.set(1);
        assert_eq!(*cleanups.borrow(), 1);
        a.set(2);
        assert_eq!(*cleanups.borrow(), 2);
    }

    #[test]
    fn scope_owned_nodes_do_not_leak() {
        let a = state(0);
        effect(move || {
            a.get();
            let _tmp = state(42); // 每次重跑创建的临时节点应随重跑回收
        });
        let n1 = debug_node_count();
        a.set(1);
        a.set(2);
        assert_eq!(debug_node_count(), n1, "effect 重跑创建的节点应被回收");
    }

    #[test]
    fn root_dispose_stops_effects() {
        let a = state(0);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        let (_, root) = create_root(move || {
            let r = r.clone();
            effect(move || {
                a.get();
                *r.borrow_mut() += 1;
            });
        });
        a.set(1);
        assert_eq!(*runs.borrow(), 2);
        root.dispose();
        a.set(2);
        assert_eq!(*runs.borrow(), 2, "root 销毁后 effect 不应再跑");
    }

    #[test]
    fn set_inside_effect_converges() {
        let a = state(0);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            *r.borrow_mut() += 1;
            if a.get() < 3 {
                a.set(a.get_untracked() + 1);
            }
        });
        assert_eq!(a.get_untracked(), 3);
        assert_eq!(*runs.borrow(), 4);
    }

    #[test]
    #[should_panic(expected = "derived")]
    fn write_in_derived_panics() {
        let a = state(1);
        let b = state(0);
        let d = derived(move || {
            b.set(9);
            a.get()
        });
        d.get();
    }

    #[test]
    #[should_panic(expected = "仍未收敛")]
    fn infinite_loop_guard() {
        let a = state(0);
        effect(move || {
            a.set(a.get() + 1);
        });
    }

    #[test]
    fn derived_chain() {
        let a = state(1);
        let b = derived(move || a.get() + 1);
        let c = derived(move || b.get() * 10);
        assert_eq!(c.get(), 20);
        a.set(4);
        assert_eq!(c.get(), 50);
    }
}
