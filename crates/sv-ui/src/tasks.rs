//! 异步桥 + `{#await}` 运行时。
//!
//! 模型:UI 单线程持有完成通道的接收端与回调表;`spawn` 把 Future 丢到
//! 后台线程(极简 block_on:thread park/unpark 做 waker),完成值(`Send`)
//! 经通道送回,UI 线程在 [`pump`] 时取出并调用回调(回调里写 signal)。
//!
//! - 窗口场景:sv-shell 在每帧/空闲时调 `pump()`,并用 [`set_waker`] 注册
//!   事件循环唤醒器(worker 完成时把 winit 拍醒)。
//! - 无窗测试:[`pump_until_idle`] 阻塞等完。
//! - [`pending_count`] 是**响应式**的在途任务数(`$effect.pending()` 的实现)。

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use sv_reactive::{Signal, effect, on_cleanup, state};

use crate::{Doc, ViewId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TaskId(u64);

type DoneMsg = (u64, Box<dyn Any + Send>);
/// 任务完成回调(拿回 worker 线程的返回值,在 UI 线程跑)
type DoneCallback = Box<dyn FnOnce(Box<dyn Any + Send>)>;
/// 事件循环唤醒闭包(跨线程调用,故 Send + Sync)
type WakeFn = Box<dyn Fn() + Send + Sync>;

struct Bridge {
    tx: Sender<DoneMsg>,
    rx: Receiver<DoneMsg>,
    callbacks: RefCell<HashMap<u64, DoneCallback>>,
    next: Cell<u64>,
    /// 在途任务数(反应式)
    pending: Signal<usize>,
}

thread_local! {
    static BRIDGE: Bridge = {
        let (tx, rx) = channel();
        // pending 是线程级单例信号:必须游离创建——桥可能在某个 effect 运行
        // 期间被惰性初始化,不能让信号挂到那个 effect 的作用域下被误销毁
        let pending = sv_reactive::detached(|| state(0usize));
        Bridge { tx, rx, callbacks: RefCell::new(HashMap::new()), next: Cell::new(1), pending }
    };
}

/// 事件循环唤醒器(worker 线程完成任务后调用,把 UI 事件循环拍醒)。
/// 进程级全局:多窗口共用一个事件循环
static WAKER: OnceLock<Mutex<Option<WakeFn>>> = OnceLock::new();

pub fn set_waker(f: impl Fn() + Send + Sync + 'static) {
    *WAKER.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some(Box::new(f));
}

fn wake_ui() {
    if let Some(m) = WAKER.get()
        && let Ok(g) = m.lock()
        && let Some(f) = g.as_ref()
    {
        f();
    }
}

/// 极简 block_on:每个任务独占一个后台线程,park 等待唤醒
fn block_on<F: Future>(fut: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};
    struct ThreadWaker(std::thread::Thread);
    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}

/// 派发一个后台任务;完成后(下次 [`pump`])在 UI 线程调用 `on_done`
pub fn spawn<T: Send + 'static>(
    fut: impl Future<Output = T> + Send + 'static,
    on_done: impl FnOnce(T) + 'static,
) -> TaskId {
    BRIDGE.with(|b| {
        let id = b.next.get();
        b.next.set(id + 1);
        b.callbacks.borrow_mut().insert(
            id,
            Box::new(move |any| {
                let v = any.downcast::<T>().expect("tasks: 完成值类型不匹配");
                on_done(*v);
            }),
        );
        b.pending.update(|p| *p += 1);
        let tx = b.tx.clone();
        std::thread::spawn(move || {
            let v = block_on(fut);
            let _ = tx.send((id, Box::new(v)));
            wake_ui();
        });
        TaskId(id)
    })
}

/// 取消(丢弃回调;后台线程照常跑完,完成值被丢弃)。返回是否确有取消
pub fn cancel(id: TaskId) -> bool {
    BRIDGE.with(|b| {
        let removed = b.callbacks.borrow_mut().remove(&id.0).is_some();
        if removed {
            b.pending.update(|p| *p -= 1);
        }
        removed
    })
}

/// UI 线程处理已完成任务,返回处理数(shell 每帧/空闲调用)
pub fn pump() -> usize {
    BRIDGE.with(|b| {
        let mut n = 0usize;
        while let Ok((id, val)) = b.rx.try_recv() {
            // 已取消的任务:回调不存在,完成值静默丢弃
            let cb = b.callbacks.borrow_mut().remove(&id);
            if let Some(cb) = cb {
                b.pending.update(|p| *p -= 1);
                cb(val);
                n += 1;
            }
        }
        n
    })
}

/// 在途任务数(**响应式**读,`$effect.pending()` 的实现)
pub fn pending_count() -> usize {
    BRIDGE.with(|b| b.pending.get())
}

/// 无窗测试用:阻塞 pump 直到没有在途任务或超时。返回是否清空
pub fn pump_until_idle(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        pump();
        let pending = BRIDGE.with(|b| b.pending.get_untracked());
        if pending == 0 {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

// ---------------------------------------------------------------------------
// {#await} 块
// ---------------------------------------------------------------------------

/// `{#await fut}{:then v}{/await}`。
/// `factory` 在 effect 里求值:读到的依赖变化时**重启**(旧任务取消,回到 pending)
pub fn await_block<T, Fut>(
    doc: &Doc,
    parent: ViewId,
    factory: impl Fn() -> Fut + 'static,
    pending_b: impl Fn(&Doc, ViewId) + 'static,
    then_b: impl Fn(&Doc, ViewId, &T) + 'static,
) where
    T: Send + 'static,
    Fut: Future<Output = T> + Send + 'static,
{
    let container = doc.create_view();
    doc.append(parent, container);
    let value = state::<Option<Rc<T>>>(None);
    let current: Rc<Cell<Option<TaskId>>> = Rc::new(Cell::new(None));

    // 工厂 effect:依赖变化 → 取消旧任务、回 pending、派发新任务
    {
        let current = current.clone();
        effect(move || {
            let fut = factory(); // 依赖在此追踪
            if let Some(prev) = current.take() {
                cancel(prev);
            }
            value.set(None);
            let id = spawn(fut, move |out| value.set(Some(Rc::new(out))));
            current.set(Some(id));
        });
    }
    // 组件卸载时取消在途任务
    {
        let current = current.clone();
        on_cleanup(move || {
            if let Some(prev) = current.take() {
                cancel(prev);
            }
        });
    }
    // 渲染 effect:pending / then 二态
    let doc = doc.clone();
    effect(move || {
        doc.clear_children(container);
        match value.get() {
            None => pending_b(&doc, container),
            Some(rc) => then_b(&doc, container, &rc),
        }
        let d = doc.clone();
        on_cleanup(move || d.clear_children(container));
    });
}

/// `{#await}{:then}{:catch}`:Future 产出 `Result<V, E>`
pub fn await_block_result<V, E, Fut>(
    doc: &Doc,
    parent: ViewId,
    factory: impl Fn() -> Fut + 'static,
    pending_b: impl Fn(&Doc, ViewId) + 'static,
    then_b: impl Fn(&Doc, ViewId, &V) + 'static,
    catch_b: impl Fn(&Doc, ViewId, &E) + 'static,
) where
    V: Send + 'static,
    E: Send + 'static,
    Fut: Future<Output = Result<V, E>> + Send + 'static,
{
    await_block(
        doc,
        parent,
        factory,
        pending_b,
        move |doc, parent, result: &Result<V, E>| match result {
            Ok(v) => then_b(doc, parent, v),
            Err(e) => catch_b(doc, parent, e),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv_reactive::create_root;

    #[test]
    fn spawn_pump_roundtrip() {
        let (_, _root) = create_root(|| {
            let got = state(0i32);
            spawn(async { 41 + 1 }, move |v| got.set(v));
            assert_eq!(pending_count(), 1);
            assert!(pump_until_idle(Duration::from_secs(5)), "任务应完成");
            assert_eq!(got.get_untracked(), 42);
            assert_eq!(pending_count(), 0);
        });
    }

    #[test]
    fn cancel_discards_completion() {
        let (_, _root) = create_root(|| {
            let got = state(0i32);
            let id = spawn(async { 7 }, move |v| got.set(v));
            assert!(cancel(id));
            // 等后台线程跑完、完成消息被静默丢弃
            std::thread::sleep(Duration::from_millis(50));
            pump();
            assert_eq!(got.get_untracked(), 0, "取消后回调不应执行");
            assert_eq!(pending_count(), 0);
        });
    }

    #[test]
    fn await_block_renders_then_restarts() {
        let doc = Doc::new();
        let (_, _root) = create_root(|| {
            let base = state(1i32);
            await_block(
                &doc,
                doc.root(),
                move || {
                    let b = base.get();
                    async move { b * 10 }
                },
                |doc, parent| {
                    let t = doc.create_text("加载中");
                    doc.append(parent, t);
                },
                |doc, parent, v: &i32| {
                    let t = doc.create_text(&format!("结果 {v}"));
                    doc.append(parent, t);
                },
            );
            assert!(
                doc.dump().contains("加载中"),
                "初始应为 pending:\n{}",
                doc.dump()
            );
            assert!(pump_until_idle(Duration::from_secs(5)));
            assert!(doc.dump().contains("结果 10"), "\n{}", doc.dump());

            // 依赖变化 → 回 pending → 新结果
            base.set(2);
            assert!(
                doc.dump().contains("加载中"),
                "重启应回 pending:\n{}",
                doc.dump()
            );
            assert!(pump_until_idle(Duration::from_secs(5)));
            assert!(doc.dump().contains("结果 20"), "\n{}", doc.dump());
        });
    }

    /// 线程契约:future 在 worker 线程跑,完成回调必须回到 **UI 线程**
    /// (回调里写 signal,而响应式图是 `!Send` 的单线程模型);
    /// 且回调只在 [`pump`] 里跑,不会在后台线程提前触发。
    /// 防的退化:图省事在 worker 线程里直接调回调
    #[test]
    fn completion_callback_runs_on_ui_thread_at_pump() {
        let ui = std::thread::current().id();
        let got: Rc<RefCell<Option<(std::thread::ThreadId, std::thread::ThreadId)>>> =
            Rc::default();
        let g = got.clone();
        spawn(
            async { std::thread::current().id() },
            move |worker: std::thread::ThreadId| {
                *g.borrow_mut() = Some((worker, std::thread::current().id()));
            },
        );
        std::thread::sleep(Duration::from_millis(50));
        assert!(got.borrow().is_none(), "pump 之前回调不该跑");
        assert!(pump_until_idle(Duration::from_secs(5)));
        let (worker, cb) = got.borrow().expect("应已回调");
        assert_eq!(cb, ui, "完成回调必须在 UI 线程执行");
        assert_ne!(worker, ui, "future 本身应在后台线程执行");
    }

    /// `pending_count` 是 `$effect.pending()` 的实现,必须是**反应式**的:
    /// 派发时推高、完成时落回,两次都要惊动下游。
    /// 防的退化:把它改成普通 Cell/裸计数——加载骨架屏就再也不会自己收起来
    #[test]
    fn pending_count_drives_effects() {
        let log: Rc<RefCell<Vec<usize>>> = Rc::default();
        let l = log.clone();
        let (_, root) = create_root(move || {
            effect(move || l.borrow_mut().push(pending_count()));
        });
        assert_eq!(*log.borrow(), vec![0]);
        spawn(async { 1u8 }, |_| {});
        assert_eq!(*log.borrow(), vec![0, 1], "派发应立刻推高在途数");
        assert!(pump_until_idle(Duration::from_secs(5)));
        assert_eq!(*log.borrow(), vec![0, 1, 0], "完成应落回 0 并通知下游");
        root.dispose();
    }

    /// 组件卸载时在途任务必须被撤掉:回调要写的 `value` signal 已随作用域销毁,
    /// 真让它跑起来就是 "signal 已随作用域销毁" 的 panic(而且是在 pump 里炸,
    /// 离现场很远)。这条测试与时序无关——任务是否已完成,取消都该兜住
    #[test]
    fn scope_dispose_cancels_inflight_await() {
        let doc = Doc::new();
        let (_, root) = create_root(|| {
            await_block(
                &doc,
                doc.root(),
                || async {
                    std::thread::sleep(Duration::from_millis(120));
                    7i32
                },
                |d, p| {
                    let t = d.create_text("加载中");
                    d.append(p, t);
                },
                |d, p, v: &i32| {
                    let t = d.create_text(&format!("结果 {v}"));
                    d.append(p, t);
                },
            );
        });
        assert_eq!(pending_count(), 1);
        root.dispose();
        assert_eq!(pending_count(), 0, "卸载应撤掉在途任务");
        assert!(
            pump_until_idle(Duration::from_secs(5)),
            "已取消任务的完成值应被静默丢弃"
        );
        assert!(!doc.dump().contains("结果"), "卸载后不该再改场景树");
    }

    /// `pending` 是 `usize`:多减一次在 debug 下直接整数下溢 panic。
    /// 重复取消、取消已完成的任务都必须是安全的 no-op
    #[test]
    fn cancel_is_idempotent_and_never_underflows_pending() {
        let id = spawn(async { 1u8 }, |_| {});
        assert_eq!(pending_count(), 1);
        assert!(cancel(id));
        assert_eq!(pending_count(), 0);
        assert!(!cancel(id), "重复取消应如实报告没取消到");
        assert_eq!(pending_count(), 0);

        let done = spawn(async { 2u8 }, |_| {});
        assert!(pump_until_idle(Duration::from_secs(5)));
        assert!(!cancel(done), "已完成的任务无从取消");
        assert_eq!(pending_count(), 0);
    }

    /// worker 完成后要拍醒事件循环:少了这一脚,窗口要等到下一次输入事件
    /// 才会 pump,异步结果看起来就是"卡住不刷新"
    #[test]
    fn worker_completion_wakes_event_loop() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        set_waker(move || {
            h.fetch_add(1, Ordering::SeqCst);
        });
        spawn(async { 1u8 }, |_| {});
        assert!(pump_until_idle(Duration::from_secs(5)));
        // wake_ui 在 send 之后调用,可能比 pump 晚一点,给它一个宽限
        let deadline = Instant::now() + Duration::from_secs(5);
        while hits.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            hits.load(Ordering::SeqCst) >= 1,
            "worker 完成后应拍醒 UI 事件循环"
        );
    }

    /// `{#await}` 依赖变化重启时,旧任务的回调必须撤掉:否则(一)在途数只涨
    /// 不落,`$effect.pending()` 永远为真;(二)旧结果后到会盖掉新结果(经典竞态)
    #[test]
    fn await_restart_cancels_previous_task() {
        let doc = Doc::new();
        let (_, root) = create_root(|| {
            let base = state(1i32);
            await_block(
                &doc,
                doc.root(),
                move || {
                    let b = base.get();
                    async move {
                        std::thread::sleep(Duration::from_millis(60));
                        b * 10
                    }
                },
                |_, _| {},
                |d, p, v: &i32| {
                    let t = d.create_text(&format!("结果 {v}"));
                    d.append(p, t);
                },
            );
            assert_eq!(pending_count(), 1);
            base.set(2);
            assert_eq!(pending_count(), 1, "重启不该把在途任务堆起来");
            assert!(pump_until_idle(Duration::from_secs(5)));
            assert!(doc.dump().contains("结果 20"), "\n{}", doc.dump());
        });
        root.dispose();
    }

    #[test]
    fn await_block_result_catch() {
        let doc = Doc::new();
        let (_, _root) = create_root(|| {
            await_block_result(
                &doc,
                doc.root(),
                || async { Err::<i32, String>("坏了".into()) },
                |_, _| {},
                |doc, parent, v: &i32| {
                    let t = doc.create_text(&format!("ok {v}"));
                    doc.append(parent, t);
                },
                |doc, parent, e: &String| {
                    let t = doc.create_text(&format!("错误:{e}"));
                    doc.append(parent, t);
                },
            );
            assert!(pump_until_idle(Duration::from_secs(5)));
            assert!(doc.dump().contains("错误:坏了"), "\n{}", doc.dump());
        });
    }
}
