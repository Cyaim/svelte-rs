//! # sv-pag —— 纯 Rust 读腾讯 PAG 容器
//!
//! 只做一件事:**把 `.pag` 文件的容器拆开**,取出宽高/帧率/时长,判定它是矢量档
//! 还是位图序列帧档,并在是序列帧档时把每帧的**编码后字节**原样交出来。
//!
//! ## 为什么不绑 libpag(以及这条论据管到哪为止)
//!
//! `docs/plans/pag-2-integration.md` §0 的裁决是**硬否**,但它的适用范围要抄全:
//!
//! - **运行期硬否,理由充分**:libpag → tgfx 的 C++ 依赖闭包里有 `pathkit`
//!   (仓库自述 "extracted from the Skia library")与 `skcms`,ADR-3 排除
//!   skia-safe 的理由在这里原样复现且更重;官方**不发 Windows/Linux 预编译库**;
//!   依赖同步还要 Node.js 的 `depsync` 在构建期联网 clone 二十多个仓库 ——
//!   cargo 生态无法 vendor、无法离线、无法发 crates.io。
//! - **构建期不是硬否 —— 这一条本 crate 早期文档漏了限定,现补上。**
//!   计划文件 `:28` 对上面那条原文加了限定:「**注意这条只管运行期**:构建期
//!   另有 wasm 通道,见 §3.3 c2′」;`:175` 记着 npm `libpag` 4.5.81 是
//!   **预编译 wasm、Apache-2.0、吃真 `.pag`、走完整 codec、不需要任何 C++
//!   工具链**,并明写这是"'Windows/Linux 无预编译库'这条结论**在构建期不成立**
//!   的原因"。
//!
//! 所以本 crate **不能**说"c2 根本不需要 libpag"。它能说的是这句窄得多的话:
//!
//! > 计划里的 c2′ 首选方案(Node + 无头 GL/puppeteer 跑 wasm libpag 逐帧
//! > seek 出帧)带着一个**计划自己标明"开工前第一个 spike,不许跳过"的未核实
//! > 前提**。而对**已经按位图序列帧模式导出**的素材,帧数据本来就在 `.pag` 里,
//! > 这一档可以完全绕开那条链路 —— 只需要我们自己会读这个容器。
//!
//! 覆盖多大一块由素材决定,不由本 crate 决定:能不能绕开,用
//! [`PagFile::is_pure_bitmap`] 一个文件一个文件地问。结构核实见 `README.md`。
//!
//! ## 能读什么 / 不能读什么
//!
//! | 能 | 不能 |
//! |---|---|
//! | 文件头(魔数/版本/体长/压缩标志) | 加密档(版本 3)—— 显式报错 |
//! | 标签流(SWF 风格 TLV) | ZLIB/LZMA 压缩体 —— 显式报错 |
//! | 宽高 / 帧率 / 时长 / 背景色 | 矢量图层(形状/文本/蒙版/效果) |
//! | 矢量档 / 位图档 / 视频档的判定 | 视频序列(H.264)—— 只计数 |
//! | 位图序列每帧的编码字节 + 编码类型 | **不解码图片**(零图像依赖) |
//!
//! ## 头号缺口
//!
//! **本解析器从未在 AE 插件真实导出的 `.pag` 上验证过。** 全部测试固件都是
//! 按 libpag 源码手工构造的字节序列。所有字段布局都能指到 libpag 的源码行
//! (见 `README.md` 核实表),但"读源码读对了"和"真文件能过"是两件事。
//!
//! ## 用法
//!
//! ```no_run
//! use sv_pag::PagFile;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let bytes = std::fs::read("loading.pag")?;
//! let pag = PagFile::parse(&bytes)?;
//!
//! if let Some(attrs) = pag.attributes() {
//!     println!("{}x{} @ {}fps", attrs.width, attrs.height, attrs.frame_rate);
//! }
//!
//! // 注意是 is_pure_bitmap() 而**不是** kind() == FileKind::Bitmap ——
//! // 后者对"矢量根套位图子 composition"的混合档同样为真,而那种档
//! // 不能只靠重放序列还原(矢量根的变换/蒙版在 LayerBlock 里,我们不解析)。
//! if pag.is_pure_bitmap() {
//!     for seq in pag.bitmap_sequences() {
//!         // 每帧的字节可以直接喂给上层的图片解码器
//!         for (i, frame) in seq.frames.iter().enumerate() {
//!             for rect in &frame.bitmaps {
//!                 println!(
//!                     "帧 {i} 块 ({},{}) {:?} {} 字节",
//!                     rect.x,
//!                     rect.y,
//!                     rect.encoding(),
//!                     rect.bytes.len()
//!                 );
//!             }
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## 纪律
//!
//! - **零依赖**,`#![forbid(unsafe_code)]`。
//! - **不 panic**:任何输入(截断/畸形/声称长度越界)都返回 `Err`。
//!   而且不靠"把 panic 转移给调用方"来实现 —— libpag 的 `verify()` 门
//!   (负宽高 / 零帧率 / 空序列 / 零长度块)我们照样把文件拒掉,
//!   见 [`VerifyFailure`]。
//! - **不猜格式**:没核实过布局的标签一律按长度跳过,绝不臆测解析。

#![forbid(unsafe_code)]

mod error;
mod model;
mod parse;
mod reader;
mod replay;
mod tag;

pub use error::{PagError, VerifyFailure};
pub use model::{
    BitmapFrame, BitmapRect, BitmapSequence, Composition, CompositionAttributes, CompositionKind,
    FileKind, ImageEncoding, PagFile,
};
pub use replay::{DecodedImage, replay_frame};
pub use tag::tag_name;
