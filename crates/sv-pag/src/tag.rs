//! 标签(tag)编码与标签号表。
//!
//! PAG 的 body 是一串 SWF 风格的 TLV 标签。标签号表逐字抄自
//! libpag `include/pag/file.h` 的 `enum class TagCode`。

use crate::error::PagError;
use crate::reader::Reader;

/// 标签头:标签号 + 体长度(字节)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TagHeader {
    pub(crate) code: u16,
    pub(crate) length: u32,
}

// ---- 本 crate 真正会**下钻解析**的标签号 ----
// 其余标签一律按长度跳过,绝不猜结构。

/// 结束标签。`WriteEndTag` 写的就是一个 `uint16 0`(code 0 + length 0)。
pub(crate) const END: u16 = 0;
pub(crate) const VECTOR_COMPOSITION_BLOCK: u16 = 2;
pub(crate) const COMPOSITION_ATTRIBUTES: u16 = 3;
pub(crate) const BITMAP_COMPOSITION_BLOCK: u16 = 45;
pub(crate) const BITMAP_SEQUENCE: u16 = 46;
pub(crate) const VIDEO_COMPOSITION_BLOCK: u16 = 50;
pub(crate) const VIDEO_SEQUENCE: u16 = 51;

/// 读一个标签头。
///
/// 打包方式(`src/codec/TagHeader.cpp::ReadTagHeader`,逐字对照):
///
/// ```text
/// codeAndLength : uint16 小端
///   length = codeAndLength & 63        // 低 6 位
///   code   = codeAndLength >> 6        // 高 10 位
/// if length == 63:
///   length = uint32 小端               // 长体的逃逸路径
/// ```
///
/// 也就是说标签号最大 1023、短体长度最大 62。`length == 63` 是**逃逸标记**
/// 而不是真长度 —— 长度恰好是 63 的体也必须走 uint32 路径
/// (`WriteTypeAndLength` 里是 `if (length < 63)`,不是 `<= 63`)。
pub(crate) fn read_tag_header(r: &mut Reader<'_>) -> Result<TagHeader, PagError> {
    let code_and_length = r.read_u16()?;
    let mut length = u32::from(code_and_length & 63);
    let code = code_and_length >> 6;
    if length == 63 {
        length = r.read_u32()?;
    }
    Ok(TagHeader { code, length })
}

/// 标签号 → 官方名字。用于诊断/调试输出。
///
/// 全表逐字抄自 libpag `include/pag/file.h` 的 `enum class TagCode`
/// (含它自己的注释:34~44 是保留段;`Count` 是哨兵不是标签)。
/// 返回 `None` 表示这个号在我们核实的表里没有 —— 可能是保留段,
/// 也可能是比我们核实时更新的 libpag 加的新标签。
pub fn tag_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0 => "End",
        1 => "FontTables",
        2 => "VectorCompositionBlock",
        3 => "CompositionAttributes",
        4 => "ImageTables",
        5 => "LayerBlock",
        6 => "LayerAttributes",
        7 => "SolidColor",
        8 => "TextSource",
        10 => "TextMoreOption",
        11 => "ImageReference",
        12 => "CompositionReference",
        13 => "Transform2D",
        14 => "MaskBlock",
        15 => "ShapeGroup",
        16 => "Rectangle",
        17 => "Ellipse",
        18 => "PolyStar",
        19 => "ShapePath",
        20 => "Fill",
        21 => "Stroke",
        22 => "GradientFill",
        23 => "GradientStroke",
        24 => "MergePaths",
        25 => "TrimPaths",
        26 => "Repeater",
        27 => "RoundCorners",
        28 => "Performance",
        29 => "DropShadowStyle",
        30 => "CachePolicy",
        31 => "FileAttributes",
        32 => "TimeStretchMode",
        33 => "Mp4Header",
        // 34 ~ 44 是官方注释标明的保留段
        45 => "BitmapCompositionBlock",
        46 => "BitmapSequence",
        47 => "ImageBytes",
        48 => "ImageBytesV2",
        49 => "ImageBytesV3",
        50 => "VideoCompositionBlock",
        51 => "VideoSequence",
        52 => "LayerAttributesV2",
        53 => "MarkerList",
        54 => "ImageFillRule",
        55 => "AudioBytes",
        56 => "MotionTileEffect",
        57 => "LevelsIndividualEffect",
        58 => "CornerPinEffect",
        59 => "BulgeEffect",
        60 => "FastBlurEffect",
        61 => "GlowEffect",
        62 => "LayerAttributesV3",
        63 => "LayerAttributesExtra",
        64 => "TextSourceV2",
        65 => "DropShadowStyleV2",
        66 => "DisplacementMapEffect",
        67 => "ImageFillRuleV2",
        68 => "TextSourceV3",
        69 => "TextPathOption",
        70 => "TextAnimator",
        71 => "TextRangeSelector",
        72 => "TextAnimatorPropertiesTrackingType",
        73 => "TextAnimatorPropertiesTrackingAmount",
        74 => "TextAnimatorPropertiesFillColor",
        75 => "TextAnimatorPropertiesStrokeColor",
        76 => "TextAnimatorPropertiesPosition",
        77 => "TextAnimatorPropertiesScale",
        78 => "TextAnimatorPropertiesRotation",
        79 => "TextAnimatorPropertiesOpacity",
        80 => "TextWigglySelector",
        81 => "RadialBlurEffect",
        82 => "MosaicEffect",
        83 => "EditableIndices",
        84 => "MaskBlockV2",
        85 => "GradientOverlayStyle",
        86 => "BrightnessContrastEffect",
        87 => "HueSaturationEffect",
        88 => "LayerAttributesExtraV2",
        89 => "EncryptedData",
        90 => "Transform3D",
        91 => "CameraOption",
        92 => "StrokeStyle",
        93 => "OuterGlowStyle",
        94 => "ImageScaleModes",
        _ => return None,
    })
}
