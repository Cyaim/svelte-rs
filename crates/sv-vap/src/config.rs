//! `vapc` 配置:VAP 用来描述"这段 MP4 该怎么拆"的那张表。
//!
//! 它同时存在于两个地方(实测同一份素材两处**逐字节一致**):
//! - MP4 里一个自定义顶层 box,box 类型就是 `vapc`;
//! - 一个同名的 `.json` 旁车文件。
//!
//! 所以只发 MP4 也能播 —— 旁车 JSON 是给不方便读 box 的地方(比如某些 Web 播放器)准备的。

use crate::VapError;

/// 一段 VAP 动画的全部几何与时间信息。
///
/// 字段名保持 VAP 原样的缩写(`w`/`f`/`aFrame`…),不改成"更好听"的名字 ——
/// 排查问题时要拿它和素材方给的 JSON 对着看,改名只会多一层翻译。
#[derive(Clone, Debug, PartialEq)]
pub struct VapConfig {
    /// 配置版本(实测素材全是 2)
    pub version: i64,
    /// 总帧数
    pub frames: u32,
    /// **显示**尺寸(不是视频尺寸)
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    /// MP4 的真实尺寸。**通常大于显示尺寸**,因为它要同时装下 RGB 与 alpha,
    /// 而且要对齐到 H.264 的宏块(实测 1136 = 71×16,而内容只到 1129)
    pub video_width: u32,
    pub video_height: u32,
    /// alpha 区在视频帧里的位置 `[x, y, w, h]`。
    /// **可以比 RGB 区小** —— alpha 是低频信息,VAP 允许它降采样
    /// (实测素材:RGB 750×1624,alpha 375×812,正好一半)
    pub alpha_rect: Rect,
    /// RGB 区在视频帧里的位置 `[x, y, w, h]`
    pub rgb_rect: Rect,
    /// 是否是 VAPX(融合动画:运行期往里塞头像/昵称等动态元素)。
    /// **本 crate 只处理 `false`**;为 true 时 [`VapConfig::parse`] 仍然解析成功,
    /// 由调用方决定要不要拒 —— 因为基础层照样能放,只是少了动态元素
    pub is_vapx: bool,
    /// 素材方打的水印/追踪串。解析出来是为了排查素材来源,不参与渲染
    pub code_tags: Vec<String>,
    /// 朝向标记(实测全是 0;VAP 用它处理横竖屏)
    pub orientation: i64,
}

/// `[x, y, w, h]`,像素,原点在视频帧左上角
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// 这个矩形是否完整落在 `vw × vh` 的帧内。
    ///
    /// 越界的配置**不是**理论问题:`x + w` 用 u32 会溢出,而溢出之后
    /// 采样偏移会绕回帧内某个位置 —— 表现是"画面错位"而不是报错,
    /// 那是最难查的一类。所以这里用 `checked_add`
    pub fn fits_in(&self, vw: u32, vh: u32) -> bool {
        matches!(self.x.checked_add(self.w), Some(r) if r <= vw)
            && matches!(self.y.checked_add(self.h), Some(b) if b <= vh)
    }
}

impl VapConfig {
    /// 解析 `vapc` JSON。
    ///
    /// **手写解析而不引 serde**:整个 crate 只需要读这一种固定形状的表,
    /// 而 sv-ui/sv-shell 一线的纪律是依赖面尽量干净(sv-pag 是零依赖)。
    /// 这个解析器**只认它需要的键**,多余的键一律跳过 —— 于是 VAP 以后
    /// 加字段不会让这里报错。
    pub fn parse(json: &str) -> Result<Self, VapError> {
        let info = json_object_at(json, "info").ok_or(VapError::MissingInfo)?;
        let need = |k: &str| -> Result<i64, VapError> {
            json_number(info, k)
                .map(|v| v as i64)
                .ok_or(VapError::MissingField(k.to_string()))
        };
        let rect = |k: &str| -> Result<Rect, VapError> {
            let a = json_int_array(info, k).ok_or(VapError::MissingField(k.to_string()))?;
            if a.len() != 4 {
                return Err(VapError::BadRect(k.to_string(), a.len()));
            }
            let neg = a.iter().any(|v| *v < 0);
            if neg {
                return Err(VapError::BadRect(k.to_string(), a.len()));
            }
            Ok(Rect {
                x: a[0] as u32,
                y: a[1] as u32,
                w: a[2] as u32,
                h: a[3] as u32,
            })
        };

        let version = need("v")?;
        let frames = need("f")?;
        let width = need("w")?;
        let height = need("h")?;
        let video_width = need("videoW")?;
        let video_height = need("videoH")?;
        let fps = json_number(info, "fps").ok_or(VapError::MissingField("fps".into()))?;
        let alpha_rect = rect("aFrame")?;
        let rgb_rect = rect("rgbFrame")?;

        if frames < 0 || width <= 0 || height <= 0 || video_width <= 0 || video_height <= 0 {
            return Err(VapError::BadGeometry);
        }
        if !(fps > 0.0 && fps.is_finite()) {
            return Err(VapError::BadGeometry);
        }
        let (vw, vh) = (video_width as u32, video_height as u32);
        // 两个区都必须落在帧内 —— 否则采样会读到别的行,画面错位而不报错
        if !alpha_rect.fits_in(vw, vh) || !rgb_rect.fits_in(vw, vh) {
            return Err(VapError::RectOutOfFrame);
        }
        // RGB 区必须能盖住显示尺寸,否则显示什么都不确定
        if rgb_rect.w < width as u32 || rgb_rect.h < height as u32 {
            return Err(VapError::RgbSmallerThanDisplay);
        }
        if alpha_rect.w == 0 || alpha_rect.h == 0 {
            return Err(VapError::BadGeometry);
        }

        Ok(VapConfig {
            version,
            frames: frames as u32,
            width: width as u32,
            height: height as u32,
            fps: fps as f32,
            video_width: vw,
            video_height: vh,
            alpha_rect,
            rgb_rect,
            is_vapx: json_number(info, "isVapx").unwrap_or(0.0) != 0.0,
            code_tags: json_string_array(info, "codeTag").unwrap_or_default(),
            orientation: json_number(info, "orien").unwrap_or(0.0) as i64,
        })
    }

    /// 总时长(毫秒)
    pub fn duration_ms(&self) -> f32 {
        self.frames as f32 * 1000.0 / self.fps
    }

    /// alpha 相对 RGB 的缩放比(实测素材是 0.5)。
    /// 用它判断要不要放大采样 —— 等于 1.0 时可以走整像素快路
    pub fn alpha_scale(&self) -> (f32, f32) {
        (
            self.alpha_rect.w as f32 / self.rgb_rect.w as f32,
            self.alpha_rect.h as f32 / self.rgb_rect.h as f32,
        )
    }
}

// ---------------------------------------------------------------------------
// 极小 JSON 取值。**不是通用解析器** —— 只够读 vapc 这一种固定形状。
//
// 刻意不做完整 JSON:完整实现要处理转义、Unicode、嵌套数组、科学计数……
// 而 vapc 是编码器生成的、形状固定的一小段。做全套只会增加出错面。
// 代价写在这里:**它假设值里不含转义的引号**(codeTag 实测是纯 ASCII 标识串)。
// ---------------------------------------------------------------------------

/// 取 `"key":{...}` 的对象体(含外层花括号)
fn json_object_at<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let i = s.find(&pat)?;
    let rest = &s[i + pat.len()..];
    let c = rest.find(':')?;
    let body = rest[c + 1..].trim_start();
    if !body.starts_with('{') {
        return None;
    }
    let mut depth = 0usize;
    for (n, ch) in body.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&body[..=n]);
                }
            }
            _ => {}
        }
    }
    None
}

fn value_after<'a>(s: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\"");
    let i = s.find(&pat)?;
    let rest = &s[i + pat.len()..];
    let c = rest.find(':')?;
    Some(rest[c + 1..].trim_start())
}

fn json_number(s: &str, key: &str) -> Option<f64> {
    let v = value_after(s, key)?;
    let end = v
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e'))
        .unwrap_or(v.len());
    v[..end].parse().ok()
}

fn json_int_array(s: &str, key: &str) -> Option<Vec<i64>> {
    let v = value_after(s, key)?;
    if !v.starts_with('[') {
        return None;
    }
    let end = v.find(']')?;
    Some(
        v[1..end]
            .split(',')
            .filter_map(|t| t.trim().parse::<i64>().ok())
            .collect(),
    )
}

fn json_string_array(s: &str, key: &str) -> Option<Vec<String>> {
    let v = value_after(s, key)?;
    if !v.starts_with('[') {
        return None;
    }
    let end = v.find(']')?;
    let mut out = Vec::new();
    let mut cur = &v[1..end];
    while let Some(a) = cur.find('"') {
        let rest = &cur[a + 1..];
        let Some(b) = rest.find('"') else { break };
        out.push(rest[..b].to_string());
        cur = &rest[b + 1..];
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实素材的配置,逐字节抄自
    /// `礼物数据/5014-龙啸苍穹/5014-龙啸苍穹.json`(2026-07-22)
    pub(crate) const REAL: &str = r#"{"info":{"v":2,"f":150,"w":750,"h":1624,"fps":30,"videoW":1136,"videoH":1632,"aFrame":[754,0,375,812],"rgbFrame":[0,0,750,1624],"isVapx":0,"codeTag":["_www.17ae.com_G202603161027464856917"],"orien":0}}"#;

    #[test]
    fn parses_a_real_asset_config() {
        let c = VapConfig::parse(REAL).expect("真实素材配置应能解析");
        assert_eq!(c.version, 2);
        assert_eq!(c.frames, 150);
        assert_eq!((c.width, c.height), (750, 1624));
        assert_eq!(c.fps, 30.0);
        assert_eq!((c.video_width, c.video_height), (1136, 1632));
        assert_eq!(
            c.alpha_rect,
            Rect {
                x: 754,
                y: 0,
                w: 375,
                h: 812
            }
        );
        assert_eq!(
            c.rgb_rect,
            Rect {
                x: 0,
                y: 0,
                w: 750,
                h: 1624
            }
        );
        assert!(!c.is_vapx);
        assert_eq!(c.code_tags, vec!["_www.17ae.com_G202603161027464856917"]);
        assert_eq!(c.orientation, 0);
        // alpha 是半分辨率 —— 这是本 crate 最容易写错的一处,钉死它
        assert_eq!(c.alpha_scale(), (0.5, 0.5));
        assert!((c.duration_ms() - 5000.0).abs() < 0.01);
    }

    #[test]
    fn rejects_rects_that_escape_the_frame() {
        // alpha 区右边越界:x+w = 1200 > videoW=1136
        let bad = REAL.replace("[754,0,375,812]", "[754,0,446,812]");
        assert_eq!(VapConfig::parse(&bad), Err(VapError::RectOutOfFrame));
        // 下边越界
        let bad = REAL.replace("[0,0,750,1624]", "[0,100,750,1624]");
        assert_eq!(VapConfig::parse(&bad), Err(VapError::RectOutOfFrame));
    }

    #[test]
    fn rect_overflow_does_not_wrap() {
        // x + w 溢出 u32:若用裸加法会绕回一个小数字、然后"通过"检查,
        // 表现是画面错位而不是报错 —— 那是最难查的一类
        let r = Rect {
            x: u32::MAX,
            y: 0,
            w: 10,
            h: 10,
        };
        assert!(!r.fits_in(1136, 1632));
    }

    #[test]
    fn rejects_degenerate_geometry() {
        for (from, to) in [
            ("\"fps\":30", "\"fps\":0"),
            ("\"w\":750", "\"w\":0"),
            ("\"videoW\":1136", "\"videoW\":0"),
        ] {
            let bad = REAL.replace(from, to);
            assert!(
                VapConfig::parse(&bad).is_err(),
                "{to} 应被拒绝(否则后面会除零或越界采样)"
            );
        }
    }

    #[test]
    fn rejects_rgb_smaller_than_display() {
        // 显示 750×1624 却只给 375 宽的 RGB:显示什么完全不确定
        let bad = REAL.replace("\"rgbFrame\":[0,0,750,1624]", "\"rgbFrame\":[0,0,375,1624]");
        assert_eq!(VapConfig::parse(&bad), Err(VapError::RgbSmallerThanDisplay));
    }

    #[test]
    fn missing_fields_are_named_not_swallowed() {
        let bad = REAL.replace("\"fps\":30,", "");
        assert_eq!(
            VapConfig::parse(&bad),
            Err(VapError::MissingField("fps".into()))
        );
        assert_eq!(VapConfig::parse("{}"), Err(VapError::MissingInfo));
    }

    #[test]
    fn unknown_keys_are_ignored_so_future_vap_versions_still_load() {
        let future = REAL.replace(
            "\"orien\":0",
            "\"orien\":0,\"someFutureThing\":{\"a\":1},\"another\":[1,2,3]",
        );
        assert!(VapConfig::parse(&future).is_ok());
    }

    #[test]
    fn vapx_parses_but_is_flagged() {
        // VAPX 的基础层照样能放,只是少了动态元素 —— 所以解析成功但标出来,
        // 由调用方决定拒不拒
        let x = REAL.replace("\"isVapx\":0", "\"isVapx\":1");
        let c = VapConfig::parse(&x).expect("VAPX 也该解析成功");
        assert!(c.is_vapx);
    }
}
