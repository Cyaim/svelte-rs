//! `.sv` → 生成 `.rs` 的位置映射:建图、落盘格式、查表。
//!
//! 为什么是这个形状(依据 `docs/plans/lsp-spike.md` §3.2 的实测结论):
//!
//! 1. **provenance 靠"虚拟行号"**。每个吃用户文本的 `parse_str` 前面垫 N 个换行,
//!    N 全局单调递增且从 1 起。于是 token 的 `Span::start().line` 唯一确定它来自
//!    哪一段用户文本,`byte_range().start - N` 就是它在那段文本里的字节偏移。
//!    `quote!` 合成出来的 token 一律是 `Span::call_site()`(fallback 下
//!    `line == 1`、`byte_range == 0..0`),于是 `line == 1` 就是"胶水"的判据,零歧义。
//! 2. **映射必须在 prettyplease 之后建**。`prettyplease::unparse` 从 `syn::File`
//!    重新打印,span 在这一步全部丢弃——"格式化前算好行号"是死路。做法是把
//!    unparse 的输出**再 parse 一遍**,拿到每个 token 在**输出文本**里的真实字节
//!    区间,再与格式化前的 token 流按 Ident/Literal 并行走对齐。它怎么折行、
//!    怎么把一行 CJK 摊成四行,与映射无关。
//! 3. **两侧一律字节偏移**。rustc 的列是 1-based **字符**列(不是字节、不是
//!    UTF-16),`lib.rs` 的 `line_col` 口径与之一致;字节 → 行列的换算收敛到
//!    输出的那一刻做,map 内部不存行列。
//!
//! 已知做不到的(诚实列):
//! - runes 改写把 `count += 1` 拆成两条语句,用户表达式在生成代码里不再连续;
//!   落在 `count.update(|__v| …)` 这类残骸上的诊断只能退到相邻锚点之间。
//! - 样式值在编译期折叠成字面量,没有 provenance——但样式域的错误本来就由
//!   编译器自报 `.sv` 行列,rustc 看不到它们。
//! - `{#snippet}` 的参数类型没有 `.sv` 偏移可用(`template.rs` 只带了 snippet
//!   节点本身的偏移),它们的 token 按胶水处理,诊断走降级路径。
//! - **包络不是真嵌套的**。`lsp-spike.md` §3.2 第三步要求 codegen 进出
//!   `emit_element`/`emit_if`/`emit_each`/… 时压一个节点栈,让包络天然嵌套;
//!   本版**没有做**(§6 显式批准第一版降级)。这里的 region 粒度是
//!   "一个 parse 入口的**一行**"(见 [`Seg::region`]),所以 `MapKind::Envelope`
//!   给的是"某一行的跨度",不是"某个模板节点的跨度"。加节点栈之前,
//!   `Envelope` 这一档的措辞必须继续说"近似"。
//!
//! 覆盖率实测(2026-07-22,仓库全部 10 个可独立编译的 `.sv`):`.sv` 里用户写的
//! Rust token **281/349 = 80.5%** 拿到精确映射;`map_coverage_floor` 把地板钉在 70%。

use std::cell::RefCell;

use proc_macro2::{Span, TokenStream, TokenTree};
use quote::ToTokens;

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// 生成侧一段字节区间 ↔ `.sv` 一段字节区间。**两侧文本逐字相等**——
/// 这是"锚点"的定义,也是 `map_segments_are_verbatim` 断言的内容。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seg {
    pub gen_start: usize,
    pub gen_end: usize,
    pub sv_start: usize,
    pub sv_end: usize,
    /// 来源区编号 = 该 token 所在的**虚拟行号**,也就是"某个 parse 入口的某一行"
    /// (虚拟行全局单调递增且不跨 site 复用,所以行号本身就是唯一的区编号)。
    ///
    /// 相邻锚点插值只在同一 region 内做。粒度取到**行**而不是"整个 script 块",
    /// 是因为块级粒度会跨语句瞎猜:实测 `let double = …` 那条语句的 `let`
    /// (rune 改写后已是胶水)会被插值到上一条语句末尾的 `);` 上,报出一个
    /// 像样但错行的位置。行级粒度让它老实降级。
    pub region: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapKind {
    /// 命中锚点:生成侧与 `.sv` 侧逐字相等
    Exact,
    /// 落在同一 region 的两个锚点之间(标点、`.get()` 之类的残骸)——
    /// 取二者之间那段 `.sv` 文本
    Between,
    /// 只知道落在某个 region 里(**行级**包络,不是节点级——节点栈还没做,
    /// 见本模块头部"已知做不到的")
    Envelope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub sv_start: usize,
    pub sv_end: usize,
    pub kind: MapKind,
}

/// 一个 region 的包络(由 `segs` 推出来,不落盘)
#[derive(Clone, Copy, Debug)]
struct Envelope {
    region: u32,
    gen_start: usize,
    gen_end: usize,
    sv_start: usize,
    sv_end: usize,
}

#[derive(Debug, Default, Clone)]
pub struct SourceMap {
    /// `.sv` 的绝对路径。**必须绝对**:build.rs 的 cwd 是包根,`sv check` 的 cwd
    /// 是 workspace 根,写相对路径两边对不上。
    pub sv_path: String,
    pub sv_len: usize,
    pub sv_hash: u64,
    pub gen_len: usize,
    /// 按 `gen_start` 升序、互不重叠
    pub segs: Vec<Seg>,
    /// **保险丝熔断的原因**;`None` = 锚点并行走走完了(表可信)。
    ///
    /// 熔断时 `segs` 恒为空,但"空表"有两种截然不同的成因:走完了却一段都没
    /// 记下(极小的 `.sv`)、以及并行走自己失配整表作废。两者给用户的解释
    /// 完全不同(前者是"这块是胶水",后者是"映射机制坏了,请上报"),
    /// 所以必须落盘区分——`sv check` 靠它选降级措辞,`build()` 靠它打
    /// cargo warning。
    pub blown: Option<String>,
    envelopes: Vec<Envelope>,
}

/// 锚点并行走的产物:段表 + 保险丝状态。
///
/// 分成两个字段而不是 `Result`,是因为熔断**不是错误**:照样出 map、照样
/// 编译成功,只是没有精确段。丢掉熔断原因才是错误(用户会被"落在胶水上"
/// 这句误导去查 runes 改写)。
#[derive(Debug, Default, Clone)]
pub struct Anchors {
    pub segs: Vec<Seg>,
    pub blown: Option<String>,
}

impl Anchors {
    fn blown(why: impl Into<String>) -> Anchors {
        Anchors {
            segs: Vec::new(),
            blown: Some(why.into()),
        }
    }
}

/// FNV-1a 64:只用来判"map 与 .sv 是不是同一份",不需要抗碰撞
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

impl SourceMap {
    fn rebuild_envelopes(&mut self) {
        let mut env: Vec<Envelope> = Vec::new();
        for s in &self.segs {
            match env.iter_mut().find(|e| e.region == s.region) {
                Some(e) => {
                    e.gen_start = e.gen_start.min(s.gen_start);
                    e.gen_end = e.gen_end.max(s.gen_end);
                    e.sv_start = e.sv_start.min(s.sv_start);
                    e.sv_end = e.sv_end.max(s.sv_end);
                }
                None => env.push(Envelope {
                    region: s.region,
                    gen_start: s.gen_start,
                    gen_end: s.gen_end,
                    sv_start: s.sv_start,
                    sv_end: s.sv_end,
                }),
            }
        }
        self.envelopes = env;
    }

    /// 生成侧字节区间 → `.sv` 字节区间。三档降级:锚点命中 → 同区相邻锚点之间
    /// → region 包络;都不成立返回 `None`(调用方**必须**把诊断原样透出去,
    /// 不许吞)。
    pub fn lookup(&self, gen_start: usize, gen_end: usize) -> Option<Hit> {
        if self.segs.is_empty() {
            return None;
        }
        // segs 按 gen_start 升序且互不重叠 → 二分
        let i = self.segs.partition_point(|s| s.gen_start <= gen_start);
        let left = if i > 0 { Some(self.segs[i - 1]) } else { None };
        let right = self.segs.get(i).copied();

        // 1. 精确:落在某个锚点内部
        if let Some(l) = left
            && gen_start < l.gen_end
        {
            return Some(Hit {
                sv_start: l.sv_start,
                sv_end: l.sv_end,
                kind: MapKind::Exact,
            });
        }

        // 2. 同区相邻锚点插值。**不做"按到左锚点的字节距离平移"**:
        //    生成侧的 `count.get() + "x"` 与 .sv 侧的 `count + "x"` 长度不同,
        //    平移会越过右锚点。取两个锚点之间那段 .sv 文本才是对的
        //    (对 `+` 这种宽度 1 的主 span,结果恰好就是那个 `+`)。
        if let (Some(l), Some(r)) = (left, right)
            && l.region == r.region
            && l.sv_end <= r.sv_start
        {
            // 诊断区间跨过右锚点 → 连右锚点一起包进去(不拼接不连续的 .sv 区间)
            let e = if gen_end > r.gen_start {
                r.sv_end
            } else {
                r.sv_start
            };
            return Some(Hit {
                sv_start: l.sv_end,
                sv_end: e,
                kind: MapKind::Between,
            });
        }

        // 3. 包络:只知道落在某个 region 里。取**最内层**(gen 跨度最小的那个)
        self.envelopes
            .iter()
            .filter(|e| gen_start >= e.gen_start && gen_start < e.gen_end)
            .min_by_key(|e| e.gen_end - e.gen_start)
            .map(|e| Hit {
                sv_start: e.sv_start,
                sv_end: e.sv_end,
                kind: MapKind::Envelope,
            })
    }

    // -- 落盘格式(刻意不用 JSON:本 crate 无 serde 依赖,且这份文件是给机器读的) --

    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::with_capacity(64 + self.segs.len() * 24);
        out.push_str("svmap 1\n");
        let _ = writeln!(out, "svlen {}", self.sv_len);
        let _ = writeln!(out, "svhash {:016x}", self.sv_hash);
        let _ = writeln!(out, "genlen {}", self.gen_len);
        // 路径含空格,必须放在行尾
        let _ = writeln!(out, "sv {}", self.sv_path);
        if let Some(why) = &self.blown {
            // 原因里不能有换行(它是行尾字段);建图侧只写常量,这里兜一道
            let _ = writeln!(out, "blown {}", why.replace(['\r', '\n'], " "));
        }
        for s in &self.segs {
            let _ = writeln!(
                out,
                "s {} {} {} {} {}",
                s.gen_start, s.gen_end, s.sv_start, s.sv_end, s.region
            );
        }
        out
    }

    pub fn parse_text(text: &str) -> Option<SourceMap> {
        let mut map = SourceMap::default();
        let mut saw_header = false;
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            // 每行必须是 `<key> <value>`;不是就当整份 map 坏了(降级到"没有 map")
            let (key, rest) = line.split_once(' ')?;
            match key {
                "svmap" => {
                    if rest != "1" {
                        return None; // 版本不认识 → 当成没有 map,走降级
                    }
                    saw_header = true;
                }
                "svlen" => map.sv_len = rest.parse().ok()?,
                "svhash" => map.sv_hash = u64::from_str_radix(rest, 16).ok()?,
                "genlen" => map.gen_len = rest.parse().ok()?,
                "sv" => map.sv_path = rest.to_string(),
                "blown" => map.blown = Some(rest.to_string()),
                "s" => {
                    let mut it = rest.split(' ');
                    let mut next = || -> Option<usize> { it.next()?.parse().ok() };
                    let (gs, ge, ss, se) = (next()?, next()?, next()?, next()?);
                    let region = next()? as u32;
                    map.segs.push(Seg {
                        gen_start: gs,
                        gen_end: ge,
                        sv_start: ss,
                        sv_end: se,
                        region,
                    });
                }
                _ => {} // 未知字段前向兼容
            }
        }
        if !saw_header {
            return None;
        }
        map.rebuild_envelopes();
        Some(map)
    }

    /// 手工构造(单测用):segs 会被排序并重建包络
    pub fn from_segs(segs: Vec<Seg>) -> SourceMap {
        let mut map = SourceMap {
            segs,
            ..Default::default()
        };
        map.segs.sort_by_key(|s| s.gen_start);
        map.rebuild_envelopes();
        map
    }
}

// ---------------------------------------------------------------------------
// 编译期 provenance 记录器
// ---------------------------------------------------------------------------

/// rune 占位替换造成的一处字节漂移(`$state` → `__sv_state`,恒 +4)
#[derive(Clone, Copy, Debug)]
pub(crate) struct RuneHit {
    /// 替换**后**文本里的起点
    pub out_start: usize,
    pub out_len: usize,
    /// 替换**前**(= .sv 原文)里的起点
    pub src_start: usize,
    pub src_len: usize,
}

struct Site {
    first_line: usize,
    last_line: usize,
    /// 用户文本在"被 parse 的那个串"里的起始字节(= pad + 前缀长度)
    text_start: usize,
    text_len: usize,
    /// 该段用户文本在 `.sv` 全文里的起始字节
    sv_offset: usize,
    /// `sv_offset` 是否**经过逐字核对**(`.sv[sv_offset..]` 确实以这段文本打头)。
    /// 核对不上时整段不建映射:偏移口径已经不知道对不对了,而 `build_segs` 的
    /// per-token 校验只比**单个 token 的文本**,同名标识符(`count` 这种)在
    /// `.sv` 里反复出现,张冠李戴照样能通过。宁可不映射。
    verified: bool,
    /// 仅 script 块非空
    runes: Vec<RuneHit>,
}

#[derive(Default)]
struct Recorder {
    /// 已分配到的虚拟行游标
    cursor: usize,
    sites: Vec<Site>,
    /// script 块拿到的 pad(`syn_err` 反算行号要减掉它)
    script_pad: usize,
    /// `.sv` 全文,用来把"指向空白的偏移"校正到用户文本真正的起点
    sv_source: String,
}

thread_local! {
    static REC: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

/// 开始记录(`compile_sv_mapped` 入口);不调用时全部 `parse_*_at` 退化成裸
/// `parse_str`,生成代码一字不差——这是 golden 不受影响的原因。
pub(crate) fn begin(sv_source: &str) {
    REC.with(|r| {
        *r.borrow_mut() = Some(Recorder {
            sv_source: sv_source.to_string(),
            ..Recorder::default()
        })
    });
}

pub(crate) fn end() {
    REC.with(|r| *r.borrow_mut() = None);
}

fn enabled() -> bool {
    REC.with(|r| r.borrow().is_some())
}

/// script 块的 pad;未开记录时为 0
pub(crate) fn script_pad() -> usize {
    REC.with(|r| r.borrow().as_ref().map_or(0, |rec| rec.script_pad))
}

/// `ExprSrc.src` 是 trim 过的,而 `ExprSrc.offset` 有几处指向 trim 掉的那段
/// 空白之前(`template.rs` 既有口径:错误定位到那个位置足够用,精确映射不够)。
/// 只跳空白、且只在跳完之后能**逐字对上**时才采纳校正。
///
/// 返回 `(偏移, 是否核对通过)`。核对不通过时偏移原样返回(错误定位还要用它),
/// 但调用方会把整段标成 `verified = false` 而不建映射:口径都不知道对不对了,
/// 再靠 per-token 文本相等去筛,只会筛出"确信但错误"的位置。
fn snap_to_text(sv: &str, offset: usize, src: &str) -> (usize, bool) {
    if sv.get(offset..).is_some_and(|s| s.starts_with(src)) {
        return (offset, true);
    }
    let mut at = offset;
    while let Some(c) = sv.get(at..).and_then(|s| s.chars().next()) {
        if !c.is_whitespace() {
            break;
        }
        at += c.len_utf8();
    }
    if sv.get(at..).is_some_and(|s| s.starts_with(src)) {
        (at, true)
    } else {
        (offset, false)
    }
}

/// 分配虚拟行段并登记一个 site,返回 pad(要垫的换行数)。
/// `content` 是 pad 之后的全部文本(可能含前后缀),`lead` 是用户文本在其中的偏移。
fn alloc(
    content: &str,
    lead: usize,
    text_len: usize,
    sv_offset: usize,
    verified: bool,
    runes: Vec<RuneHit>,
) -> usize {
    REC.with(|r| {
        let mut r = r.borrow_mut();
        let Some(rec) = r.as_mut() else { return 0 };
        // pad 从 1 起 → 用户 token 的 line >= 2,line == 1 恒为 call_site 胶水
        let pad = rec.cursor + 1;
        let lines = content.matches('\n').count() + 1;
        rec.cursor = pad + lines;
        rec.sites.push(Site {
            first_line: pad + 1,
            last_line: pad + lines,
            text_start: pad + lead,
            text_len,
            sv_offset,
            verified,
            runes,
        });
        pad
    })
}

/// 垫虚拟行之后再 parse:用户文本原样进 syn,token 因此带上可反查的 provenance。
pub(crate) fn parse_str_at<T: syn::parse::Parse>(src: &str, sv_offset: usize) -> syn::Result<T> {
    if !enabled() {
        return syn::parse_str(src);
    }
    let (off, ok) = snap(sv_offset, src);
    let pad = alloc(src, 0, src.len(), off, ok, Vec::new());
    syn::parse_str(&format!("{}{src}", "\n".repeat(pad)))
}

fn snap(sv_offset: usize, src: &str) -> (usize, bool) {
    REC.with(|r| {
        r.borrow().as_ref().map_or((sv_offset, false), |rec| {
            snap_to_text(&rec.sv_source, sv_offset, src)
        })
    })
}

/// 同上,给 `Pat::parse_single` 这类 `Parser` 用
pub(crate) fn parse_with_at<P: syn::parse::Parser>(
    parser: P,
    src: &str,
    sv_offset: usize,
) -> syn::Result<P::Output> {
    if !enabled() {
        return parser.parse_str(src);
    }
    let (off, ok) = snap(sv_offset, src);
    let pad = alloc(src, 0, src.len(), off, ok, Vec::new());
    parser.parse_str(&format!("{}{src}", "\n".repeat(pad)))
}

/// 同上,但被 parse 的文本经过占位替换(`$bindable` → `__sv_bindable`),
/// 要带上漂移表才能算回 `.sv` 的字节偏移
pub(crate) fn parse_with_drift_at<P: syn::parse::Parser>(
    parser: P,
    src: &str,
    sv_offset: usize,
    runes: Vec<RuneHit>,
) -> syn::Result<P::Output> {
    if !enabled() {
        return parser.parse_str(src);
    }
    // 占位替换过的文本没法与 `.sv` 原文 starts_with 对比,口径靠 `runes` 漂移表
    // 保证(`script.rs` 建表时就是按替换点算的),按已核对处理
    let pad = alloc(src, 0, src.len(), sv_offset, true, runes);
    parser.parse_str(&format!("{}{src}", "\n".repeat(pad)))
}

/// script 块:整块一次 parse,外面包 `{\n … \n}`(前缀 2 字节要一起减掉),
/// 里面还有 rune 占位替换造成的 +4 漂移表。
pub(crate) fn parse_script_block(
    pre: &str,
    sv_offset: usize,
    runes: Vec<RuneHit>,
) -> syn::Result<syn::Block> {
    let wrapped = format!("{{\n{pre}\n}}");
    if !enabled() {
        return syn::parse_str(&wrapped);
    }
    let pad = alloc(&wrapped, 2, pre.len(), sv_offset, true, runes);
    REC.with(|r| {
        if let Some(rec) = r.borrow_mut().as_mut() {
            rec.script_pad = pad;
        }
    });
    syn::parse_str(&format!("{}{wrapped}", "\n".repeat(pad)))
}

impl Recorder {
    fn site_of(&self, line: usize) -> Option<&Site> {
        let i = self.sites.partition_point(|s| s.first_line <= line);
        let s = self.sites.get(i.checked_sub(1)?)?;
        (line <= s.last_line).then_some(s)
    }

    /// parse 串里的字节偏移 → 用户文本内的字节偏移(补掉 rune 漂移)
    fn map_offset(site: &Site, abs: usize) -> Option<usize> {
        let rel = abs.checked_sub(site.text_start)?;
        if rel > site.text_len {
            return None;
        }
        let mut delta = 0usize;
        for h in &site.runes {
            if h.out_start + h.out_len <= rel {
                delta += h.out_len - h.src_len;
            } else if rel > h.out_start {
                // 落在 `__sv_state` 这种占位符内部 → 收敛到 `$state` 的起点
                return Some(h.src_start);
            } else {
                break;
            }
        }
        Some(rel - delta)
    }

    /// token 的 (line, 字节区间) → (.sv 起, .sv 止, region);`None` = 胶水
    ///
    /// region 直接取**虚拟行号**:虚拟行全局单调、site 之间不重叠,所以行号
    /// 天然是"某个 parse 入口的某一行"的唯一编号。见 [`Seg::region`] 里为什么
    /// 粒度必须细到行。
    fn resolve(&self, line: usize, bs: usize, be: usize) -> Option<(usize, usize, u32)> {
        if line <= 1 {
            return None; // Span::call_site() —— quote! 造的胶水
        }
        let site = self.site_of(line)?;
        if !site.verified {
            return None;
        }
        let s = Self::map_offset(site, bs)?;
        let e = Self::map_offset(site, be)?;
        if e < s {
            return None;
        }
        Some((site.sv_offset + s, site.sv_offset + e, line as u32))
    }
}

// ---------------------------------------------------------------------------
// 锚点并行走
// ---------------------------------------------------------------------------

/// 只收 Ident / Literal:标点在 prettyplease 手里不保证一一对应
/// (`quote!{ g(a, b,); }` 的尾逗号会被吃掉),而 Ident/Literal 一定对应。
fn anchors(ts: TokenStream, out: &mut Vec<(String, Span)>) {
    for tt in ts {
        match tt {
            TokenTree::Ident(id) => out.push((id.to_string(), id.span())),
            TokenTree::Literal(l) => out.push((l.to_string(), l.span())),
            TokenTree::Group(g) => anchors(g.stream(), out),
            TokenTree::Punct(_) => {}
        }
    }
}

/// 建映射段:格式化前的 AST(带 provenance)与格式化后的输出文本并行走。
///
/// - `formatted` 是 `prettyplease::unparse` 的输出;
/// - `header_len` 是最终文件里 unparse 输出之前那段文件头的字节数;
/// - 返回 [`Anchors`];`blown` 非空 = **保险丝熔断**(整表作废)。熔断原因必须
///   一路带到 `.svmap` 和 `sv check` 的输出里:空表若被当成"这里全是胶水",
///   用户会被指去查 runes 改写,而真实原因是映射机制本身失配了。
pub(crate) fn build_segs(
    file: &syn::File,
    formatted: &str,
    header_len: usize,
    sv_source: &str,
) -> Anchors {
    let mut pre = Vec::new();
    anchors(file.to_token_stream(), &mut pre);
    let Ok(reparsed) = syn::parse_file(formatted) else {
        return Anchors::blown("prettyplease 的输出无法被 syn 重解析");
    };
    let mut post = Vec::new();
    anchors(reparsed.to_token_stream(), &mut post);
    // 保险丝:prettyplease 哪天改了打印策略,这里先失配、整表作废(降级到
    // "没有精确段"),而不是让用户先遇到错位的诊断。
    if pre.len() != post.len() {
        return Anchors::blown(format!(
            "锚点数格式化前后不等({} vs {},prettyplease 改了打印策略?)",
            pre.len(),
            post.len()
        ));
    }

    REC.with(|r| {
        let r = r.borrow();
        let Some(rec) = r.as_ref() else {
            return Anchors::blown("provenance 记录器未开启");
        };
        let mut segs: Vec<Seg> = Vec::with_capacity(pre.len() / 2);
        for (a, b) in pre.iter().zip(post.iter()) {
            if a.0 != b.0 {
                return Anchors::blown(format!("锚点文本失配(`{}` vs `{}`)", a.0, b.0));
            }
            let ar = a.1.byte_range();
            let Some((sv_s, sv_e, region)) = rec.resolve(a.1.start().line, ar.start, ar.end) else {
                continue;
            };
            let br = b.1.byte_range();
            let (gs, ge) = (br.start + header_len, br.end + header_len);
            // 逐字校验:两侧文本必须相等。rune 占位符(`__sv_state` ↔ `$state`)
            // 这类不是逐字区,直接不收——留着会让"锚点即原文"这条不变量失效。
            if sv_source.get(sv_s..sv_e) != formatted.get(br.start..br.end) {
                continue;
            }
            segs.push(Seg {
                gen_start: gs,
                gen_end: ge,
                sv_start: sv_s,
                sv_end: sv_e,
                region,
            });
        }
        segs.sort_by_key(|s| s.gen_start);
        // 同一 token 不可能被登记两次,但同一 gen 位置若出现重叠段,只留第一条
        let mut dedup: Vec<Seg> = Vec::with_capacity(segs.len());
        for s in segs {
            if dedup.last().is_some_and(|p| s.gen_start < p.gen_end) {
                continue;
            }
            dedup.push(s);
        }
        Anchors {
            segs: dedup,
            blown: None,
        }
    })
}

pub(crate) fn finish_map(
    sv_path: String,
    sv_source: &str,
    generated: &str,
    anchors: Anchors,
) -> SourceMap {
    let mut map = SourceMap {
        sv_path,
        sv_len: sv_source.len(),
        sv_hash: fnv1a(sv_source.as_bytes()),
        gen_len: generated.len(),
        segs: anchors.segs,
        blown: anchors.blown,
        envelopes: Vec::new(),
    };
    map.rebuild_envelopes();
    map
}

// ---------------------------------------------------------------------------
// 字节 ↔ 行列
// ---------------------------------------------------------------------------

/// 字节偏移 → (1-based 行, 1-based **字符**列)。
/// 列的口径刻意与 rustc 对齐:rustc JSON 的 `column_start` 是 1-based 字符列
/// (实测:含 7 个 3 字节汉字的一行,同一位置 byte=53 / column=40 / LSP utf16=39)。
pub fn byte_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    crate::line_col(source, offset)
}

/// (1-based 行, 1-based 字符列) → 字节偏移。越界时收敛到行尾 / 文件尾。
/// 这是 `sv check` 消费 rustc JSON 的入口:`byte_start` 字段虽然也在,但
/// 行列是 rustc 输出里最稳的一对,以它为准。
pub fn line_col_to_byte(source: &str, line: usize, col: usize) -> usize {
    let mut off = 0usize;
    for _ in 1..line.max(1) {
        match source[off..].find('\n') {
            Some(rel) => off += rel + 1,
            None => return source.len(),
        }
    }
    let rest = &source[off..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let mut chars = 1usize;
    for (i, _) in rest[..line_end].char_indices() {
        if chars == col {
            return off + i;
        }
        chars += 1;
    }
    off + line_end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(gs: usize, ge: usize, ss: usize, se: usize, r: u32) -> Seg {
        Seg {
            gen_start: gs,
            gen_end: ge,
            sv_start: ss,
            sv_end: se,
            region: r,
        }
    }

    #[test]
    fn lookup_exact_hit() {
        let m = SourceMap::from_segs(vec![seg(100, 105, 10, 15, 0), seg(120, 121, 20, 21, 0)]);
        let h = m.lookup(100, 105).expect("应命中锚点");
        assert_eq!((h.sv_start, h.sv_end, h.kind), (10, 15, MapKind::Exact));
        // 落在锚点内部(rustc 常把 span 划在子表达式的一部分)
        let h = m.lookup(102, 103).expect("锚点内部也算命中");
        assert_eq!(h.kind, MapKind::Exact);
    }

    #[test]
    fn lookup_between_two_anchors_takes_the_gap() {
        // 生成侧 `count.get() + "x"`,.sv 侧 `count + "x"`:
        // 主 span 是那个 `+`(宽度 1、落在标点上,不在锚点里)
        let m = SourceMap::from_segs(vec![
            seg(0, 5, 0, 5, 0),    // count
            seg(13, 16, 8, 11, 0), // "x"
        ]);
        let h = m.lookup(12, 13).expect("应插值到两锚点之间");
        assert_eq!((h.sv_start, h.sv_end, h.kind), (5, 8, MapKind::Between));
    }

    #[test]
    fn lookup_refuses_to_interpolate_across_regions() {
        // 左右锚点分属两个模板表达式:插值出来的位置会"像样但错误",宁可降级
        let m = SourceMap::from_segs(vec![seg(0, 5, 0, 5, 0), seg(40, 43, 90, 93, 1)]);
        let h = m.lookup(20, 21);
        assert_eq!(h, None, "跨 region 不插值,也没有包络覆盖 → 必须降级");
    }

    #[test]
    fn lookup_falls_back_to_region_envelope() {
        // region 0 的两个锚点把 [0,60) 圈起来;中间那个 region 1 的锚点让
        // "相邻锚点"这条路走不通(region 不同),于是退到包络
        let m = SourceMap::from_segs(vec![
            seg(0, 5, 0, 5, 0),
            seg(20, 24, 200, 204, 1),
            seg(55, 60, 30, 35, 0),
        ]);
        let h = m.lookup(30, 31).expect("应退到包络");
        assert_eq!((h.sv_start, h.sv_end, h.kind), (0, 35, MapKind::Envelope));
    }

    #[test]
    fn lookup_returns_none_outside_everything() {
        let m = SourceMap::from_segs(vec![seg(100, 105, 10, 15, 0)]);
        assert_eq!(m.lookup(0, 1), None, "所有锚点之左");
        assert_eq!(m.lookup(500, 501), None, "所有锚点之右");
        assert_eq!(SourceMap::default().lookup(1, 2), None, "空表");
    }

    #[test]
    fn text_roundtrip() {
        let mut m = SourceMap::from_segs(vec![seg(100, 105, 10, 15, 0), seg(120, 121, 20, 21, 1)]);
        m.sv_path = "E:/a b/Counter.sv".into();
        m.sv_len = 1043;
        m.sv_hash = 0x3f2a_0000_0000_0001;
        m.gen_len = 6382;
        let back = SourceMap::parse_text(&m.to_text()).expect("应能解析回来");
        assert_eq!(back.sv_path, m.sv_path);
        assert_eq!(back.sv_len, m.sv_len);
        assert_eq!(back.sv_hash, m.sv_hash);
        assert_eq!(back.gen_len, m.gen_len);
        assert_eq!(back.segs, m.segs);
        assert_eq!(back.blown, None, "没熔断就不该有 blown 字段");

        // 熔断原因必须能落盘再读回来:`sv check` 全靠它把"整表作废"和
        // "这里是胶水"分开说
        m.blown = Some("锚点数不等(3 vs 4)".into());
        let back = SourceMap::parse_text(&m.to_text()).expect("熔断的 map 也应能解析");
        assert_eq!(back.blown.as_deref(), Some("锚点数不等(3 vs 4)"));
    }

    #[test]
    fn parse_text_rejects_unknown_version() {
        assert!(SourceMap::parse_text("svmap 2\nsvlen 1\n").is_none());
        assert!(SourceMap::parse_text("svlen 1\n").is_none(), "无表头");
    }

    #[test]
    fn line_col_roundtrip_with_crlf_and_cjk() {
        // CRLF + 中文:列必须是**字符**列(与 rustc 口径一致),不是字节列
        let src = "abc\r\n中文 x = 1;\r\n末行";
        let off = src.find("x =").unwrap();
        let (l, c) = byte_to_line_col(src, off);
        assert_eq!((l, c), (2, 4), "`x` 前面是 2 个汉字 + 1 空格 = 3 个字符");
        assert_eq!(line_col_to_byte(src, l, c), off);

        let off2 = src.find("末").unwrap();
        let (l2, c2) = byte_to_line_col(src, off2);
        assert_eq!((l2, c2), (3, 1));
        assert_eq!(line_col_to_byte(src, l2, c2), off2);
    }

    #[test]
    fn line_col_to_byte_clamps() {
        let src = "ab\ncd\n";
        assert_eq!(line_col_to_byte(src, 1, 99), 2, "越界收敛到行尾");
        assert_eq!(line_col_to_byte(src, 99, 1), src.len(), "越界收敛到文件尾");
    }
}
