//! 拿**真实 VAP 素材**验证本 crate。
//!
//! sv-pag 的头号缺口是"从未在真实文件上验证过"。VAP 这条线不重蹈覆辙:
//! 这个例子直接吃线上礼物素材,做四件事 ——
//!
//! 1. 从 MP4 的 `vapc` box 读配置;
//! 2. 与旁车 `.json` 解析结果**逐字段对拍**(两份应当完全一致);
//! 3. 用 ffmpeg 解一帧,交给 [`sv_vap::composite_rgba`] 合成;
//! 4. 把 RGBA 落盘,并统计 alpha 分布(全 0 或全 255 都说明解读错了)。
//!
//! 素材是商用礼物资源,**不入库**,所以这是个 example 而不是测试 ——
//! CI 上没有素材可跑。用法:
//!
//! ```sh
//! cargo run -p sv-vap --example verify_real_asset -- <某个.mp4> [输出.rgba]
//! ```

use std::process::Command;

use sv_vap::{AlphaMode, VapConfig, composite_rgba, find_vapc};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(mp4_path) = args.next() else {
        eprintln!("用法: verify_real_asset <某个.mp4> [输出.rgba]");
        std::process::exit(2);
    };
    let out_path = args.next();

    let mp4 = std::fs::read(&mp4_path).expect("读不到 mp4");
    println!("素材: {mp4_path}  ({} 字节)", mp4.len());

    // ---- 1. 从 MP4 box 读配置 ----
    let embedded = find_vapc(&mp4).expect("MP4 里应有 vapc box");
    let cfg = VapConfig::parse(embedded).expect("vapc 应能解析");
    println!(
        "vapc(内嵌): {}x{} 显示 / {}x{} 视频 / {} 帧 @{}fps / alpha {:?} / rgb {:?} / vapx={}",
        cfg.width,
        cfg.height,
        cfg.video_width,
        cfg.video_height,
        cfg.frames,
        cfg.fps,
        cfg.alpha_rect,
        cfg.rgb_rect,
        cfg.is_vapx
    );
    let (sx, sy) = cfg.alpha_scale();
    println!("alpha 缩放: {sx} × {sy};时长 {:.0}ms", cfg.duration_ms());

    // ---- 2. 与旁车 JSON 对拍 ----
    let sidecar = std::path::Path::new(&mp4_path).with_extension("json");
    match std::fs::read_to_string(&sidecar) {
        Ok(text) => match VapConfig::parse(&text) {
            Ok(side) if side == cfg => println!("旁车 JSON: 与内嵌 vapc **逐字段一致** ✅"),
            Ok(side) => {
                println!("⚠️ 旁车 JSON 与内嵌 vapc **不一致**");
                println!("   内嵌: {cfg:?}");
                println!("   旁车: {side:?}");
            }
            Err(e) => println!("⚠️ 旁车 JSON 解析失败: {e}"),
        },
        Err(_) => println!("(没有旁车 JSON,跳过对拍)"),
    }

    // ---- 3. ffmpeg 解一帧 ----
    let frame_no = cfg.frames / 3;
    let out = Command::new("ffmpeg")
        .args([
            "-loglevel",
            "error",
            "-i",
            &mp4_path,
            "-vf",
            &format!("select=eq(n\\,{frame_no})"),
            "-vframes",
            "1",
            "-pix_fmt",
            "rgb24",
            "-f",
            "rawvideo",
            "pipe:1",
        ])
        .output();
    let Ok(out) = out else {
        println!("(没有 ffmpeg,跳过合成;配置部分已验证)");
        return;
    };
    if !out.status.success() {
        println!(
            "(ffmpeg 失败,跳过合成:{})",
            String::from_utf8_lossy(&out.stderr)
        );
        return;
    }
    println!("解出第 {frame_no} 帧: {} 字节", out.stdout.len());

    // ---- 4. 合成 + 统计 ----
    let rgba = composite_rgba(&cfg, &out.stdout, AlphaMode::Straight).expect("合成应成功");
    let mut buckets = [0usize; 4];
    let mut opaque = 0usize;
    for px in rgba.chunks_exact(4) {
        buckets[(px[3] / 64).min(3) as usize] += 1;
        if px[3] > 250 {
            opaque += 1;
        }
    }
    let total = rgba.len() / 4;
    println!(
        "合成 {}x{}:alpha 分布 0-63/64-127/128-191/192-255 = {:?}",
        cfg.width, cfg.height, buckets
    );
    println!("完全不透明 {:.1}%", opaque as f64 * 100.0 / total as f64);
    // 【别在这里断言 alpha 的"形状"】第一版写的是"应当同时有透明区与不透明区,
    // 否则多半是 aFrame 取错了"。拿 10 个真实素材一跑,当场被打脸两次:
    //   · 4370-流星背景图 —— alpha **全程恒 255**(它就是个全屏不透明背景);
    //   · 50989_糖果扫把 —— 细长物体,画面绝大部分是空的,不透明像素不足 1%。
    // 两个都是完全合法的素材。真实素材的形态跨度比直觉大得多,
    // 任何对"内容长什么样"的断言都会变成误报。这里只**报告**分布,不断言。
    let profile = match (buckets[0] * 100 / total, buckets[3] * 100 / total) {
        (_, o) if o > 95 => "全屏不透明(背景类素材)",
        (t, _) if t > 95 => "几乎全透明(细小/稀疏元素,或这一帧本就是空的)",
        _ => "常规:透明与不透明区并存",
    };
    println!("alpha 形态: {profile}");

    // 预乘口径也跑一遍,顺带验一条硬约束
    let pre = composite_rgba(&cfg, &out.stdout, AlphaMode::Premultiplied).expect("预乘应成功");
    for px in pre.chunks_exact(4) {
        assert!(
            px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3],
            "预乘像素不得出现通道值大于 alpha"
        );
    }
    println!("预乘口径:全图满足 r,g,b ≤ a ✅");

    if let Some(p) = out_path {
        std::fs::write(&p, &rgba).expect("写不出文件");
        println!("已写出 {p}({}x{} RGBA8)", cfg.width, cfg.height);
        println!(
            "  转 PNG: ffmpeg -y -f rawvideo -pix_fmt rgba -s {}x{} -i {p} out.png",
            cfg.width, cfg.height
        );
    }
}
