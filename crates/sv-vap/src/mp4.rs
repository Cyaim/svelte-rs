//! 从 MP4 里把 `vapc` box 抠出来。
//!
//! VAP 把配置作为一个**自定义顶层 box** 塞在 `moov` 之后、`mdat` 之前
//! (实测偏移 7750,紧跟在 `udta/meta` 后面)。box 类型就是 ASCII 的 `vapc`,
//! 载荷是一段 UTF-8 JSON。
//!
//! # 为什么值得单独读它,而不是只用旁车 `.json`
//!
//! 因为**只发 MP4 也能播**。素材方给的一包里旁车 JSON 是有的,但线上分发
//! 常常只推一个 mp4 —— 配置丢了就完全不知道 alpha 在哪儿,而这是致命的:
//! 猜错 alpha 区不会报错,只会画出一张糊图。

use crate::VapError;

/// 扫描 MP4 顶层 box,返回 `vapc` 的载荷(UTF-8 JSON)。
///
/// **只扫顶层**:实测 VAP 的编码器就写在顶层,而深扫要处理 `moov` 内部
/// 各种嵌套容器,徒增出错面。找不到就老实说找不到,让调用方回落到旁车 JSON。
pub fn find_vapc(mp4: &[u8]) -> Result<&str, VapError> {
    let mut p = 0usize;
    while p + 8 <= mp4.len() {
        let size = u32::from_be_bytes([mp4[p], mp4[p + 1], mp4[p + 2], mp4[p + 3]]) as u64;
        let typ = &mp4[p + 4..p + 8];
        // box 尺寸的三种编码(ISO/IEC 14496-12):
        //   1 = 后面跟 u64 大尺寸;0 = 一直到文件尾;其余 = 就是它自己
        let (size, hdr) = match size {
            1 => {
                if p + 16 > mp4.len() {
                    return Err(VapError::TruncatedMp4);
                }
                let mut b = [0u8; 8];
                b.copy_from_slice(&mp4[p + 8..p + 16]);
                (u64::from_be_bytes(b), 16usize)
            }
            0 => ((mp4.len() - p) as u64, 8usize),
            n => (n, 8usize),
        };
        if size < hdr as u64 {
            // 尺寸比头还小 = 畸形。**必须在这里停**,否则下面的 p += size
            // 不前进,循环永远转下去(这类死循环在畸形文件上很常见)
            return Err(VapError::MalformedBox);
        }
        // largesize 分支的 `size` 是攻击者可控的完整 u64,`p + size` 会溢出:
        // debug 直接 panic;release 环绕成 end < p 绕过下面的越界检查,再在
        // `&mp4[p+hdr..end]` 以 start > end 切片 panic。checked_add 把它挡在门外。
        let end = match (p as u64).checked_add(size) {
            Some(e) => e,
            None => return Err(VapError::MalformedBox),
        };
        if end > mp4.len() as u64 {
            return Err(VapError::TruncatedMp4);
        }
        if typ == b"vapc" {
            let body = &mp4[p + hdr..end as usize];
            return std::str::from_utf8(body).map_err(|_| VapError::VapcNotUtf8);
        }
        p = end as usize;
    }
    Err(VapError::NoVapcBox)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(typ: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(typ);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn largesize_overflow_reports_error_instead_of_panicking() {
        // 回归:前导 box 之后跟一个 largesize=u64::MAX 的 box。
        // 修复前:debug 在 `p + size` 算术溢出 panic;release 环绕后越界切片 panic。
        for typ in [b"vapc", b"free"] {
            let mut f = boxed(b"ftyp", b"isomiso2avc1mp41");
            f.extend_from_slice(&[0, 0, 0, 1]); // size==1 → largesize
            f.extend_from_slice(typ);
            f.extend_from_slice(&u64::MAX.to_be_bytes());
            assert!(matches!(find_vapc(&f), Err(VapError::MalformedBox)));
        }
    }

    #[test]
    fn finds_vapc_among_other_boxes() {
        // 顺序照真实文件:ftyp / free / moov / vapc / mdat
        let mut f = boxed(b"ftyp", b"isomiso2avc1mp41");
        f.extend(boxed(b"free", b""));
        f.extend(boxed(b"moov", &[0u8; 64]));
        f.extend(boxed(b"vapc", br#"{"info":{"v":2}}"#));
        f.extend(boxed(b"mdat", &[0u8; 128]));
        assert_eq!(find_vapc(&f).unwrap(), r#"{"info":{"v":2}}"#);
    }

    #[test]
    fn absent_vapc_is_reported_not_guessed() {
        let mut f = boxed(b"ftyp", b"isom");
        f.extend(boxed(b"mdat", &[0u8; 32]));
        assert_eq!(find_vapc(&f), Err(VapError::NoVapcBox));
    }

    #[test]
    fn zero_sized_box_does_not_hang() {
        // size=0 的语义是"到文件尾",不是"长度 0" —— 若照字面当 0 处理,
        // p 不前进,循环永远转下去
        let mut f = 0u32.to_be_bytes().to_vec();
        f.extend_from_slice(b"mdat");
        f.extend_from_slice(&[0u8; 16]);
        assert_eq!(find_vapc(&f), Err(VapError::NoVapcBox));
    }

    #[test]
    fn size_smaller_than_header_is_rejected_not_looped() {
        // size=4 比 8 字节的头还小。裸实现会 p += 4 然后在 box 中间继续解,
        // 或者干脆不前进 —— 两种都能挂死
        let mut f = 4u32.to_be_bytes().to_vec();
        f.extend_from_slice(b"junk");
        f.extend_from_slice(&[0u8; 32]);
        assert_eq!(find_vapc(&f), Err(VapError::MalformedBox));
    }

    #[test]
    fn size_beyond_file_is_rejected() {
        let mut f = 9999u32.to_be_bytes().to_vec();
        f.extend_from_slice(b"vapc");
        f.extend_from_slice(b"{}");
        assert_eq!(find_vapc(&f), Err(VapError::TruncatedMp4));
    }

    #[test]
    fn non_utf8_payload_is_reported() {
        let f = boxed(b"vapc", &[0xff, 0xfe, 0xfd]);
        assert_eq!(find_vapc(&f), Err(VapError::VapcNotUtf8));
    }

    #[test]
    fn truncated_at_every_byte_never_panics() {
        // 与 sv-pag 同款纪律:合法文件从每个位置切一刀,只准 Err 不准 panic
        let mut f = boxed(b"ftyp", b"isom");
        f.extend(boxed(b"vapc", br#"{"info":{}}"#));
        f.extend(boxed(b"mdat", &[0u8; 16]));
        for cut in 0..f.len() {
            let _ = find_vapc(&f[..cut]);
        }
    }

    #[test]
    fn large_size_box_is_handled() {
        // size==1 → 后面是 u64 大尺寸
        let body = br#"{"info":{"v":2}}"#;
        let mut f = 1u32.to_be_bytes().to_vec();
        f.extend_from_slice(b"vapc");
        f.extend_from_slice(&((body.len() + 16) as u64).to_be_bytes());
        f.extend_from_slice(body);
        assert_eq!(find_vapc(&f).unwrap(), r#"{"info":{"v":2}}"#);
    }
}
