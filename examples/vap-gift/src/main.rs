//! 把一个真实的 VAP 礼物动画放进 svelte-rs 的场景树,渲染成 PNG 序列。
//!
//! 这是 VAP 支持的**端到端证明**:配置解析 → H.264 解码 → RGB+alpha 合成 →
//! 注册进内容表 → `ElementKind::Animation` → 时间轴驱动 → `draw_image` → 像素。
//! 每一段都有自己的单元测试,但"接起来能不能出图"只有跑一遍才知道。
//!
//! ```sh
//! cargo run -p vap-gift --release -- <礼物.mp4> [输出目录] [要导出几帧]
//! ```
//!
//! # 为什么解码要 shell 出去调 ffmpeg
//!
//! `sv-vap` 刻意不含 H.264 解码器(引一个视频解码器是独立的重裁决,且与平台
//! 强相关)。这个 demo 用 ffmpeg 把整段解成 RGB24 —— **它是 demo 的依赖,
//! 不是库的依赖**。真产品里这一步该换成平台解码器(MediaCodec /
//! VideoToolbox / Media Foundation),或者干脆离线转成帧序列。

use std::io::Read;
use std::process::{Command, Stdio};

use sv_reactive::create_root;
use sv_shell::{PixelImage, register_frames, render_frame};
use sv_ui::{AnimData, AnimSource, Doc};
use sv_vap::{AlphaMode, VapConfig, composite_rgba, find_vapc};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(mp4_path) = args.next() else {
        eprintln!("用法: vap-gift <礼物.mp4> [输出目录] [要导出几帧]");
        std::process::exit(2);
    };
    let out_dir = args.next().unwrap_or_else(|| ".".to_string());
    let want: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(3);

    // ---- 1. 配置 ----
    let mp4 = std::fs::read(&mp4_path).expect("读不到 mp4");
    let cfg =
        VapConfig::parse(find_vapc(&mp4).expect("MP4 里应有 vapc box")).expect("vapc 应能解析");
    println!(
        "VAP: {}x{} / {} 帧 @{}fps / {:.1}s",
        cfg.width,
        cfg.height,
        cfg.frames,
        cfg.fps,
        cfg.duration_ms() / 1000.0
    );
    if cfg.is_vapx {
        // 不是拒绝:VAPX 的基础层照样能放,只是少了运行期动态元素
        println!("⚠️ 这是 VAPX 素材,动态元素(头像/昵称)不会出现");
    }

    // ---- 2. 解码整段 + 合成 ----
    // 一次性解完再合成,是因为 demo 要的是"能不能出图"而不是流式播放。
    // 真产品里显然该按需解码 —— 一段 1080p×150 帧的 RGBA 是 700MB 量级
    let t = std::time::Instant::now();
    let frames = decode_and_composite(&mp4_path, &cfg);
    println!(
        "解码 + 合成 {} 帧,耗时 {:.2}s(约 {:.1}ms/帧)",
        frames.len(),
        t.elapsed().as_secs_f64(),
        t.elapsed().as_secs_f64() * 1000.0 / frames.len().max(1) as f64
    );
    let bytes: usize = frames.len() * (cfg.width as usize) * (cfg.height as usize) * 4;
    println!(
        "常驻像素 {:.1} MB —— 真产品必须按需解码,不能这么放",
        bytes as f64 / 1e6
    );

    // ---- 3. 进场景树 ----
    let handle = register_frames(frames);
    let doc = Doc::new();
    let (id, _scope) = create_root(|| {
        let id = doc.create_animation(AnimData {
            source: AnimSource::Frames { handle },
            intrinsic: (cfg.width as f32, cfg.height as f32),
            frame_rate: cfg.fps,
            frame_count: cfg.frames,
            frame: 0,
            looped: true,
            playing: false,
        });
        doc.append(doc.root(), id);
        id
    });
    sv_ui::anim::play(&doc, id);

    // ---- 4. 时间轴驱动 + 出图 ----
    std::fs::create_dir_all(&out_dir).expect("建不了输出目录");
    let step = cfg.duration_ms() / want as f32;
    for i in 0..want {
        let t_ms = step * i as f32;
        sv_ui::anim::pump(t_ms as f64);
        let shown = doc.anim_of(id).map(|a| a.frame).unwrap_or(0);
        let (pixmap, _) = render_frame(&doc, cfg.width, cfg.height, 1.0);
        let path = format!("{out_dir}/vap_{i:02}_frame{shown:03}.png");
        pixmap.save_png(&path).expect("写不出 PNG");
        println!("  t={t_ms:7.0}ms → 第 {shown:3} 帧 → {path}");
    }
    sv_ui::anim::stop(&doc, id);
    println!("完成。PNG 是**叠在白底上**的(渲染壳的背景),透明区因此是白的。");
}

/// 用 ffmpeg 把整段解成 RGB24,逐帧交给 `sv-vap` 合成成预乘 RGBA。
fn decode_and_composite(mp4: &str, cfg: &VapConfig) -> Vec<PixelImage> {
    let frame_bytes = (cfg.video_width as usize) * (cfg.video_height as usize) * 3;
    let mut child = Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-i",
            mp4,
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .spawn()
        .expect("起不了 ffmpeg —— 这个 demo 需要它来解 H.264");
    let mut out = child.stdout.take().expect("拿不到 ffmpeg 的 stdout");

    let mut frames = Vec::new();
    let mut buf = vec![0u8; frame_bytes];
    loop {
        // 必须读满一整帧再处理:管道是流式的,单次 read 给多少全看缓冲区
        let mut got = 0;
        while got < frame_bytes {
            match out.read(&mut buf[got..]) {
                Ok(0) => break,
                Ok(n) => got += n,
                Err(e) => panic!("读 ffmpeg 输出失败: {e}"),
            }
        }
        if got < frame_bytes {
            break; // 流结束(最后一段不足一帧 = 正常收尾)
        }
        // 预乘:sv_shell::PixelImage 要的就是这个口径
        let rgba = composite_rgba(cfg, &buf, AlphaMode::Premultiplied).expect("合成失败");
        frames.push(PixelImage::new(cfg.width, cfg.height, rgba).expect("尺寸与字节数应当自洽"));
        if frames.len() as u32 >= cfg.frames {
            break;
        }
    }
    let _ = child.wait();
    frames
}
