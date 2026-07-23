//! `sv check` 的引擎:跑 `cargo check --message-format=json`,把落在生成
//! `.rs` 上的诊断按 source map 搬回 `.sv`,以 rustc 风格单行输出
//! (`路径:行:列: level[code]: 消息`),让 VS Code 的 problemMatcher 直接能吃。
//!
//! **铁律:输入 N 条诊断,输出必须 N 条。** "我映射不了所以我不说了"是最坏的
//! 失败——用户会以为编译通过了。所有降级路径都保留原诊断全文,只是位置退回
//! 生成文件并附一句说明。
//!
//! 铁律守在 [`Session`] 这一层,不是 [`render`] 这一层:`render` 的签名
//! (`&Value -> Rendered`)在类型上就吞不掉东西,拿它做"守恒"的证据是空的;
//! 真正会丢东西的是"这行 JSON 我解析不了"——那条路必须走
//! [`Line::Unparsed`] 原样透出去。`check_never_drops_diagnostic` 测的是
//! `Session`。
//!
//! 降级的**理由必须是真的**。同样是"没映射成",成因至少五种(见
//! [`DegradeKind`]),给错理由会把用户支到错误的方向去查——比如把"整张
//! 锚点表作废"说成"你这行落在 runes 改写的胶水上"。
//!
//! 位置口径(三种编码同时在场,别混):
//! - map 内部:**字节**;
//! - rustc JSON 的 `line_start/column_start`:1-based、列是**字符**列
//!   (实测:含 7 个 3 字节汉字的一行,同一位置 byte=53 / column=40);
//! - 我们的输出:与 rustc 同口径,即 1-based 字符列。
//!
//! 所以本模块**只用行列**与 rustc 交换位置,`byte_start` 字段不碰:行列是
//! rustc 输出里最稳的一对,而且 `.sv` 与生成 `.rs` 两边的换算都收敛到
//! `sourcemap::{byte_to_line_col, line_col_to_byte}` 这一对函数里。
//!
//! **为什么不 feature gate**(本模块只服务 `sv-check` 二进制,却进了每个 `.sv`
//! 消费者的 build-dependency):实测(2026-07-22,增量,取 5 次稳态)
//! `cargo build -p sv-compiler --lib` 带本模块 0.76s、去掉 0.71s,**差 ~0.05s**,
//! 而 sv-compiler 的 build-dep 树里 syn + prettyplease 是数量级更大的项。
//! 加 feature 反而要给集成测试挂一条自引用 `dev-dependencies`(把本 crate
//! 再编一遍),省下的还不够赔上的。零依赖、纯函数、~700 行,留在 lib 里。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::sourcemap::{self, MapKind, SourceMap};

// ---------------------------------------------------------------------------
// 极小 JSON(本 crate 刻意不引 serde:依赖面就是维护面,这里只需要读几个字段)
// ---------------------------------------------------------------------------

pub mod json {
    #[derive(Debug, Clone, PartialEq)]
    pub enum Value {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<Value>),
        Obj(Vec<(String, Value)>),
    }

    impl Value {
        pub fn get(&self, key: &str) -> Option<&Value> {
            match self {
                Value::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
                _ => None,
            }
        }
        pub fn as_str(&self) -> Option<&str> {
            match self {
                Value::Str(s) => Some(s),
                _ => None,
            }
        }
        pub fn as_arr(&self) -> &[Value] {
            match self {
                Value::Arr(a) => a,
                _ => &[],
            }
        }
        pub fn as_usize(&self) -> Option<usize> {
            match self {
                Value::Num(n) if *n >= 0.0 => Some(*n as usize),
                _ => None,
            }
        }
        pub fn is_true(&self) -> bool {
            matches!(self, Value::Bool(true))
        }
    }

    struct P<'a> {
        b: &'a [u8],
        i: usize,
    }

    /// 解析一行 JSON;格式不对返回 `None`(cargo 偶尔会往 stdout 混别的东西)
    pub fn parse(s: &str) -> Option<Value> {
        let mut p = P {
            b: s.as_bytes(),
            i: 0,
        };
        p.ws();
        let v = p.value()?;
        p.ws();
        (p.i == p.b.len()).then_some(v)
    }

    impl P<'_> {
        fn ws(&mut self) {
            while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                self.i += 1;
            }
        }
        fn eat(&mut self, c: u8) -> Option<()> {
            (self.b.get(self.i) == Some(&c)).then(|| self.i += 1)
        }
        fn lit(&mut self, s: &str) -> Option<()> {
            self.b[self.i..]
                .starts_with(s.as_bytes())
                .then(|| self.i += s.len())
        }
        fn value(&mut self) -> Option<Value> {
            match *self.b.get(self.i)? {
                b'{' => self.obj(),
                b'[' => self.arr(),
                b'"' => self.string().map(Value::Str),
                b't' => self.lit("true").map(|_| Value::Bool(true)),
                b'f' => self.lit("false").map(|_| Value::Bool(false)),
                b'n' => self.lit("null").map(|_| Value::Null),
                _ => self.num(),
            }
        }
        fn obj(&mut self) -> Option<Value> {
            self.eat(b'{')?;
            let mut kv = Vec::new();
            self.ws();
            if self.eat(b'}').is_some() {
                return Some(Value::Obj(kv));
            }
            loop {
                self.ws();
                let k = self.string()?;
                self.ws();
                self.eat(b':')?;
                self.ws();
                kv.push((k, self.value()?));
                self.ws();
                if self.eat(b',').is_some() {
                    continue;
                }
                self.eat(b'}')?;
                return Some(Value::Obj(kv));
            }
        }
        fn arr(&mut self) -> Option<Value> {
            self.eat(b'[')?;
            let mut a = Vec::new();
            self.ws();
            if self.eat(b']').is_some() {
                return Some(Value::Arr(a));
            }
            loop {
                self.ws();
                a.push(self.value()?);
                self.ws();
                if self.eat(b',').is_some() {
                    continue;
                }
                self.eat(b']')?;
                return Some(Value::Arr(a));
            }
        }
        fn string(&mut self) -> Option<String> {
            self.eat(b'"')?;
            let mut out = String::new();
            loop {
                let c = *self.b.get(self.i)?;
                self.i += 1;
                match c {
                    b'"' => return Some(out),
                    b'\\' => {
                        let e = *self.b.get(self.i)?;
                        self.i += 1;
                        match e {
                            b'"' => out.push('"'),
                            b'\\' => out.push('\\'),
                            b'/' => out.push('/'),
                            b'b' => out.push('\u{8}'),
                            b'f' => out.push('\u{c}'),
                            b'n' => out.push('\n'),
                            b'r' => out.push('\r'),
                            b't' => out.push('\t'),
                            b'u' => {
                                let hi = self.hex4()?;
                                // 代理对:rustc 的消息里有 emoji/CJK 时会用到
                                let ch = if (0xD800..0xDC00).contains(&hi) {
                                    self.eat(b'\\')?;
                                    self.eat(b'u')?;
                                    let lo = self.hex4()?;
                                    char::from_u32(
                                        0x1_0000 + ((hi - 0xD800) << 10) + (lo - 0xDC00),
                                    )?
                                } else {
                                    char::from_u32(hi)?
                                };
                                out.push(ch);
                            }
                            _ => return None,
                        }
                    }
                    _ => {
                        // 多字节 UTF-8 原样搬运
                        let start = self.i - 1;
                        let len = utf8_len(c);
                        self.i = start + len;
                        out.push_str(std::str::from_utf8(self.b.get(start..self.i)?).ok()?);
                    }
                }
            }
        }
        fn hex4(&mut self) -> Option<u32> {
            let s = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
            self.i += 4;
            u32::from_str_radix(s, 16).ok()
        }
        fn num(&mut self) -> Option<Value> {
            let start = self.i;
            while matches!(self.b.get(self.i), Some(c) if c.is_ascii_digit()
                || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E'))
            {
                self.i += 1;
            }
            std::str::from_utf8(&self.b[start..self.i])
                .ok()?
                .parse()
                .ok()
                .map(Value::Num)
        }
    }

    fn utf8_len(b: u8) -> usize {
        match b {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            _ => 4,
        }
    }
}

// ---------------------------------------------------------------------------
// 重映射(纯函数,单测的主战场)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapped {
    pub sv_path: String,
    pub line: usize,
    pub col: usize,
    pub kind: MapKind,
}

/// rustc JSON 主 span 在**生成文件**里的位置。
///
/// 四个数一起收:rustc 的 `column_end` 属于 `line_end` 那一行,而 prettyplease
/// 会把长行折断(实测:一行 28 个汉字折成 4 行),所以主 span 跨行是常态。
/// 拿 `line_start` 去换算 `column_end` 会算出一个既不是起点也不是终点的字节。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenSpan {
    pub line: usize,
    pub col: usize,
    pub line_end: usize,
    pub col_end: usize,
}

/// 生成文件的 (1-based 行, 1-based 字符列) → `.sv` 的 (行, 列)。
///
/// 返回 `None` = **映射不到**。调用方必须降级成"原样透出生成文件的位置 + 一句
/// 说明",绝不能把诊断吞掉。
pub fn relocate(
    map: &SourceMap,
    sv_source: &str,
    gen_source: &str,
    span: GenSpan,
) -> Option<Mapped> {
    let start = sourcemap::line_col_to_byte(gen_source, span.line, span.col);
    let end = sourcemap::line_col_to_byte(gen_source, span.line_end, span.col_end);
    // 终点必须严格在起点之后:rustc 偶尔给出退化 span(start == end),
    // 而 `lookup` 的"诊断区间跨过右锚点"那一档要靠 end 判断
    let end = end.max(start + 1);
    let hit = map.lookup(start, end)?;
    // 两锚点之间那段 .sv 文本通常是 ` + ` 这种带空白的,掐掉前导空白才能让
    // 插入符落在真正的运算符上(招牌用例 `{count + "x"}` 就靠这一下)
    let mut at = hit.sv_start;
    if hit.kind == MapKind::Between {
        while at < hit.sv_end
            && sv_source[at..]
                .chars()
                .next()
                .is_some_and(|c| c.is_whitespace())
        {
            at += sv_source[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    let (line, col) = sourcemap::byte_to_line_col(sv_source, at);
    Some(Mapped {
        sv_path: map.sv_path.clone(),
        line,
        col,
        kind: hit.kind,
    })
}

/// 为什么这条诊断没能落到 `.sv` 上——**必须**跟着诊断一起说出来。
///
/// 每一条都要能被**构造出来**:一个永远打不出来的分支等于没写,而用户会拿到
/// 另一条(错的)解释。`degrade_notes_are_all_reachable` 守这条。
pub fn degrade_note(kind: DegradeKind) -> &'static str {
    match kind {
        DegradeKind::Glue => {
            "该位置落在 sv-compiler 生成的胶水代码上,未能映射回 .sv(通常是 runes 改写的产物;\
             请按上面的生成文件坐标定位,并附 .sv 源码上报)"
        }
        DegradeKind::NoMap => {
            "旁边的 .svmap 读不出或解析不了(版本不认识 / 文件被截断 / 生成文件读不到),\
             诊断保持生成文件坐标"
        }
        DegradeKind::MissingSv => {
            "map 记的 .sv 原文读不到(文件被删/改名,或这份 map 是在另一台机器上构建的——\
             map 存的是构建机上的绝对路径),诊断保持生成文件坐标"
        }
        DegradeKind::StaleMap => {
            "span map 与 .sv 内容对不上(.sv 改了但 build.rs 没重跑?),\
             为免报错位置骗人,诊断保持生成文件坐标"
        }
        DegradeKind::NoAnchors => {
            "该文件的锚点并行走整表作废,这个 .sv 的**所有**诊断都回不到源码——\
             不是你这一行的问题,是映射机制失配了,请连同 .sv 上报;原因"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradeKind {
    /// 位置真的落在生成的胶水上(锚点表可信,只是这里没有 provenance)
    Glue,
    /// `.svmap` 在,但读不出/解析不了;或生成文件本身读不到
    NoMap,
    /// map 指向的 `.sv` 原文读不到
    MissingSv,
    /// map 与 `.sv` 内容对不上
    StaleMap,
    /// 建图时保险丝熔断(`SourceMap::blown`):整张表作废,与"落在胶水上"
    /// 是**完全不同**的成因,不能共用一句话
    NoAnchors,
}

/// 一次 `sv check` 里所有生成文件的 map + 原文缓存
#[derive(Default)]
pub struct Maps {
    loaded: HashMap<PathBuf, MapState>,
}

/// 一个生成文件的 map 处于什么状态。
///
/// **`Absent` 与 `Broken` 必须分开**:前者是"这压根不是我们的产物"(普通 `.rs`,
/// 诊断原样透出、一个字都不加),后者是"是我们的产物但用不了"(必须附降级说明)。
/// 两者都吐一模一样的一行,用户没有任何线索知道 map 坏了。
pub enum MapState {
    Absent,
    Broken(DegradeKind),
    Ready(Loaded),
}

pub struct Loaded {
    pub map: SourceMap,
    pub sv_source: String,
    pub gen_source: String,
}

impl Maps {
    /// 生成文件路径 → 它的 map(旁边的 `<file>.svmap`)。
    /// 不做 out_dir 猜测:文件存在就用,不存在就没有——这比 glob
    /// `target/debug/build/<pkg>-<hash>/out` 稳(那底下常有旧构建的残留)。
    pub fn get(&mut self, gen_file: &str) -> &MapState {
        let key = PathBuf::from(gen_file);
        self.loaded
            .entry(key.clone())
            .or_insert_with(|| Self::load(&key))
    }

    fn load(gen_file: &Path) -> MapState {
        let mut map_path = gen_file.as_os_str().to_os_string();
        map_path.push(".svmap");
        // 只有旁边真有 .svmap 才算"我们的"生成文件;否则是普通 .rs
        let Ok(text) = std::fs::read_to_string(PathBuf::from(map_path)) else {
            return MapState::Absent;
        };
        let Some(map) = SourceMap::parse_text(&text) else {
            return MapState::Broken(DegradeKind::NoMap);
        };
        let Ok(gen_source) = std::fs::read_to_string(gen_file) else {
            return MapState::Broken(DegradeKind::NoMap);
        };
        let Ok(sv_source) = std::fs::read_to_string(&map.sv_path) else {
            return MapState::Broken(DegradeKind::MissingSv);
        };
        // 廉价一致性校验:.sv 换了内容而 build.rs 没重跑时,宁可不映射
        if sourcemap::fnv1a(sv_source.as_bytes()) != map.sv_hash || gen_source.len() != map.gen_len
        {
            return MapState::Broken(DegradeKind::StaleMap);
        }
        MapState::Ready(Loaded {
            map,
            sv_source,
            gen_source,
        })
    }
}

// ---------------------------------------------------------------------------
// 诊断渲染
// ---------------------------------------------------------------------------

pub struct Rendered {
    /// problemMatcher 吃的那一行(`路径:行:列: level[code]: 消息`)
    pub headline: String,
    /// 缩进的上下文(源码摘录 + 插入符 + note/help 子诊断);
    /// 刻意缩进,免得被 problemMatcher 当成第二条诊断
    pub context: Vec<String>,
    pub is_error: bool,
}

/// 把一条 rustc JSON 诊断渲染成输出。**永不返回 None** —— 见本模块头部铁律。
pub fn render(msg: &json::Value, maps: &mut Maps) -> Rendered {
    let level = msg.get("level").and_then(|v| v.as_str()).unwrap_or("error");
    let code = msg
        .get("code")
        .and_then(|c| c.get("code"))
        .and_then(|v| v.as_str())
        .map(|c| format!("[{c}]"))
        .unwrap_or_default();
    let text = msg.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let is_error = level.starts_with("error");

    let primary = msg
        .get("spans")
        .map(|s| s.as_arr())
        .unwrap_or_default()
        .iter()
        .find(|s| s.get("is_primary").is_some_and(|b| b.is_true()));

    let mut context = Vec::new();
    let headline = match primary {
        None => format!("{level}{code}: {text}"),
        Some(sp) => {
            let file = sp.get("file_name").and_then(|v| v.as_str()).unwrap_or("");
            let num = |k: &str, d: usize| sp.get(k).and_then(|v| v.as_usize()).unwrap_or(d);
            let line = num("line_start", 1);
            let col = num("column_start", 1);
            let span = GenSpan {
                line,
                col,
                // 缺字段时退化成"起点后一格",不要退化成"同一行的 col_end"
                line_end: num("line_end", line),
                col_end: num("column_end", col + 1),
            };
            match resolve(maps, file, span) {
                Resolved::Sv { m, sv_source } => {
                    let approx = match m.kind {
                        MapKind::Exact => "",
                        MapKind::Between => {
                            "  (位置由相邻锚点插值得出:主 span 落在标点或 runes 改写的残骸上)"
                        }
                        MapKind::Envelope => {
                            "  (位置为行级近似:主 span 落在生成的胶水上;节点栈未做,只能定位到行)"
                        }
                    };
                    context.extend(excerpt(&sv_source, m.line, m.col));
                    context.push(format!("   = 生成文件对应位置: {file}:{line}:{col}"));
                    format!(
                        "{}:{}:{}: {level}{code}: {text}{approx}",
                        m.sv_path, m.line, m.col
                    )
                }
                Resolved::Degraded { kind, detail } => format!(
                    "{file}:{line}:{col}: {level}{code}: {text}  [sv-check: {}{}]",
                    degrade_note(kind),
                    detail.map(|d| format!(":{d}")).unwrap_or_default()
                ),
                Resolved::Plain => format!("{file}:{line}:{col}: {level}{code}: {text}"),
            }
        }
    };

    // 子诊断(note/help)缩进输出:它们是同一条诊断的一部分,不该在
    // Problems 面板里各占一行
    for child in msg
        .get("children")
        .map(|c| c.as_arr())
        .unwrap_or_default()
        .iter()
    {
        let cl = child
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("note");
        let ct = child.get("message").and_then(|v| v.as_str()).unwrap_or("");
        context.push(format!("   {cl}: {ct}"));
    }
    Rendered {
        headline,
        context,
        is_error,
    }
}

enum Resolved {
    Sv {
        m: Mapped,
        sv_source: String,
    },
    Degraded {
        kind: DegradeKind,
        detail: Option<String>,
    },
    /// 普通 `.rs` 的诊断:不该被我们碰
    Plain,
}

fn resolve(maps: &mut Maps, file: &str, span: GenSpan) -> Resolved {
    // rustc 吐出来的路径在 Windows 上是反斜杠与正斜杠混用的
    // (前半段来自 cargo,`/counter.rs` 来自 include! 的字面量拼接),
    // 直接交给 OS 打开即可,不要做字符串相等比较
    let loaded = match maps.get(file) {
        MapState::Absent => return Resolved::Plain,
        MapState::Broken(kind) => {
            return Resolved::Degraded {
                kind: *kind,
                detail: None,
            };
        }
        MapState::Ready(l) => l,
    };
    match relocate(&loaded.map, &loaded.sv_source, &loaded.gen_source, span) {
        Some(m) => Resolved::Sv {
            m,
            sv_source: loaded.sv_source.clone(),
        },
        // 查表落空有两种成因,措辞完全不同:锚点表可信 → 这里真是胶水;
        // 保险丝烧了 → 整表作废,说"胶水"就是把用户支去查 runes 改写
        None => match &loaded.map.blown {
            Some(why) => Resolved::Degraded {
                kind: DegradeKind::NoAnchors,
                detail: Some(why.clone()),
            },
            None => Resolved::Degraded {
                kind: DegradeKind::Glue,
                detail: None,
            },
        },
    }
}

/// rustc 风格的源码摘录:一行原文 + 插入符
fn excerpt(source: &str, line: usize, col: usize) -> Vec<String> {
    let Some(text) = source.lines().nth(line.saturating_sub(1)) else {
        return Vec::new();
    };
    let text = text.trim_end_matches('\r');
    let gutter = line.to_string();
    let pad = " ".repeat(gutter.len());
    // 列是**字符**列(与 rustc 口径一致),但插入符要落在终端的正确位置,
    // 所以缩进按**显示宽度**算:本仓库所有示例都是中文界面,不算宽度的话
    // 插入符会往左偏"该行汉字数"个格子
    let caret = " ".repeat(
        text.chars()
            .take(col.saturating_sub(1))
            .map(display_width)
            .sum(),
    );
    // 全部缩进一格:problemMatcher 的正则用 `^\S` 排除上下文行,
    // 行号打头的那一行不缩进就会被当成新的一条诊断
    vec![
        format!("  {pad} |"),
        format!("  {gutter} | {text}"),
        format!("  {pad} | {caret}^"),
    ]
}

/// 终端显示宽度(East Asian Wide/Fullwidth 与常见 emoji 记 2)。
/// 只服务插入符对齐,不追求 UAX#11 全覆盖——错一格不影响定位,
/// 而引一个 unicode-width 依赖只为画一个 `^` 不划算。
fn display_width(c: char) -> usize {
    let c = c as u32;
    let wide = matches!(c,
        0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0x33FF | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF | 0xA000..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE6F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F | 0x1F900..=0x1F9FF | 0x20000..=0x3FFFD);
    if wide { 2 } else { 1 }
}

// ---------------------------------------------------------------------------
// 编译器域错误(build.rs 里的 panic)—— 它根本不进 JSON 流
// ---------------------------------------------------------------------------

/// 从 cargo 的 stderr 里捞 `.sv` 编译器域错误。
///
/// `build()` 的编译失败走 `panic!`,只出现在 cargo stderr 的 panic dump 里,
/// **不进 `--message-format=json` 的 compiler-message**。用户改 `.sv` 时语法错
/// 的频率远高于类型错,这条路不能不管。
pub fn scrape_build_script_error(stderr_line: &str) -> Option<String> {
    let rest = stderr_line.trim_start().strip_prefix("--> ")?;
    // 形如 `--> src\Counter.sv:19:15: 属性 `fg`:颜色 `#zzz` 不是合法十六进制`
    let (path, tail) = rest.split_once(".sv:")?;
    let mut it = tail.splitn(3, ':');
    let line: usize = it.next()?.trim().parse().ok()?;
    let col: usize = it.next()?.trim().parse().ok()?;
    let msg = it.next()?.trim();
    Some(format!("{path}.sv:{line}:{col}: error: {msg}"))
}

// ---------------------------------------------------------------------------
// 一次 check 的累加器(进程编排之外的**全部**决策)
// ---------------------------------------------------------------------------

/// 喂一行 cargo stdout 之后该做什么
pub enum Line {
    /// 不是诊断(`artifact` / `build-finished` 之类):丢掉即可
    Skip,
    /// 一条诊断
    Diag(Box<Rendered>),
    /// **这行没解析成 JSON**。调用方必须原样透出去,不许静默丢:
    /// "我读不懂所以我不说了"和吞诊断是同一种失败。
    /// 手写解析器只认它需要的那几个字段,cargo 换格式时这条是唯一的报警。
    Unparsed,
}

/// 一次 `sv check` 的累加器。
///
/// 存在的理由是**可测**:`bin/sv-check.rs` 里 `continue` 一下诊断就没了,
/// 而 bin 是单测照不到的地方。所以那里只留管道,判断全在这里。
#[derive(Default)]
pub struct Session {
    maps: Maps,
    /// 诊断条数(含 build.rs 域捞出来的)
    pub total: usize,
    pub errors: usize,
    /// 解析不了的 stdout 行数
    pub unparsed: usize,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    pub fn feed_stdout(&mut self, line: &str) -> Line {
        let Some(v) = json::parse(line) else {
            self.unparsed += 1;
            return Line::Unparsed;
        };
        if v.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            return Line::Skip;
        }
        let Some(msg) = v.get("message") else {
            return Line::Skip;
        };
        let r = render(msg, &mut self.maps);
        self.total += 1;
        self.errors += usize::from(r.is_error);
        Line::Diag(Box::new(r))
    }

    /// 喂一行 cargo stderr:捞 `.sv` 编译器域错误(build.rs 的 panic dump)。
    /// 捞到的**计入 errors**,否则 `.sv` 语法错会以退出码 0 收场。
    pub fn feed_stderr(&mut self, line: &str) -> Option<String> {
        let d = scrape_build_script_error(line)?;
        self.total += 1;
        self.errors += 1;
        Some(d)
    }

    pub fn summary(&self) -> String {
        let unparsed = if self.unparsed == 0 {
            String::new()
        } else {
            format!(
                ",{} 行 cargo 输出解析不了(已原样转到 stderr)",
                self.unparsed
            )
        };
        format!(
            "sv-check: {} 条诊断({} 条 error){unparsed},{}",
            self.total,
            self.errors,
            if self.errors == 0 {
                "通过"
            } else {
                "未通过"
            }
        )
    }

    /// 有 error 就 1;否则沿用 cargo 自己的退出码(链接失败之类我们看不见的错)
    pub fn exit_code(&self, cargo_status: i32) -> i32 {
        if self.errors > 0 { 1 } else { cargo_status }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sourcemap::Seg;

    #[test]
    fn json_parses_a_rustc_message_line() {
        let line = r#"{"reason":"compiler-message","message":{"level":"error","code":{"code":"E0277"},"message":"cannot add `&str` to `i32`","spans":[{"file_name":"a/out/counter.rs","line_start":44,"column_start":44,"column_end":45,"is_primary":true}],"children":[{"level":"note","message":"中文 中"}]},"escaped":"a\"b\\c\n"}"#;
        let v = json::parse(line).expect("应能解析");
        assert_eq!(v.get("reason").unwrap().as_str(), Some("compiler-message"));
        let m = v.get("message").unwrap();
        assert_eq!(m.get("level").unwrap().as_str(), Some("error"));
        assert_eq!(
            m.get("code").unwrap().get("code").unwrap().as_str(),
            Some("E0277")
        );
        let sp = &m.get("spans").unwrap().as_arr()[0];
        assert_eq!(sp.get("line_start").unwrap().as_usize(), Some(44));
        assert!(sp.get("is_primary").unwrap().is_true());
        assert_eq!(
            m.get("children").unwrap().as_arr()[0]
                .get("message")
                .unwrap()
                .as_str(),
            Some("中文 中")
        );
        assert_eq!(v.get("escaped").unwrap().as_str(), Some("a\"b\\c\n"));
    }

    #[test]
    fn json_rejects_trailing_garbage() {
        assert!(json::parse("{\"a\":1} oops").is_none());
        assert!(json::parse("not json").is_none());
    }

    /// 生成侧 `count.get()`,.sv 侧 `count`:落在 `get` 上的诊断退到
    /// "两锚点之间",落在 `count` 上的精确命中
    #[test]
    fn relocate_maps_gen_line_col_to_sv_line_col() {
        let sv = "<script>\nlet count = $state(0i32);\n</script>\n\n<text>{count}</text>\n";
        let generated = "// head\nfn f() {\n    __s.push_str(&count.get().to_string());\n}\n";
        let sv_count = sv.rfind("count").unwrap();
        let gen_count = generated.find("count.get").unwrap();
        let map = SourceMap::from_segs(vec![Seg {
            gen_start: gen_count,
            gen_end: gen_count + 5,
            sv_start: sv_count,
            sv_end: sv_count + 5,
            region: 0,
        }]);
        let (gl, gc) = sourcemap::byte_to_line_col(generated, gen_count);
        let m = relocate(&map, sv, generated, span(gl, gc, gl, gc + 5)).expect("应命中");
        assert_eq!((m.line, m.col, m.kind), (5, 8, MapKind::Exact));
    }

    fn span(line: usize, col: usize, line_end: usize, col_end: usize) -> GenSpan {
        GenSpan {
            line,
            col,
            line_end,
            col_end,
        }
    }

    /// rustc 的 `column_end` 属于 `line_end` 那一行。prettyplease 会把长行折断,
    /// 主 span 跨行是常态;拿 `line_start` 去换算 `column_end` 会算出一个既不是
    /// 起点也不是终点的字节,而 `lookup` 的"跨过右锚点"那一档就靠它。
    #[test]
    fn relocate_uses_line_end_for_column_end() {
        // 生成侧:第 2 行 `aaa` 起,第 3 行 `bbb` 止(span 跨行)
        let sv = "xx yy";
        let generated = "// h\naaa +\n  bbb\n";
        let ga = generated.find("aaa").unwrap();
        let gb = generated.find("bbb").unwrap();
        let map = SourceMap::from_segs(vec![
            Seg {
                gen_start: ga,
                gen_end: ga + 3,
                sv_start: 0,
                sv_end: 2,
                region: 7,
            },
            Seg {
                gen_start: gb,
                gen_end: gb + 3,
                sv_start: 3,
                sv_end: 5,
                region: 7,
            },
        ]);
        // 落在 `+` 上、终点在下一行的 `bbb` 之后 → 应吃进右锚点(sv_end = 5)
        let plus = generated.find('+').unwrap();
        let (pl, pc) = sourcemap::byte_to_line_col(generated, plus);
        let hit = map
            .lookup(
                sourcemap::line_col_to_byte(generated, pl, pc),
                sourcemap::line_col_to_byte(generated, 3, 6),
            )
            .expect("应插值");
        assert_eq!((hit.sv_start, hit.sv_end), (2, 5), "终点应按 line_end 换算");
        // 走 relocate 的完整路径:col_end=6 若被当成第 2 行,算出的字节还在 `+` 之前
        let m = relocate(&map, sv, generated, span(pl, pc, 3, 6)).expect("应命中");
        assert_eq!((m.line, m.col), (1, 4), "掐掉空白后落在 `yy` 上");
    }

    #[test]
    fn relocate_returns_none_on_glue() {
        let sv = "abc";
        let generated = "let __el1 = 0;\n";
        let map = SourceMap::from_segs(vec![Seg {
            gen_start: 100,
            gen_end: 103,
            sv_start: 0,
            sv_end: 3,
            region: 0,
        }]);
        assert_eq!(
            relocate(&map, sv, generated, span(1, 5, 1, 9)),
            None,
            "落在胶水上必须返回 None,交给调用方降级"
        );
    }

    #[test]
    fn render_never_drops_a_diagnostic_without_map() {
        // 指向一个没有 .svmap 的文件 → 原样透出,不吞
        let line = r#"{"level":"error","code":{"code":"E0308"},"message":"mismatched types","spans":[{"file_name":"src/main.rs","line_start":7,"column_start":9,"column_end":12,"is_primary":true}],"children":[]}"#;
        let v = json::parse(line).unwrap();
        let r = render(&v, &mut Maps::default());
        assert_eq!(
            r.headline,
            "src/main.rs:7:9: error[E0308]: mismatched types"
        );
        assert!(r.is_error);
    }

    #[test]
    fn render_keeps_messages_without_spans() {
        let v = json::parse(
            r#"{"level":"error","message":"aborting due to 1 previous error","spans":[]}"#,
        )
        .unwrap();
        let r = render(&v, &mut Maps::default());
        assert_eq!(r.headline, "error: aborting due to 1 previous error");
    }

    #[test]
    fn build_script_error_is_scraped_from_panic_dump() {
        let got = scrape_build_script_error(
            "  --> src\\Counter.sv:19:15: 属性 `fg`:颜色 `#zzz` 不是合法十六进制",
        );
        assert_eq!(
            got.as_deref(),
            Some("src\\Counter.sv:19:15: error: 属性 `fg`:颜色 `#zzz` 不是合法十六进制")
        );
        // build() 现在报绝对路径:盘符自带冒号,分割逻辑不能按第一个 `:` 来
        let got = scrape_build_script_error(
            r"  --> E:\WorkSpaces\svelte-rs\examples\counter-sfc\src\Counter.sv:19:15: 颜色不合法",
        );
        assert_eq!(
            got.as_deref(),
            Some(
                r"E:\WorkSpaces\svelte-rs\examples\counter-sfc\src\Counter.sv:19:15: error: 颜色不合法"
            )
        );
        assert_eq!(scrape_build_script_error("thread 'main' panicked"), None);
        assert_eq!(
            scrape_build_script_error("  --> src/main.rs:1:1: 不是 .sv"),
            None,
            "只认 .sv 的编译器域错误"
        );
    }

    /// `.sv` 的编译器域错误从 stderr 一路走到**退出码**。
    ///
    /// bin 里那条链(stderr 线程 → scrape → 计入 errors → 退出码)以前一行
    /// 测试都没有,而它是"改 `.sv` 写错语法"的主路径——比类型错常见得多。
    /// `Session` 存在的理由就是把这条链拉进单测射程。
    #[test]
    fn check_surfaces_build_script_error() {
        let mut s = Session::new();
        assert_eq!(s.feed_stderr("   Compiling counter-sfc v0.1.0"), None);
        let got = s.feed_stderr(
            "  --> E:\\ws\\examples\\counter-sfc\\src\\Counter.sv:19:15: 属性 `fg` 不是合法十六进制",
        );
        assert_eq!(
            got.as_deref(),
            Some(
                "E:\\ws\\examples\\counter-sfc\\src\\Counter.sv:19:15: error: 属性 `fg` 不是合法十六进制"
            )
        );
        assert_eq!((s.total, s.errors), (1, 1), "编译器域错误必须计入");
        assert_eq!(
            s.exit_code(0),
            1,
            "cargo 自己退 0(build.rs panic 也可能被别的包吃掉)也要退非零"
        );
        assert!(s.summary().contains("未通过"), "{}", s.summary());
    }

    /// **输入 N 行,输出 N 条**——守在 `Session` 上,不是守在 `render` 上。
    ///
    /// `render` 的签名 `&Value -> Rendered` 在类型上就吞不掉东西,拿
    /// `map().collect()` 的长度去断言"守恒"是恒真的空断言。真正会丢的是
    /// "这行 JSON 我解析不了"那条路。
    #[test]
    fn session_never_swallows_a_line() {
        let mut s = Session::new();
        let lines = [
            r#"{"reason":"compiler-message","message":{"level":"error","message":"boom","spans":[]}}"#,
            r#"{"reason":"compiler-message","message":{"level":"warning","message":"warn","spans":[]}}"#,
            r#"{"reason":"compiler-artifact","package_id":"x"}"#,
            "{这不是 JSON",
            r#"{"reason":"compiler-message","message":{"level":"error","message":"又一条","spans":[]}}"#,
        ];
        let mut diags = 0usize;
        let mut unparsed = 0usize;
        let mut skipped = 0usize;
        for l in lines {
            match s.feed_stdout(l) {
                Line::Diag(r) => {
                    assert!(!r.headline.is_empty(), "诊断被吞了: {l}");
                    diags += 1;
                }
                Line::Unparsed => unparsed += 1,
                Line::Skip => skipped += 1,
            }
        }
        assert_eq!((diags, unparsed, skipped), (3, 1, 1));
        assert_eq!((s.total, s.errors, s.unparsed), (3, 2, 1));
        assert!(
            s.summary().contains("解析不了"),
            "解析失败必须出现在汇总里,否则它就是被静默丢了: {}",
            s.summary()
        );
    }

    /// 每一种降级理由都必须**构造得出来**。
    /// 一个永远打不出来的分支等于没写,而用户会拿到另一条(错的)解释。
    #[test]
    fn degrade_notes_are_all_reachable() {
        use DegradeKind::*;
        let dir = std::env::temp_dir().join(format!("sv-degrade-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gen_file = dir.join("g.rs");
        std::fs::write(&gen_file, "fn f() {}\n").unwrap();
        let sv_file = dir.join("G.sv");
        let diag = |kind: &str| {
            format!(
                r#"{{"level":"error","message":"boom","spans":[{{"file_name":"{}","line_start":1,"column_start":1,"column_end":2,"is_primary":true}}],"children":[],"k":"{kind}"}}"#,
                gen_file.display().to_string().replace('\\', "\\\\")
            )
        };
        let headline = |svmap: &str| {
            std::fs::write(gen_file.with_extension("rs.svmap"), svmap).unwrap();
            let v = json::parse(&diag("x")).unwrap();
            render(&v, &mut Maps::default()).headline
        };

        // NoMap:.svmap 在,但根本不是 svmap(乱码)
        let h = headline("这不是 svmap\n");
        assert!(h.contains(degrade_note(NoMap)), "{h}");
        assert!(h.contains("boom"), "降级不许弄丢诊断内容: {h}");

        // MissingSv:map 指的 .sv 不存在(换了构建机 / 文件被删)
        let base = format!(
            "svmap 1\nsvlen 3\nsvhash {:016x}\ngenlen 10\nsv {}\n",
            sourcemap::fnv1a(b"abc"),
            sv_file.display()
        );
        let h = headline(&base);
        assert!(h.contains(degrade_note(MissingSv)), "{h}");

        // StaleMap:.sv 在,但内容对不上
        std::fs::write(&sv_file, "改过了").unwrap();
        let h = headline(&base);
        assert!(h.contains(degrade_note(StaleMap)), "{h}");

        // Glue / NoAnchors:同样是空表,但成因不同 → 措辞必须不同
        std::fs::write(&sv_file, "abc").unwrap();
        // 段落在 `{}` 那儿,诊断落在文件开头 → 表可信但这里确实是胶水
        let h = headline(&format!("{base}s 6 9 0 3 7\n"));
        assert!(h.contains(degrade_note(Glue)), "{h}");
        let h = headline(&format!("{base}blown 锚点数不等(3 vs 4)\n"));
        assert!(h.contains(degrade_note(NoAnchors)), "{h}");
        assert!(
            h.contains("锚点数不等(3 vs 4)"),
            "熔断原因必须带出来,否则没人查得下去: {h}"
        );
        assert!(!h.contains(degrade_note(Glue)), "熔断不能说成胶水: {h}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **落在胶水上的 suggestion 绝不能透出去。**
    ///
    /// rustc 的 `suggested_replacement` 是**生成文件**里的文本(`count.get()`
    /// 这种),位置又映射不回 `.sv`;照抄给用户等于教他往 `.sv` 里粘一段
    /// 编译器胶水。当前实现干脆一条 suggestion 都不输出,这条测试把
    /// "至少胶水档不许输出"钉住——以后给精确命中加 suggestion 时,
    /// 它会拦住顺手把降级档也放出去的写法。
    #[test]
    fn check_drops_suggestion_in_glue() {
        let line = r#"{"level":"error","code":{"code":"E0599"},"message":"no method named `foo`","spans":[{"file_name":"nowhere/gen.rs","line_start":3,"column_start":9,"column_end":12,"is_primary":true,"suggested_replacement":"__el1.get().foo()"}],"children":[{"level":"help","message":"there is a method","spans":[{"file_name":"nowhere/gen.rs","line_start":3,"column_start":9,"column_end":12,"is_primary":true,"suggested_replacement":"__el1.get().bar()"}]}]}"#;
        let v = json::parse(line).unwrap();
        let r = render(&v, &mut Maps::default());
        let all = std::iter::once(r.headline.clone())
            .chain(r.context.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("no method named `foo`"), "{all}");
        assert!(
            !all.contains("__el1"),
            "生成侧的 suggestion 文本不许出现在给 .sv 作者看的输出里:\n{all}"
        );
        assert!(!all.contains("suggested_replacement"), "{all}");
    }
}
