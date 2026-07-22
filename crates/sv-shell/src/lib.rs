//! # sv-shell
//!
//! 桌面渲染壳(原型):winit 窗口 + 纯 CPU 自绘(softbuffer + tiny-skia + swash)。
//!
//! 定位:先用最稳的 CPU 栈跑通「signal → 场景树 → 像素」闭环;
//! 后续按调研结论迁移到 wgpu/vello + parley,并把窗口层抽成窄 trait
//! (桌面 winit 实现 / 鸿蒙 XComponent 实现)。
//!
//! - [`run_app`]:开窗运行,树变更自动 request_redraw,点击自动派发。
//! - [`render_to_png`]:离屏渲染一帧存 PNG(CI / 可视化验证用,不需要窗口)。

mod a11y;
mod paint;
mod render;
mod text;
#[cfg(feature = "backend-vello")]
mod vello_backend;

pub use a11y::{A11yCache, build_tree_update, dispatch_action, incremental_tree_update};
pub use paint::{
    GlyphKey, GlyphPos, PaintCmd, Painter, PainterCaps, RecordingPainter, TinySkiaPainter,
};
pub use render::{
    Layout, OverlayRegion, Placed, Rect, ScrollArea, hit_click_target, ime_caret_rect,
    input_caret_at, input_caret_line_move, layout_full_cached, layout_tree, layout_tree_full,
    overlay_click_gate, paint_scrollbars, paint_tree, render_frame, route_wheel,
    scrollbar_drag_offset, scrollbar_grab, scrollbar_thumb,
};
pub use text::{FontHandle, caret_index_at, caret_x, selection_rects};
// ShellError 定义在下方(与窗口/呈现代码同处),此处不重复导出
#[cfg(feature = "backend-vello")]
pub use vello_backend::{VelloPainter, VelloWin, render_frame_vello};

use std::num::NonZeroU32;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use sv_reactive::{RootHandle, create_root};
use sv_ui::{Doc, ViewId};

/// 渲染后端(ADR-3b:多后端;默认 CPU,vello 走 feature + 运行时探测)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    Cpu,
    #[cfg(feature = "backend-vello")]
    Vello,
}

/// 选择逻辑:SV_RENDERER=cpu|vello 显式覆盖 → feature 开启时探测 adapter
/// (拿不到静默回退 Cpu)→ 默认 Cpu
fn select_backend() -> Backend {
    match std::env::var("SV_RENDERER").ok().as_deref() {
        Some("cpu") => Backend::Cpu,
        Some("vello") => vello_or_fallback(true),
        _ => vello_or_fallback(false),
    }
}

#[cfg(feature = "backend-vello")]
fn vello_or_fallback(explicit: bool) -> Backend {
    // 显式指定不预探测:开窗时建 surface 失败会再回退一次
    if explicit || vello_backend::probe_adapter() {
        Backend::Vello
    } else {
        eprintln!("sv-shell: 未探测到可用 GPU adapter,回退 CPU 渲染后端");
        Backend::Cpu
    }
}

#[cfg(not(feature = "backend-vello"))]
fn vello_or_fallback(explicit: bool) -> Backend {
    if explicit {
        eprintln!("sv-shell: SV_RENDERER=vello 需要 backend-vello feature,回退 CPU 渲染后端");
    }
    Backend::Cpu
}

// ---------------------------------------------------------------------------
// 错误类型(R4 去 panic 审计,调研 25 §3.4)
//
// 纪律:**窗口/呈现层的失败一律类型化**,事件循环里不 expect 崩——用户 App
// 不该因为一次 present 失败整个进程消失。分两级:
// - 致命(建窗/建 surface/事件循环):`ShellError` 冒泡出 `run_app`;
// - 帧内(resize/取缓冲/present):记一次日志 + 丢该帧,下一帧重试。
// 保留的 panic 只剩"自证不变量"(taffy 自建树取回、字体注册表键),它们
// 不依赖外部环境,崩了是本仓库的 bug 而非运行时状况——`shell_panics_are_
// invariants_only` 测试守住这条线。sv-reactive 里 derived 写保护的 panic 是
// 语义设计(对齐 Svelte state_unsafe_mutation),不在本次范围。
// ---------------------------------------------------------------------------

/// 渲染壳致命错误(能起来就不该发生,起不来必须让调用方知道)
#[derive(Debug)]
pub enum ShellError {
    /// 事件循环创建/运行失败(winit)
    EventLoop(String),
    /// 窗口创建失败
    Window(String),
    /// 绘图上下文 / surface 创建失败(softbuffer)
    Surface(String),
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EventLoop(e) => write!(f, "sv-shell: 事件循环失败:{e}"),
            Self::Window(e) => write!(f, "sv-shell: 创建窗口失败:{e}"),
            Self::Surface(e) => write!(f, "sv-shell: 创建绘图 surface 失败:{e}"),
        }
    }
}

impl std::error::Error for ShellError {}

/// 窗口呈现器:CPU(softbuffer)或 GPU(vello/wgpu)
enum Presenter {
    Cpu {
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        _context: softbuffer::Context<Arc<Window>>,
    },
    #[cfg(feature = "backend-vello")]
    Vello(vello_backend::VelloWin),
}

fn cpu_presenter(window: &Arc<Window>) -> Result<Presenter, ShellError> {
    let context = softbuffer::Context::new(window.clone())
        .map_err(|e| ShellError::Surface(format!("上下文:{e}")))?;
    let surface = softbuffer::Surface::new(&context, window.clone())
        .map_err(|e| ShellError::Surface(e.to_string()))?;
    Ok(Presenter::Cpu {
        surface,
        _context: context,
    })
}

/// 事件循环用户事件:后台任务唤醒 + AccessKit 事件(调研 24 §4.2)
enum UserEvent {
    Wake,
    Access(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(e: accesskit_winit::Event) -> Self {
        UserEvent::Access(e)
    }
}

struct WinState {
    window: Arc<Window>,
    presenter: Presenter,
    /// AccessKit 平台适配器(懒激活:AT 首次请求前零成本,egui PR#2294 形态)
    access: accesskit_winit::Adapter,
}

/// 首次 resumed 时跑一次的建树闭包(此后一切更新由 signal 驱动)
type BuildFn = Box<dyn FnOnce(&Doc, ViewId)>;

struct App {
    title: String,
    doc: Doc,
    build: Option<BuildFn>,
    _scope: Option<RootHandle>,
    backend: Backend,
    win: Option<WinState>,
    layout: Layout,
    cursor: (f64, f64),
    hovered: Option<ViewId>,
    pressed: Option<ViewId>,
    /// 文本拖选中的输入框(按下时记,松开清;Placed 是 Copy 快照)
    drag_input: Option<(ViewId, Placed)>,
    /// 滚动条拖动中的容器 + 按下时指针在 thumb 内的偏移(S4)。
    /// 桌面没有显式指针捕获:按住期间一直跟指针,松开即止 —— 与拖出控件
    /// 外仍继续滚的原生行为一致
    drag_scroll: Option<(ViewId, f32)>,
    /// 连击串:(上次按下时刻, 物理坐标, 第几击)——双击选词/三击全选。
    /// winit 不给点击计数,阈值自持:500ms + 4 物理 px(平台惯例中值)
    last_click: Option<(std::time::Instant, (f64, f64), u32)>,
    /// 当前修饰键状态(winit 单独派发 ModifiersChanged,应用自存)
    mods: ModifiersState,
    /// IME 会话开关镜像(焦点落在 accepts_text 节点时开;避免重复系统调用)
    ime_allowed: bool,
    epoch: std::time::Instant,
    /// 上一帧的 (版本, 宽, 高, scale位):静止帧跳过重绘制
    last_frame_key: Option<(u64, u32, u32, u32)>,
    /// SV_SHOW_FPS=1:连续重绘 + 每 60 帧打印帧率(基准/诊断用)
    show_fps: bool,
    fps_frames: u32,
    fps_t0: std::time::Instant,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    /// 累计丢帧次数(present/resize 失败;仅用于限流日志)
    frame_drops: u32,
    /// 语义树增量推送的记忆(上次交给平台的节点内容)
    a11y: a11y::A11yCache,
    /// 致命错误:记下并退事件循环,由 [`run_app`] 返给调用方
    fatal: Option<ShellError>,
}

impl App {
    /// 起不来时的体面退出:错误留给 run_app,事件循环立刻收摊
    fn abort(&mut self, event_loop: &ActiveEventLoop, e: ShellError) {
        eprintln!("{e}");
        self.fatal = Some(e);
        event_loop.exit();
    }

    fn paint(&mut self) {
        // 帧前流水线(ADR-6):后台任务完成值 → 动画推进 → **响应式统一冲刷**
        // → 布局 → 绘制。上面两步都会写 signal,所以 tick 必须排在它们之后;
        // 冲刷完成前不读版本号,否则会拿到上一帧的 key 而跳过本帧
        sv_ui::tasks::pump();
        let now_ms = self.epoch.elapsed().as_secs_f64() * 1000.0;
        let animating = sv_ui::anim::pump(now_ms);
        sv_reactive::tick();
        let Some(ws) = &mut self.win else { return };
        let size = ws.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let scale = ws.window.scale_factor() as f32;
        // 静止帧短路:版本/尺寸/缩放全没变 → 不重绘制(呈现内容仍在 surface 上)。
        // 细粒度更新模型下"静止"是常态,这一步把静止功耗归零
        let frame_key = (self.doc.version(), size.width, size.height, scale.to_bits());
        let unchanged = self.last_frame_key == Some(frame_key);
        if unchanged && !animating && !self.show_fps {
            return;
        }
        self.last_frame_key = Some(frame_key);
        match &mut ws.presenter {
            Presenter::Cpu { surface, .. } => {
                let (pixmap, _) = render_frame(&self.doc, size.width, size.height, scale);
                self.layout = layout_full_cached(
                    &self.doc,
                    size.width as f32 / scale,
                    size.height as f32 / scale,
                );

                let (Some(w), Some(h)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                else {
                    return;
                };
                // 帧内失败一律降级为丢帧:窗口最小化/GPU 复位/合成器重启时
                // 这些调用会短暂失败,崩进程是最坏的处理方式
                let frame = surface
                    .resize(w, h)
                    .and_then(|()| surface.buffer_mut())
                    .and_then(|mut buffer| {
                        for (dst, src) in buffer.iter_mut().zip(pixmap.pixels()) {
                            let c = src.demultiply();
                            *dst = ((c.red() as u32) << 16)
                                | ((c.green() as u32) << 8)
                                | (c.blue() as u32);
                        }
                        buffer.present()
                    });
                if let Err(e) = frame {
                    // 前三次与之后每 600 次各记一条:偶发不刷屏,持续故障看得见
                    self.frame_drops += 1;
                    // `%` 而非 `is_multiple_of`:后者 1.87 才稳定,MSRV 是 1.85
                    if self.frame_drops <= 3 || self.frame_drops % 600 == 0 {
                        eprintln!("sv-shell: 丢帧({e});累计 {} 次", self.frame_drops);
                    }
                    // 本帧作废:清帧键让下一帧重画(否则静止短路会永久跳过)
                    self.last_frame_key = None;
                    ws.window.request_redraw();
                    return;
                }
            }
            #[cfg(feature = "backend-vello")]
            Presenter::Vello(vw) => {
                vw.resize(size.width, size.height);
                // 静止帧(FPS 模式下仍在跑):跳过场景重编码,只重呈现
                let (_, presented) = vw.render_cached(&self.doc, scale, unchanged);
                self.layout = layout_full_cached(
                    &self.doc,
                    size.width as f32 / scale,
                    size.height as f32 / scale,
                );
                if !presented {
                    // surface 过期/被遮挡等:下一帧重试(与 vello 官方示例一致)
                    ws.window.request_redraw();
                }
            }
        }
        // IME 候选窗跟随光标(调研 21:cursor_area 上报是候选窗定位的全部机制;
        // 光标一动 bump → 重绘 → 重报,与版本键控布局缓存天然一致)
        if self.ime_allowed
            && let Some((cx, cy, cw, ch)) = ime_caret_rect(&self.doc, &self.layout.placed, scale)
        {
            ws.window.set_ime_cursor_area(
                winit::dpi::PhysicalPosition::new(cx as f64, cy as f64),
                winit::dpi::PhysicalSize::new(cw as f64, ch as f64),
            );
        }
        // 帧率计数(SV_SHOW_FPS=1):连续重绘,每 120 帧打印一次
        if self.show_fps {
            self.fps_frames += 1;
            if self.fps_frames >= 30 {
                let dt = self.fps_t0.elapsed().as_secs_f64();
                println!("FPS {:.0}", self.fps_frames as f64 / dt);
                self.fps_frames = 0;
                self.fps_t0 = std::time::Instant::now();
            }
            ws.window.request_redraw();
        }
        if animating {
            ws.window.request_redraw();
        }
        // 语义树跟随版本节拍(静止帧短路已在上方 return;懒激活未开时零成本)
        self.push_access_tree();
    }

    /// 语义树推送(调研 24 §4.2 + P6):按版本节拍触发,**只推变动节点**。
    /// 懒激活未开时 `update_if_active` 里的闭包根本不跑 —— 零成本
    fn push_access_tree(&mut self) {
        let Some(ws) = &mut self.win else { return };
        let scale = ws.window.scale_factor() as f32;
        let doc = self.doc.clone();
        let placed = &self.layout.placed;
        let cache = &mut self.a11y;
        ws.access
            .update_if_active(|| a11y::incremental_tree_update(cache, &doc, placed, scale));
    }

    /// 多行输入的 ↑/↓:按视觉行移动光标(Shift 扩选)。
    /// 归渲染壳是因为"上一行的同一 x"只有排版知道;单行输入不消费,
    /// 方向键照旧留给导航段
    fn route_line_move(&mut self, e: &sv_ui::KeyEvent) -> bool {
        let down = match e.key {
            sv_ui::Key::ArrowDown => true,
            sv_ui::Key::ArrowUp => false,
            _ => return false,
        };
        let Some(id) = self.doc.focused() else {
            return false;
        };
        let Some(p) = self.layout.placed.iter().find(|p| p.id == id).copied() else {
            return false;
        };
        match input_caret_line_move(&self.doc, &p, down) {
            Some(byte) => {
                self.doc.set_caret(id, byte, e.mods.shift);
                true
            }
            None => false,
        }
    }

    /// 本次按下是第几击(1/2/3 循环)。窗口 500ms、位移 4 物理 px 内算连击
    fn click_streak(&mut self) -> u32 {
        const WINDOW_MS: u128 = 500;
        const SLOP: f64 = 4.0;
        let now = std::time::Instant::now();
        let n = match self.last_click {
            Some((t, (cx, cy), n))
                if now.duration_since(t).as_millis() < WINDOW_MS
                    && (cx - self.cursor.0).abs() <= SLOP
                    && (cy - self.cursor.1).abs() <= SLOP =>
            {
                if n >= 3 {
                    1
                } else {
                    n + 1
                }
            }
            _ => 1,
        };
        self.last_click = Some((now, self.cursor, n));
        n
    }

    /// 悬停派发:命中最上层带 enter/leave 回调的节点,变化时先 leave 后 enter
    fn update_hover(&mut self) {
        let Some(ws) = &self.win else { return };
        let scale = ws.window.scale_factor();
        let (lx, ly) = (
            (self.cursor.0 / scale) as f32,
            (self.cursor.1 / scale) as f32,
        );
        let target = self
            .layout
            .placed
            .iter()
            .enumerate()
            .rev()
            .find(|(i, p)| {
                self.layout.hit_allowed(*i)
                    && p.hit(lx, ly)
                    && (self.doc.pointer_enter_handler(p.id).is_some()
                        || self.doc.pointer_leave_handler(p.id).is_some())
            })
            .map(|(_, p)| p.id);
        if target != self.hovered {
            if let Some(old) = self.hovered
                && let Some(h) = self.doc.pointer_leave_handler(old)
            {
                h();
            }
            if let Some(new) = target
                && let Some(h) = self.doc.pointer_enter_handler(new)
            {
                h();
            }
            self.hovered = target;
        }
        // cursor 属性:最上层设了 cursor 的节点决定光标
        let icon = self
            .layout
            .placed
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, p)| {
                if !self.layout.hit_allowed(i) || !p.hit(lx, ly) {
                    return None;
                }
                self.doc
                    .read(|inner| inner.nodes.get(p.id).and_then(|n| n.style.cursor))
            })
            .map(|c| match c {
                sv_ui::Cursor::Pointer => winit::window::CursorIcon::Pointer,
                sv_ui::Cursor::Text => winit::window::CursorIcon::Text,
                sv_ui::Cursor::Grab => winit::window::CursorIcon::Grab,
                sv_ui::Cursor::NotAllowed => winit::window::CursorIcon::NotAllowed,
                sv_ui::Cursor::Default => winit::window::CursorIcon::Default,
            })
            .unwrap_or(winit::window::CursorIcon::Default);
        if let Some(ws) = &self.win {
            ws.window.set_cursor(winit::window::Cursor::Icon(icon));
        }
    }

    /// 焦点 ↔ IME 会话同步:焦点在 accepts_text 节点上才开 IME
    /// (Masonry `accepts_text_input` 语义;开关有系统成本,镜像位去重)
    fn sync_ime(&mut self) {
        let Some(ws) = &self.win else { return };
        let want = self.doc.focused().is_some_and(|id| {
            self.doc
                .read(|inner| inner.nodes.get(id).is_some_and(|n| n.accepts_text))
        });
        if want != self.ime_allowed {
            ws.window.set_ime_allowed(want);
            self.ime_allowed = want;
        }
    }

    fn click(&mut self) {
        let Some(ws) = &self.win else { return };
        let scale = ws.window.scale_factor();
        let (lx, ly) = (
            (self.cursor.0 / scale) as f32,
            (self.cursor.1 / scale) as f32,
        );
        if let Some(id) = self.layout.hit_click(&self.doc, lx, ly)
            && let Some(handler) = self.doc.click_handler(id)
        {
            // 回调里写 signal → effect 改树 → on_mutate → request_redraw
            handler();
        }
    }
}

/// arboard 剪贴板(懒建:首次用到才建实例;失败静默降级为无剪贴板)
#[derive(Default)]
struct ShellClipboard(Option<arboard::Clipboard>);

impl ShellClipboard {
    fn ensure(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.0.is_none() {
            self.0 = arboard::Clipboard::new().ok();
        }
        self.0.as_mut()
    }
}

impl sv_ui::Clipboard for ShellClipboard {
    fn get_text(&mut self) -> Option<String> {
        self.ensure()?.get_text().ok()
    }
    fn set_text(&mut self, text: &str) {
        if let Some(c) = self.ensure() {
            let _ = c.set_text(text.to_string());
        }
    }
}

/// 读系统剪贴板文本(业务按钮用;run_app 外调用需先 `sv_ui::set_clipboard`)
pub fn clipboard_text() -> Option<String> {
    sv_ui::clipboard_get()
}

/// 写系统剪贴板文本
pub fn set_clipboard_text(text: &str) {
    sv_ui::clipboard_set(text);
}

/// winit 键盘事件 → sv-ui 自有 [`sv_ui::KeyEvent`](ADR-4:事件类型归 sv-ui,
/// 鸿蒙 XComponent 端喂同一类型)。v0 裁剪面 ~20 具名键 + `Char` 兜底,
/// 漏配键返回 None 静默丢弃
fn map_key(
    logical: &WinitKey,
    text: Option<&str>,
    repeat: bool,
    mods: ModifiersState,
) -> Option<sv_ui::KeyEvent> {
    use sv_ui::Key;
    let key = match logical {
        WinitKey::Named(n) => match n {
            NamedKey::Enter => Key::Enter,
            NamedKey::Tab => Key::Tab,
            NamedKey::Escape => Key::Escape,
            NamedKey::Backspace => Key::Backspace,
            NamedKey::Delete => Key::Delete,
            NamedKey::Space => Key::Space,
            NamedKey::ArrowUp => Key::ArrowUp,
            NamedKey::ArrowDown => Key::ArrowDown,
            NamedKey::ArrowLeft => Key::ArrowLeft,
            NamedKey::ArrowRight => Key::ArrowRight,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            NamedKey::F1 => Key::F(1),
            NamedKey::F2 => Key::F(2),
            NamedKey::F3 => Key::F(3),
            NamedKey::F4 => Key::F(4),
            NamedKey::F5 => Key::F(5),
            NamedKey::F6 => Key::F(6),
            NamedKey::F7 => Key::F(7),
            NamedKey::F8 => Key::F(8),
            NamedKey::F9 => Key::F(9),
            NamedKey::F10 => Key::F(10),
            NamedKey::F11 => Key::F(11),
            NamedKey::F12 => Key::F(12),
            _ => return None,
        },
        WinitKey::Character(s) => {
            let mut chars = s.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None; // 多字符 IME 序列走 Ime 事件(R1 第 2 步)
            }
            Key::Char(c)
        }
        _ => return None,
    };
    let mods = sv_ui::Mods {
        ctrl: mods.control_key(),
        shift: mods.shift_key(),
        alt: mods.alt_key(),
        meta: mods.super_key(),
    };
    let mut e = sv_ui::KeyEvent::new(key, mods).with_repeat(repeat);
    if let Some(t) = text {
        e = e.with_text(t);
    }
    Some(e)
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.win.is_some() {
            return;
        }
        // AccessKit 要求:adapter 必须在窗口首次可见前创建 → 先隐身建窗
        let attrs = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(480.0, 400.0))
            .with_visible(false);
        // 起不来就体面退出:记下错误、退事件循环,由 run_app 返给调用方
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => return self.abort(event_loop, ShellError::Window(e.to_string())),
        };
        let access = accesskit_winit::Adapter::with_event_loop_proxy(
            event_loop,
            &window,
            self.proxy.clone(),
        );
        window.set_visible(true);
        let presenter = match self.backend {
            #[cfg(feature = "backend-vello")]
            Backend::Vello => {
                let size = window.inner_size();
                match vello_backend::VelloWin::new(window.clone(), size.width, size.height) {
                    Ok(vw) => Presenter::Vello(vw),
                    Err(e) => {
                        eprintln!("sv-shell: vello 初始化失败({e}),回退 CPU 渲染后端");
                        self.backend = Backend::Cpu;
                        cpu_presenter(&window)
                    }
                }
            }
            Backend::Cpu => cpu_presenter(&window),
        };
        let presenter = match presenter {
            Ok(p) => p,
            Err(e) => return self.abort(event_loop, e),
        };

        // 帧对齐(ADR-6):**先于 build**开启——建树期的 effect 首跑是创建时
        // 同步执行(ADR-1),不走队列,不受影响;之后一切写入攒到帧前统一冲刷。
        // 一次事件里连写 N 个 state 只重绘一帧,effect 写树与布局/绘制严格有序
        {
            let w = window.clone();
            sv_reactive::set_frame_scheduler(move || w.request_redraw());
        }

        // 首次 resumed 时才构建 UI(此后 signal → 树 → 重绘的链路开始工作)
        if let Some(build) = self.build.take() {
            let doc = self.doc.clone();
            let (_, scope) = create_root(move || build(&doc, doc.root()));
            self._scope = Some(scope);
        }
        let w = window.clone();
        self.doc.set_on_mutate(move || w.request_redraw());
        let proxy = self.proxy.clone();
        sv_ui::tasks::set_waker(move || {
            let _ = proxy.send_event(UserEvent::Wake);
        });

        self.win = Some(WinState {
            window,
            presenter,
            access,
        });
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Wake => {
                // 后台任务完成的唤醒:立即套用并请求重绘
                sv_ui::tasks::pump();
                if let Some(ws) = &self.win {
                    ws.window.request_redraw();
                }
            }
            UserEvent::Access(ev) => match ev.window_event {
                accesskit_winit::WindowEvent::InitialTreeRequested => {
                    // 懒激活:AT 首次请求时才建全量语义树
                    self.push_access_tree();
                }
                accesskit_winit::WindowEvent::ActionRequested(req) => {
                    if req.target_tree == accesskit::TreeId::ROOT
                        && a11y::dispatch_action(&self.doc, req.action, req.target_node)
                    {
                        // 树可能因动作变化(焦点/点击),下一帧节拍推送
                        if let Some(ws) = &self.win {
                            ws.window.request_redraw();
                        }
                    }
                }
                accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
            },
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // 每个 winit 事件先过 AccessKit 适配器(官方要求的接线形态)
        if let Some(ws) = &mut self.win {
            ws.access.process_event(&ws.window, &event);
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.paint(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                if let Some(ws) = &self.win {
                    ws.window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                // 滚动条拖动优先于文本拖选(两者不会同时进入)
                if let Some((id, grab)) = self.drag_scroll
                    && let Some(ws) = &self.win
                {
                    let ly = (position.y / ws.window.scale_factor()) as f32;
                    if let Some(off) =
                        scrollbar_drag_offset(&self.layout.scroll_areas, id, ly, grab)
                    {
                        let (sx, _) = self.doc.scroll_of(id);
                        self.doc.set_scroll(id, sx, off);
                    }
                    return;
                }
                // 拖选:按住左键在输入框上移动 = 扩选(anchor 停在按下处)。
                // 指针未捕获,拖出控件外仍跟随——与桌面文本框一致
                if let Some((id, p)) = self.drag_input
                    && let Some(ws) = &self.win
                {
                    let sf = ws.window.scale_factor();
                    let (lx, ly) = ((position.x / sf) as f32, (position.y / sf) as f32);
                    let byte = input_caret_at(&self.doc, &p, lx, ly);
                    self.doc.set_caret(id, byte, true);
                }
                self.update_hover();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let Some(ws) = &self.win else { return };
                let scale = ws.window.scale_factor();
                let (lx, ly) = (
                    (self.cursor.0 / scale) as f32,
                    (self.cursor.1 / scale) as f32,
                );
                // 行滚 ≈ 40 逻辑 px;触摸板 PixelDelta 直通(设备已平滑)
                const LINE_PX: f32 = 40.0;
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => (x * LINE_PX, y * LINE_PX),
                    winit::event::MouseScrollDelta::PixelDelta(p) => {
                        ((p.x / scale) as f32, (p.y / scale) as f32)
                    }
                };
                let size = ws.window.inner_size();
                let layout = layout_full_cached(
                    &self.doc,
                    size.width as f32 / scale as f32,
                    size.height as f32 / scale as f32,
                );
                // winit 正值 = 内容向右/下移 → offset 减
                if route_wheel(
                    &self.doc,
                    &layout.placed,
                    &layout.scroll_areas,
                    lx,
                    ly,
                    -dx,
                    -dy,
                )
                .is_some()
                {
                    // 滚动后指针下的内容变了,重派发悬停
                    self.update_hover();
                }
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                // is_synthetic:X11 窗口获焦时合成的按下事件,不过滤会误触发
                if is_synthetic {
                    return;
                }
                let released = event.state == ElementState::Released;
                if let Some(e) = map_key(
                    &event.logical_key,
                    event.text.as_deref(),
                    event.repeat,
                    self.mods,
                )
                .map(|e| if released { e.released() } else { e })
                {
                    // 路由(冒泡/编辑/导航/激活/快捷键)在 sv-ui,离屏可测。
                    // 没人消费才轮到几何相关的编辑动作 —— 多行输入的上下行
                    // 移动要知道视觉行在哪,那是排版的产物,模型层不该猜
                    if !sv_ui::dispatch_key(&self.doc, &e) {
                        self.route_line_move(&e);
                    }
                    // Tab/Esc 可能移焦进出输入框
                    self.sync_ime();
                }
            }
            WindowEvent::Ime(ime) => {
                // 预编辑期间 winit 抑制 KeyboardInput,与编辑段无竞争
                if let Some(id) = self.doc.focused()
                    && self
                        .doc
                        .read(|inner| inner.nodes.get(id).is_some_and(|n| n.accepts_text))
                {
                    let ev = match ime {
                        winit::event::Ime::Enabled => sv_ui::ImeEvent::Enabled,
                        winit::event::Ime::Preedit(s, range) => sv_ui::ImeEvent::Preedit(s, range),
                        winit::event::Ime::Commit(s) => sv_ui::ImeEvent::Commit(s),
                        winit::event::Ime::Disabled => sv_ui::ImeEvent::Disabled,
                    };
                    sv_ui::handle_ime(&self.doc, id, ev);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                // :active / 按压回调:先派发 pointer_down,再派发点击
                let Some(ws) = &self.win else { return };
                let scale = ws.window.scale_factor();
                let (lx, ly) = (
                    (self.cursor.0 / scale) as f32,
                    (self.cursor.1 / scale) as f32,
                );
                // 弹层关闭手势(调研 25 O2):点最上层自动关闭弹层之外 →
                // dismiss;OnClickOutside 吞掉该次点击
                if overlay_click_gate(&self.doc, &self.layout, lx, ly) {
                    self.sync_ime();
                    return;
                }
                // 滚动条 thumb 优先(S4):它是 shell 合成绘制的,不在场景树里,
                // 所以必须在命中树之前拦一道,否则会穿透去点底下的内容
                if let Some((id, grab)) =
                    scrollbar_grab(&self.doc, &self.layout.scroll_areas, lx, ly)
                {
                    self.drag_scroll = Some((id, grab));
                    return;
                }
                self.pressed = self
                    .layout
                    .placed
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(i, p)| {
                        self.layout.hit_allowed(*i)
                            && p.hit(lx, ly)
                            && self.doc.pointer_down_handler(p.id).is_some()
                    })
                    .map(|(_, p)| p.id);
                if let Some(id) = self.pressed
                    && let Some(h) = self.doc.pointer_down_handler(id)
                {
                    h();
                }
                // 点击设焦(Slint focus-on-click 同款);
                // 空白区点击不清焦点(桌面惯例)
                if let Some(p) = self
                    .layout
                    .placed
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(i, p)| {
                        self.layout.hit_allowed(*i) && p.hit(lx, ly) && self.doc.focusable(p.id)
                    })
                    .map(|(_, p)| *p)
                {
                    self.doc.focus(p.id);
                    // 输入框:点击处换算字节偏移定光标;连击升级为选词/全选
                    if self.doc.read(|inner| {
                        inner
                            .nodes
                            .get(p.id)
                            .is_some_and(|n| n.kind == sv_ui::ElementKind::TextInput)
                    }) {
                        let byte = input_caret_at(&self.doc, &p, lx, ly);
                        match self.click_streak() {
                            1 => {
                                self.doc.set_caret(p.id, byte, false);
                                // 按下即进入拖选(移动时扩选,松开结束)
                                self.drag_input = Some((p.id, p));
                            }
                            2 => self.doc.select_word_at(p.id, byte),
                            _ => {
                                let len = self.doc.input_value(p.id).map(|v| v.len()).unwrap_or(0);
                                self.doc.select_range(p.id, 0, len);
                            }
                        }
                    }
                }
                self.sync_ime();
                self.click();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.drag_input = None;
                self.drag_scroll = None;
                if let Some(id) = self.pressed.take()
                    && let Some(h) = self.doc.pointer_up_handler(id)
                {
                    h();
                }
            }
            _ => {}
        }
    }
}

/// 开窗运行一个 sv 应用。`build` 在窗口就绪后执行一次,之后一切更新由 signal 驱动
pub fn run_app(
    title: &str,
    build: impl FnOnce(&Doc, ViewId) + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| ShellError::EventLoop(e.to_string()))?;
    let proxy = event_loop.create_proxy();
    // 系统剪贴板接入编辑内核(Ctrl+C/X/V;测试路径注入假实现,互不相扰)
    sv_ui::set_clipboard(ShellClipboard::default());
    let mut app = App {
        title: title.to_string(),
        doc: Doc::new(),
        build: Some(Box::new(build)),
        _scope: None,
        backend: select_backend(),
        win: None,
        layout: Layout::default(),
        cursor: (0.0, 0.0),
        hovered: None,
        pressed: None,
        drag_input: None,
        drag_scroll: None,
        last_click: None,
        mods: ModifiersState::empty(),
        ime_allowed: false,
        epoch: std::time::Instant::now(),
        last_frame_key: None,
        show_fps: std::env::var("SV_SHOW_FPS").is_ok_and(|v| v == "1"),
        fps_frames: 0,
        fps_t0: std::time::Instant::now(),
        proxy,
        frame_drops: 0,
        a11y: a11y::A11yCache::default(),
        fatal: None,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| ShellError::EventLoop(e.to_string()))?;
    // 循环里发生的致命错误(建窗/建 surface)在这里冒泡,而不是当场 panic
    match app.fatal.take() {
        Some(e) => Err(e.into()),
        None => Ok(()),
    }
}

/// 离屏渲染一帧到 PNG(验证渲染栈用,不开窗)。返回构建好的 Doc 供进一步断言
pub fn render_to_png(
    build: impl FnOnce(&Doc, ViewId),
    phys_w: u32,
    phys_h: u32,
    scale: f32,
    path: &str,
) -> Result<Doc, Box<dyn std::error::Error>> {
    let doc = Doc::new();
    let (_, _scope) = create_root(|| build(&doc, doc.root()));
    render_doc_to_png(&doc, phys_w, phys_h, scale, path)?;
    Ok(doc)
}

/// 把一个(可能已被交互修改过的)Doc 离屏渲染成 PNG
pub fn render_doc_to_png(
    doc: &Doc,
    phys_w: u32,
    phys_h: u32,
    scale: f32,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (pixmap, _) = render_frame(doc, phys_w, phys_h, scale);
    pixmap.save_png(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv_reactive::state;
    use sv_ui::bind_text;

    /// 离屏整链路:signal → 树 → 布局 → 命中测试模拟点击 → 精准更新 → 重新布局
    #[test]
    fn offscreen_click_roundtrip() {
        let doc = Doc::new();
        let count = state(0);
        let (_, _scope) = create_root(|| {
            let root = doc.root();
            doc.update_style(root, |s| {
                s.padding = 16.0.into();
                s.gap = 8.0;
            });
            let label = doc.create_text("");
            doc.append(root, label);
            bind_text(&doc, label, move || format!("Count: {}", count.get()));
            let btn = doc.create_button("+1");
            doc.append(root, btn);
            doc.update_style(btn, |s| {
                s.padding = 8.0.into();
                s.bg = Some(sv_ui::Color::rgb(60, 120, 255));
            });
            doc.set_on_click(btn, move || count.update(|c| *c += 1));
        });

        let placed = layout_tree(&doc, 480.0, 400.0);
        // 按钮在 label 下方:padding 16 + label 高度 + gap 之后
        let btn_place = placed
            .iter()
            .find(|p| doc.click_handler(p.id).is_some())
            .copied()
            .expect("应能布局出可点击的按钮");
        let (cx, cy) = (
            btn_place.rect.x + btn_place.rect.w / 2.0,
            btn_place.rect.y + btn_place.rect.h / 2.0,
        );
        let target = hit_click_target(&doc, &placed, cx, cy).expect("按钮中心应命中");
        doc.click_handler(target).unwrap()();
        assert!(
            doc.dump().contains("Count: 1"),
            "点击后应精准更新:\n{}",
            doc.dump()
        );
    }

    /// 可切换后端的支点验证:同一 paint_tree 对记录型后端产出稳定命令流
    /// (金样测试:零像素、零 GPU;未来新后端先对拍这条命令流)
    #[test]
    fn recording_painter_golden() {
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let card = doc.create_view();
            doc.append(doc.root(), card);
            doc.update_style(card, |s| {
                s.bg = Some(sv_ui::Color::rgb(240, 240, 246));
                s.corner_radius = 10.0;
                s.border = Some(sv_ui::Border {
                    width: 2.0,
                    color: sv_ui::Color::rgb(0, 0, 128),
                });
                s.padding = 8.0.into();
            });
            let t = doc.create_text("你好");
            doc.append(card, t);
        });
        let placed = layout_tree(&doc, 200.0, 100.0);
        let mut rec = RecordingPainter::default();
        paint_tree(&doc, &placed, &mut rec, 1.0);
        // 期望命令流:卡片底色填充 → 边框描边 → 文本字形
        assert!(
            matches!(rec.cmds[0], PaintCmd::FillRect { radius: 10, .. }),
            "第一条应为卡片填充:{:?}",
            rec.cmds
        );
        assert!(
            matches!(rec.cmds[1], PaintCmd::StrokeRect { width: 2, .. }),
            "第二条应为边框:{:?}",
            rec.cmds
        );
        assert!(
            matches!(rec.cmds[2], PaintCmd::Glyphs { count: 2, .. }),
            "第三条应为两枚字形:{:?}",
            rec.cmds
        );
        // 同一命令流可稳定重放(缓存/对拍的前提)
        let mut rec2 = RecordingPainter::default();
        paint_tree(&doc, &placed, &mut rec2, 1.0);
        assert_eq!(rec.cmds, rec2.cmds, "命令流应确定性可重放");
    }

    /// GPU/CPU 离屏对拍:同一 Doc 各自出图,非白像素数应同数量级
    /// (字体光栅路径不同,不做逐像素对比;无 GPU adapter 时跳过)
    #[cfg(feature = "backend-vello")]
    #[test]
    fn vello_offscreen_parity() {
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let card = doc.create_view();
            doc.append(doc.root(), card);
            doc.update_style(card, |s| {
                s.bg = Some(sv_ui::Color::rgb(240, 240, 246));
                s.corner_radius = 10.0;
                s.border = Some(sv_ui::Border {
                    width: 2.0,
                    color: sv_ui::Color::rgb(0, 0, 128),
                });
                s.padding = 12.0.into();
            });
            let t = doc.create_text("你好,vello!");
            doc.append(card, t);
        });

        let (w, h, scale) = (240u32, 120u32, 1.0f32);
        let Some(gpu) = render_frame_vello(&doc, w, h, scale) else {
            println!("跳过 vello_offscreen_parity:无可用 GPU adapter(回退路径已由 CPU 测试覆盖)");
            return;
        };
        assert_eq!(gpu.len(), (w * h * 4) as usize, "回读字节数应为 w*h*4");
        let (pixmap, _) = render_frame(&doc, w, h, scale);

        let non_white = |r: u8, g: u8, b: u8| r < 250 || g < 250 || b < 250;
        let gpu_count = gpu
            .chunks_exact(4)
            .filter(|p| non_white(p[0], p[1], p[2]))
            .count();
        let cpu_count = pixmap
            .pixels()
            .iter()
            .filter(|p| {
                let c = p.demultiply();
                non_white(c.red(), c.green(), c.blue())
            })
            .count();
        assert!(gpu_count > 0, "vello 应画出非白像素");
        assert!(cpu_count > 0, "CPU 应画出非白像素");
        let ratio = gpu_count as f64 / cpu_count as f64;
        println!("vello_offscreen_parity: gpu={gpu_count} cpu={cpu_count} ratio={ratio:.3}");
        assert!(
            (0.5..=2.0).contains(&ratio),
            "两后端非白像素数应同数量级:gpu={gpu_count} cpu={cpu_count} ratio={ratio:.3}"
        );
    }

    /// 离屏键盘整链路:Tab 移焦 → Enter 激活按钮 → signal 更新 → 树精准变更
    /// (复用 offscreen_click_roundtrip 模式,不开窗)
    #[test]
    fn offscreen_tab_enter_activates_button() {
        use sv_ui::{Key, KeyEvent, Mods, dispatch_key};
        let doc = Doc::new();
        let count = state(0);
        let (_, _scope) = create_root(|| {
            let label = doc.create_text("");
            doc.append(doc.root(), label);
            bind_text(&doc, label, move || format!("Count: {}", count.get()));
            let btn = doc.create_button("+1");
            doc.append(doc.root(), btn);
            doc.set_on_click(btn, move || count.update(|c| *c += 1));
        });

        // 合成键序:Tab(焦点落到按钮)→ Enter(激活)→ Shift+Tab(环绕仍是它)→ Space
        assert!(dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE)));
        assert!(doc.focused().is_some(), "Tab 应把焦点带到按钮");
        dispatch_key(&doc, &KeyEvent::new(Key::Enter, Mods::NONE));
        assert!(doc.dump().contains("Count: 1"), "\n{}", doc.dump());
        dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::SHIFT));
        dispatch_key(&doc, &KeyEvent::new(Key::Space, Mods::NONE));
        assert!(doc.dump().contains("Count: 2"), "\n{}", doc.dump());
    }

    /// winit → sv-ui 键映射:具名键、字符、修饰键、synthetic 丢弃面
    #[test]
    fn map_key_covers_v0_surface() {
        use winit::keyboard::{Key as WKey, NamedKey, SmolStr};
        let none = ModifiersState::empty();
        let e = map_key(&WKey::Named(NamedKey::Tab), None, false, none).unwrap();
        assert_eq!(e.key, sv_ui::Key::Tab);
        let e = map_key(&WKey::Named(NamedKey::F5), None, false, none).unwrap();
        assert_eq!(e.key, sv_ui::Key::F(5));
        let e = map_key(
            &WKey::Character(SmolStr::new("s")),
            Some("s"),
            false,
            ModifiersState::CONTROL,
        )
        .unwrap();
        assert_eq!(e.key, sv_ui::Key::Char('s'));
        assert!(e.mods.ctrl && !e.mods.shift);
        assert_eq!(e.text.as_deref(), Some("s"));
        // 漏配键静默丢弃
        assert!(map_key(&WKey::Named(NamedKey::CapsLock), None, false, none).is_none());
    }

    /// 焦点环金样:获焦节点绘制后应多出一条外扩 2px 的 StrokeRect,
    /// 失焦后命令流回到原样
    #[test]
    fn recording_painter_focus_ring_golden() {
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let btn = doc.create_button("确定");
            doc.append(doc.root(), btn);
            doc.update_style(btn, |s| {
                s.padding = 8.0.into();
                s.bg = Some(sv_ui::Color::rgb(60, 120, 255));
            });
        });
        let placed = layout_tree(&doc, 200.0, 100.0);
        let mut plain = RecordingPainter::default();
        paint_tree(&doc, &placed, &mut plain, 1.0);

        doc.focus_next(); // 焦点落到按钮
        let mut focused = RecordingPainter::default();
        paint_tree(&doc, &placed, &mut focused, 1.0);
        assert_eq!(
            focused.cmds.len(),
            plain.cmds.len() + 1,
            "获焦应只多一条焦点环命令"
        );
        let ring = focused.cmds.last().unwrap();
        let btn_place = placed.iter().find(|p| doc.focusable(p.id)).unwrap();
        match ring {
            PaintCmd::StrokeRect {
                x, y, w, h, width, ..
            } => {
                assert_eq!(*width, 2, "焦点环宽 2px");
                assert_eq!(*x, (btn_place.rect.x - 2.0) as i32, "外扩 2px");
                assert_eq!(*y, (btn_place.rect.y - 2.0) as i32);
                assert_eq!(*w, (btn_place.rect.w + 4.0) as i32);
                assert_eq!(*h, (btn_place.rect.h + 4.0) as i32);
            }
            other => panic!("最后一条应是焦点环 StrokeRect:{other:?}"),
        }

        doc.blur();
        let mut blurred = RecordingPainter::default();
        paint_tree(&doc, &placed, &mut blurred, 1.0);
        assert_eq!(blurred.cmds, plain.cmds, "失焦后命令流应回到无环原样");
    }

    /// TextInput 金样:焦点框命令流 = 默认底/边 → PushClip → Glyphs(值)→
    /// 光标矩形 → PopClip;失焦无光标;选区时多一条高亮矩形
    #[test]
    fn input_paint_golden() {
        use sv_ui::{Caret, EditOp, apply_edit};
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let input = doc.create_text_input();
            doc.append(doc.root(), input);
            doc.set_placeholder(input, "请输入");
        });
        let input = doc.read(|inner| inner.nodes[inner.root].children[0]);
        let placed = layout_tree(&doc, 480.0, 100.0);

        // 未获焦 + 空值:底/边 + 裁剪内 placeholder 字形,无光标
        let mut rec = RecordingPainter::default();
        paint_tree(&doc, &placed, &mut rec, 1.0);
        assert!(
            matches!(
                rec.cmds.as_slice(),
                [
                    PaintCmd::FillRect { .. },   // 默认底
                    PaintCmd::StrokeRect { .. }, // 默认边
                    PaintCmd::PushClip { .. },
                    PaintCmd::Glyphs { .. }, // placeholder
                    PaintCmd::PopClip,
                ]
            ),
            "未获焦空值命令流:{:?}",
            rec.cmds
        );

        // 获焦 + 键入 + 选区:多出选区矩形与光标,外加默认焦点环
        doc.focus(input);
        apply_edit(&doc, input, EditOp::InsertStr("你好".into()));
        apply_edit(&doc, input, EditOp::Move(Caret::Left, true)); // Shift+Left 选"好"
        let mut rec = RecordingPainter::default();
        paint_tree(&doc, &placed, &mut rec, 1.0);
        assert!(
            matches!(
                rec.cmds.as_slice(),
                [
                    PaintCmd::FillRect { .. },   // 默认底
                    PaintCmd::StrokeRect { .. }, // 默认边
                    PaintCmd::PushClip { .. },
                    PaintCmd::FillRect { .. }, // 选区高亮
                    PaintCmd::Glyphs { count: 2, .. },
                    PaintCmd::FillRect { .. }, // 光标
                    PaintCmd::PopClip,
                    PaintCmd::StrokeRect { width: 2, .. }, // 默认焦点环
                ]
            ),
            "获焦选区命令流:{:?}",
            rec.cmds
        );

        // 预编辑:组合文本上屏前可见(字形数=值2+预编辑2),带 2px 下划线
        sv_ui::handle_ime(
            &doc,
            input,
            sv_ui::ImeEvent::Preedit("shi".into(), Some((3, 3))),
        );
        let mut rec = RecordingPainter::default();
        paint_tree(&doc, &placed, &mut rec, 1.0);
        let glyphs: usize = rec
            .cmds
            .iter()
            .filter_map(|c| match c {
                PaintCmd::Glyphs { count, .. } => Some(*count),
                _ => None,
            })
            .sum();
        assert_eq!(glyphs, 5, "显示串应为 值(2) + 预编辑(shi=3):{:?}", rec.cmds);
    }

    /// 光标几何互逆:任意 char 边界 caret_x → caret_index_at 回到原位;
    /// caret_x 单调不减。P3 起两者都出自 Parley 的同一次单行排版
    #[test]
    fn caret_geometry_roundtrip() {
        let text = "a你b好c!";
        let px = 16.0;
        let mut last = -1.0f32;
        for (i, _) in text.char_indices().chain([(text.len(), ' ')]) {
            let x = caret_x(text, px, i);
            assert!(x >= last, "caret_x 应单调:{i}");
            last = x;
            assert_eq!(
                caret_index_at(text, px, x + 0.1),
                i,
                "caret_x({i}) 处点击应回到 {i}"
            );
        }
        // 远超行尾 → 末尾;负数 → 0
        assert_eq!(caret_index_at(text, px, 10_000.0), text.len());
        assert_eq!(caret_index_at(text, px, -5.0), 0);
        // 空串退化:光标恒 0
        assert_eq!(caret_x("", px, 0), 0.0);
        assert_eq!(caret_index_at("", px, 12.0), 0);
    }

    /// 光标 x 现在带 shaping:CJK/Latin 混排下不再是"逐字 advance 求和",
    /// 且与实际绘制的 glyph run 起点对齐(画/点同源的最小可观测证据)
    #[test]
    fn caret_x_follows_shaped_runs() {
        let text = "AV你好";
        let px = 32.0;
        // 末尾光标 == 整串排版宽(fallback 换字体后仍成立)
        let (w, _) = crate::text::measure(text, px, None);
        assert!(
            (caret_x(text, px, text.len()) - w).abs() < 0.5,
            "末尾光标应落在排版宽处:{} vs {w}",
            caret_x(text, px, text.len())
        );
        // 每个 CJK 字起点应与该处 shaping 出的字形 x 对齐(±0.5px)
        let runs = crate::text::shape(text, px, None, sv_ui::TextAlign::Left, 0.0, 0.0, 1.0);
        let xs: Vec<f32> = runs
            .iter()
            .flat_map(|r| r.glyphs.iter().map(|g| g.x))
            .collect();
        let cx = caret_x(text, px, 2); // "你" 起点
        assert!(
            xs.iter().any(|x| (x - cx).abs() < 0.5),
            "光标 x={cx} 应命中某个字形起点:{xs:?}"
        );
    }

    /// 选区矩形:覆盖选中区间、区间为空时无矩形
    #[test]
    fn selection_rects_cover_range() {
        let text = "hello 世界";
        let px = 16.0;
        assert!(selection_rects(text, px, 3, 3).is_empty(), "空选区无矩形");
        let rects = selection_rects(text, px, 0, text.len());
        assert!(!rects.is_empty());
        let (x0, w) = (rects[0].0, rects.iter().map(|r| r.2).sum::<f32>());
        let (tw, _) = crate::text::measure(text, px, None);
        assert!(
            x0 <= 0.5 && (w - tw).abs() < 1.0,
            "全选矩形应覆盖整串:{rects:?}"
        );
    }

    /// a11y 档 B:滚动与弹层语义。**只用树里确实存在的信息**——
    /// 可不可滚看 overflow,弹层角色看层与 modal 位,都不猜
    #[test]
    fn a11y_scroll_and_overlay_semantics() {
        use accesskit::{Action, Role};
        let (doc, container, _rows) = scroll_doc();
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let update = build_tree_update(&doc, &layout.placed, 1.0);
        let nid = |v: sv_ui::ViewId| accesskit::NodeId(sv_ui::view_id_ffi(v));
        let node = |v: sv_ui::ViewId| {
            update
                .nodes
                .iter()
                .find(|(id, _)| *id == nid(v))
                .map(|(_, n)| n.clone())
                .expect("语义树应含该节点")
        };

        let c = node(container);
        assert_eq!(c.role(), Role::ScrollView, "可滚容器应报 ScrollView");
        assert_eq!(c.scroll_y(), Some(0.0));
        assert!(c.supports_action(Action::ScrollDown), "AT 应能要求滚动");
        assert!(c.clips_children(), "裁剪容器应报 clips_children");

        // AT 请求滚动 → 走与滚轮同一条写入口
        assert!(dispatch_action(&doc, Action::ScrollDown, nid(container)));
        assert!(doc.scroll_of(container).1 > 0.0, "ScrollDown 应推动偏移");
        let before = doc.scroll_of(container).1;
        dispatch_action(&doc, Action::ScrollUp, nid(container));
        assert!(doc.scroll_of(container).1 < before, "ScrollUp 应回滚");
        // 钳到 0,不会变负
        for _ in 0..10 {
            dispatch_action(&doc, Action::ScrollUp, nid(container));
        }
        assert_eq!(doc.scroll_of(container).1, 0.0);

        // 多行输入报 MultilineTextInput(单行仍是 TextInput)
        let ta = doc.create_text_input();
        doc.append(doc.root(), ta);
        doc.set_multiline(ta, true, 3);
        let input = doc.create_text_input();
        doc.append(doc.root(), input);
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let update = build_tree_update(&doc, &layout.placed, 1.0);
        let role_of = |v: sv_ui::ViewId| {
            update
                .nodes
                .iter()
                .find(|(id, _)| *id == nid(v))
                .map(|(_, n)| n.role())
                .unwrap()
        };
        assert_eq!(role_of(ta), Role::MultilineTextInput);
        assert_eq!(role_of(input), Role::TextInput);

        // 弹层角色出自层与 modal 位(树里就有,不是猜的)
        use sv_ui::{Anchor, OverlayLayer, OverlayOpts, overlay_block};
        let doc2 = Doc::new();
        let (_, _scope) = create_root(|| {
            let modal = sv_reactive::state(true);
            overlay_block(
                &doc2,
                move || modal.get(),
                move || Anchor::WindowCenter,
                OverlayOpts {
                    modal: true,
                    ..OverlayOpts::default()
                },
                |d, root| {
                    let t = d.create_text("对话框");
                    d.append(root, t);
                },
            );
            let menu = sv_reactive::state(true);
            overlay_block(
                &doc2,
                move || menu.get(),
                move || Anchor::Point(10.0, 10.0),
                OverlayOpts::default(),
                |d, root| {
                    let t = d.create_text("菜单项");
                    d.append(root, t);
                },
            );
            let tip = sv_reactive::state(true);
            overlay_block(
                &doc2,
                move || tip.get(),
                move || Anchor::Point(20.0, 20.0),
                OverlayOpts {
                    layer: OverlayLayer::Tooltip,
                    ..OverlayOpts::default()
                },
                |d, root| {
                    let t = d.create_text("提示");
                    d.append(root, t);
                },
            );
        });
        let layout2 = layout_tree_full(&doc2, 480.0, 400.0);
        let update2 = build_tree_update(&doc2, &layout2.placed, 1.0);
        let roots: Vec<sv_ui::ViewId> =
            doc2.read(|inner| inner.overlays.iter().map(|e| e.root).collect());
        let role2 = |v: sv_ui::ViewId| {
            update2
                .nodes
                .iter()
                .find(|(id, _)| *id == nid(v))
                .map(|(_, n)| n.role())
                .expect("弹层根应在语义树里")
        };
        let roles: Vec<Role> = roots.iter().map(|r| role2(*r)).collect();
        assert!(
            roles.contains(&Role::Dialog),
            "modal 弹层应报 Dialog:{roles:?}"
        );
        assert!(
            roles.contains(&Role::Menu),
            "非模态 Popup 应报 Menu:{roles:?}"
        );
        assert!(
            roles.contains(&Role::Tooltip),
            "Tooltip 层应报 Tooltip:{roles:?}"
        );
    }

    /// P6 增量语义树(调研 24 §5 验收名 `a11y_update_only_dirty_nodes`):
    /// 首次全量,之后**只推内容真变了的节点**。全量推会让屏幕阅读器把整棵树
    /// 重扫一遍——一次键入本该只动一个节点
    #[test]
    fn a11y_update_only_dirty_nodes() {
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            for label in ["甲", "乙", "丙"] {
                let t = doc.create_text(label);
                doc.append(doc.root(), t);
            }
            let b = doc.create_button("确定");
            doc.append(doc.root(), b);
            doc.set_on_click(b, || {});
        });
        let placed = layout_tree(&doc, 480.0, 400.0);
        let ids: Vec<sv_ui::ViewId> = doc.read(|inner| inner.nodes[inner.root].children.clone());
        let nid = |v: sv_ui::ViewId| accesskit::NodeId(sv_ui::view_id_ffi(v));

        let mut cache = A11yCache::default();
        let first = incremental_tree_update(&mut cache, &doc, &placed, 1.0);
        assert_eq!(first.nodes.len(), 5, "首次应全量(root + 4 子)");
        assert!(first.tree.is_some(), "首次必须带 Tree");

        // 没变:零节点(focus 仍必填)
        let idle = incremental_tree_update(&mut cache, &doc, &placed, 1.0);
        assert!(
            idle.nodes.is_empty(),
            "无变化不该推任何节点:{:?}",
            idle.nodes.len()
        );
        assert!(idle.tree.is_none(), "Tree 只在首次带");
        assert_eq!(idle.focus, nid(doc.read(|i| i.root)));

        // 改一个文本 → 只推那一个
        doc.set_text(ids[1], "乙乙");
        let one = incremental_tree_update(&mut cache, &doc, &placed, 1.0);
        assert_eq!(one.nodes.len(), 1, "只该推改动的那个节点");
        assert_eq!(one.nodes[0].0, nid(ids[1]));
        assert_eq!(one.nodes[0].1.label(), Some("乙乙"));

        // 焦点变化不改节点内容 → 零节点,但 focus 字段跟上
        doc.focus(ids[3]);
        let f = incremental_tree_update(&mut cache, &doc, &placed, 1.0);
        assert!(f.nodes.is_empty(), "只改焦点不该重推节点");
        assert_eq!(f.focus, nid(ids[3]));

        // 删一个节点:父的 children 变了 → 推父(被删的不必显式上报)
        doc.remove(ids[0]);
        let placed2 = layout_tree(&doc, 480.0, 400.0);
        let del = incremental_tree_update(&mut cache, &doc, &placed2, 1.0);
        let root_id = nid(doc.read(|i| i.root));
        assert!(
            del.nodes.iter().any(|(id, _)| *id == root_id),
            "父节点 children 变了应被推:{:?}",
            del.nodes.len()
        );
        assert!(
            !del.nodes.iter().any(|(id, _)| *id == nid(ids[0])),
            "被删节点不该出现在更新里"
        );
    }

    /// ADR-6 帧对齐在场景树上的可观测契约(渲染壳侧,零窗口):
    /// 写 signal 只催帧、不改树;帧前 `tick`(paint 的第一段)后树才更新
    #[test]
    fn frame_aligned_tree_updates_only_at_frame() {
        let doc = Doc::new();
        let label = std::cell::RefCell::new(None);
        let (_, _scope) = create_root(|| {
            let text = sv_reactive::state(String::from("旧"));
            let t = doc.create_text("");
            doc.append(doc.root(), t);
            sv_ui::bind_text(&doc, t, move || text.get());
            *label.borrow_mut() = Some(text);
        });
        let text = label.borrow().unwrap();
        let node_text = || doc.read(|inner| inner.nodes[inner.root].children[0]);
        let read = || doc.read(|inner| inner.nodes[node_text()].text.clone());
        assert_eq!(read(), "旧");

        // 渲染壳在 resumed 里做的事:把"催帧"接到 request_redraw
        let wakes = std::rc::Rc::new(std::cell::Cell::new(0));
        let w = wakes.clone();
        sv_reactive::set_frame_scheduler(move || w.set(w.get() + 1));

        let v0 = doc.version();
        text.set("新".into());
        text.set("更新".into());
        assert_eq!(read(), "旧", "帧对齐下写入不该当场改树");
        assert_eq!(doc.version(), v0, "树没动,版本号也不该动");
        assert_eq!(wakes.get(), 1, "连写两次只催一帧");

        // paint() 的第一段
        sv_reactive::tick();
        assert_eq!(read(), "更新", "帧前冲刷后树才更新");
        assert!(doc.version() > v0, "树变了,版本号该 bump(触发重绘)");
        sv_reactive::clear_frame_scheduler();
    }

    /// 去 panic 门禁(R4,调研 25 §3.4):事件循环/呈现层(lib.rs 非测试段)
    /// 不许有 `expect`/`unwrap`/`panic!`——那里的失败全部来自运行时环境
    /// (窗口系统、合成器、GPU),必须走 [`ShellError`] 或丢帧降级。
    /// 其余文件保留的 expect 只允许是"自证不变量"(taffy 自建树取回、
    /// 字体注册表键):不依赖外部环境,触发即本仓库的 bug。
    #[test]
    fn shell_panics_are_invariants_only() {
        let src = include_str!("lib.rs");
        let non_test = src
            .split_once("#[cfg(test)]")
            .map(|(head, _)| head)
            .unwrap_or(src);
        let offenders: Vec<&str> = non_test
            .lines()
            .filter(|l| {
                let code = l.trim_start();
                !code.starts_with("//")
                    && !code.starts_with("///")
                    && (code.contains(".expect(")
                        || code.contains(".unwrap()")
                        || code.contains("panic!("))
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "呈现层不许 panic,请改走 ShellError 或丢帧降级:{offenders:#?}"
        );
        // 不变量白名单:剩下的 expect 集中在这两处,数量变化需连同理由一起改
        let invariant_files = [
            ("render.rs", include_str!("render.rs")),
            ("text.rs", include_str!("text.rs")),
        ];
        for (name, src) in invariant_files {
            let head = src
                .split_once("#[cfg(test)]")
                .map(|(h, _)| h)
                .unwrap_or(src);
            for l in head.lines() {
                let code = l.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                assert!(
                    !code.contains(".unwrap()") && !code.contains("panic!("),
                    "{name} 里只允许带解释的 expect(不变量),不允许裸 unwrap/panic!:{l}"
                );
            }
        }
    }

    /// 多行 textarea:高度按 rows 算、文本按内容宽折行、光标带行号、
    /// 点第二行命中第二行、↑/↓ 按视觉行走
    #[test]
    fn textarea_multiline_geometry() {
        use sv_ui::{EditOp, apply_edit};
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let ta = doc.create_text_input();
            doc.append(doc.root(), ta);
            doc.update_style(ta, |s| s.width = Some(160.0));
            doc.set_multiline(ta, true, 4);
        });
        let ta = doc.read(|inner| inner.nodes[inner.root].children[0]);
        let placed = layout_tree(&doc, 480.0, 400.0);
        let p = *placed.iter().find(|pp| pp.id == ta).unwrap();

        // 布局高 = 4 行(单行输入是 1 行高)
        let line_h = crate::text::line_height(16.0);
        assert!(
            (p.rect.h - line_h * 4.0).abs() < 1.0,
            "rows=4 的高应是 4 行:{} vs {}",
            p.rect.h,
            line_h * 4.0
        );

        // 硬换行 → 光标 y 落到第二行
        apply_edit(
            &doc,
            ta,
            EditOp::InsertStr(
                "ab
cd"
                .into(),
            ),
        );
        let (x1, y1, h1) = crate::text::caret_rect(
            "ab
cd",
            16.0,
            Some(160.0),
            2,
        );
        let (x2, y2, _) = crate::text::caret_rect(
            "ab
cd",
            16.0,
            Some(160.0),
            3,
        );
        assert!(y2 > y1, "换行后光标应下移一行:{y1} → {y2}");
        assert!(x2 < x1, "新行光标应回到行首:{x1} → {x2}");
        assert!(h1 > 0.0);

        // 点第二行:命中第二行的字节(不再恒落在第一行)
        let bw = 0.0;
        let text_y = p.rect.y + bw;
        let hit = input_caret_at(&doc, &p, p.rect.x + 1.0, text_y + y2 + h1 * 0.5);
        assert!(hit >= 3, "点第二行应命中换行符之后:{hit}");

        // ↑/↓ 按视觉行走
        doc.set_caret(ta, 4, false); // 第二行中间
        let up = input_caret_line_move(&doc, &p, false).expect("多行应支持上下移动");
        assert!(up <= 2, "↑ 应回到第一行:{up}");
        doc.set_caret(ta, up, false);
        let down = input_caret_line_move(&doc, &p, true).expect("多行应支持上下移动");
        assert!(down >= 3, "↓ 应回到第二行:{down}");

        // 单行输入不参与行移动(方向键留给导航段)
        let input = doc.create_text_input();
        doc.append(doc.root(), input);
        let placed = layout_tree(&doc, 480.0, 400.0);
        let pi = *placed.iter().find(|pp| pp.id == input).unwrap();
        assert!(input_caret_line_move(&doc, &pi, true).is_none());
    }

    /// 溢出输入框的点击命中:文本被光标跟随推左后,点右端应落在串尾附近
    /// (P3 前 `input_caret_at` 忽略滚移,长文本点哪都偏到左边)
    #[test]
    fn click_hits_scrolled_text() {
        use sv_ui::{Caret, EditOp, apply_edit};
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let input = doc.create_text_input();
            doc.append(doc.root(), input);
            doc.update_style(input, |s| s.width = Some(120.0));
        });
        let input = doc.read(|inner| inner.nodes[inner.root].children[0]);
        let text = "abcdefghijklmnopqrstuvwxyz0123456789";
        apply_edit(&doc, input, EditOp::InsertStr(text.into()));
        apply_edit(&doc, input, EditOp::Move(Caret::End, false));
        let placed = layout_tree(&doc, 480.0, 100.0);
        let p = *placed.iter().find(|p| p.id == input).unwrap();

        // 光标在末尾 → 已滚到最右;点内容区右缘应命中串尾附近
        let right = p.rect.x + p.rect.w - 4.0;
        let hit = input_caret_at(&doc, &p, right, p.rect.y);
        assert!(
            hit >= text.len() - 2,
            "右缘点击应命中串尾附近,得到 {hit}/{}",
            text.len()
        );
        // 回到串首(滚移归零)后,同一 x 命中的是可见窗口内的字符,不再是串尾
        apply_edit(&doc, input, EditOp::Move(Caret::Home, false));
        let hit0 = input_caret_at(&doc, &p, right, p.rect.y);
        assert!(hit0 < hit, "滚移归零后同一坐标应命中更靠前的字符:{hit0}");
    }

    /// IME 光标区域上报:随键入右移、随 HiDPI 缩放、无焦点输入框时为 None
    #[test]
    fn ime_cursor_area_tracks_caret() {
        use sv_ui::{EditOp, apply_edit};
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let input = doc.create_text_input();
            doc.append(doc.root(), input);
        });
        let input = doc.read(|inner| inner.nodes[inner.root].children[0]);
        let placed = layout_tree(&doc, 480.0, 100.0);

        assert!(
            ime_caret_rect(&doc, &placed, 1.0).is_none(),
            "无焦点时不应上报光标区域"
        );
        doc.focus(input);
        let (x0, _, _, h) = ime_caret_rect(&doc, &placed, 1.0).unwrap();
        apply_edit(&doc, input, EditOp::InsertStr("你好".into()));
        let (x1, _, _, _) = ime_caret_rect(&doc, &placed, 1.0).unwrap();
        assert!(x1 > x0, "键入后光标区域应右移:{x0} → {x1}");
        assert!(h > 0.0);
        // HiDPI:2x 缩放下 x 坐标同步放大
        let (x2, _, _, _) = ime_caret_rect(&doc, &placed, 2.0).unwrap();
        assert!(
            (x2 - x1 * 2.0).abs() < 2.0,
            "2x 缩放光标 x 应约为 1x 的两倍:{x1} vs {x2}"
        );
    }

    /// 建一个 200 高的滚动容器,内放 10 行 40 高 → 内容 400,max_y = 240
    /// (padding 20×2 → 内区 160)
    fn scroll_doc() -> (Doc, sv_ui::ViewId, Vec<sv_ui::ViewId>) {
        let doc = Doc::new();
        let mut rows = Vec::new();
        let container = doc.create_view();
        doc.append(doc.root(), container);
        doc.update_style(container, |s| {
            s.overflow = sv_ui::Overflow::Scroll;
            s.width = Some(300.0);
            s.height = Some(200.0);
            s.padding = 20.0.into();
        });
        for i in 0..10 {
            let row = doc.create_view();
            doc.update_style(row, |s| s.height = Some(40.0));
            let t = doc.create_text(&format!("行 {i}"));
            doc.append(row, t);
            doc.append(container, row);
            rows.push(row);
        }
        (doc, container, rows)
    }

    /// R2 档 B:overflow 按轴拆分。"横向裁掉、纵向滚"是最常见的组合,
    /// 过去只有一个 overflow 字段,横向也会跟着可滚
    #[test]
    fn overflow_axis_split() {
        let doc = Doc::new();
        let container = doc.create_view();
        doc.append(doc.root(), container);
        doc.update_style(container, |s| {
            s.overflow = sv_ui::Overflow::Scroll; // 纵向滚
            s.overflow_x = sv_ui::Overflow::Hidden; // 横向裁
            s.width = Some(200.0);
            s.height = Some(100.0);
        });
        // 内容比容器又宽又高
        let wide = doc.create_view();
        doc.update_style(wide, |s| {
            s.width = Some(600.0);
            s.height = Some(400.0);
        });
        doc.append(container, wide);

        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let a = layout
            .scroll_areas
            .iter()
            .find(|a| a.id == container)
            .expect("纵向可滚 → 应有滚动区");
        assert!(a.max.1 > 0.0, "纵向应可滚:{:?}", a.max);
        assert_eq!(a.max.0, 0.0, "横向 hidden 不该给出滚动范围:{:?}", a.max);

        // 滚轮:纵向消费,横向因为无范围而不动
        let (cx, cy) = (a.viewport.x + 10.0, a.viewport.y + 10.0);
        assert!(
            route_wheel(
                &doc,
                &layout.placed,
                &layout.scroll_areas,
                cx,
                cy,
                0.0,
                50.0
            )
            .is_some(),
            "纵向滚轮应被消费"
        );
        let before = doc.scroll_of(container).0;
        route_wheel(
            &doc,
            &layout.placed,
            &layout.scroll_areas,
            cx,
            cy,
            50.0,
            0.0,
        );
        assert_eq!(doc.scroll_of(container).0, before, "横向 hidden 不该被推动");

        // 两轴都 hidden:只裁不滚,连滚动区都不该产生
        doc.update_style(container, |s| s.overflow = sv_ui::Overflow::Hidden);
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        assert!(
            !layout.scroll_areas.iter().any(|a| a.id == container),
            "都 hidden 不该有滚动区"
        );
        assert!(
            layout
                .placed
                .iter()
                .any(|p| p.id == wide && p.clip.is_some()),
            "hidden 仍然要裁剪"
        );
    }

    #[test]
    fn scroll_offset_shifts_children_and_clamps() {
        let (doc, container, rows) = scroll_doc();
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let a = layout
            .scroll_areas
            .iter()
            .find(|a| a.id == container)
            .expect("应产出滚动区元数据");
        // taffy/CSS 口径:scrollable overflow 含容器 padding(400 + 20×2)
        assert_eq!(a.content.1, 440.0, "内容高 = 10 行 × 40 + padding 40");
        assert_eq!(a.max.1, 240.0, "max = scroll_height = 440 − 200");

        let row0_y = |placed: &[Placed]| {
            placed
                .iter()
                .find(|p| p.id == rows[0])
                .map(|p| p.rect.y)
                .unwrap()
        };
        let y_before = row0_y(&layout.placed);
        doc.set_scroll(container, 0.0, 100.0);
        let layout2 = layout_tree_full(&doc, 480.0, 400.0);
        assert_eq!(
            row0_y(&layout2.placed),
            y_before - 100.0,
            "滚动 100 应把子行上移 100"
        );
        // 布局期钳制:超出 max 的 offset 不产生额外平移
        doc.set_scroll(container, 0.0, 9999.0);
        let layout3 = layout_tree_full(&doc, 480.0, 400.0);
        assert_eq!(row0_y(&layout3.placed), y_before - 240.0);
        // 子行携带裁剪矩形 = 容器 border-box
        let p = layout3.placed.iter().find(|p| p.id == rows[0]).unwrap();
        assert!(p.clip.is_some() && p.clip_depth == 1);
    }

    #[test]
    fn clipped_child_not_hit() {
        let (doc, container, rows) = scroll_doc();
        for r in &rows {
            doc.set_on_click(*r, || {});
        }
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        // 行 8 在视口(高 200)外
        let p8 = layout.placed.iter().find(|p| p.id == rows[8]).unwrap();
        let (cx, cy) = (p8.rect.x + p8.rect.w / 2.0, p8.rect.y + p8.rect.h / 2.0);
        assert!(
            hit_click_target(&doc, &layout.placed, cx, cy).is_none()
                || hit_click_target(&doc, &layout.placed, cx, cy) != Some(rows[8]),
            "视口外的行不应命中"
        );
        // 滚到底后行 8 进入视口,可命中
        doc.set_scroll(container, 0.0, 240.0);
        let layout2 = layout_tree_full(&doc, 480.0, 400.0);
        let p8 = layout2.placed.iter().find(|p| p.id == rows[8]).unwrap();
        let (cx, cy) = (p8.rect.x + p8.rect.w / 2.0, p8.rect.y + p8.rect.h / 2.0);
        assert_eq!(
            hit_click_target(&doc, &layout2.placed, cx, cy),
            Some(rows[8]),
            "滚入视口后应可命中"
        );
    }

    #[test]
    fn wheel_routes_and_chains_to_ancestor_at_edge() {
        // 外层滚动容器嵌内层滚动容器
        let doc = Doc::new();
        let outer = doc.create_view();
        doc.append(doc.root(), outer);
        doc.update_style(outer, |s| {
            s.overflow = sv_ui::Overflow::Scroll;
            s.width = Some(300.0);
            s.height = Some(200.0);
        });
        let inner = doc.create_view();
        doc.append(outer, inner);
        doc.update_style(inner, |s| {
            s.overflow = sv_ui::Overflow::Scroll;
            s.width = Some(300.0);
            s.height = Some(100.0);
        });
        let filler = doc.create_view();
        doc.update_style(filler, |s| {
            s.width = Some(100.0);
            s.height = Some(150.0); // inner 内容 150 > 视口 100 → max 50
        });
        doc.append(inner, filler);
        let tall = doc.create_view();
        doc.update_style(tall, |s| {
            s.width = Some(100.0);
            s.height = Some(400.0); // outer 内容 100+400 > 200 → 可滚
        });
        doc.append(outer, tall);

        let layout = layout_tree_full(&doc, 480.0, 400.0);
        // 指针悬在内层上:滚轮先消费内层
        let consumed = route_wheel(
            &doc,
            &layout.placed,
            &layout.scroll_areas,
            50.0,
            50.0,
            0.0,
            30.0,
        );
        assert_eq!(consumed, Some(inner));
        assert_eq!(doc.scroll_of(inner).1, 30.0);
        // 再滚 100:内层只剩 20 到边界,但 v0 语义为整段交给内层直到贴边,
        // 下一次滚动才链到外层
        route_wheel(
            &doc,
            &layout.placed,
            &layout.scroll_areas,
            50.0,
            50.0,
            0.0,
            100.0,
        );
        assert_eq!(doc.scroll_of(inner).1, 50.0, "内层应贴边");
        let consumed = route_wheel(
            &doc,
            &layout.placed,
            &layout.scroll_areas,
            50.0,
            50.0,
            0.0,
            40.0,
        );
        assert_eq!(consumed, Some(outer), "内层到边后应链到外层祖先");
        assert_eq!(doc.scroll_of(outer).1, 40.0);
        // 反向同理:外层在顶、内层也在顶时,向上滚不消费
        doc.set_scroll(inner, 0.0, 0.0);
        doc.set_scroll(outer, 0.0, 0.0);
        let consumed = route_wheel(
            &doc,
            &layout.placed,
            &layout.scroll_areas,
            50.0,
            50.0,
            0.0,
            -10.0,
        );
        assert_eq!(consumed, None, "全部贴顶时向上滚不应有消费者");
    }

    /// 裁剪金样:滚动容器命令流 = 容器底色 → PushClip → 子行内容 → PopClip
    /// → 滚动条 thumb;CPU 像素级:视口外行的文字不落像素
    #[test]
    fn scroll_clip_golden_and_cpu_pixels() {
        let (doc, container, _rows) = scroll_doc();
        doc.update_style(container, |s| s.bg = Some(sv_ui::Color::rgb(240, 240, 246)));
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let mut rec = RecordingPainter::default();
        paint_tree(&doc, &layout.placed, &mut rec, 1.0);
        paint_scrollbars(&doc, &layout.scroll_areas, &mut rec, 1.0);
        let pushes = rec
            .cmds
            .iter()
            .filter(|c| matches!(c, PaintCmd::PushClip { .. }))
            .count();
        let pops = rec
            .cmds
            .iter()
            .filter(|c| matches!(c, PaintCmd::PopClip))
            .count();
        assert_eq!(pushes, 1, "一个滚动容器一层裁剪:{:?}", rec.cmds);
        assert_eq!(pushes, pops, "push/pop 应配平");
        assert!(
            matches!(rec.cmds.last(), Some(PaintCmd::FillRect { .. })),
            "最后应是滚动条 thumb:{:?}",
            rec.cmds.last()
        );

        // CPU 像素:容器(y∈[0,200))外不应有文字像素(白底)
        let (pixmap, _) = render_frame(&doc, 480, 400, 1.0);
        let below_viewport_nonwhite = pixmap
            .pixels()
            .iter()
            .enumerate()
            .filter(|(i, p)| {
                let y = i / 480;
                y > 210 && p.red() < 250 // 210 留滚动条/抗锯齿余量
            })
            .count();
        assert_eq!(
            below_viewport_nonwhite, 0,
            "视口下方不应有内容像素(裁剪生效)"
        );
    }

    #[test]
    fn scrollbar_thumb_geometry() {
        // 内容未溢出 → 无 thumb
        assert!(scrollbar_thumb(200.0, 200.0, 100.0, 0.0).is_none());
        // 视口 200 / 内容 400 → thumb 半轨道;顶部在 0,滚到底贴底
        let (pos, len) = scrollbar_thumb(200.0, 200.0, 400.0, 0.0).unwrap();
        assert_eq!((pos, len), (0.0, 100.0));
        let (pos, len) = scrollbar_thumb(200.0, 200.0, 400.0, 200.0).unwrap();
        assert_eq!(pos + len, 200.0, "滚到底 thumb 应贴轨道底");
        // 超长内容:thumb 不小于 24
        let (_, len) = scrollbar_thumb(200.0, 200.0, 100_000.0, 0.0).unwrap();
        assert_eq!(len, 24.0);
    }

    /// S4 验收:滚动条 thumb 拖动(命中 → 按比例反算 offset → 钳制)。
    /// thumb 是 shell 合成绘制的,不在场景树里,所以命中要单独一条通道
    #[test]
    fn scrollbar_thumb_drag() {
        let (doc, container, _rows) = scroll_doc();
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let a = *layout
            .scroll_areas
            .iter()
            .find(|a| a.id == container)
            .expect("应有滚动区");

        // 条在容器右缘内侧;点内容区中间不该抓到 thumb
        let miss = scrollbar_grab(
            &doc,
            &layout.scroll_areas,
            a.viewport.x + 10.0,
            a.viewport.y + 10.0,
        );
        assert!(miss.is_none(), "内容区不该抓到滚动条");

        // 抓 thumb 顶部附近
        let bar_x = a.viewport.x + a.viewport.w - 5.0;
        let (id, grab) = scrollbar_grab(&doc, &layout.scroll_areas, bar_x, a.viewport.y + 4.0)
            .expect("thumb 顶部应可抓");
        assert_eq!(id, container);
        assert!(grab >= 0.0);

        // 往下拖:offset 增大且钳在 max 内
        let off = scrollbar_drag_offset(&layout.scroll_areas, id, a.viewport.y + 60.0, grab)
            .expect("拖动应给出 offset");
        assert!(
            off > 0.0 && off <= a.max.1,
            "offset 应在 (0, max]:{off}/{}",
            a.max.1
        );

        // 拖过头钳到底;往回拖过头钳到 0
        let bottom =
            scrollbar_drag_offset(&layout.scroll_areas, id, a.viewport.y + 9999.0, grab).unwrap();
        assert_eq!(bottom, a.max.1, "拖过底应钳到 max");
        let top =
            scrollbar_drag_offset(&layout.scroll_areas, id, a.viewport.y - 9999.0, grab).unwrap();
        assert_eq!(top, 0.0, "拖过顶应钳到 0");

        // 抓点偏移被记住:同一指针位置、不同抓点 → 不同 offset
        let (_, grab_mid) = scrollbar_grab(&doc, &layout.scroll_areas, bar_x, a.viewport.y + 20.0)
            .expect("thumb 中部应可抓");
        let off_mid =
            scrollbar_drag_offset(&layout.scroll_areas, id, a.viewport.y + 60.0, grab_mid).unwrap();
        assert!(off_mid < off, "抓得靠下,同一指针位置应滚得更少(thumb 不跳)");
    }

    /// S5 验收:virtual_list + virtual_scroll 由 route_wheel 真实输入驱动
    #[test]
    fn virtual_list_driven_by_wheel() {
        use sv_reactive::state;
        let doc = Doc::new();
        let offset = state(0usize);
        let container = doc.create_view();
        doc.append(doc.root(), container);
        doc.update_style(container, |s| {
            s.overflow = sv_ui::Overflow::Scroll;
            s.width = Some(300.0);
            s.height = Some(200.0);
        });
        let (_, _scope) = create_root(|| {
            sv_ui::virtual_list(
                &doc,
                container,
                || 100_000usize,
                offset,
                10,
                |i| format!("行 {i}"),
                |doc, parent, slot, _| {
                    let t = doc.create_text("");
                    doc.append(parent, t);
                    bind_text(doc, t, move || slot.get().unwrap_or_default());
                },
            );
            sv_ui::virtual_scroll(&doc, container, || 100_000usize, 20.0, offset);
        });
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let a = layout
            .scroll_areas
            .iter()
            .find(|a| a.id == container)
            .unwrap();
        assert_eq!(
            a.content.1, 2_000_000.0,
            "虚拟内容高 = 100k 行 × 20(content_override)"
        );
        // 滚轮滚 4000px → 行号 200,槽位内容更新
        route_wheel(
            &doc,
            &layout.placed,
            &layout.scroll_areas,
            50.0,
            50.0,
            0.0,
            4000.0,
        );
        assert_eq!(offset.get(), 200, "像素域应换算到行域");
        assert!(doc.dump().contains("行 200"), "\n{}", doc.dump());
        assert!(!doc.dump().contains("行 0\n"), "旧首行应被替换");
        // 节点数不随滚动增长(虚拟化不变量)
        assert!(doc.read(|inner| inner.nodes.len()) < 30);
    }

    // -----------------------------------------------------------------------
    // 调研 23:taffy + 折行 + flex 验收
    // -----------------------------------------------------------------------

    #[test]
    fn text_wraps_at_container_width() {
        // parley 度量:限宽 = 最宽单词 + 2px,恰好每词一行
        let px = 16.0;
        let one_word_w = ["hello", "world", "again"]
            .iter()
            .map(|s| crate::text::measure(s, px, None).0)
            .fold(0.0f32, f32::max);
        let lines = crate::text::line_ranges("hello world again", px, Some(one_word_w + 2.0));
        assert_eq!(lines.len(), 3, "三个词应折成三行:{lines:?}");
        let (w, h) = crate::text::measure("hello world again", px, Some(one_word_w + 2.0));
        assert!(w <= one_word_w + 2.0);
        assert!(
            h > crate::text::measure("hello", px, None).1 * 2.0,
            "三行高"
        );
        // 单行模式(NoWrap)不折
        assert_eq!(
            crate::text::line_ranges("hello world again", px, None).len(),
            1
        );
    }

    #[test]
    fn cjk_wraps_without_spaces_and_respects_punct() {
        let px = 16.0;
        let two_cjk_w = crate::text::measure("中文", px, None).0;
        // 无空格的 CJK 应能逐字断行
        let lines = crate::text::line_ranges("中文换行测试", px, Some(two_cjk_w + 0.5));
        assert_eq!(lines.len(), 3, "六字限宽两字应三行:{lines:?}");
        // 标点禁则:句号不能落行首("。"跟随前字)
        let text = "你好。世界";
        let lines = crate::text::line_ranges(text, px, Some(two_cjk_w + 0.5));
        for r in &lines {
            assert!(
                !text[r.clone()].starts_with('。'),
                "行首不应出现句号:{lines:?}"
            );
        }
    }

    #[test]
    fn long_token_force_breaks() {
        let px = 16.0;
        let w4 = crate::text::measure("abcd", px, None).0;
        let lines =
            crate::text::line_ranges("https://example.com/very/long/url", px, Some(w4 + 0.5));
        assert!(lines.len() > 3, "超长不可断段应按字符强制断:{lines:?}");
        let (w, _) = crate::text::measure("https://example.com/very/long/url", px, Some(w4 + 0.5));
        assert!(w <= w4 + 1.0, "强制断后行宽不应超限:{w}");
    }

    /// P1 验收(调研 24):fallback 混排——CJK 与 Latin 由 fontique 按
    /// script 选字体,多字体 run 经 P0 载体发射;不应出现 .notdef(id=0)
    #[test]
    fn mixed_cjk_fallback_no_notdef() {
        let runs = crate::text::shape(
            "Hello你好",
            16.0,
            None,
            sv_ui::TextAlign::Left,
            0.0,
            0.0,
            1.0,
        );
        assert!(!runs.is_empty());
        let mut keys = std::collections::HashSet::new();
        for run in &runs {
            // 回归卫兵:fontique Blob id 从 0 起,注册键必须避开内置字体
            // 的保留键 0(撞键 = Latin 全员错字,实测踩过)
            assert_ne!(run.font.key, 0, "fontique 字体键不得与内置键 0 相撞");
            keys.insert(run.font.key);
            for g in &run.glyphs {
                assert_ne!(g.id, 0, "fallback 后不应出现 .notdef 方框");
            }
        }
        // Windows CI(默认 sans-serif=Segoe UI,CJK fallback=微软雅黑)应出双字体;
        // 其它平台字体配置不可控,只验 notdef
        #[cfg(target_os = "windows")]
        assert!(
            keys.len() >= 2,
            "CJK/Latin 混排应触发 fallback 多字体:{keys:?}"
        );
    }

    /// 两趟测量协议:MaxContent(不限宽)= 单行固有宽;
    /// Definite(限宽)= 折行后高度增长
    #[test]
    fn wrapped_measure_two_pass() {
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let container = doc.create_view();
            doc.append(doc.root(), container);
            doc.update_style(container, |s| s.width = Some(120.0));
            let t = doc.create_text("这是一段需要在一百二十像素宽的容器里折成多行的长文本");
            doc.append(container, t);
        });
        let placed = layout_tree(&doc, 480.0, 400.0);
        let text_p = placed.last().unwrap();
        let (_, single_h) = crate::text::measure("字", 16.0, None);
        assert!(
            text_p.rect.h > single_h * 2.5,
            "长文本在 120px 容器内应折成多行(高 {} vs 单行 {single_h})",
            text_p.rect.h
        );
        assert!(text_p.rect.w <= 120.0 + 0.5, "折行后宽不超容器");
    }

    #[test]
    fn flex_grow_and_justify_and_align() {
        let doc = Doc::new();
        let container = doc.create_view();
        doc.append(doc.root(), container);
        doc.update_style(container, |s| {
            s.direction = sv_ui::Direction::Row;
            s.width = Some(300.0);
            s.height = Some(100.0);
        });
        let fixed = doc.create_view();
        doc.update_style(fixed, |s| {
            s.width = Some(100.0);
            s.height = Some(20.0);
        });
        doc.append(container, fixed);
        let grower = doc.create_view();
        doc.update_style(grower, |s| {
            s.flex_grow = 1.0;
            s.height = Some(20.0);
        });
        doc.append(container, grower);

        let placed = layout_tree(&doc, 480.0, 400.0);
        let rect = |id| placed.iter().find(|p| p.id == id).map(|p| p.rect).unwrap();
        assert_eq!(rect(grower).w, 200.0, "flex-grow 应吃掉剩余 200px");

        // justify-content: space-between —— 两个定宽子项分居两端
        doc.update_style(container, |s| {
            s.justify_content = sv_ui::JustifyContent::SpaceBetween;
        });
        doc.update_style(grower, |s| {
            s.flex_grow = 0.0;
            s.width = Some(50.0);
            s.height = Some(20.0);
        });
        let placed = layout_tree(&doc, 480.0, 400.0);
        let rect = |id| placed.iter().find(|p| p.id == id).map(|p| p.rect).unwrap();
        assert_eq!(rect(fixed).x, 0.0);
        assert_eq!(
            rect(grower).x + rect(grower).w,
            300.0,
            "space-between 尾项应贴容器右缘"
        );

        // align-items: center —— 交叉轴居中
        doc.update_style(container, |s| {
            s.align_items = sv_ui::AlignItems::Center;
        });
        let placed = layout_tree(&doc, 480.0, 400.0);
        let r = placed.iter().find(|p| p.id == fixed).unwrap().rect;
        assert_eq!(r.y, 40.0, "100 高容器内 20 高子项应居中于 y=40");
    }

    /// gap 语义钉住(调研 23 §2.2):nowrap 单轴与旧引擎等价;
    /// wrap 后交叉轴 gap 也生效(taffy/CSS 双轴语义,与旧引擎不同,记录在案)
    #[test]
    fn gap_cross_axis_semantics_pinned() {
        let doc = Doc::new();
        let container = doc.create_view();
        doc.append(doc.root(), container);
        doc.update_style(container, |s| {
            s.direction = sv_ui::Direction::Row;
            s.gap = 10.0;
            s.width = Some(110.0);
            s.flex_wrap = sv_ui::FlexWrap::Wrap;
        });
        let mut items = Vec::new();
        for _ in 0..2 {
            let it = doc.create_view();
            doc.update_style(it, |s| {
                s.width = Some(60.0);
                s.height = Some(20.0);
            });
            doc.append(container, it);
            items.push(it);
        }
        let placed = layout_tree(&doc, 480.0, 400.0);
        let r = |id| placed.iter().find(|p| p.id == id).map(|p| p.rect).unwrap();
        // 60+10+60 > 110 → 换行;第二项 y = 20(首行高)+ 10(交叉轴 gap)
        assert_eq!(r(items[1]).y, 30.0, "wrap 后交叉轴 gap 应生效(双轴语义)");
        assert_eq!(r(items[1]).x, 0.0);
    }

    /// 调研 23 §2.6 触发线探针:30k 节点全量档 build+layout 的 2ms 触发线
    /// **已确认越线**(2026-07-18 实测 release ≈130–160ms:taffy 裸 compute
    /// ~45ms + 叶子 measure ~70ms + build ~30ms)→ 按预案启动"Blitz 式低层
    /// trait 增量布局"升级路径(档 B 欠账,2–3 人周);全量大树的档 A 出路
    /// 仍是虚拟化(1M 虚拟化 p99=5.56ms 达标,ADR-9)。
    /// 这里只设灾难性回归上限;`cargo test --release -- layout_30k --nocapture` 看真值
    #[test]
    fn layout_30k_full_tree_budget_probe() {
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            for _ in 0..6000 {
                let row = doc.create_view();
                doc.update_style(row, |s| s.direction = sv_ui::Direction::Row);
                doc.append(doc.root(), row);
                for j in 0..5 {
                    let t = doc.create_text(if j % 2 == 0 { "标签" } else { "value" });
                    doc.append(row, t);
                }
            }
        });
        let t = std::time::Instant::now();
        let _ = layout_tree_full(&doc, 1920.0, 1080.0);
        let cold = t.elapsed().as_secs_f64() * 1000.0;
        let t = std::time::Instant::now();
        let layout = layout_tree_full(&doc, 1920.0, 1080.0);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!(
            "[probe] 30k 全量 build+layout:冷 {cold:.2}ms / 热 {ms:.2}ms(2ms 触发线已越,增量升级列档 B)"
        );
        assert!(layout.placed.len() > 30_000);
        if cfg!(not(debug_assertions)) {
            assert!(
                ms <= 500.0,
                "30k 全量布局 {ms:.2}ms 出现灾难性回归(基线 ~130–160ms)"
            );
        }
    }

    /// AccessKit 语义树金样(调研 24 P4:零窗口零平台):
    /// role/名称/bounds/焦点/动作面逐项断言
    #[test]
    fn a11y_roles_names_bounds_golden() {
        use accesskit::{Action, Role, Toggled};
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let t = doc.create_text("标题");
            doc.append(doc.root(), t);
            let b = doc.create_button("确定");
            doc.append(doc.root(), b);
            doc.set_on_click(b, || {});
            let c = doc.create_checkbox();
            doc.set_text(c, "同意");
            doc.set_checked(c, true);
            doc.append(doc.root(), c);
            let i = doc.create_text_input();
            doc.set_placeholder(i, "请输入");
            doc.append(doc.root(), i);
            doc.set_accessible_label(i, "用户名");
            doc.focus(b);
        });
        let placed = layout_tree(&doc, 480.0, 400.0);
        let update = build_tree_update(&doc, &placed, 2.0);

        let ids: Vec<sv_ui::ViewId> = doc.read(|inner| inner.nodes[inner.root].children.clone());
        let node_of = |vid: sv_ui::ViewId| {
            let nid = accesskit::NodeId(sv_ui::view_id_ffi(vid));
            update
                .nodes
                .iter()
                .find(|(id, _)| *id == nid)
                .map(|(_, n)| n.clone())
                .expect("语义树应含该节点")
        };

        let text = node_of(ids[0]);
        assert_eq!(text.role(), Role::Label);
        assert_eq!(text.label(), Some("标题"));

        let btn = node_of(ids[1]);
        assert_eq!(btn.role(), Role::Button);
        assert_eq!(btn.label(), Some("确定"));
        assert!(btn.supports_action(Action::Click), "按钮应可点");
        assert!(btn.supports_action(Action::Focus), "按钮应可聚焦");

        let cb = node_of(ids[2]);
        assert_eq!(cb.role(), Role::CheckBox);
        assert_eq!(cb.toggled(), Some(Toggled::True), "勾选态应直通");

        let input = node_of(ids[3]);
        assert_eq!(input.role(), Role::TextInput);
        assert_eq!(input.label(), Some("用户名"), "aria-label 应覆盖占位符");

        // bounds:逻辑 rect × scale(与命中测试同源)
        let p = placed.iter().find(|p| p.id == ids[1]).unwrap();
        let b = btn.bounds().expect("按钮应有 bounds");
        assert_eq!(b.x0, (p.rect.x * 2.0) as f64);
        assert_eq!(b.y1, ((p.rect.y + p.rect.h) * 2.0) as f64);

        // focus 必填:当前焦点在按钮
        assert_eq!(update.focus, accesskit::NodeId(sv_ui::view_id_ffi(ids[1])));
        // 根与树信息
        assert!(update.tree.is_some());
    }

    /// AccessKit 动作回派(P5 纯逻辑面):Click/Focus/Blur → 场景树
    #[test]
    fn a11y_action_dispatch_roundtrip() {
        use accesskit::Action;
        use std::cell::RefCell;
        use std::rc::Rc;
        let doc = Doc::new();
        let clicks: Rc<RefCell<u32>> = Default::default();
        let c = clicks.clone();
        let btn = doc.create_button("加");
        doc.append(doc.root(), btn);
        doc.set_on_click(btn, move || *c.borrow_mut() += 1);

        let nid = accesskit::NodeId(sv_ui::view_id_ffi(btn));
        assert!(dispatch_action(&doc, Action::Click, nid));
        assert_eq!(*clicks.borrow(), 1, "Click 动作应走点击回调");
        assert!(dispatch_action(&doc, Action::Focus, nid));
        assert_eq!(doc.focused(), Some(btn), "Focus 动作应走焦点链");
        assert!(dispatch_action(&doc, Action::Blur, nid));
        assert_eq!(doc.focused(), None);
        // 已删节点的动作静默失败(世代键防复用)
        doc.remove(btn);
        assert!(!dispatch_action(&doc, Action::Click, nid));
    }

    // -----------------------------------------------------------------------
    // 调研 25:弹层体系验收(O1 锚定 / O2 关闭 / O3 模态 / O5 tooltip)
    // -----------------------------------------------------------------------

    use sv_reactive::state as ov_state;
    use sv_ui::{Anchor, CloseBehavior, OverlayLayer, OverlayOpts, Side, overlay_block};

    /// O1:弹层 Placed 追加在基础层之后(后画即在上)、dump 可见、命中优先弹层
    #[test]
    fn overlay_paints_after_base_and_hit_prefers_it() {
        let doc = Doc::new();
        let open = ov_state(true);
        let pop_clicks = std::rc::Rc::new(std::cell::RefCell::new(0));
        let pc = pop_clicks.clone();
        let (_, _scope) = create_root(|| {
            let btn = doc.create_button("底部按钮");
            doc.append(doc.root(), btn);
            doc.update_style(btn, |s| {
                s.width = Some(200.0);
                s.height = Some(60.0);
            });
            doc.set_on_click(btn, || {});
            overlay_block(
                &doc,
                move || open.get(),
                move || Anchor::Point(10.0, 10.0),
                OverlayOpts::default(),
                move |d, root| {
                    d.update_style(root, |s| {
                        s.width = Some(150.0);
                        s.height = Some(80.0);
                    });
                    let ob = d.create_button("弹层按钮");
                    d.append(root, ob);
                    let pc = pc.clone();
                    d.set_on_click(ob, move || *pc.borrow_mut() += 1);
                },
            );
        });
        assert!(
            doc.dump().contains("== overlay"),
            "dump 应含弹层段:{}",
            doc.dump()
        );
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let r = *layout.overlay_regions.first().expect("应有弹层区间");
        assert!(
            r.start > 0 && r.end == layout.placed.len(),
            "弹层应追加在末尾"
        );
        // 弹层按钮盖在底部按钮上方 → 重叠处优先命中弹层
        let hit = layout.hit_click(&doc, 30.0, 30.0).expect("应命中");
        assert!(
            layout.placed[r.start..r.end].iter().any(|p| p.id == hit),
            "重叠处应优先命中弹层"
        );
        // 关闭后区间消失、底部按钮恢复命中
        open.set(false);
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        assert!(layout.overlay_regions.is_empty());
        assert!(layout.hit_click(&doc, 30.0, 30.0).is_some());
    }

    /// O1:Below 锚定放不下时翻转 Above,最终 clamp 进窗口
    #[test]
    fn anchor_below_flips_when_clipped() {
        let doc = Doc::new();
        let open = ov_state(true);
        let anchor_btn = std::rc::Rc::new(std::cell::RefCell::new(None));
        let ab = anchor_btn.clone();
        let (_, _scope) = create_root(|| {
            let spacer = doc.create_view();
            doc.update_style(spacer, |s| s.height = Some(350.0));
            doc.append(doc.root(), spacer);
            let btn = doc.create_button("锚点");
            doc.append(doc.root(), btn);
            *ab.borrow_mut() = Some(btn);
            overlay_block(
                &doc,
                move || open.get(),
                move || Anchor::Node {
                    id: btn,
                    side: Side::Below,
                    gap: 4.0,
                },
                OverlayOpts::default(),
                |d, root| {
                    d.update_style(root, |s| {
                        s.width = Some(100.0);
                        s.height = Some(120.0);
                    });
                },
            );
        });
        // 窗口 400 高,锚点 y≈350,下方剩 ~30 放不下 120 → 应翻到上方
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let r = layout.overlay_regions[0];
        let overlay_rect = layout.placed[r.start].rect;
        let anchor_rect = layout
            .placed
            .iter()
            .find(|p| p.id == anchor_btn.borrow().unwrap())
            .unwrap()
            .rect;
        assert!(
            overlay_rect.y + overlay_rect.h <= anchor_rect.y + 0.5,
            "放不下应翻转到锚点上方:overlay={overlay_rect:?} anchor={anchor_rect:?}"
        );
        assert!(overlay_rect.y >= 0.0, "翻转后仍应在窗口内");
    }

    /// O2:点弹层外 dismiss(OnClickOutside 吞点击);Esc LIFO 逐层关
    #[test]
    fn click_outside_and_esc_dismiss_lifo() {
        use sv_ui::{Key, KeyEvent, Mods, dispatch_key};
        let doc = Doc::new();
        let open1 = ov_state(true);
        let open2 = ov_state(true);
        let (_, _scope) = create_root(|| {
            overlay_block(
                &doc,
                move || open1.get(),
                move || Anchor::Point(10.0, 10.0),
                OverlayOpts {
                    close: CloseBehavior::OnClickOutside,
                    on_dismiss: Some(std::rc::Rc::new(move || open1.set(false))),
                    ..Default::default()
                },
                |d, root| {
                    d.update_style(root, |s| {
                        s.width = Some(100.0);
                        s.height = Some(100.0);
                    });
                },
            );
            overlay_block(
                &doc,
                move || open2.get(),
                move || Anchor::Point(200.0, 10.0),
                OverlayOpts {
                    close: CloseBehavior::OnClickOutside,
                    on_dismiss: Some(std::rc::Rc::new(move || open2.set(false))),
                    ..Default::default()
                },
                |d, root| {
                    d.update_style(root, |s| {
                        s.width = Some(100.0);
                        s.height = Some(100.0);
                    });
                },
            );
        });
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        assert_eq!(layout.overlay_regions.len(), 2);
        // 点在最上层(弹层 2)之内:不 dismiss、不吞
        assert!(!overlay_click_gate(&doc, &layout, 250.0, 50.0));
        assert!(open2.get());
        // 点在其外:最上层先关,点击被吞
        assert!(overlay_click_gate(&doc, &layout, 400.0, 300.0));
        assert!(!open2.get(), "最上层应先关");
        assert!(open1.get(), "下层不受影响");
        // Esc:LIFO 关剩下的弹层 1
        dispatch_key(&doc, &KeyEvent::new(Key::Escape, Mods::NONE));
        assert!(!open1.get(), "Esc 应关最上层剩余弹层");
    }

    /// O3:modal 阻断底层命中;Tab 环限定弹层内;关闭恢复原焦点
    #[test]
    fn modal_blocks_base_and_traps_focus() {
        use sv_ui::{Key, KeyEvent, Mods, dispatch_key};
        let doc = Doc::new();
        let open = ov_state(false);
        let base_btn = std::rc::Rc::new(std::cell::RefCell::new(None));
        let bb = base_btn.clone();
        let (_, _scope) = create_root(|| {
            let btn = doc.create_button("底部");
            doc.append(doc.root(), btn);
            doc.set_on_click(btn, || {});
            *bb.borrow_mut() = Some(btn);
            overlay_block(
                &doc,
                move || open.get(),
                move || Anchor::WindowCenter,
                OverlayOpts {
                    modal: true,
                    close: CloseBehavior::None,
                    ..Default::default()
                },
                |d, root| {
                    d.update_style(root, |s| {
                        s.width = Some(200.0);
                        s.height = Some(100.0);
                    });
                    let ok = d.create_button("确定");
                    d.append(root, ok);
                    let cancel = d.create_button("取消");
                    d.append(root, cancel);
                },
            );
        });
        let base = base_btn.borrow().unwrap();
        doc.focus(base);
        // 打开 modal:焦点应移入弹层
        open.set(true);
        let focused = doc.focused().expect("modal 打开应带焦点");
        assert_ne!(focused, base, "焦点应离开底层");
        // Tab 环限定在弹层内(两个按钮来回)
        let mut seen = std::collections::HashSet::new();
        for _ in 0..4 {
            seen.insert(doc.focused().unwrap());
            dispatch_key(&doc, &KeyEvent::new(Key::Tab, Mods::NONE));
        }
        assert_eq!(seen.len(), 2, "Tab 环应只在弹层两按钮间循环");
        assert!(!seen.contains(&base), "底层按钮不应进入 Tab 环");
        // 命中阻断:底层按钮不可点
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let base_rect = layout.placed.iter().find(|p| p.id == base).unwrap().rect;
        assert!(
            layout
                .hit_click(&doc, base_rect.x + 1.0, base_rect.y + 1.0)
                .is_none(),
            "modal 之下整体不可命中"
        );
        // 关闭恢复原焦点
        open.set(false);
        assert_eq!(doc.focused(), Some(base), "关闭应恢复原焦点");
    }

    /// O5:tooltip 悬停延时(代数计数防错位)+ Tooltip 层不可命中
    #[test]
    fn tooltip_delay_and_never_hit() {
        let doc = Doc::new();
        let target = std::rc::Rc::new(std::cell::RefCell::new(None));
        let t = target.clone();
        let (_, _scope) = create_root(|| {
            let btn = doc.create_button("悬停我");
            doc.append(doc.root(), btn);
            *t.borrow_mut() = Some(btn);
            sv_ui::tooltip(&doc, btn, 10, |d, root| {
                let txt = d.create_text("提示内容");
                d.append(root, txt);
            });
        });
        let btn = target.borrow().unwrap();
        // 悬停 → 延时后出现
        doc.pointer_enter_handler(btn).unwrap()();
        std::thread::sleep(std::time::Duration::from_millis(80));
        sv_ui::tasks::pump();
        assert!(
            doc.dump().contains("提示内容"),
            "延时后应出现:{}",
            doc.dump()
        );
        let layout = layout_tree_full(&doc, 480.0, 400.0);
        let r = layout
            .overlay_regions
            .iter()
            .find(|r| r.layer == OverlayLayer::Tooltip)
            .expect("应有 Tooltip 区间");
        for i in r.start..r.end {
            assert!(!layout.hit_allowed(i), "Tooltip 恒不可命中");
        }
        // 离开即隐;延时期间离开(代数变化)不应再打开
        doc.pointer_leave_handler(btn).unwrap()();
        assert!(!doc.dump().contains("提示内容"));
        doc.pointer_enter_handler(btn).unwrap()();
        doc.pointer_leave_handler(btn).unwrap()();
        std::thread::sleep(std::time::Duration::from_millis(80));
        sv_ui::tasks::pump();
        assert!(
            !doc.dump().contains("提示内容"),
            "代数已变,过期延时不应再打开"
        );
    }

    #[test]
    fn render_frame_produces_pixels() {
        let doc = Doc::new();
        let (_, _scope) = create_root(|| {
            let t = doc.create_text("你好,sv!");
            doc.append(doc.root(), t);
        });
        let (pixmap, placed) = render_frame(&doc, 200, 100, 1.0);
        assert!(!placed.is_empty());
        // 画了黑色文字,不应全是白色像素
        let non_white = pixmap.pixels().iter().filter(|p| p.red() < 250).count();
        assert!(non_white > 10, "文字应该被光栅化出来,non_white={non_white}");
    }
}
