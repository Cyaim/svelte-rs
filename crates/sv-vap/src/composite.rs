//! 合成内核:一帧解码后的 RGB 视频 → 一张带 alpha 的 RGBA 图。
//!
//! VAP 的把戏很朴素:H.264 不支持 alpha,那就把 alpha 当成灰度画面
//! **并排塞进同一帧**,播放时再拼回去。所以这一步的全部工作是
//! "从两个矩形各采一次样,拼成 RGBA"。
//!
//! # 实测确认过的三件事(2026-07-22,10 个真实礼物素材)
//!
//! 1. alpha 区**确实是灰度**:抽样 315 个像素,314 个满足 R=G=B(99.7%);
//! 2. alpha 是**半分辨率**(RGB 750×1624 / alpha 375×812),需要放大采样;
//! 3. 是**直通 alpha 不是预乘**:按 `src*a + bg*(1-a)` 叠到品红底上,
//!    柔边过渡干净、没有暗边 —— 暗边正是把预乘当直通用的典型症状。

use crate::{VapConfig, VapError};

/// 输出像素格式。
///
/// 两种都要有,是因为下游要什么口径由下游定,而**转换只在这里做一次**:
/// 让每个后端各转一遍必然有人转错(预乘算错的症状是柔边发暗,极难归因)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlphaMode {
    /// 直通(straight):`(r, g, b, a)` 各自独立。VAP 素材的原生口径
    Straight,
    /// 预乘:`(r*a, g*a, b*a, a)`。`sv_shell::PixelImage` 要的是这个
    Premultiplied,
}

/// 把一帧解码后的 RGB24 视频帧合成成 RGBA8。
///
/// `frame` 必须是 `video_width × video_height × 3` 字节的 RGB24
/// (ffmpeg 的 `-pix_fmt rgb24 -f rawvideo` 就是这个)。
/// 输出是 `width × height × 4`。
///
/// # 为什么 alpha 用最近邻放大而不是双线性
///
/// alpha 边缘正是"半透明羽化"所在,双线性会把它再糊一次 —— 而素材侧
/// 已经按半分辨率做过一次降采样了,再糊一次边缘会明显变肉。
/// 最近邻在这里是**保边**的选择,不是偷懒。
/// (真要更好应该在**放大后的 RGBA** 上做一次整体缩放,那是渲染端的事。)
pub fn composite_rgba(cfg: &VapConfig, frame: &[u8], mode: AlphaMode) -> Result<Vec<u8>, VapError> {
    let vw = cfg.video_width as usize;
    let vh = cfg.video_height as usize;
    let expect = vw
        .checked_mul(vh)
        .and_then(|n| n.checked_mul(3))
        .ok_or(VapError::BadGeometry)?;
    if frame.len() != expect {
        return Err(VapError::FrameSizeMismatch {
            expected: expect,
            got: frame.len(),
        });
    }

    let (w, h) = (cfg.width as usize, cfg.height as usize);
    let out_len = w
        .checked_mul(h)
        .and_then(|n| n.checked_mul(4))
        .ok_or(VapError::BadGeometry)?;
    let mut out = vec![0u8; out_len];

    let (rx, ry) = (cfg.rgb_rect.x as usize, cfg.rgb_rect.y as usize);
    let (ax, ay) = (cfg.alpha_rect.x as usize, cfg.alpha_rect.y as usize);
    // 定点数放大:每输出一列/行,alpha 源前进多少个 1/65536 像素。
    // 用定点而不是逐像素浮点乘,是因为这段是逐像素热路径
    // (750×1624 = 121 万次),而且定点消掉了 f32 累加的漂移
    let step_x = ((cfg.alpha_rect.w as u64) << 16) / cfg.rgb_rect.w.max(1) as u64;
    let step_y = ((cfg.alpha_rect.h as u64) << 16) / cfg.rgb_rect.h.max(1) as u64;

    for y in 0..h {
        let sy = ry + y;
        let asy = ay + ((y as u64 * step_y) >> 16) as usize;
        // 行内两个源行的起始字节;行首算一次,列循环里只做加法
        let src_row = (sy * vw + rx) * 3;
        let a_row = asy * vw * 3;
        let dst_row = y * w * 4;
        let mut ax_fx = 0u64;
        for x in 0..w {
            let s = src_row + x * 3;
            let a_off = a_row + (ax + (ax_fx >> 16) as usize) * 3;
            ax_fx += step_x;
            // alpha 区是灰度,取 R 通道即可(实测 99.7% 满足 R=G=B;
            // 剩下 0.3% 是 H.264 色度子采样在边缘留下的零头,取哪个通道都一样)
            let a = frame[a_off];
            let d = dst_row + x * 4;
            match mode {
                AlphaMode::Straight => {
                    out[d] = frame[s];
                    out[d + 1] = frame[s + 1];
                    out[d + 2] = frame[s + 2];
                }
                AlphaMode::Premultiplied => {
                    // `(c*a + 127)/255` 是对 `round(c*a/255)` 的精确整数写法,
                    // 且保证 r ≤ a(否则某些后端会判定为非法预乘像素)
                    out[d] = mul255(frame[s], a);
                    out[d + 1] = mul255(frame[s + 1], a);
                    out[d + 2] = mul255(frame[s + 2], a);
                }
            }
            out[d + 3] = a;
        }
    }
    Ok(out)
}

#[inline]
fn mul255(c: u8, a: u8) -> u8 {
    let v = c as u32 * a as u32;
    ((v + 127) / 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Rect;

    /// 造一个 VAP 布局:左 RGB(w×h),右 alpha(w/2 × h/2),模仿真实素材
    fn cfg(w: u32, h: u32) -> VapConfig {
        VapConfig {
            version: 2,
            frames: 1,
            width: w,
            height: h,
            fps: 30.0,
            video_width: w + w / 2,
            video_height: h,
            alpha_rect: Rect {
                x: w,
                y: 0,
                w: w / 2,
                h: h / 2,
            },
            rgb_rect: Rect { x: 0, y: 0, w, h },
            is_vapx: false,
            code_tags: vec![],
            orientation: 0,
        }
    }

    /// 按布局造一帧:RGB 区给可定位的渐变,alpha 区给灰度
    fn frame(c: &VapConfig, alpha_of: impl Fn(u32, u32) -> u8) -> Vec<u8> {
        let (vw, vh) = (c.video_width as usize, c.video_height as usize);
        let mut f = vec![0u8; vw * vh * 3];
        for y in 0..c.rgb_rect.h {
            for x in 0..c.rgb_rect.w {
                let i = ((y as usize) * vw + x as usize) * 3;
                f[i] = (x % 256) as u8;
                f[i + 1] = (y % 256) as u8;
                f[i + 2] = 77;
            }
        }
        for y in 0..c.alpha_rect.h {
            for x in 0..c.alpha_rect.w {
                let i = ((c.alpha_rect.y + y) as usize * vw + (c.alpha_rect.x + x) as usize) * 3;
                let a = alpha_of(x, y);
                f[i] = a;
                f[i + 1] = a;
                f[i + 2] = a;
            }
        }
        f
    }

    #[test]
    fn rgb_is_taken_verbatim_and_alpha_is_upscaled() {
        let c = cfg(8, 8);
        // alpha 左半 0、右半 255 —— 放大后输出应在 x=4 处翻转
        let f = frame(&c, |x, _| if x < 2 { 0 } else { 255 });
        let out = composite_rgba(&c, &f, AlphaMode::Straight).unwrap();
        let px = |x: usize, y: usize| {
            let i = (y * 8 + x) * 4;
            (out[i], out[i + 1], out[i + 2], out[i + 3])
        };
        // RGB 逐字节照抄
        assert_eq!(px(3, 5).0, 3);
        assert_eq!(px(3, 5).1, 5);
        assert_eq!(px(3, 5).2, 77);
        // alpha 半分辨率放大:源 x<2 → 输出 x<4
        assert_eq!(px(0, 0).3, 0);
        assert_eq!(px(3, 0).3, 0);
        assert_eq!(px(4, 0).3, 255);
        assert_eq!(px(7, 0).3, 255);
    }

    #[test]
    fn alpha_upscales_on_the_y_axis_too() {
        // 只测 x 会漏掉 step_y 写错(比如把 step_x 用在两个轴上)——
        // 那种错在正方形素材上完全看不出来
        let c = cfg(8, 8);
        let f = frame(&c, |_, y| if y < 2 { 10 } else { 200 });
        let out = composite_rgba(&c, &f, AlphaMode::Straight).unwrap();
        let a = |x: usize, y: usize| out[(y * 8 + x) * 4 + 3];
        assert_eq!(a(0, 0), 10);
        assert_eq!(a(0, 3), 10);
        assert_eq!(a(0, 4), 200);
        assert_eq!(a(0, 7), 200);
    }

    #[test]
    fn premultiplied_matches_the_exact_integer_formula() {
        let c = cfg(4, 4);
        let f = frame(&c, |_, _| 128);
        let s = composite_rgba(&c, &f, AlphaMode::Straight).unwrap();
        let p = composite_rgba(&c, &f, AlphaMode::Premultiplied).unwrap();
        for i in (0..s.len()).step_by(4) {
            let a = s[i + 3];
            assert_eq!(p[i + 3], a, "alpha 通道本身不该被乘");
            for ch in 0..3 {
                let expect = ((s[i + ch] as u32 * a as u32) + 127) / 255;
                assert_eq!(p[i + ch] as u32, expect);
                // 预乘的硬约束:任何通道都不得超过 alpha
                assert!(p[i + ch] <= a, "预乘像素 r>a 会被后端判为非法");
            }
        }
    }

    #[test]
    fn fully_transparent_and_fully_opaque_are_exact() {
        let c = cfg(4, 4);
        let f = frame(&c, |x, _| if x == 0 { 0 } else { 255 });
        let p = composite_rgba(&c, &f, AlphaMode::Premultiplied).unwrap();
        // a=0 → 预乘后必须是全 0(留下颜色会在缩放时把脏色晕出来)
        assert_eq!(&p[0..4], &[0, 0, 0, 0]);
        // a=255 → 预乘必须恒等,不能因为舍入掉 1
        let s = composite_rgba(&c, &f, AlphaMode::Straight).unwrap();
        let i = 2 * 4;
        assert_eq!(&p[i..i + 4], &s[i..i + 4]);
    }

    #[test]
    fn frame_size_mismatch_is_refused_not_read_out_of_bounds() {
        let c = cfg(8, 8);
        let short = vec![0u8; 10];
        assert!(matches!(
            composite_rgba(&c, &short, AlphaMode::Straight),
            Err(VapError::FrameSizeMismatch { .. })
        ));
        // 多一个字节也拒:多出来的字节意味着口径对不上,
        // 宽容地接受只会让"用错了 pix_fmt"变成一张糊图而不是一条报错
        let long = vec![0u8; (c.video_width * c.video_height * 3 + 1) as usize];
        assert!(composite_rgba(&c, &long, AlphaMode::Straight).is_err());
    }

    #[test]
    fn one_to_one_alpha_needs_no_scaling() {
        // 不是所有素材都半分辨率:等分辨率时 step 必须正好是 1.0
        let mut c = cfg(8, 8);
        c.alpha_rect = Rect {
            x: 8,
            y: 0,
            w: 8,
            h: 8,
        };
        c.video_width = 16;
        let f = frame(&c, |x, y| ((x * 8 + y) % 256) as u8);
        let out = composite_rgba(&c, &f, AlphaMode::Straight).unwrap();
        for y in 0..8usize {
            for x in 0..8usize {
                assert_eq!(out[(y * 8 + x) * 4 + 3], ((x * 8 + y) % 256) as u8);
            }
        }
    }
}
