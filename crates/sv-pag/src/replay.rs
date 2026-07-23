//! 位图序列的**差分帧重放**:关键帧 + 脏矩形 → 一整张 RGBA 画布。
//!
//! PAG 的位图序列不是"每帧一张完整图",而是关键帧 + 脏矩形差分(见
//! [`BitmapSequence::start_frame_for`] 的长注释)。要画出第 N 帧,得从最近的
//! 关键帧起,逐帧把每个 [`BitmapRect`] 解码后**覆盖**贴到画布上。本模块就做这件
//! 复原,且**仍然零图像依赖**:解码通过一个注入的回调完成(上层插自己的
//! WebP/PNG 解码器),本模块只管容器语义 —— 找关键帧、按 (x,y) 覆盖贴、边界裁剪。
//!
//! 这样分层的原因和整个 crate 一致(crate 文档"能读/不能读"表最后一行):
//! **不解码图片**。解码器是平台强相关的一次独立裁决;把它做成回调,重放逻辑
//! 就能脱离任何解码器被单独测试(见本模块测试:用一个把 1 字节标记映射成纯色的
//! 假解码器,验证关键帧铺底 + 差分覆盖 + 边界裁剪都对)。

use crate::BitmapSequence;

/// 一张解码后的图片:RGBA8,行主序,无行间 padding(`rgba.len() == width*height*4`)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl DecodedImage {
    /// 校验 `rgba` 长度与宽高自洽。解码器回调的产物先过这一关,不然贴图会越界读。
    fn is_consistent(&self) -> bool {
        (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|px| px.checked_mul(4))
            .map(|need| need == self.rgba.len())
            .unwrap_or(false)
    }
}

/// 把位图序列的第 `target` 帧(0 基)差分重放成一整张 RGBA 画布。
///
/// `decode` 把一个 [`BitmapRect`](crate::BitmapRect) 的编码字节变成 [`DecodedImage`];
/// 返回 `None` 表示解不了(上层的解码器不认这个编码)。
///
/// 返回 `None` 的情形:`target` 越界 / 找不到关键帧 / 画布宽高非正 / 某块解码失败
/// / 解码产物宽高与像素数对不上 / 某块的原点落在画布外。**块超出画布右下边界会被
/// 裁剪**(libpag 同款:脏矩形贴边很常见),但原点为负或落在画布外视为畸形。
pub fn replay_frame<F>(seq: &BitmapSequence, target: usize, decode: F) -> Option<DecodedImage>
where
    F: Fn(&[u8]) -> Option<DecodedImage>,
{
    let start = seq.start_frame_for(target)?;
    if seq.width <= 0 || seq.height <= 0 {
        return None;
    }
    let (cw, ch) = (seq.width as u32, seq.height as u32);
    let mut canvas = vec![0u8; (cw as usize) * (ch as usize) * 4];

    for frame in &seq.frames[start..=target] {
        for rect in &frame.bitmaps {
            let img = decode(rect.bytes)?;
            if !img.is_consistent() {
                return None;
            }
            blit_over(&mut canvas, cw, ch, &img, rect.x, rect.y)?;
        }
    }
    Some(DecodedImage {
        width: cw,
        height: ch,
        rgba: canvas,
    })
}

/// 覆盖贴(libpag `writePixels` 语义,**不是** alpha 混合:差分块就是那块的新内容)。
/// 原点为负 / 落在画布外 → `None`(畸形);超出右/下边界 → 裁掉那部分。
fn blit_over(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    img: &DecodedImage,
    x: i32,
    y: i32,
) -> Option<()> {
    if x < 0 || y < 0 {
        return None;
    }
    let (ox, oy) = (x as u32, y as u32);
    if ox >= cw || oy >= ch {
        return None;
    }
    // 裁到画布内的行列范围
    let cols = img.width.min(cw - ox);
    let rows = img.height.min(ch - oy);
    for row in 0..rows {
        let src = ((row * img.width) as usize) * 4;
        let dst = (((oy + row) * cw + ox) as usize) * 4;
        let n = (cols as usize) * 4;
        canvas[dst..dst + n].copy_from_slice(&img.rgba[src..src + n]);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BitmapFrame, BitmapRect};

    // 假解码器:第一个字节是颜色标记 → 一张 s×s 的纯色图。真解码器会认 WebP 头,
    // 这里只为脱离解码器验证**重放逻辑**(关键帧铺底 + 差分覆盖 + 裁剪)。
    fn fake_decode(bytes: &[u8]) -> Option<DecodedImage> {
        let (tag, s) = (bytes[0], bytes[1] as u32);
        let color = match tag {
            b'R' => [255, 0, 0, 255],
            b'G' => [0, 255, 0, 255],
            b'B' => [0, 0, 255, 255],
            _ => return None,
        };
        Some(DecodedImage {
            width: s,
            height: s,
            rgba: (0..s * s).flat_map(|_| color).collect(),
        })
    }

    fn px(img: &DecodedImage, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * img.width + x) * 4) as usize;
        img.rgba[i..i + 4].try_into().unwrap()
    }

    fn seq(frames: Vec<BitmapFrame<'static>>) -> BitmapSequence<'static> {
        BitmapSequence {
            width: 4,
            height: 4,
            frame_rate: 30.0,
            frames,
        }
    }

    // 关键帧:一整块 4×4 红;差分帧:(1,1) 处 2×2 绿
    const KEY_RED: &[u8] = b"R\x04"; // R, size 4
    const DIFF_GREEN: &[u8] = b"G\x02"; // G, size 2

    #[test]
    fn keyframe_fills_canvas() {
        let s = seq(vec![BitmapFrame {
            is_keyframe: true,
            bitmaps: vec![BitmapRect {
                x: 0,
                y: 0,
                bytes: KEY_RED,
            }],
        }]);
        let out = replay_frame(&s, 0, fake_decode).expect("重放关键帧");
        assert_eq!(out.width, 4);
        // 四角都应是红
        for (x, y) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
            assert_eq!(px(&out, x, y), [255, 0, 0, 255], "({x},{y}) 应红");
        }
    }

    #[test]
    fn diff_frame_composites_over_keyframe() {
        let s = seq(vec![
            BitmapFrame {
                is_keyframe: true,
                bitmaps: vec![BitmapRect {
                    x: 0,
                    y: 0,
                    bytes: KEY_RED,
                }],
            },
            BitmapFrame {
                is_keyframe: false,
                bitmaps: vec![BitmapRect {
                    x: 1,
                    y: 1,
                    bytes: DIFF_GREEN,
                }],
            },
        ]);
        // 帧 1 = 从关键帧铺红,再把 (1,1)-(2,2) 覆盖成绿
        let out = replay_frame(&s, 1, fake_decode).expect("重放差分帧");
        assert_eq!(px(&out, 0, 0), [255, 0, 0, 255], "画布外圈仍红");
        assert_eq!(px(&out, 1, 1), [0, 255, 0, 255], "差分区应绿");
        assert_eq!(px(&out, 2, 2), [0, 255, 0, 255], "差分区应绿");
        assert_eq!(px(&out, 3, 3), [255, 0, 0, 255], "差分区外仍红");

        // 帧 0 不受帧 1 影响(从关键帧独立重放)
        let f0 = replay_frame(&s, 0, fake_decode).unwrap();
        assert_eq!(px(&f0, 1, 1), [255, 0, 0, 255], "帧 0 该处仍红");
    }

    #[test]
    fn oversized_rect_is_clipped_not_panicking() {
        // (3,3) 处贴一张 2×2,右下各超出 1 像素 → 只画进画布内的 1×1
        let big = seq(vec![BitmapFrame {
            is_keyframe: true,
            bitmaps: vec![
                BitmapRect {
                    x: 0,
                    y: 0,
                    bytes: KEY_RED,
                },
                BitmapRect {
                    x: 3,
                    y: 3,
                    bytes: DIFF_GREEN,
                },
            ],
        }]);
        let out = replay_frame(&big, 0, fake_decode).expect("越界块应裁剪而非崩");
        assert_eq!(px(&out, 3, 3), [0, 255, 0, 255], "画布内那一角应绿");
    }

    #[test]
    fn undecodable_or_inconsistent_returns_none() {
        // 解码器不认的标记
        let bad = seq(vec![BitmapFrame {
            is_keyframe: true,
            bitmaps: vec![BitmapRect {
                x: 0,
                y: 0,
                bytes: b"X\x04",
            }],
        }]);
        assert!(
            replay_frame(&bad, 0, fake_decode).is_none(),
            "解不了应 None"
        );

        // 解码器谎报尺寸(说 4×4 却给 1 个像素)→ 一致性检查拦下,不越界读
        let liar = seq(vec![BitmapFrame {
            is_keyframe: true,
            bitmaps: vec![BitmapRect {
                x: 0,
                y: 0,
                bytes: b"!",
            }],
        }]);
        let decode_liar = |_: &[u8]| {
            Some(DecodedImage {
                width: 4,
                height: 4,
                rgba: vec![0, 0, 0, 255],
            })
        };
        assert!(
            replay_frame(&liar, 0, decode_liar).is_none(),
            "宽高与像素数不符应 None,不该越界读"
        );
    }

    #[test]
    fn no_keyframe_before_target_is_none() {
        // target 之前没有关键帧
        let s = seq(vec![BitmapFrame {
            is_keyframe: false,
            bitmaps: vec![],
        }]);
        assert!(replay_frame(&s, 0, fake_decode).is_none());
    }
}
