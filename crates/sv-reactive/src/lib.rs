//! # sv-reactive
//!
//! Svelte 5 runes 风格的细粒度响应式内核(原型)。
//!
//! 对应关系:
//! - `$state`   → [`state`] / [`Signal`]
//! - `$derived` → [`derived`] / [`Derived`](惰性求值 + 值相等剪枝;可写覆盖做乐观 UI)
//! - `$effect`  → [`effect`](自动追踪依赖、重跑前自动清理子作用域)
//! - `$effect.pre` → [`effect_pre`](同一轮 flush 内先于普通 effect)
//! - `$effect.tracking()` → [`is_tracking`]
//! - `$props.id()` → [`unique_id`]
//! - `tick` → [`tick`]
//! - `setContext` / `getContext` → [`provide_context`] / [`use_context`]
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

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
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

/// effect 的调度相位。拆相位而不是拆节点类型:除了 flush 里的执行顺序,
/// pre 与普通 effect 的其余行为(追踪、清理、销毁)完全一致
#[derive(Clone, Copy, PartialEq, Eq)]
enum EffectPhase {
    /// 对应 Svelte 的 `$effect.pre`("渲染前"),每轮批处理先跑
    Pre,
    Normal,
}

enum NodeKind {
    Signal,
    Derived {
        f: Rc<dyn Fn() -> Box<dyn Any>>,
        eq: fn(&dyn Any, &dyn Any) -> bool,
    },
    Effect {
        f: Rc<RefCell<dyn FnMut()>>,
        phase: EffectPhase,
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
    /// **创建时**的所有权作用域(children 的反向边)。context 沿这条链向上查:
    /// 记录创建时刻而非查找时刻,才能让 create_root 内的节点穿过 root 边界
    /// 取到外层 context(keyed each 行作用域的关键)
    owner: Option<NodeId>,
    /// provide_context 挂在本作用域的上下文,按类型索引。
    /// 惰性分配:绝大多数节点没有 context,别为它们付 HashMap 的开销
    contexts: Option<HashMap<TypeId, Rc<dyn Any>>>,
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
    /// 帧对齐模式(ADR-6):写入不再当场 flush,攒到帧前由渲染壳统一冲刷
    frame_scheduler: Option<Rc<dyn Fn()>>,
    /// 本轮是否已催过帧(避免一次事件里 N 次写入催 N 次重绘)
    frame_requested: bool,
    /// [`unique_id`] 的自增计数器(线程内单调)
    next_unique_id: u64,
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
        owner,
        contexts: None,
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
        panic!(
            "sv-reactive: 不允许在 derived 计算过程中写入 state(对应 Svelte 的 state_unsafe_mutation)"
        );
    }
}

/// 写入后的默认冲刷路径。帧对齐模式下**不 flush**,只催一帧
/// (ADR-6:事件 → batch 写入 → 帧前统一 flush → 布局 → 绘制)
fn maybe_flush(rtc: &RefCell<Runtime>) {
    let sched = {
        let rt = rtc.borrow();
        if rt.batch_depth != 0 || rt.flushing {
            return;
        }
        match &rt.frame_scheduler {
            None => None,
            Some(_) if rt.queue.is_empty() || rt.frame_requested => return,
            Some(f) => Some(f.clone()),
        }
    };
    match sched {
        // 帧对齐:标记已催帧,回调在**不持有 RT 借用**时调用
        // (渲染壳的回调可能反过来写 signal)
        Some(f) => {
            rtc.borrow_mut().frame_requested = true;
            f();
        }
        None => flush(rtc),
    }
}

/// 无视帧对齐,立刻冲刷(帧前调用 / [`tick`] 逃生舱)
fn force_flush(rtc: &RefCell<Runtime>) {
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
    {
        let mut rt = rtc.borrow_mut();
        rt.flushing = true;
        // 队列即将清空:下一次写入应重新催帧
        rt.frame_requested = false;
    }
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
        // 两阶段:每轮批处理里 pre effect 先于普通 effect(对应 Svelte 的
        // `$effect.pre` 在"渲染"前执行——本模型里普通 effect 承担渲染写入)。
        // 每轮单独分相而不是全局排序:pre 里再触发的写入照常进下一轮
        let (pre, normal): (Vec<NodeId>, Vec<NodeId>) = {
            let rt = rtc.borrow();
            batch.into_iter().partition(|id| {
                matches!(
                    rt.nodes.get(*id).map(|n| &n.kind),
                    Some(NodeKind::Effect {
                        phase: EffectPhase::Pre,
                        ..
                    })
                )
            })
        };
        for id in pre.into_iter().chain(normal) {
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
                matches!(
                    rt.nodes.get(s).map(|n| &n.kind),
                    Some(NodeKind::Derived { .. })
                )
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
    let (cleanups, children, sources, contexts) = {
        let mut rt = rtc.borrow_mut();
        let Some(n) = rt.nodes.get_mut(id) else {
            return;
        };
        (
            std::mem::take(&mut n.cleanups),
            std::mem::take(&mut n.children),
            std::mem::take(&mut n.sources),
            // 重跑会重新执行 provide_context,上一轮挂的 context 一并清掉,
            // 否则本轮没再 provide 时后代会读到陈旧值
            n.contexts.take(),
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
    // context 值可能带用户 Drop 逻辑,同样在借用释放后丢弃
    drop(contexts);
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
                    NodeKind::Effect { f, .. } => Some(Job::Effect(f.clone())),
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
    let _g = Restore {
        rtc,
        obs: prev_obs,
        owner: prev_owner,
    };

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
        let id = create_node(
            rtc,
            NodeKind::Signal,
            Some(Box::new(value)),
            Dirtiness::Clean,
        );
        Signal {
            id,
            _t: PhantomData,
        }
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
        Derived {
            id,
            _t: PhantomData,
        }
    })
}

/// `$effect`:立即执行一次并自动追踪依赖,依赖变化后自动重跑。
/// 重跑前会销毁上次运行创建的子节点并执行 [`on_cleanup`] 注册的回调。
///
/// 与 Svelte 的差异:Svelte 把 effect 推迟到微任务,这里为桌面场景选择
/// **创建时同步首跑**;首跑视作一次原子刷新,期间写入的 state 在首跑结束后统一 flush。
pub fn effect(f: impl FnMut() + 'static) -> EffectHandle {
    create_effect(f, EffectPhase::Normal)
}

/// `$effect.pre`:pre 相位的 effect。与 [`effect`] 的唯一差别是调度顺序:
/// 同一轮 flush 里所有 pre effect 先于普通 effect 执行(Svelte 里普通 effect
/// 承担"渲染"写入,pre 用于在渲染前读取旧布局/滚动位置等)。创建时同样同步首跑
pub fn effect_pre(f: impl FnMut() + 'static) -> EffectHandle {
    create_effect(f, EffectPhase::Pre)
}

fn create_effect(f: impl FnMut() + 'static, phase: EffectPhase) -> EffectHandle {
    RT.with(|rtc| {
        let f: Rc<RefCell<dyn FnMut()>> = Rc::new(RefCell::new(f));
        let id = create_node(rtc, NodeKind::Effect { f, phase }, None, Dirtiness::Clean);
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

/// `$effect.tracking()`:当前是否处于依赖追踪上下文
/// (effect 重跑或 derived 重算期间为 true;顶层与 [`untrack`] 内为 false)
pub fn is_tracking() -> bool {
    RT.with(|rtc| rtc.borrow().observer.is_some())
}

/// `$props.id()`:生成线程内唯一的自增 id("sv-1"、"sv-2"…),
/// 供需要稳定唯一标识(无障碍属性、label 关联等)的场景使用
pub fn unique_id() -> String {
    RT.with(|rtc| {
        let mut rt = rtc.borrow_mut();
        rt.next_unique_id += 1;
        format!("sv-{}", rt.next_unique_id)
    })
}

/// `tick`:立即冲刷待决 effect —— 帧对齐模式下的**逃生舱**
/// (ADR-6:写入攒到帧前才生效,需要"现在就要看到结果"时调它)。
/// 非帧对齐模式下写入本就同步 flush,此函数是 API 对齐;
/// batch 内调用恒为 no-op(不破坏批处理原子性),batch 结束照常统一 flush
pub fn tick() {
    RT.with(force_flush);
}

// ---------------------------------------------------------------------------
// 帧调度(ADR-6)
// ---------------------------------------------------------------------------

/// 开启**帧对齐**:此后写入 signal 不再当场跑 effect,而是入队并调用
/// `f`(渲染壳把它接到 `request_redraw`),由渲染壳在帧前调用 [`tick`] 统一冲刷。
///
/// 为什么要对齐:一次输入事件里连写 10 个 state,同步模型会跑 10 轮 effect、
/// 改 10 次场景树;对齐后只在帧前跑一轮,且 effect 写入与"布局 → 绘制"严格
/// 有序(Svelte 用 microtask flush 达成同一件事,桌面端的等价物是帧边界)。
///
/// **语义变化**:开启后,写完立刻读 derived / 查场景树看到的是**旧值**,
/// 直到下一帧或显式 [`tick`]。默认(如离屏测试)不开启,行为与过去一致。
pub fn set_frame_scheduler(f: impl Fn() + 'static) {
    RT.with(|rtc| {
        let mut rt = rtc.borrow_mut();
        rt.frame_scheduler = Some(Rc::new(f));
        rt.frame_requested = false;
    });
}

/// 关掉帧对齐,回到"写入即同步 flush"(离屏渲染/测试路径)
pub fn clear_frame_scheduler() {
    RT.with(|rtc| rtc.borrow_mut().frame_scheduler = None);
    tick();
}

/// 在**无所有者、无追踪**环境下执行 `f`:期间创建的节点不挂进任何作用域,
/// 永不随作用域销毁,期间的读取也不建立依赖。
/// 用途:线程级单例信号(如异步桥的在途计数)——它们可能在某个 effect 运行
/// 期间被惰性初始化,若不游离创建,会随那个 effect 的重跑被误销毁
pub fn detached<R>(f: impl FnOnce() -> R) -> R {
    RT.with(|rtc| {
        let (prev_owner, prev_obs) = {
            let mut rt = rtc.borrow_mut();
            (rt.owner.take(), rt.observer.take())
        };
        struct G<'a>(&'a RefCell<Runtime>, Option<NodeId>, Option<NodeId>);
        impl Drop for G<'_> {
            fn drop(&mut self) {
                let mut rt = self.0.borrow_mut();
                rt.owner = self.1;
                rt.observer = self.2;
            }
        }
        let _g = G(rtc, prev_owner, prev_obs);
        f()
    })
}

/// `setContext`:把一份上下文按类型挂到**当前所有权作用域**(effect/root)上,
/// 后代作用域用 [`use_context`] 读取。同一作用域重复 provide 同类型会覆盖;
/// 作用域销毁/重跑时上下文一并清理
pub fn provide_context<T: 'static>(value: T) {
    RT.with(|rtc| {
        let mut rt = rtc.borrow_mut();
        if let Some(o) = rt.owner
            && let Some(n) = rt.nodes.get_mut(o)
        {
            n.contexts
                .get_or_insert_with(HashMap::new)
                .insert(TypeId::of::<T>(), Rc::new(value) as Rc<dyn Any>);
            return;
        }
        #[cfg(debug_assertions)]
        eprintln!("sv-reactive: provide_context 在响应式作用域外调用,不会有任何效果");
    })
}

/// `getContext`:从当前作用域沿 owner 链向上找**最近**一层提供的 `T`,
/// 找不到返回 `None`。owner 记录的是节点创建时刻的作用域,所以查找能穿过
/// create_root 边界——keyed each 的行作用域也能取到组件层的 context
pub fn use_context<T: 'static>() -> Option<Rc<T>> {
    RT.with(|rtc| {
        let rt = rtc.borrow();
        let mut cur = rt.owner;
        while let Some(id) = cur {
            let node = rt.nodes.get(id)?;
            if let Some(map) = &node.contexts
                && let Some(v) = map.get(&TypeId::of::<T>())
            {
                // clone 的是 Rc(引用计数),不触碰用户值,借用期间安全
                return v.clone().downcast::<T>().ok();
            }
            cur = node.owner;
        }
        None
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

/// 在 `root` 作用域**之下**执行 `f`:期间创建的节点挂到它名下,而不是当前 owner。
///
/// 为什么需要它:`create_root` 挂在**当前** owner 下,所以在 effect 内部建的
/// 作用域会成为该 effect 的子节点 —— effect 重跑先销毁子树,那个作用域连同
/// 里面的 signal/effect 一起没了。keyed each 的行必须活过列表 effect 的重跑,
/// 又要保住 context 沿 owner 链的可达性(`detached` 会把链整个断掉),
/// 于是:预先在调用方作用域里建一个宿主 root,行统统挂它名下。
pub fn with_owner<R>(root: &RootHandle, f: impl FnOnce() -> R) -> R {
    RT.with(|rtc| {
        let prev = rtc.borrow_mut().owner.replace(root.id);
        struct G<'a>(&'a RefCell<Runtime>, Option<NodeId>);
        impl Drop for G<'_> {
            fn drop(&mut self) {
                self.0.borrow_mut().owner = self.1;
            }
        }
        let _g = G(rtc, prev);
        f()
    })
}

/// 当前线程 runtime 里的节点总数(测试/调试用)
#[doc(hidden)]
pub fn debug_node_count() -> usize {
    RT.with(|rtc| rtc.borrow().nodes.len())
}

/// 句柄的类型标记:`fn() -> T` 让 `T` 协变且不牵动自动 trait,
/// `*const ()` 关掉 `Send`/`Sync`(ADR-1:响应式图是单线程模型)
type HandleMarker<T> = PhantomData<(fn() -> T, *const ())>;

/// `$state` 的句柄。`Copy`、`!Send`,可自由塞进闭包
pub struct Signal<T: 'static> {
    id: NodeId,
    _t: HandleMarker<T>,
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
            f(boxed
                .downcast_mut::<T>()
                .expect("sv-reactive: 内部错误——值类型不匹配"));
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

/// `$derived` 的句柄。`Copy`、`!Send`;平时只读,
/// 可用 [`Derived::set`]/[`Derived::update`] 临时覆盖(乐观 UI)
pub struct Derived<T: 'static> {
    id: NodeId,
    _t: HandleMarker<T>,
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

    /// **writable derived**(Svelte 5.25 的乐观 UI):手动覆盖派生值并通知下游,
    /// 与当前派生值相等时剪枝(不惊动下游)。覆盖不是永久的:**任一依赖变化后**,
    /// 现有的 mark → 重算机制会用重算结果盖回来,自动回退——这里只需写入值、
    /// 清脏标记、mark 下游 Dirty,不需要额外状态。
    ///
    /// 在 derived 计算过程中调用仍会 panic(`assert_writable` 保护照旧);
    /// 允许的是"从外部"写 derived。
    pub fn set(&self, value: T) {
        RT.with(|rtc| {
            assert_writable(rtc);
            // 先拉到最新:一是让相等剪枝对着**最新**派生值比较,二是确保依赖边
            // 已建立——从未计算过的 derived 没有 sources,依赖变化收不到标记,
            // 覆盖值将永远不回退
            update_if_necessary(rtc, self.id);
            let changed = {
                let mut rt = rtc.borrow_mut();
                let node = rt
                    .nodes
                    .get_mut(self.id)
                    .expect("sv-reactive: derived 已随作用域销毁,不能再写入");
                let eq = match &node.kind {
                    NodeKind::Derived { eq, .. } => *eq,
                    _ => unreachable!("sv-reactive: 内部错误——Derived 句柄指向非 derived 节点"),
                };
                let new_value: Box<dyn Any> = Box::new(value);
                let changed = match &node.value {
                    Some(old) => !eq(old.as_ref(), new_value.as_ref()),
                    None => true,
                };
                node.value = Some(new_value);
                // 覆盖后视为最新:清掉脏标记,后续读取返回覆盖值而不是触发重算
                node.state = Dirtiness::Clean;
                changed
            };
            if changed {
                notify(rtc, self.id);
            }
        })
    }

    /// 在**最新派生值**的基础上原地修改覆盖值。与 [`Signal::update`] 一致
    /// 不做相等剪枝:不 clone 拿不到旧值副本,无从比较
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        RT.with(|rtc| {
            assert_writable(rtc);
            // 同 set:先算出最新值再就地修改(否则可能在陈旧值上改)
            update_if_necessary(rtc, self.id);
            let mut boxed = {
                let mut rt = rtc.borrow_mut();
                let node = rt
                    .nodes
                    .get_mut(self.id)
                    .expect("sv-reactive: derived 已随作用域销毁,不能再写入");
                node.value
                    .take()
                    .expect("sv-reactive: 检测到对同一个 derived 的重入访问")
            };
            f(boxed
                .downcast_mut::<T>()
                .expect("sv-reactive: 内部错误——值类型不匹配"));
            {
                let mut rt = rtc.borrow_mut();
                if let Some(node) = rt.nodes.get_mut(self.id) {
                    node.value = Some(boxed);
                    node.state = Dirtiness::Clean;
                }
            }
            notify(rtc, self.id);
        })
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

    // -- writable derived ---------------------------------------------------

    #[test]
    fn writable_derived_override_and_fallback() {
        let a = state(1);
        let d = derived(move || a.get() * 2);
        let log: Rc<RefCell<Vec<i32>>> = Rc::default();
        let l = log.clone();
        effect(move || l.borrow_mut().push(d.get()));
        assert_eq!(*log.borrow(), vec![2]);

        d.set(100); // 乐观覆盖
        assert_eq!(*log.borrow(), vec![2, 100], "下游应立即看到覆盖值");
        assert_eq!(d.get_untracked(), 100);

        a.set(5); // 任一依赖变化 → 重算盖回
        assert_eq!(*log.borrow(), vec![2, 100, 10], "依赖变化后应回退为重算值");
    }

    #[test]
    fn writable_derived_same_value_skips_downstream() {
        let a = state(1);
        let d = derived(move || a.get() * 2);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            d.get();
            *r.borrow_mut() += 1;
        });
        assert_eq!(*runs.borrow(), 1);
        d.set(2); // 与当前派生值相同
        assert_eq!(*runs.borrow(), 1, "覆盖相同值不应惊动下游");
        d.set(3);
        assert_eq!(*runs.borrow(), 2);
    }

    #[test]
    fn writable_derived_update_on_fresh_value() {
        let a = state(1);
        let d = derived(move || a.get() + 1);
        // 从未读过也能 update:set/update 都会先算出最新值(顺带建立依赖边)
        d.update(|v| *v *= 10);
        assert_eq!(d.get(), 20);
        a.set(5);
        assert_eq!(d.get(), 6, "依赖变化后覆盖值应回退");
    }

    #[test]
    #[should_panic(expected = "derived")]
    fn write_derived_in_derived_panics() {
        let a = state(1);
        let d1 = derived(move || a.get());
        let d2 = derived(move || {
            d1.set(9); // derived 计算过程中写 derived 同样被拒
            a.get()
        });
        d2.get();
    }

    // -- 两阶段 flush -------------------------------------------------------

    #[test]
    fn effect_pre_runs_before_normal_effects() {
        let a = state(0);
        let order: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        // 故意先注册普通 effect:验证靠的是相位而不是注册顺序
        let o = order.clone();
        effect(move || {
            a.get();
            o.borrow_mut().push("普通");
        });
        let o = order.clone();
        let pre = effect_pre(move || {
            a.get();
            o.borrow_mut().push("pre");
        });
        order.borrow_mut().clear();
        a.set(1);
        assert_eq!(
            *order.borrow(),
            vec!["pre", "普通"],
            "同一轮 flush 里 pre 应先跑"
        );
        pre.dispose();
        a.set(2);
        assert_eq!(*order.borrow(), vec!["pre", "普通", "普通"]);
    }

    // -- is_tracking / unique_id / tick ------------------------------------

    #[test]
    fn is_tracking_reflects_observer() {
        assert!(!is_tracking(), "作用域外不追踪");
        let a = state(0);
        let seen: Rc<RefCell<Vec<bool>>> = Rc::default();
        let s = seen.clone();
        effect(move || {
            a.get();
            s.borrow_mut().push(is_tracking());
            s.borrow_mut().push(untrack(is_tracking));
        });
        assert_eq!(
            *seen.borrow(),
            vec![true, false],
            "effect 内 true、untrack 内 false"
        );
        // derived 计算过程中同样处于追踪上下文
        let d = derived(is_tracking);
        assert!(d.get());
    }

    #[test]
    fn unique_id_is_incrementing() {
        let a = unique_id();
        let b = unique_id();
        assert!(a.starts_with("sv-") && b.starts_with("sv-"));
        let na: u64 = a["sv-".len()..].parse().unwrap();
        let nb: u64 = b["sv-".len()..].parse().unwrap();
        assert_eq!(nb, na + 1, "同线程内应自增,保证互不相同");
    }

    /// ADR-6 帧对齐:写入只催帧不跑 effect,帧前 `tick` 统一冲刷。
    /// 关键收益是"一次事件连写 N 次 = 一帧一轮",这里逐条钉住
    #[test]
    fn frame_aligned_defers_effects_until_tick() {
        let (_, _scope) = create_root(|| {
            let count = state(0);
            let runs = Rc::new(RefCell::new(0));
            let r = runs.clone();
            effect(move || {
                count.get();
                *r.borrow_mut() += 1;
            });
            assert_eq!(*runs.borrow(), 1, "创建时同步首跑(ADR-1),不进队列");

            let wakes = Rc::new(RefCell::new(0));
            let w = wakes.clone();
            set_frame_scheduler(move || *w.borrow_mut() += 1);

            // 一次"事件"里连写三次:effect 一次都不该跑,帧只该被催一次
            count.set(1);
            count.set(2);
            count.set(3);
            assert_eq!(*runs.borrow(), 1, "帧对齐下写入不该当场跑 effect");
            assert_eq!(*wakes.borrow(), 1, "连写只催一帧");

            // 帧前冲刷:三次写入合成一轮 effect
            tick();
            assert_eq!(*runs.borrow(), 2, "帧前统一冲刷 = 一轮");
            assert_eq!(count.get(), 3);

            // 下一轮写入重新催帧
            count.set(4);
            assert_eq!(*wakes.borrow(), 2, "新一轮写入应重新催帧");
            tick();
            assert_eq!(*runs.borrow(), 3);

            // 逃生舱:帧对齐下 tick 就是 flush_sync
            count.set(5);
            tick();
            assert_eq!(*runs.borrow(), 4, "tick 是逃生舱,立刻看到结果");

            // 关掉帧对齐 → 回到写入即同步
            clear_frame_scheduler();
            count.set(6);
            assert_eq!(*runs.borrow(), 5, "关掉后应恢复同步 flush");
        });
    }

    /// 帧对齐与 batch 叠加:batch 内不催帧,batch 结束催一次
    #[test]
    fn frame_aligned_composes_with_batch() {
        let (_, _scope) = create_root(|| {
            let a = state(0);
            let b = state(0);
            let runs = Rc::new(RefCell::new(0));
            let r = runs.clone();
            effect(move || {
                a.get();
                b.get();
                *r.borrow_mut() += 1;
            });
            let wakes = Rc::new(RefCell::new(0));
            let w = wakes.clone();
            set_frame_scheduler(move || *w.borrow_mut() += 1);

            batch(|| {
                a.set(1);
                b.set(1);
                assert_eq!(*wakes.borrow(), 0, "batch 内不催帧");
            });
            assert_eq!(*wakes.borrow(), 1, "batch 结束催一次");
            assert_eq!(*runs.borrow(), 1, "但仍不跑 effect,等帧前");
            tick();
            assert_eq!(*runs.borrow(), 2);
            clear_frame_scheduler();
        });
    }

    #[test]
    fn tick_is_noop_in_batch_and_flushes_after() {
        let a = state(0);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            a.get();
            *r.borrow_mut() += 1;
        });
        assert_eq!(*runs.borrow(), 1);
        let r = runs.clone();
        batch(move || {
            a.set(1);
            tick(); // batch 内 no-op,不破坏批处理原子性
            assert_eq!(*r.borrow(), 1, "batch 内 tick 不应提前触发 effect");
        });
        assert_eq!(*runs.borrow(), 2, "batch 结束照常 flush");
        tick(); // 没有待决 effect 时调用无害
        assert_eq!(*runs.borrow(), 2);
    }

    // -- context ------------------------------------------------------------

    #[test]
    fn context_provide_lookup_and_shadowing() {
        struct Theme(&'static str);
        struct Session;

        // 作用域外取不到
        assert!(use_context::<Theme>().is_none());

        let (_, root) = create_root(|| {
            provide_context(Theme("外层"));
            assert_eq!(use_context::<Theme>().unwrap().0, "外层");
            // 未提供过的类型返回 None
            assert!(use_context::<Session>().is_none());

            // 内层同类型就近覆盖,且不影响外层
            let (got, inner) = create_root(|| {
                provide_context(Theme("内层"));
                provide_context(Session);
                use_context::<Theme>().unwrap().0
            });
            assert_eq!(got, "内层", "同类型应就近覆盖");
            // 挂在内层作用域的类型,外层沿链向上查不到(只向上、不向下)
            assert!(use_context::<Session>().is_none());
            inner.dispose();
            assert_eq!(use_context::<Theme>().unwrap().0, "外层");
        });
        root.dispose();
    }

    #[test]
    fn context_crosses_root_boundary() {
        // keyed each 的行作用域 = effect 里再 create_root。节点记录的是
        // **创建时**的 owner,所以查找要能穿过这个 root 边界
        struct Theme(&'static str);
        let seen: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        let s = seen.clone();
        let (_, root) = create_root(move || {
            provide_context(Theme("dark"));
            effect(move || {
                let s = s.clone();
                let (_, _row) = create_root(move || {
                    let got = use_context::<Theme>().map_or("取不到", |t| t.0);
                    s.borrow_mut().push(got);
                });
            });
        });
        assert_eq!(
            *seen.borrow(),
            vec!["dark"],
            "create_root 内应取到外层 context"
        );
        root.dispose();
    }

    // -- 图一致性(glitch-free)与剪枝 ---------------------------------------

    /// 菱形只数"跑了几次"是不够的:真正的 glitch 是**跑的那一次读到半新半旧**
    /// (b 已更新、c 还是旧值)。push-pull 里 effect 一律等到 flush 才跑、
    /// derived 一律读时才 pull,才能保证每次快照自洽。
    /// 防的退化:任何"写 signal 时顺手推算 derived / 立刻跑部分 effect"的改写
    #[test]
    fn diamond_effect_never_sees_intermediate_state() {
        let a = state(1);
        let b = derived(move || a.get() * 2);
        let c = derived(move || a.get() + 10);
        let seen: Rc<RefCell<Vec<(i32, i32, i32)>>> = Rc::default();
        let s = seen.clone();
        effect(move || s.borrow_mut().push((a.get(), b.get(), c.get())));
        a.set(2);
        a.set(7);
        for (av, bv, cv) in seen.borrow().iter() {
            assert_eq!(
                (*bv, *cv),
                (av * 2, av + 10),
                "effect 读到了不自洽的快照:a={av} b={bv} c={cv}"
            );
        }
        assert_eq!(seen.borrow().len(), 3, "每次写入只该产生一个快照");
    }

    /// 相等剪枝要在 **derived → derived** 这一段也成立:上游重算出同样的值时,
    /// 下游停在 Check 上原地转 Clean,不该重算。
    /// 防的退化:derived 重算后无条件把下游标 Dirty(等于剪枝只对 effect 生效),
    /// 那么"a 变 → 只有极少数派生真的变"的常见形态会全图重算
    #[test]
    fn derived_equality_cuts_downstream_recompute() {
        let a = state(1);
        let parity = derived(move || a.get() % 2);
        let computes = Rc::new(RefCell::new(0));
        let c = computes.clone();
        let label = derived(move || {
            *c.borrow_mut() += 1;
            parity.get() * 100
        });
        assert_eq!(label.get(), 100);
        assert_eq!(*computes.borrow(), 1);
        a.set(3); // 变了,但 parity 还是 1
        assert_eq!(label.get(), 100);
        assert_eq!(*computes.borrow(), 1, "上游派生值未变,下游不该重算");
        a.set(2);
        assert_eq!(label.get(), 0);
        assert_eq!(*computes.borrow(), 2);
    }

    /// signal 与 derived 的语义差别:**signal 写即通知,不做相等剪枝**。
    /// 防的退化:有人"顺手"给 signal 加上值相等剪枝——那么靠 `update` 触发
    /// 副作用(容器原地改内容、强制重新渲染)的代码会静默失效
    #[test]
    fn signal_write_notifies_even_when_value_unchanged() {
        let a = state(1);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            a.get();
            *r.borrow_mut() += 1;
        });
        a.set(1); // 同值
        assert_eq!(*runs.borrow(), 2, "signal 写同值也应通知下游");
        a.update(|_| {}); // 原地什么都没改
        assert_eq!(*runs.borrow(), 3, "update 无条件通知");
    }

    // -- 清理顺序与作用域回收 ------------------------------------------------

    /// 清理的两条顺序契约:①子作用域先于父作用域清理(父的 cleanup 常在拆
    /// 子作用域赖以存在的东西,顺序反了就是 use-after-free 式的逻辑错);
    /// ②清理跑在**重跑之前**,不是之后(否则新一轮刚建好的订阅/节点会被自己清掉)
    #[test]
    fn cleanup_order_children_first_then_self_then_body() {
        let a = state(0);
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        let l = log.clone();
        effect(move || {
            a.get();
            l.borrow_mut().push("外层跑");
            let li = l.clone();
            effect(move || {
                let li = li.clone();
                on_cleanup(move || li.borrow_mut().push("内层清理"));
            });
            let lo = l.clone();
            on_cleanup(move || lo.borrow_mut().push("外层清理"));
        });
        a.set(1);
        assert_eq!(
            *log.borrow(),
            vec!["外层跑", "内层清理", "外层清理", "外层跑"]
        );
    }

    /// `RootHandle::dispose` 是组件卸载的基石:挂在**根作用域本身**上的
    /// on_cleanup(不是挂在某个 effect 上)也必须跑,且子作用域先跑。
    /// 防的退化:dispose 只递归销毁子节点、忘了执行自己的 cleanups
    #[test]
    fn root_dispose_runs_scope_cleanups_child_first() {
        let log: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        let l = log.clone();
        let (_, root) = create_root(move || {
            let li = l.clone();
            effect(move || {
                let li = li.clone();
                on_cleanup(move || li.borrow_mut().push("effect 清理"));
            });
            on_cleanup(move || l.borrow_mut().push("根清理"));
        });
        assert!(log.borrow().is_empty(), "dispose 之前不该清理");
        root.dispose();
        assert_eq!(*log.borrow(), vec!["effect 清理", "根清理"]);
    }

    /// 组件卸载不能漏节点:root 下的 signal/derived/嵌套 effect 全部回收,
    /// 节点数回到建根之前。防的退化:dispose 只摘链不 remove(arena 泄漏),
    /// 或递归只下一层(嵌套 effect 的子节点留着)
    #[test]
    fn root_dispose_reclaims_whole_subtree() {
        let base = debug_node_count();
        let (_, root) = create_root(|| {
            let a = state(0);
            let d = derived(move || a.get() + 1);
            effect(move || {
                d.get();
                effect(move || {
                    a.get();
                });
            });
        });
        assert!(
            debug_node_count() >= base + 5,
            "至少建了 root/signal/derived/两个 effect"
        );
        root.dispose();
        assert_eq!(debug_node_count(), base, "root 销毁后应无残留节点");
    }

    /// 死循环保护数的是 **flush 轮数**,不是 effect 执行次数。
    /// 防的退化:把计数挪进内层循环——那样"一个 signal 挂了上千个 effect"
    /// (长列表逐行绑定的常态)会被误判成死循环而 panic
    #[test]
    fn flush_guard_counts_passes_not_effect_runs() {
        let n = MAX_FLUSH_PASSES + 100;
        let a = state(0);
        let runs = Rc::new(RefCell::new(0usize));
        let ro = runs.clone();
        let (_, root) = create_root(move || {
            for _ in 0..n {
                let r = ro.clone();
                effect(move || {
                    a.get();
                    *r.borrow_mut() += 1;
                });
            }
        });
        assert_eq!(*runs.borrow(), n);
        a.set(1); // 一次写入 = 一轮,哪怕这一轮里跑了 n 个 effect
        assert_eq!(*runs.borrow(), 2 * n);
        root.dispose();
    }

    // -- untrack / detached 边界 --------------------------------------------

    /// untrack 只摘掉**当前** observer;它内部新建的 effect 有自己的追踪上下文,
    /// 照常收集依赖。防的退化:把 untrack 实现成全局"追踪开关",
    /// 那么 `untrack(|| effect(..))` 建出来的 effect 会一辈子不重跑
    #[test]
    fn untrack_does_not_disable_nested_effect_tracking() {
        let a = state(0);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        let (_, root) = create_root(move || {
            untrack(move || {
                effect(move || {
                    a.get();
                    *r.borrow_mut() += 1;
                });
            });
        });
        assert_eq!(*runs.borrow(), 1);
        a.set(1);
        assert_eq!(
            *runs.borrow(),
            2,
            "untrack 内建的 effect 仍应追踪自己的依赖"
        );
        untrack(|| a.set(2)); // 写入不受 untrack 影响,照常通知
        assert_eq!(*runs.borrow(), 3, "untrack 只管读,不该屏蔽写通知");
        root.dispose();
    }

    /// `detached` 的正牌用途(tasks 桥的 pending 计数):线程级单例信号可能在
    /// **某个 effect 运行期间**被惰性初始化。若不游离创建,它会成为那个 effect
    /// 的子节点,下一次重跑就把它销毁了——之后再读直接 panic。
    /// 防的退化:detached 只 take observer、忘了 take owner
    #[test]
    fn detached_node_survives_owner_rerun() {
        let dep = state(0);
        let holder: Rc<RefCell<Option<Signal<i32>>>> = Rc::default();
        let h = holder.clone();
        effect(move || {
            dep.get();
            if h.borrow().is_none() {
                let s = detached(|| state(7));
                *h.borrow_mut() = Some(s);
            }
        });
        dep.set(1);
        dep.set(2);
        let s = holder.borrow().expect("首跑应已创建");
        assert_eq!(s.get_untracked(), 7, "游离节点不该随宿主 effect 重跑被销毁");
    }

    /// detached 同时也不建立依赖(它把 observer 一并摘掉):
    /// 桥内部读自己的计数不该把调用方 effect 绑上去
    #[test]
    fn detached_read_does_not_subscribe() {
        let a = state(0);
        let runs = Rc::new(RefCell::new(0));
        let r = runs.clone();
        effect(move || {
            detached(|| a.get());
            *r.borrow_mut() += 1;
        });
        a.set(1);
        assert_eq!(*runs.borrow(), 1, "detached 内的读取不该建立依赖");
    }

    // -- 所有权与 context 的组合 --------------------------------------------

    /// keyed each 的行作用域契约(`with_owner`):行挂**宿主 root** 名下,
    /// 所以活得过列表 effect 的重跑;同时 owner 链没被切断,行内仍能取到组件层
    /// context。防的退化:用 detached 代替 with_owner(行活了但 context 断链),
    /// 或忘了替换 owner(行成了列表 effect 的子节点,重跑即被销毁)
    #[test]
    fn with_owner_rows_survive_rerun_and_keep_context() {
        struct Theme(&'static str);
        let dep = state(0);
        let rows: Rc<RefCell<Vec<Signal<i32>>>> = Rc::default();
        let themes: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        let (rw, th) = (rows.clone(), themes.clone());
        let (_, root) = create_root(move || {
            provide_context(Theme("dark"));
            // 宿主建在列表 effect 之外,行才不会被 effect 的重跑清理连坐
            let (_, host) = create_root(|| {});
            effect(move || {
                let n = dep.get();
                let (sig, theme) = with_owner(&host, || {
                    (state(n), use_context::<Theme>().map_or("取不到", |t| t.0))
                });
                rw.borrow_mut().push(sig);
                th.borrow_mut().push(theme);
            });
        });
        dep.set(1);
        let first = rows.borrow()[0];
        assert_eq!(
            first.get_untracked(),
            0,
            "宿主作用域下的行不该随列表 effect 重跑被销毁"
        );
        assert_eq!(
            *themes.borrow(),
            vec!["dark", "dark"],
            "with_owner 内 owner 链应完好,context 仍可达"
        );
        root.dispose();
    }

    /// 重跑要把上一轮 provide 的 context 一并清掉:本轮走了别的分支、没再
    /// provide,后代却还读到上一轮的值,是最难查的一类陈旧状态 bug
    #[test]
    fn context_dropped_when_not_reprovided_on_rerun() {
        struct Theme(&'static str);
        let flag = state(true);
        let seen: Rc<RefCell<Vec<&'static str>>> = Rc::default();
        let s = seen.clone();
        let (_, root) = create_root(move || {
            effect(move || {
                if flag.get() {
                    provide_context(Theme("dark"));
                }
                s.borrow_mut()
                    .push(use_context::<Theme>().map_or("无", |t| t.0));
            });
        });
        assert_eq!(*seen.borrow(), vec!["dark"]);
        flag.set(false);
        assert_eq!(
            *seen.borrow(),
            vec!["dark", "无"],
            "本轮没再 provide,不该读到上一轮的陈旧 context"
        );
        root.dispose();
    }

    // -- 读取重入 ------------------------------------------------------------

    /// `with` 是"偷值"读取:值被 take 出来交给闭包,期间同一节点再读会撞空。
    /// 这条钉住它 panic 而不是静默给出错值/默认值
    #[test]
    #[should_panic(expected = "重入读取")]
    fn reentrant_read_of_same_signal_panics() {
        let a = state(1);
        a.with(|_| a.get());
    }

    /// 反面:偷值读取期间**不持有 RefCell 借用**,闭包里访问别的响应式值必须自如
    /// (`{#each}` 的行渲染就是在一个 with 里读一堆别的 signal)。
    /// 防的退化:把 with 改成"借用期间执行闭包"——那会变成 BorrowMutError
    #[test]
    fn with_allows_reading_other_nodes() {
        let a = state(1);
        let b = state(2);
        let d = derived(move || b.get() * 10);
        assert_eq!(a.with(|x| x + b.get() + d.get()), 23);
        // 写别的 signal 同样允许(读 a 的过程中改 b)
        a.with(|_| b.set(5));
        assert_eq!(b.get(), 5);
    }

    // -- 帧对齐的边界(ADR-6) ----------------------------------------------

    /// 催帧回调就是渲染壳的 `request_redraw`,壳里顺手写 signal 很正常
    /// (同步窗口尺寸/dpi)。所以 maybe_flush 必须在**放开 RT 借用之后**才调它,
    /// 否则这里直接 BorrowMutError;并且回调里的写入应并进同一帧,不重复催帧
    #[test]
    fn frame_scheduler_callback_may_write_state() {
        let (_, root) = create_root(|| {
            let a = state(0);
            let mirror = state(0);
            let runs = Rc::new(RefCell::new(0));
            let r = runs.clone();
            effect(move || {
                a.get();
                mirror.get();
                *r.borrow_mut() += 1;
            });
            let wakes = Rc::new(RefCell::new(0));
            let w = wakes.clone();
            set_frame_scheduler(move || {
                *w.borrow_mut() += 1;
                mirror.set(mirror.get_untracked() + 1);
            });

            a.set(1);
            assert_eq!(mirror.get_untracked(), 1, "催帧回调里的写入应生效");
            assert_eq!(*wakes.borrow(), 1, "回调内的写入不该再催一帧");
            assert_eq!(*runs.borrow(), 1, "两处写入都只入队");
            tick();
            assert_eq!(*runs.borrow(), 2, "帧前统一冲刷成一轮");
            clear_frame_scheduler();
        });
        root.dispose();
    }

    /// 帧对齐推迟的只有 **effect 冲刷**:derived 是 pull 语义,读到就地算最新;
    /// 新建 effect 也照旧同步首跑(ADR-1),不然刚挂载的组件会空一帧。
    /// 防的退化:把"帧对齐"理解成"整个图冻结到下一帧"
    #[test]
    fn frame_aligned_keeps_derived_pull_and_sync_first_run() {
        let (_, root) = create_root(|| {
            let a = state(1);
            let d = derived(move || a.get() * 10);
            assert_eq!(d.get_untracked(), 10);
            set_frame_scheduler(|| {});
            a.set(2);
            assert_eq!(d.get_untracked(), 20, "derived 是 pull,读时即最新");
            let runs = Rc::new(RefCell::new(0));
            let r = runs.clone();
            effect(move || {
                a.get();
                *r.borrow_mut() += 1;
            });
            assert_eq!(*runs.borrow(), 1, "帧对齐下新建 effect 仍同步首跑");
            clear_frame_scheduler();
        });
        root.dispose();
    }
}
