//! vello(wgpu)GPU 后端 —— 第二个真实 Painter 实现(ADR-3b / 调研 14)。
//!
//! 结构:
//! - [`VelloPainter`]:Painter → `vello::Scene` 的 1:1 映射(fill/stroke/glyph run);
//!   文本走 GlyphPos 的 (id, ox, oy) 基线原点语义,正对上 `Scene::draw_glyphs`;
//! - [`VelloWin`]:窗口呈现器(RenderContext + Renderer + RenderSurface),
//!   vello 0.9 走 render_to_texture → TextureBlitter → present;
//! - [`render_frame_vello`]:离屏渲染 + buffer 回读(测试/CI;无 adapter 返回 None)。
//!
//! 坐标:物理像素(paint_tree 已乘 scale),与 CPU 后端一致;底色白,与
//! `render_frame` 的白底对齐。

use std::sync::Arc;

use vello::kurbo::{Affine, RoundedRect, Stroke};
use vello::peniko::{Blob, Fill, FontData};
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu::{self, CurrentSurfaceTexture};
use vello::{AaConfig, Glyph, RenderParams, Renderer, RendererOptions, Scene};
use winit::window::Window;

use sv_ui::{Color, Doc};

use crate::font::ui_font_data;
use crate::paint::{GlyphPos, Painter, PainterCaps};
use crate::render::{Placed, layout_tree, paint_tree};

fn pcolor(c: Color) -> vello::peniko::Color {
    vello::peniko::Color::from_rgba8(c.r, c.g, c.b, c.a)
}

const BASE_WHITE: Color = Color::WHITE;

/// 抗锯齿方式:SV_VELLO_AA=area|msaa8|msaa16(默认 msaa16;area 为解析式 AA,
/// 零 MSAA 缓冲——内存敏感场景用,基准 17 号做归因)
fn aa_config() -> AaConfig {
    match std::env::var("SV_VELLO_AA").as_deref() {
        Ok("area") => AaConfig::Area,
        Ok("msaa8") => AaConfig::Msaa8,
        _ => AaConfig::Msaa16,
    }
}


// ---------------------------------------------------------------------------
// VelloPainter:Painter → Scene
// ---------------------------------------------------------------------------

pub struct VelloPainter {
    pub scene: Scene,
    font: FontData,
}

impl VelloPainter {
    pub fn new() -> Self {
        let (bytes, index) = ui_font_data();
        // 与 fontdue 共用同一份 'static 字节;Blob 只包一层 Arc<&[u8]>,零拷贝
        let font = FontData::new(Blob::new(Arc::new(bytes)), index);
        Self { scene: Scene::new(), font }
    }
}

impl Default for VelloPainter {
    fn default() -> Self {
        Self::new()
    }
}

impl Painter for VelloPainter {
    fn caps(&self) -> PainterCaps {
        // vello 有 draw_blurred_rounded_rect → blur 可用;
        // 外部纹理合成(<surface3d>)还没接 → false
        PainterCaps { external_texture: false, blur: true }
    }

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let rect = RoundedRect::new(
            x as f64,
            y as f64,
            (x + w) as f64,
            (y + h) as f64,
            radius as f64,
        );
        self.scene.fill(Fill::NonZero, Affine::IDENTITY, pcolor(color), None, &rect);
    }

    fn stroke_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, width: f32, color: Color) {
        // 与 CPU 后端一致:沿边框中心线描边(内缩半宽),视觉贴合 border-box
        let half = width / 2.0;
        let rect = RoundedRect::new(
            (x + half) as f64,
            (y + half) as f64,
            (x + w - half) as f64,
            (y + h - half) as f64,
            (radius - half).max(0.0) as f64,
        );
        self.scene.stroke(
            &Stroke::new(width as f64),
            Affine::IDENTITY,
            pcolor(color),
            None,
            &rect,
        );
    }

    fn glyph_run(&mut self, _font: &fontdue::Font, glyphs: &[GlyphPos], color: Color) {
        let Some(first) = glyphs.first() else { return };
        // 一段 run 内字号一致(paint_tree 按节点发射);px 语义 = font size
        let px = first.key.px;
        self.scene
            .draw_glyphs(&self.font)
            .font_size(px)
            .brush(pcolor(color))
            .draw(
                Fill::NonZero,
                glyphs.iter().map(|g| Glyph { id: g.id as u32, x: g.ox, y: g.oy }),
            );
    }
}

// ---------------------------------------------------------------------------
// 窗口呈现器
// ---------------------------------------------------------------------------

pub struct VelloWin {
    context: RenderContext,
    renderer: Renderer,
    surface: RenderSurface<'static>,
    painter: VelloPainter,
}

impl VelloWin {
    /// 建 surface + renderer;失败(无 adapter / surface 不兼容)由调用方回退 CPU
    pub fn new(window: Arc<Window>, width: u32, height: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let mut context = RenderContext::new();
        let surface = pollster::block_on(context.create_surface(
            window,
            width.max(1),
            height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .map_err(|e| format!("vello create_surface 失败: {e}"))?;
        let renderer = Renderer::new(
            &context.devices[surface.dev_id].device,
            RendererOptions::default(),
        )
        .map_err(|e| format!("vello Renderer 创建失败: {e}"))?;
        Ok(Self { context, renderer, surface, painter: VelloPainter::new() })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.surface.config.width == width && self.surface.config.height == height {
            return;
        }
        self.context.resize_surface(&mut self.surface, width, height);
    }

    /// 渲染一帧到窗口。返回 (布局结果, 是否已成功呈现);
    /// 未呈现(surface 过期/被遮挡等)时调用方应 request_redraw 重试
    pub fn render(&mut self, doc: &Doc, scale: f32) -> (Vec<Placed>, bool) {
        let width = self.surface.config.width;
        let height = self.surface.config.height;
        let placed = layout_tree(doc, width as f32 / scale, height as f32 / scale);

        self.painter.scene.reset();
        paint_tree(doc, &placed, &mut self.painter, scale);

        let device_handle = &self.context.devices[self.surface.dev_id];
        if let Err(e) = self.renderer.render_to_texture(
            &device_handle.device,
            &device_handle.queue,
            &self.painter.scene,
            &self.surface.target_view,
            &RenderParams {
                base_color: pcolor(BASE_WHITE),
                width,
                height,
                antialiasing_method: aa_config(),
            },
        ) {
            eprintln!("sv-shell: vello render_to_texture 失败: {e}");
            return (placed, false);
        }

        let surface_texture = match self.surface.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(t) => t,
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Suboptimal(_) => {
                self.context.configure_surface(&self.surface);
                return (placed, false);
            }
            CurrentSurfaceTexture::Occluded | CurrentSurfaceTexture::Timeout => {
                return (placed, false);
            }
            CurrentSurfaceTexture::Lost => {
                eprintln!("sv-shell: vello surface 丢失,尝试重建配置");
                self.context.configure_surface(&self.surface);
                return (placed, false);
            }
            CurrentSurfaceTexture::Validation => {
                eprintln!("sv-shell: vello surface 校验错误,跳过本帧");
                return (placed, false);
            }
        };

        let mut encoder = device_handle
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("sv-shell surface blit") });
        self.surface.blitter.copy(
            &device_handle.device,
            &mut encoder,
            &self.surface.target_view,
            &surface_texture.texture.create_view(&wgpu::TextureViewDescriptor::default()),
        );
        device_handle.queue.submit([encoder.finish()]);
        surface_texture.present();
        let _ = device_handle.device.poll(wgpu::PollType::Poll);
        (placed, true)
    }
}

// ---------------------------------------------------------------------------
// 探测 + 离屏
// ---------------------------------------------------------------------------

/// 是否有可用 GPU adapter(后端自动选择用;拿不到则回退 CPU)
pub fn probe_adapter() -> bool {
    let mut context = RenderContext::new();
    pollster::block_on(context.device(None)).is_some()
}

/// 离屏上下文缓存:device/renderer 建一次,目标纹理与回读缓冲按尺寸复用。
/// (基准测试与连续离屏渲染的帧率口径需要稳态,而非每帧重建管线)
struct Offscreen {
    context: RenderContext,
    device_id: usize,
    renderer: Renderer,
    target: wgpu::Texture,
    view: wgpu::TextureView,
    buffer: wgpu::Buffer,
    padded_bytes_per_row: u32,
    size: (u32, u32),
}

thread_local! {
    // ManuallyDrop:缓存与进程同寿命。TLS 析构期 drop wgpu 资源会触碰
    // wgpu-core 自己已销毁的 TLS(LockTrace)而 abort——刻意泄漏以避开
    static OFFSCREEN: std::cell::RefCell<Option<std::mem::ManuallyDrop<Offscreen>>> =
        const { std::cell::RefCell::new(None) };
}

fn offscreen_targets(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Buffer, u32) {
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sv-shell offscreen target"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let padded = (width * 4).next_multiple_of(256);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("sv-shell readback"),
        size: (padded as u64) * (height as u64),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    (target, view, buffer, padded)
}

/// 离屏渲染一帧,返回紧致 RGBA8 字节(len = w*h*4);无 GPU adapter 时 None
pub fn render_frame_vello(doc: &Doc, phys_w: u32, phys_h: u32, scale: f32) -> Option<Vec<u8>> {
    let width = phys_w.max(1);
    let height = phys_h.max(1);

    OFFSCREEN.with(|cell| {
        let mut slot = cell.borrow_mut();
        // 惰性建上下文;尺寸变化只重建纹理/缓冲
        if slot.is_none() {
            let mut context = RenderContext::new();
            let device_id = pollster::block_on(context.device(None))?;
            let device = &context.devices[device_id].device;
            let renderer = match Renderer::new(device, RendererOptions::default()) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("sv-shell: vello Renderer 创建失败: {e}");
                    return None;
                }
            };
            let (target, view, buffer, padded) = offscreen_targets(device, width, height);
            *slot = Some(std::mem::ManuallyDrop::new(Offscreen {
                context,
                device_id,
                renderer,
                target,
                view,
                buffer,
                padded_bytes_per_row: padded,
                size: (width, height),
            }));
        }
        let off = slot.as_mut().unwrap();
        if off.size != (width, height) {
            let device = &off.context.devices[off.device_id].device;
            let (target, view, buffer, padded) = offscreen_targets(device, width, height);
            off.target = target;
            off.view = view;
            off.buffer = buffer;
            off.padded_bytes_per_row = padded;
            off.size = (width, height);
        }
        render_offscreen_frame(doc, off, width, height, scale)
    })
}

fn render_offscreen_frame(
    doc: &Doc,
    off: &mut Offscreen,
    width: u32,
    height: u32,
    scale: f32,
) -> Option<Vec<u8>> {
    let device = &off.context.devices[off.device_id].device;
    let queue = &off.context.devices[off.device_id].queue;
    let renderer = &mut off.renderer;
    let target = &off.target;
    let view = &off.view;
    let buffer = &off.buffer;
    let padded_bytes_per_row = off.padded_bytes_per_row;
    let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };

    let placed = layout_tree(doc, width as f32 / scale, height as f32 / scale);
    let mut painter = VelloPainter::new();
    paint_tree(doc, &placed, &mut painter, scale);

    if let Err(e) = renderer.render_to_texture(
        device,
        queue,
        &painter.scene,
        view,
        &RenderParams {
            base_color: pcolor(BASE_WHITE),
            width,
            height,
            antialiasing_method: aa_config(),
        },
    ) {
        eprintln!("sv-shell: vello 离屏渲染失败: {e}");
        return None;
    }

    // 回读:bytes_per_row 已 256 对齐(offscreen_targets),取回后去 padding
    let mut encoder = device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("sv-shell readback") });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: None,
            },
        },
        size,
    );
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
    rx.recv().ok()?.ok()?;

    let data = slice.get_mapped_range();
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * padded_bytes_per_row) as usize;
        out.extend_from_slice(&data[start..start + (width * 4) as usize]);
    }
    // 缓冲是跨帧复用的:必须显式解除映射,否则下一帧 map_async panic
    drop(data);
    buffer.unmap();
    Some(out)
}
