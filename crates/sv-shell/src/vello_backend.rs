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

use vello::kurbo::{Affine, BezPath, RoundedRect, Stroke};
use vello::peniko::{
    Blob, Fill, FontData, ImageAlphaType, ImageBrushRef, ImageData, ImageFormat, ImageQuality,
};
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu::{self, CurrentSurfaceTexture};
use vello::{AaConfig, Glyph, RenderParams, Renderer, RendererOptions, Scene};
use winit::window::Window;

use sv_ui::{Color, Doc};

use crate::paint::{
    GlyphPos, LineCap, LineJoin, Painter, PainterCaps, PathCmd, PathFill, PixelImage, StrokeStyle,
    dst_rect_drawable, image_filter_nearest, warn_dropped_image,
};
use crate::render::paint_tree;

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

/// 把 `Arc<[u8]>` 包成 peniko `Blob` 能接的 `Arc<dyn AsRef<[u8]> + Send + Sync>`。
///
/// 为什么需要这层壳:`Blob::new` 的签名是
/// `Arc<dyn AsRef<[u8]> + Send + Sync>`(linebender_resource_handle 0.1.1
/// `blob.rs:95`),而 `Arc<[u8]>` **不能** unsize 成它 —— `Unsize` 只对
/// `Sized → dyn Trait` 成立,`[u8]` 本身已经是 unsized。
/// 唯二的出路是 `Arc<Vec<u8>>`(双重间接,upstream 自己在
/// vello `scene.rs:736` 注释里吐槽过 "the design of the Blob type forces the
/// double boxing")或这个 newtype。选 newtype:**零像素拷贝**,只多一次
/// 指针大小的分配,而且载体那边可以保持干净的 `Arc<[u8]>`
struct SharedPixels(std::sync::Arc<[u8]>);

impl AsRef<[u8]> for SharedPixels {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// 图片缓存条目寿命(帧数)。**对齐 vello 自己的图集淘汰口径**:
/// vello_encoding 0.9 `image_cache.rs:11` `EVICT_AFTER_GENERATIONS: u64 = 2`
/// (它在图集分配失败时调 `evict_stale_entries`,见 `resolve.rs:515`)。
///
/// 数值取 2 的语义:**连续 2 帧没被画到**才丢。停画 1 帧不丢,是为了不让
/// "这一帧碰巧被裁掉了/滚出视口了"变成下一帧的整图重传抖动
const IMAGE_EVICT_AFTER_FRAMES: u64 = 2;

/// 图片缓存条目:peniko 侧句柄 + 最近一次被画到的帧代
struct CachedImage {
    image: ImageData,
    last_used: u64,
}

pub struct VelloPainter {
    pub scene: Scene,
    /// 按字体身份缓存的 FontData(调研 24 P0:fallback 后同帧多字体)。
    /// 这张表不淘汰:条目数以**系统字体数**为界,且每条只包一层
    /// `Arc<&'static [u8]>`,与下面那张按图片数增长的表不是一回事
    fonts: std::collections::HashMap<u64, FontData>,
    /// 按 [`PixelImage::id`] 缓存的 peniko ImageData。
    ///
    /// **这个缓存是必需品不是优化**:vello 的图集 residency 表按
    /// `Blob::id()` 索引(vello_encoding 0.9 `image_cache.rs:114`),而
    /// `Blob::new` 每次调用都领一个**新** id(`blob.rs:95` 的全局
    /// `ID_COUNTER.fetch_add`)。不缓存 = 每帧一个新 id = 每帧把整张图重新
    /// `write_texture` 进图集。一张 1080p 就是 8.29 MB/帧。
    ///
    /// **同时它必须会淘汰**,由 [`VelloPainter::begin_frame`] 按
    /// `IMAGE_EVICT_AFTER_FRAMES` 清扫。一条条目钉住的是**整张解码后的
    /// RGBA**(`ImageData.data` → `Blob` → `Arc<dyn AsRef<[u8]>>` →
    /// 载体的 `Arc<[u8]>`),而 `VelloPainter` 与窗口同寿命
    /// (`VelloWin::painter`)—— 逐帧换图正是这个动词的动机场景
    /// (序列帧动画),1080p × 60fps 只增不减就是 **500 MB/秒的常驻增长**。
    /// GPU 那一侧本来就有生命周期语义,CPU 侧这张表跟着它走即可。
    ///
    /// **已知缺口**:vello 的图集是方形、初始 1024、上限 **8192**
    /// (vello_encoding 0.9 `image_cache.rs:9-10`)。任一边超过 8192 的图
    /// 永远进不了图集,`get_or_insert` 返回 None 之后**静默不画** ——
    /// 这一条发生在 vello 内部,我们这侧看不见也留不了痕(不镜像那个私有
    /// 常数:抄过来就会和上游悄悄漂移)。真要放超大图得在上层切片,
    /// 或走 `external_texture` 那条还没接的通道
    images: std::collections::HashMap<u64, CachedImage>,
    /// 帧代计数([`VelloPainter::begin_frame`] 自增)
    frame: u64,
}

impl VelloPainter {
    pub fn new() -> Self {
        Self {
            scene: Scene::new(),
            fonts: std::collections::HashMap::new(),
            images: std::collections::HashMap::new(),
            frame: 0,
        }
    }

    /// 帧边界:清场景 + 推进帧代 + 淘汰连续 `IMAGE_EVICT_AFTER_FRAMES` 帧
    /// 没被画到的图片。
    ///
    /// **复用 painter 的调用方必须用它,而不是直接 `painter.scene.reset()`**
    /// —— `reset` 只清编码好的命令,不动图片缓存,那正是"每秒钉住 60 张
    /// 1080p"的那条路径(见 `VelloPainter::images`)
    pub fn begin_frame(&mut self) {
        self.scene.reset();
        self.frame = self.frame.wrapping_add(1);
        let now = self.frame;
        // 用 wrapping 差值而不是 `last_used < now - N`:后者在帧代回绕时会
        // 把整张表误杀(u64 @60fps 要跑约 97 亿年才回绕,但写对不花钱)
        self.images
            .retain(|_, e| now.wrapping_sub(e.last_used) < IMAGE_EVICT_AFTER_FRAMES);
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
        PainterCaps {
            external_texture: false,
            blur: true,
        }
    }

    fn fill_rounded_rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, color: Color) {
        let rect = RoundedRect::new(
            x as f64,
            y as f64,
            (x + w) as f64,
            (y + h) as f64,
            radius as f64,
        );
        self.scene
            .fill(Fill::NonZero, Affine::IDENTITY, pcolor(color), None, &rect);
    }

    fn stroke_rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        width: f32,
        color: Color,
    ) {
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

    fn glyph_run(&mut self, font: crate::text::FontHandle, glyphs: &[GlyphPos], color: Color) {
        let Some(first) = glyphs.first() else { return };
        // 一段 run 内字号一致(paint_tree 按节点发射);px 语义 = font size。
        // FontData 按字体身份缓存建一次:与 CPU 端 swash 共用同一份 'static
        // 字节,Blob 只包一层 Arc<&[u8]>,零拷贝;glyph id 语义两端一致
        let fd = self.fonts.entry(font.key).or_insert_with(|| {
            let (bytes, index) = font.data();
            FontData::new(Blob::new(Arc::new(bytes)), index)
        });
        let px = first.px();
        self.scene
            .draw_glyphs(fd)
            .font_size(px)
            .brush(pcolor(color))
            .draw(
                Fill::NonZero,
                glyphs.iter().map(|g| Glyph {
                    id: g.id as u32,
                    x: g.ox,
                    y: g.oy,
                }),
            );
    }

    fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        // 1:1 映射 vello 图层裁剪(嵌套自动取交集,圆角精确)。
        // 用 push_layer + Normal blend 而非 push_clip_layer:后者 blend 语义
        // 尚不正确(vello issue #1198,调研 22 §2.3)
        let rect = RoundedRect::new(
            x as f64,
            y as f64,
            (x + w) as f64,
            (y + h) as f64,
            radius as f64,
        );
        self.scene.push_layer(
            Fill::NonZero,
            vello::peniko::Mix::Normal,
            1.0,
            Affine::IDENTITY,
            &rect,
        );
    }

    fn pop_clip(&mut self) {
        self.scene.pop_layer();
    }

    fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color) {
        let p = bez_path(path);
        if p.is_empty() {
            return;
        }
        self.scene.fill(
            match fill {
                PathFill::NonZero => Fill::NonZero,
                PathFill::EvenOdd => Fill::EvenOdd,
            },
            Affine::IDENTITY,
            pcolor(color),
            None,
            &p,
        );
    }

    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color) {
        let p = bez_path(path);
        if p.is_empty() {
            return;
        }
        let stroke = Stroke::new(style.width as f64)
            .with_miter_limit(style.miter_limit as f64)
            .with_caps(match style.cap {
                LineCap::Butt => vello::kurbo::Cap::Butt,
                LineCap::Round => vello::kurbo::Cap::Round,
                LineCap::Square => vello::kurbo::Cap::Square,
            })
            .with_join(match style.join {
                LineJoin::Miter => vello::kurbo::Join::Miter,
                LineJoin::Round => vello::kurbo::Join::Round,
                LineJoin::Bevel => vello::kurbo::Join::Bevel,
            });
        self.scene
            .stroke(&stroke, Affine::IDENTITY, pcolor(color), None, &p);
    }

    fn draw_image(&mut self, x: f32, y: f32, w: f32, h: f32, img: &PixelImage) {
        // 退化输入与 CPU 后端同一口径(paint.rs 的 valid_len / dst_rect_drawable)。
        // 畸形载体这一条**尤其不能静默**:vello 0.9 在图集上传处对"宣称 w×h
        // 但字节不够"的图直接 panic(`wgpu_engine.rs:507-513`
        // `Tried to draw an invalid empty image`)——闸门失效 = 崩窗口
        if img.valid_len().is_none() {
            warn_dropped_image("载体尺寸与字节数不符", img);
            return;
        }
        // 退化 dst 不留痕:同 CPU 后端,零尺寸矩形是日常合法情形
        if !dst_rect_drawable(x, y, w, h) {
            return;
        }
        let frame = self.frame;
        let entry = self.images.entry(img.id()).or_insert_with(|| CachedImage {
            image: ImageData {
                // Blob 只包一层 Arc:像素零拷贝(见 SharedPixels 的注释)
                data: Blob::new(std::sync::Arc::new(SharedPixels(img.shared_pixels()))),
                format: ImageFormat::Rgba8,
                // **预乘**。vello 0.9 的 fine shader 认这一位:
                // `shader/fine.wgsl:850-862` 的 maybe_premul_alpha 在
                // PREMULTIPLIED_ALPHA(=1)分支原样返回,只有 Alpha(=0)才现场乘。
                // 填错这里的表现是半透明区域整体偏亮(等于乘了两次 alpha)
                alpha_type: ImageAlphaType::AlphaPremultiplied,
                width: img.width(),
                height: img.height(),
            },
            last_used: frame,
        });
        // 每次画到都续命(begin_frame 按这个字段清扫)
        entry.last_used = frame;
        let data = &entry.image;
        // Scene::draw_image 画的是"自然尺寸矩形 (0,0,w,h) + 这个 transform"
        // (vello 0.9 `scene.rs:443-451`),所以缩放全靠 transform ——
        // 正好对上"拉伸铺满 dst"的语义
        let sx = w as f64 / img.width() as f64;
        let sy = h as f64 / img.height() as f64;
        let brush = ImageBrushRef::from(data).with_quality(
            // 与 CPU 后端同一条裁决(paint.rs image_filter_nearest)。
            // peniko 文档:Low ≈ nearest,Medium ≈ bilinear(`image.rs:50-59`)
            if image_filter_nearest(sx as f32, sy as f32) {
                ImageQuality::Low
            } else {
                ImageQuality::Medium
            },
        );
        // extend 用 ImageSampler 的默认值 Extend::Pad(`image.rs:109-110`),
        // 与 CPU 端的 SpreadMode::Pad 一致
        self.scene.draw_image(
            brush,
            Affine::translate((x as f64, y as f64)) * Affine::scale_non_uniform(sx, sy),
        );
    }
}

/// 自有 PathCmd → kurbo BezPath。**类型翻译只在 GPU 后端内部发生**,
/// Painter 接口不沾 kurbo(vello 是 optional dependency,见 paint.rs 里
/// PathCmd 的裁决)。填充与描边共用
fn bez_path(path: &[PathCmd]) -> BezPath {
    let mut p = BezPath::new();
    for c in path {
        match *c {
            PathCmd::MoveTo(x, y) => p.move_to((x as f64, y as f64)),
            PathCmd::LineTo(x, y) => p.line_to((x as f64, y as f64)),
            PathCmd::QuadTo(cx, cy, x, y) => {
                p.quad_to((cx as f64, cy as f64), (x as f64, y as f64))
            }
            PathCmd::CubicTo(c1x, c1y, c2x, c2y, x, y) => p.curve_to(
                (c1x as f64, c1y as f64),
                (c2x as f64, c2y as f64),
                (x as f64, y as f64),
            ),
            PathCmd::Close => p.close_path(),
        }
    }
    p
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
    pub fn new(
        window: Arc<Window>,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
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
        Ok(Self {
            context,
            renderer,
            surface,
            painter: VelloPainter::new(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.surface.config.width == width && self.surface.config.height == height {
            return;
        }
        self.context
            .resize_surface(&mut self.surface, width, height);
    }

    /// 渲染一帧到窗口。返回 (布局结果, 是否已成功呈现);
    /// 未呈现(surface 过期/被遮挡等)时调用方应 request_redraw 重试
    pub fn render(&mut self, doc: &Doc, scale: f32) -> (std::rc::Rc<crate::render::Layout>, bool) {
        self.render_cached(doc, scale, false)
    }

    /// `scene_unchanged=true` 时跳过场景重编码(布局走版本缓存),只重渲染呈现
    pub fn render_cached(
        &mut self,
        doc: &Doc,
        scale: f32,
        scene_unchanged: bool,
    ) -> (std::rc::Rc<crate::render::Layout>, bool) {
        let width = self.surface.config.width;
        let height = self.surface.config.height;
        let layout =
            crate::render::layout_full_cached(doc, width as f32 / scale, height as f32 / scale);
        if !scene_unchanged {
            // begin_frame 而不是 scene.reset():后者不动图片缓存
            // (见 VelloPainter::begin_frame)。scene_unchanged 那条分支
            // 刻意不推进帧代——没重编码场景就没"这一帧画了谁"可言,
            // 推进只会把还在屏上的图误判成没人用
            self.painter.begin_frame();
            paint_tree(doc, &layout.placed, &mut self.painter, scale);
            crate::render::paint_scrollbars(doc, &layout.scroll_areas, &mut self.painter, scale);
        }

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
            log::warn!("sv-shell: vello render_to_texture 失败: {e}");
            return (layout, false);
        }

        let surface_texture = match self.surface.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(t) => t,
            CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Suboptimal(_) => {
                self.context.configure_surface(&self.surface);
                return (layout, false);
            }
            CurrentSurfaceTexture::Occluded | CurrentSurfaceTexture::Timeout => {
                return (layout, false);
            }
            CurrentSurfaceTexture::Lost => {
                log::warn!("sv-shell: vello surface 丢失,尝试重建配置");
                self.context.configure_surface(&self.surface);
                return (layout, false);
            }
            CurrentSurfaceTexture::Validation => {
                log::warn!("sv-shell: vello surface 校验错误,跳过本帧");
                return (layout, false);
            }
        };

        let mut encoder =
            device_handle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("sv-shell surface blit"),
                });
        self.surface.blitter.copy(
            &device_handle.device,
            &mut encoder,
            &self.surface.target_view,
            &surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default()),
        );
        device_handle.queue.submit([encoder.finish()]);
        surface_texture.present();
        let _ = device_handle.device.poll(wgpu::PollType::Poll);
        (layout, true)
    }
}

// ---------------------------------------------------------------------------
// 探测 + 离屏
// ---------------------------------------------------------------------------

/// 是否有可用 GPU adapter(后端自动选择用;拿不到则回退 CPU)
pub fn probe_adapter() -> bool {
    usable_adapter().is_some()
}

/// 拿一个"可用"的 adapter。默认拒绝软件光栅(`DeviceType::Cpu`,即 WARP/
/// lavapipe):无 GPU 的 CI 跑道上 WARP 曾在管线执行中访问违例(0xc0000005),
/// 且软件 GPU 相对 CPU 后端毫无收益;`SV_ALLOW_SOFTWARE_GPU=1` 显式启用
/// (Linux CI 用 lavapipe 跑真渲染覆盖就走这个开关)
fn usable_adapter() -> Option<wgpu::Adapter> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        display: None,
        backends: wgpu::Backends::from_env().unwrap_or_default(),
        flags: wgpu::InstanceFlags::from_build_config().with_env(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        backend_options: wgpu::BackendOptions::from_env_or_default(),
    });
    let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
        &instance, None,
    ))
    .ok()?;
    let info = adapter.get_info();
    if info.device_type == wgpu::DeviceType::Cpu
        && std::env::var("SV_ALLOW_SOFTWARE_GPU").as_deref() != Ok("1")
    {
        log::warn!(
            "sv-shell: 忽略软件渲染 adapter「{}」(SV_ALLOW_SOFTWARE_GPU=1 可启用)",
            info.name
        );
        return None;
    }
    Some(adapter)
}

/// 离屏上下文缓存:device/renderer 建一次,目标纹理与回读缓冲按尺寸复用。
/// (基准测试与连续离屏渲染的帧率口径需要稳态,而非每帧重建管线)
///
/// 不走 vello `RenderContext`(其 device 固定 `Limits::default()`,
/// 128MB 存储绑定上限在 10 万控件档会被 scene buffer 撑爆,调研 17):
/// 离屏无 surface 兼容性约束,自建 device 并按 adapter 实际能力抬高上限。
struct Offscreen {
    device: wgpu::Device,
    queue: wgpu::Queue,
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

/// 自建离屏 device:存储缓冲绑定上限抬到 adapter 实际能力
/// (vello scene buffer 随控件数线性膨胀,100k 档 ≈192MB)
fn create_offscreen_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = usable_adapter()?;
    let caps = adapter.limits();
    let limits = wgpu::Limits {
        max_storage_buffer_binding_size: caps.max_storage_buffer_binding_size,
        max_buffer_size: caps.max_buffer_size,
        ..wgpu::Limits::default()
    };
    let maybe = wgpu::Features::CLEAR_TEXTURE | wgpu::Features::PIPELINE_CACHE;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("sv-shell offscreen"),
        required_features: adapter.features() & maybe,
        required_limits: limits,
        ..Default::default()
    }))
    .ok()
}

fn offscreen_targets(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::Buffer, u32) {
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
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

/// 借一个尺寸已就绪的离屏上下文;无 GPU adapter / Renderer 建不起来时 None。
///
/// 从 `render_frame_vello` 里抽出来的:场景的来源不止 `Doc` 一种(测试要
/// 直接喂一个手搓的 `Scene`),而"惰性建 device + 按尺寸复用纹理/缓冲"
/// 这套逻辑抄第二遍必然会跟第一遍长歪
fn with_offscreen<R>(width: u32, height: u32, f: impl FnOnce(&mut Offscreen) -> R) -> Option<R> {
    OFFSCREEN.with(|cell| {
        let mut slot = cell.borrow_mut();
        // 惰性建上下文;尺寸变化只重建纹理/缓冲
        if slot.is_none() {
            let (device, queue) = create_offscreen_device()?;
            let renderer = match Renderer::new(&device, RendererOptions::default()) {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("sv-shell: vello Renderer 创建失败: {e}");
                    return None;
                }
            };
            let (target, view, buffer, padded) = offscreen_targets(&device, width, height);
            *slot = Some(std::mem::ManuallyDrop::new(Offscreen {
                device,
                queue,
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
            let (target, view, buffer, padded) = offscreen_targets(&off.device, width, height);
            off.target = target;
            off.view = view;
            off.buffer = buffer;
            off.padded_bytes_per_row = padded;
            off.size = (width, height);
        }
        Some(f(off))
    })
}

/// 离屏渲染一帧,返回紧致 RGBA8 字节(len = w*h*4);无 GPU adapter 时 None
pub fn render_frame_vello(doc: &Doc, phys_w: u32, phys_h: u32, scale: f32) -> Option<Vec<u8>> {
    let width = phys_w.max(1);
    let height = phys_h.max(1);
    with_offscreen(width, height, |off| {
        let layout =
            crate::render::layout_full_cached(doc, width as f32 / scale, height as f32 / scale);
        let mut painter = VelloPainter::new();
        paint_tree(doc, &layout.placed, &mut painter, scale);
        crate::render::paint_scrollbars(doc, &layout.scroll_areas, &mut painter, scale);
        render_scene_offscreen(&painter.scene, off, width, height)
    })
    .flatten()
}

/// Scene → 紧致 RGBA8 字节。渲染 + 回读,不关心场景是谁编码的
fn render_scene_offscreen(
    scene: &Scene,
    off: &mut Offscreen,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let device = &off.device;
    let queue = &off.queue;
    let renderer = &mut off.renderer;
    let target = &off.target;
    let view = &off.view;
    let buffer = &off.buffer;
    let padded_bytes_per_row = off.padded_bytes_per_row;
    let size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };

    if let Err(e) = renderer.render_to_texture(
        device,
        queue,
        scene,
        view,
        &RenderParams {
            base_color: pcolor(BASE_WHITE),
            width,
            height,
            antialiasing_method: aa_config(),
        },
    ) {
        log::warn!("sv-shell: vello 离屏渲染失败: {e}");
        return None;
    }

    // 回读:bytes_per_row 已 256 对齐(offscreen_targets),取回后去 padding
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("sv-shell readback"),
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::PixelImage;

    /// 把一个只含 draw_image 的场景真的渲染出来对拍像素。
    ///
    /// 为什么值得单独跑一趟 GPU:`draw_image` 的 GPU 侧有三处 CPU 侧完全
    /// 覆盖不到的口径 —— 图集上传的行主序、`ImageFormat::Rgba8` 的通道序、
    /// `ImageAlphaType::AlphaPremultiplied` 那一位。这三处**任何一处填错都
    /// 能编译、能跑、能画出"看着像那么回事"的图**,只有回读像素才抓得住。
    ///
    /// 三条断言合在一个测试里:每个测试线程都会建一套独立的 wgpu device
    /// (OFFSCREEN 是 thread_local),拆成三个测试就是三套 device。
    ///
    /// 无 GPU adapter 时跳过(与 `vello_offscreen_parity` 同款处理)。
    #[test]
    fn vello_draw_image_pixels_match_the_cpu_contract() {
        // 2×2:左上红、右上绿、左下蓝、右下白(全不透明)
        let opaque = PixelImage::new(
            2,
            2,
            vec![
                255, 0, 0, 255, //
                0, 255, 0, 255, //
                0, 0, 255, 255, //
                255, 255, 255, 255,
            ],
        )
        .unwrap();

        let mut p = VelloPainter::new();
        // 2 倍整数放大 → ImageQuality::Low(最近邻),块状可逐像素断言
        p.draw_image(0.0, 0.0, 4.0, 4.0, &opaque);
        let Some(px) =
            with_offscreen(4, 4, |off| render_scene_offscreen(&p.scene, off, 4, 4)).flatten()
        else {
            println!("跳过 vello_draw_image:无可用 GPU adapter(CPU 后端已覆盖同一契约)");
            return;
        };
        let at = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            (px[i], px[i + 1], px[i + 2], px[i + 3])
        };
        for (x, y) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            assert_eq!(at(x, y), (255, 0, 0, 255), "({x},{y}) 应是源左上的红");
        }
        for (x, y) in [(2, 0), (3, 0), (2, 1), (3, 1)] {
            assert_eq!(at(x, y), (0, 255, 0, 255), "({x},{y}) 应是源右上的绿");
        }
        for (x, y) in [(0, 2), (1, 3)] {
            assert_eq!(at(x, y), (0, 0, 255, 255), "({x},{y}) 应是源左下的蓝");
        }
        assert_eq!(at(3, 3), (255, 255, 255, 255), "右下应是源右下的白");

        // 预乘那一位:半透明源叠在白底(RenderParams::base_color)上。
        // 期望值与 CPU 后端那条测试同一算式:src + dst*(1-a),
        // (64,128,32,128) over 白 = (191,255,159,255)。
        // 若 alpha_type 填成 Alpha,shader 会再乘一次 alpha,r 会掉到 ~159
        let translucent = PixelImage::new(1, 1, vec![64, 128, 32, 128]).unwrap();
        let mut p = VelloPainter::new();
        p.draw_image(0.0, 0.0, 4.0, 4.0, &translucent);
        let px = with_offscreen(4, 4, |off| render_scene_offscreen(&p.scene, off, 4, 4))
            .flatten()
            .expect("上一步已确认有 adapter");
        let got = (px[0], px[1], px[2], px[3]);
        let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
        // GPU 侧容差放到 ±2:采样与合成全程 f32,最后一步量化回 u8,
        // 与 CPU 的定点管线不保证末位一致(语义错会差几十)
        assert!(
            d(got.0, 191) <= 2 && d(got.1, 255) <= 2 && d(got.2, 159) <= 2 && d(got.3, 255) <= 2,
            "预乘口径:期望 ~(191,255,159,255),实得 {got:?}"
        );
    }

    /// 同一张图画两帧,peniko ImageData 只建一次 —— 缓存不生效就是每帧
    /// 重传整张图(见 `VelloPainter::images` 的注释)。
    /// 用 `Blob::id()` 当证据:`Blob::new` 每次调用都换 id,id 没变就说明
    /// 走的是缓存而不是重建
    #[test]
    fn vello_image_upload_is_cached_across_frames() {
        let img = PixelImage::new(1, 1, vec![1, 2, 3, 255]).unwrap();
        let mut p = VelloPainter::new();
        p.begin_frame();
        p.draw_image(0.0, 0.0, 1.0, 1.0, &img);
        let first = p.images[&img.id()].image.data.id();
        p.begin_frame();
        p.draw_image(0.0, 0.0, 1.0, 1.0, &img);
        assert_eq!(p.images.len(), 1, "同一张图不该在缓存里占两条");
        assert_eq!(
            p.images[&img.id()].image.data.id(),
            first,
            "Blob id 变了 = 又 new 了一个 Blob = 下一帧整张图重新上传"
        );

        // 内容相同但身份不同的另一张图:必须是两条(id 才是缓存键)
        let other = PixelImage::new(1, 1, vec![1, 2, 3, 255]).unwrap();
        p.draw_image(0.0, 0.0, 1.0, 1.0, &other);
        assert_eq!(p.images.len(), 2, "不同身份的图各占一条");
    }

    /// 缓存**不是只增不减**:连续两帧没被画到的图必须被丢掉。
    ///
    /// 反面就是这个动词的动机场景本身 —— 序列帧动画每帧换一张图,
    /// 每条条目钉住一整张解码后的 RGBA,1080p × 60fps = 500 MB/秒常驻增长,
    /// 而 `VelloWin` 的 painter 与窗口同寿命。口径对齐 vello 自己的
    /// `EVICT_AFTER_GENERATIONS = 2`(见 IMAGE_EVICT_AFTER_FRAMES)
    #[test]
    fn vello_image_cache_evicts_entries_that_stopped_being_drawn() {
        let a = PixelImage::new(1, 1, vec![1, 2, 3, 255]).unwrap();
        let b = PixelImage::new(1, 1, vec![4, 5, 6, 255]).unwrap();
        let mut p = VelloPainter::new();

        p.begin_frame();
        p.draw_image(0.0, 0.0, 1.0, 1.0, &a);
        assert_eq!(p.images.len(), 1);

        // 换图:a 不再被画。停画一帧还留着(避免"这帧碰巧被裁掉"的重传抖动)
        p.begin_frame();
        p.draw_image(0.0, 0.0, 1.0, 1.0, &b);
        assert_eq!(p.images.len(), 2, "刚停画一帧不该立刻丢");

        // 再一帧:a 已连续两帧没用 → 淘汰
        p.begin_frame();
        p.draw_image(0.0, 0.0, 1.0, 1.0, &b);
        assert_eq!(p.images.len(), 1, "连续两帧没画到的图必须被丢掉");
        assert!(p.images.contains_key(&b.id()), "还在画的那张必须留着");
        assert!(!p.images.contains_key(&a.id()), "被淘汰的必须是 a");

        // 逐帧全换图(序列帧动画的最坏情形):表长必须有界,不能线性增长
        for i in 0..32u8 {
            p.begin_frame();
            let frame = PixelImage::new(1, 1, vec![i, 0, 0, 255]).unwrap();
            p.draw_image(0.0, 0.0, 1.0, 1.0, &frame);
        }
        assert!(
            p.images.len() <= IMAGE_EVICT_AFTER_FRAMES as usize,
            "逐帧换图时缓存必须有界,实得 {} 条",
            p.images.len()
        );
    }

    /// GPU 侧的**畸形载体**闸。
    ///
    /// 之前这条分支一行测试都没有:`vello_refuses_degenerate_input` 喂的全是
    /// **合法**载体 + 退化 dst,走的是 `dst_rect_drawable` 那半边;而畸形载体
    /// 那半边挡的是 vello 0.9 的硬 panic(`wgpu_engine.rs:507-513`
    /// `Tried to draw an invalid empty image`)。造值靠
    /// `PixelImage::bogus_for_test` —— 字段私有于 paint.rs,这边写不出字面量
    #[test]
    fn vello_refuses_a_malformed_pixel_image() {
        // 声称 64×64(需 16384 字节),实际 4 字节
        let bogus = PixelImage::bogus_for_test(64, 64, &[1, 2, 3, 4]);
        let mut p = VelloPainter::new();
        p.draw_image(0.0, 0.0, 8.0, 8.0, &bogus);
        assert!(p.images.is_empty(), "畸形载体不该进缓存,更不该编进场景");

        // 阳性对照:合法载体 + 合法 dst 必须真的走通。
        // 没有这一条的话,一个"draw_image 整个坏掉"的实现也能让上面那句全绿
        let ok = PixelImage::new(2, 2, vec![0u8; 16]).unwrap();
        p.draw_image(0.0, 0.0, 4.0, 4.0, &ok);
        assert_eq!(p.images.len(), 1, "合法输入必须真的走通");
    }

    /// 退化 **dst** 在 GPU 后端也必须丢弃(`dst_rect_drawable` 那半边闸)。
    /// 这里喂的载体全是**合法**的 —— 畸形载体那半边在
    /// `vello_refuses_a_malformed_pixel_image`,两条别混
    #[test]
    fn vello_refuses_degenerate_input() {
        let img = PixelImage::new(2, 2, vec![0u8; 16]).unwrap();
        let mut p = VelloPainter::new();
        p.draw_image(0.0, 0.0, 0.0, 4.0, &img);
        p.draw_image(0.0, 0.0, 4.0, -1.0, &img);
        p.draw_image(f32::NAN, 0.0, 4.0, 4.0, &img);
        p.draw_image(0.0, 0.0, f32::INFINITY, 4.0, &img);
        assert!(p.images.is_empty(), "退化调用不该在缓存里留下任何条目");
    }
}
