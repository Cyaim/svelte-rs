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

mod font;
mod paint;
mod render;
#[cfg(feature = "backend-vello")]
mod vello_backend;

pub use paint::{
    GlyphKey, GlyphPos, PaintCmd, Painter, PainterCaps, RecordingPainter, TinySkiaPainter,
};
pub use render::{
    Placed, Rect, caret_index_at, caret_x, hit_click_target, ime_caret_rect, input_caret_at,
    layout_tree, paint_tree, render_frame,
};
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

/// 窗口呈现器:CPU(softbuffer)或 GPU(vello/wgpu)
enum Presenter {
    Cpu {
        surface: softbuffer::Surface<Arc<Window>, Arc<Window>>,
        _context: softbuffer::Context<Arc<Window>>,
    },
    #[cfg(feature = "backend-vello")]
    Vello(vello_backend::VelloWin),
}

fn cpu_presenter(window: &Arc<Window>) -> Presenter {
    let context = softbuffer::Context::new(window.clone()).expect("sv-shell: 创建绘图上下文失败");
    let surface =
        softbuffer::Surface::new(&context, window.clone()).expect("sv-shell: 创建 surface 失败");
    Presenter::Cpu {
        surface,
        _context: context,
    }
}

struct WinState {
    window: Arc<Window>,
    presenter: Presenter,
}

struct App {
    title: String,
    doc: Doc,
    build: Option<Box<dyn FnOnce(&Doc, ViewId)>>,
    _scope: Option<RootHandle>,
    backend: Backend,
    win: Option<WinState>,
    placed: Vec<Placed>,
    cursor: (f64, f64),
    hovered: Option<ViewId>,
    pressed: Option<ViewId>,
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
    proxy: winit::event_loop::EventLoopProxy<()>,
}

impl App {
    fn paint(&mut self) {
        // 先套用后台任务完成值、推进动画,再渲染本帧
        sv_ui::tasks::pump();
        let now_ms = self.epoch.elapsed().as_secs_f64() * 1000.0;
        let animating = sv_ui::anim::pump(now_ms);
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
                let (pixmap, placed) = render_frame(&self.doc, size.width, size.height, scale);
                self.placed = placed;

                let (Some(w), Some(h)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                else {
                    return;
                };
                surface.resize(w, h).expect("sv-shell: surface resize 失败");
                let mut buffer = surface.buffer_mut().expect("sv-shell: 取帧缓冲失败");
                for (dst, src) in buffer.iter_mut().zip(pixmap.pixels()) {
                    let c = src.demultiply();
                    *dst = ((c.red() as u32) << 16) | ((c.green() as u32) << 8) | (c.blue() as u32);
                }
                buffer.present().expect("sv-shell: present 失败");
            }
            #[cfg(feature = "backend-vello")]
            Presenter::Vello(vw) => {
                vw.resize(size.width, size.height);
                // 静止帧(FPS 模式下仍在跑):跳过场景重编码,只重呈现
                let (placed, presented) = vw.render_cached(&self.doc, scale, unchanged);
                self.placed = placed;
                if !presented {
                    // surface 过期/被遮挡等:下一帧重试(与 vello 官方示例一致)
                    ws.window.request_redraw();
                }
            }
        }
        // IME 候选窗跟随光标(调研 21:cursor_area 上报是候选窗定位的全部机制;
        // 光标一动 bump → 重绘 → 重报,与版本键控布局缓存天然一致)
        if self.ime_allowed
            && let Some((cx, cy, cw, ch)) = ime_caret_rect(&self.doc, &self.placed, scale)
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
            .placed
            .iter()
            .rev()
            .find(|p| {
                p.rect.contains(lx, ly)
                    && (self.doc.pointer_enter_handler(p.id).is_some()
                        || self.doc.pointer_leave_handler(p.id).is_some())
            })
            .map(|p| p.id);
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
            .placed
            .iter()
            .rev()
            .find_map(|p| {
                if !p.rect.contains(lx, ly) {
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
        if let Some(id) = hit_click_target(&self.doc, &self.placed, lx, ly)
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

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.win.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(480.0, 400.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("sv-shell: 创建窗口失败"),
        );
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
            let _ = proxy.send_event(());
        });

        self.win = Some(WinState { window, presenter });
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        // 后台任务完成的唤醒:立即套用并请求重绘
        sv_ui::tasks::pump();
        if let Some(ws) = &self.win {
            ws.window.request_redraw();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
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
                self.update_hover();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                // is_synthetic:X11 窗口获焦时合成的按下事件,不过滤会误触发;
                // v0 只派发 keydown(keyup 留给拖拽/游戏后议)
                if is_synthetic || event.state != ElementState::Pressed {
                    return;
                }
                if let Some(e) = map_key(
                    &event.logical_key,
                    event.text.as_deref(),
                    event.repeat,
                    self.mods,
                ) {
                    // 路由(冒泡/编辑/导航/激活/快捷键)在 sv-ui,离屏可测
                    sv_ui::dispatch_key(&self.doc, &e);
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
                self.pressed = self
                    .placed
                    .iter()
                    .rev()
                    .find(|p| {
                        p.rect.contains(lx, ly) && self.doc.pointer_down_handler(p.id).is_some()
                    })
                    .map(|p| p.id);
                if let Some(id) = self.pressed
                    && let Some(h) = self.doc.pointer_down_handler(id)
                {
                    h();
                }
                // 点击设焦(Slint focus-on-click 同款);
                // 空白区点击不清焦点(桌面惯例)
                if let Some(p) = self
                    .placed
                    .iter()
                    .rev()
                    .find(|p| p.rect.contains(lx, ly) && self.doc.focusable(p.id))
                    .copied()
                {
                    self.doc.focus(p.id);
                    // 输入框:点击处换算字节偏移定光标
                    if self.doc.read(|inner| {
                        inner
                            .nodes
                            .get(p.id)
                            .is_some_and(|n| n.kind == sv_ui::ElementKind::TextInput)
                    }) {
                        let byte = input_caret_at(&self.doc, &p, lx);
                        self.doc.set_caret(p.id, byte, false);
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
    let event_loop = EventLoop::new()?;
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
        placed: Vec::new(),
        cursor: (0.0, 0.0),
        hovered: None,
        pressed: None,
        mods: ModifiersState::empty(),
        ime_allowed: false,
        epoch: std::time::Instant::now(),
        last_frame_key: None,
        show_fps: std::env::var("SV_SHOW_FPS").is_ok_and(|v| v == "1"),
        fps_frames: 0,
        fps_t0: std::time::Instant::now(),
        proxy,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
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
    /// caret_x 单调不减
    #[test]
    fn caret_geometry_roundtrip() {
        use crate::render::{caret_index_at, caret_x};
        let font = crate::font::ui_font();
        let text = "a你b好c!";
        let px = 16.0;
        let mut last = -1.0f32;
        for (i, _) in text.char_indices().chain([(text.len(), ' ')]) {
            let x = caret_x(&font, text, px, i);
            assert!(x >= last, "caret_x 应单调:{i}");
            last = x;
            assert_eq!(
                caret_index_at(&font, text, px, x + 0.1),
                i,
                "caret_x({i}) 处点击应回到 {i}"
            );
        }
        // 远超行尾 → 末尾;负数 → 0
        assert_eq!(caret_index_at(&font, text, px, 10_000.0), text.len());
        assert_eq!(caret_index_at(&font, text, px, -5.0), 0);
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
