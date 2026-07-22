//! VAP(腾讯 Video Animation Player)素材解析与合成 —— **纯 Rust、零依赖**。
//!
//! # 这是什么,以及它为什么不是 PAG
//!
//! VAP 与 PAG 都出自腾讯、都用在直播礼物特效上,业内口头常混着叫,
//! 但**文件格式毫无关系**:
//!
//! | | PAG | VAP |
//! |---|---|---|
//! | 载体 | 单个 `.pag` 二进制(magic `PAG`) | **MP4 + 一段 JSON 配置** |
//! | 内容 | 矢量图层树 / 位图序列 / 视频序列 | H.264 视频 |
//! | alpha | 格式自带 | **没有** —— 把 alpha 当灰度并排塞进同一帧 |
//! | 解析 | 见 `sv-pag` | 见本 crate |
//!
//! 所以"买到的素材是 json + mp4"完全正常 —— 那就是 VAP 的交付形态,
//! **不是缺文件**。配置在 MP4 里也内嵌了一份(`vapc` box),旁车 JSON 是副本。
//!
//! # 本 crate 做什么 / 不做什么
//!
//! **做**:读 `vapc` 配置(从 MP4 box 或旁车 JSON)、校验几何、
//! 把一帧解码后的 RGB 合成成 RGBA。
//!
//! **不做:H.264 解码。** 那要引一个视频解码器(ffmpeg/openh264/系统解码器),
//! 是独立于本文件的一次重裁决,而且解码器的选择跟平台强相关。
//! 与 `sv-pag` 把 WebP 解码挡在外面是同一条纪律:**这层只有一个职责。**
//!
//! ```no_run
//! # fn main() -> Result<(), sv_vap::VapError> {
//! let mp4 = std::fs::read("gift.mp4").unwrap();
//! let cfg = sv_vap::VapConfig::parse(sv_vap::find_vapc(&mp4)?)?;
//! // …用任意解码器把第 n 帧解成 RGB24(video_width × video_height × 3)…
//! # let rgb_frame: Vec<u8> = vec![];
//! let rgba = sv_vap::composite_rgba(&cfg, &rgb_frame, sv_vap::AlphaMode::Premultiplied)?;
//! # Ok(())
//! # }
//! ```

mod composite;
mod config;
mod mp4;

pub use composite::{AlphaMode, composite_rgba};
pub use config::{Rect, VapConfig};
pub use mp4::find_vapc;

/// 出错原因。**一律返回 Err,绝不 panic**(仓库 R4 去 panic 纪律):
/// 素材是外部输入,畸形文件不该崩掉整个 app。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum VapError {
    /// MP4 里没有 `vapc` box —— 可能不是 VAP 素材,或者配置只在旁车 JSON 里
    NoVapcBox,
    /// box 声称的长度超出文件
    TruncatedMp4,
    /// box 长度比 box 头还小(畸形;照字面处理会死循环)
    MalformedBox,
    /// `vapc` 载荷不是合法 UTF-8
    VapcNotUtf8,
    /// JSON 里没有 `info` 对象
    MissingInfo,
    /// 缺必需字段(带字段名 —— 不带名字的"解析失败"没法查)
    MissingField(String),
    /// 矩形字段不是 4 个非负整数
    BadRect(String, usize),
    /// 尺寸/帧率为 0 或非有限
    BadGeometry,
    /// RGB 区或 alpha 区跑到视频帧外面了。
    /// **这条必须报错而不是钳**:钳回去只会画出一张错位的图,不会有人发现
    RectOutOfFrame,
    /// RGB 区比显示尺寸还小 —— 显示什么完全不确定
    RgbSmallerThanDisplay,
    /// 送进来的帧字节数与配置里的视频尺寸对不上
    FrameSizeMismatch { expected: usize, got: usize },
}

impl std::fmt::Display for VapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VapError::NoVapcBox => write!(f, "MP4 里没有 vapc box(可能不是 VAP 素材)"),
            VapError::TruncatedMp4 => write!(f, "MP4 被截断:box 声称的长度超出文件"),
            VapError::MalformedBox => write!(f, "MP4 畸形:box 长度比 box 头还小"),
            VapError::VapcNotUtf8 => write!(f, "vapc 载荷不是合法 UTF-8"),
            VapError::MissingInfo => write!(f, "vapc JSON 里没有 info 对象"),
            VapError::MissingField(k) => write!(f, "vapc 缺字段 `{k}`"),
            VapError::BadRect(k, n) => write!(f, "vapc 的 `{k}` 应是 4 个非负整数,实得 {n} 个"),
            VapError::BadGeometry => write!(f, "vapc 的尺寸或帧率非法(为 0 或非有限)"),
            VapError::RectOutOfFrame => write!(f, "RGB 区或 alpha 区超出了视频帧"),
            VapError::RgbSmallerThanDisplay => write!(f, "RGB 区比显示尺寸还小"),
            VapError::FrameSizeMismatch { expected, got } => {
                write!(
                    f,
                    "帧字节数不符:期望 {expected},实得 {got}(pix_fmt 用错了?)"
                )
            }
        }
    }
}

impl std::error::Error for VapError {}
