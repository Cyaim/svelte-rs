//! sv-lottie 端到端测试。
//!
//! 固件是**代码内嵌的手写 lottie**,不下载、不往仓库塞资产:
//! - 一个 40×30 的圆角矩形从 x=40 移到 x=160(蓝填充 + 3px 黑描边);
//! - 一个直径 30 的圆停在正中,填充色从红渐变到绿。
//!
//! 两者合起来覆盖了适配器的四条主路径:填充 / 描边 / 位置动画 / 颜色动画。

use sv_lottie::{
    Color, Error, LineCap, LineJoin, Lottie, PathCmd, PathFill, PathSink, Placement, RecordingSink,
    SinkCmd, StrokeStyle, Timeline,
};

/// 一个矩形图层遮罩(`mode: "s"` = subtract),盖住左半边。
///
/// **正解是右半边可见**;实际画出来的是左半边 —— velato 的
/// `render.rs:129-133` 把 `Mask::mode` 丢了,到适配器这一层只剩一个裸的
/// `push_clip_layer`。所以下面那条测试断言的是
/// `mask_clips_intersected != 0`(即"这一帧的遮罩方向不可信"),
/// 而不是"画对了"
const MASK_SUBTRACT: &str = r#"{
  "v": "5.9.0", "fr": 60, "ip": 0, "op": 60, "w": 200, "h": 100, "ddd": 0,
  "layers": [
    { "ddd": 0, "ty": 4, "ind": 1, "nm": "masked", "sr": 1, "st": 0, "ip": 0, "op": 60,
      "ks": { "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[0,0]}, "s": {"a":0,"k":[100,100]},
              "r": {"a":0,"k":0}, "o": {"a":0,"k":100} },
      "hasMask": true,
      "masksProperties": [
        { "inv": false, "mode": "s", "o": {"a":0,"k":100},
          "pt": { "a": 0, "k": { "c": true,
            "v": [[0,0],[100,0],[100,100],[0,100]],
            "i": [[0,0],[0,0],[0,0],[0,0]],
            "o": [[0,0],[0,0],[0,0],[0,0]] } } }
      ],
      "shapes": [
        { "ty": "gr", "nm": "g", "it": [
          { "ty": "rc", "nm": "r", "p": {"a":0,"k":[100,50]}, "s": {"a":0,"k":[200,100]}, "r": {"a":0,"k":0} },
          { "ty": "fl", "nm": "f", "o": {"a":0,"k":100}, "c": {"a":0,"k":[0,0,1]} },
          { "ty": "tr", "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[0,0]},
            "s": {"a":0,"k":[100,100]}, "r": {"a":0,"k":0}, "o": {"a":0,"k":100} }
        ] }
      ] }
  ]
}"#;

/// 三角形图层遮罩:不是轴对齐矩形 → 整条裁剪被忽略(fail-open)
const MASK_TRIANGLE: &str = r#"{
  "v": "5.9.0", "fr": 60, "ip": 0, "op": 60, "w": 200, "h": 100, "ddd": 0,
  "layers": [
    { "ddd": 0, "ty": 4, "ind": 1, "nm": "masked", "sr": 1, "st": 0, "ip": 0, "op": 60,
      "ks": { "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[0,0]}, "s": {"a":0,"k":[100,100]},
              "r": {"a":0,"k":0}, "o": {"a":0,"k":100} },
      "hasMask": true,
      "masksProperties": [
        { "inv": false, "mode": "a", "o": {"a":0,"k":100},
          "pt": { "a": 0, "k": { "c": true,
            "v": [[0,0],[100,0],[0,100]],
            "i": [[0,0],[0,0],[0,0]],
            "o": [[0,0],[0,0],[0,0]] } } }
      ],
      "shapes": [
        { "ty": "gr", "nm": "g", "it": [
          { "ty": "rc", "nm": "r", "p": {"a":0,"k":[100,50]}, "s": {"a":0,"k":[200,100]}, "r": {"a":0,"k":0} },
          { "ty": "fl", "nm": "f", "o": {"a":0,"k":100}, "c": {"a":0,"k":[0,0,1]} },
          { "ty": "tr", "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[0,0]},
            "s": {"a":0,"k":[100,100]}, "r": {"a":0,"k":0}, "o": {"a":0,"k":100} }
        ] }
      ] }
  ]
}"#;

/// 线性渐变填充(红 → 蓝),用来驱动 `gradient_fallbacks`
const GRADIENT_FILL: &str = r#"{
  "v": "5.9.0", "fr": 60, "ip": 0, "op": 60, "w": 200, "h": 100, "ddd": 0,
  "layers": [
    { "ddd": 0, "ty": 4, "ind": 1, "nm": "g", "sr": 1, "st": 0, "ip": 0, "op": 60,
      "ks": { "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[100,50]}, "s": {"a":0,"k":[100,100]},
              "r": {"a":0,"k":0}, "o": {"a":0,"k":100} },
      "shapes": [
        { "ty": "gr", "nm": "g", "it": [
          { "ty": "rc", "nm": "r", "p": {"a":0,"k":[0,0]}, "s": {"a":0,"k":[100,50]}, "r": {"a":0,"k":0} },
          { "ty": "gf", "nm": "gf", "o": {"a":0,"k":100}, "t": 1,
            "s": {"a":0,"k":[-50,0]}, "e": {"a":0,"k":[50,0]},
            "g": { "p": 2, "k": {"a":0,"k":[0, 1,0,0, 1, 0,0,1]} } },
          { "ty": "tr", "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[0,0]},
            "s": {"a":0,"k":[100,100]}, "r": {"a":0,"k":0}, "o": {"a":0,"k":100} }
        ] }
      ] }
  ]
}"#;

/// 轨道遮罩(track matte):`td` 图层在前、`tt` 图层在后。
/// velato 会为它发 `push_layer`,而我们把隔离层拍平 —— 后果是
/// **遮罩图层(白圆)本身被当成可见内容画出来**
const TRACK_MATTE: &str = r#"{
  "v": "5.9.0", "fr": 60, "ip": 0, "op": 60, "w": 200, "h": 100, "ddd": 0,
  "layers": [
    { "ddd": 0, "ty": 4, "ind": 1, "nm": "matte", "sr": 1, "st": 0, "ip": 0, "op": 60,
      "td": 1,
      "ks": { "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[100,50]}, "s": {"a":0,"k":[100,100]},
              "r": {"a":0,"k":0}, "o": {"a":0,"k":100} },
      "shapes": [
        { "ty": "gr", "nm": "g", "it": [
          { "ty": "el", "nm": "e", "p": {"a":0,"k":[0,0]}, "s": {"a":0,"k":[40,40]} },
          { "ty": "fl", "nm": "f", "o": {"a":0,"k":100}, "c": {"a":0,"k":[1,1,1]} },
          { "ty": "tr", "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[0,0]},
            "s": {"a":0,"k":[100,100]}, "r": {"a":0,"k":0}, "o": {"a":0,"k":100} }
        ] }
      ] },
    { "ddd": 0, "ty": 4, "ind": 2, "nm": "matted", "sr": 1, "st": 0, "ip": 0, "op": 60,
      "tt": 1,
      "ks": { "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[100,50]}, "s": {"a":0,"k":[100,100]},
              "r": {"a":0,"k":0}, "o": {"a":0,"k":100} },
      "shapes": [
        { "ty": "gr", "nm": "g", "it": [
          { "ty": "rc", "nm": "r", "p": {"a":0,"k":[0,0]}, "s": {"a":0,"k":[120,60]}, "r": {"a":0,"k":0} },
          { "ty": "fl", "nm": "f", "o": {"a":0,"k":100}, "c": {"a":0,"k":[0,0,1]} },
          { "ty": "tr", "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[0,0]},
            "s": {"a":0,"k":[100,100]}, "r": {"a":0,"k":0}, "o": {"a":0,"k":100} }
        ] }
      ] }
  ]
}"#;

/// 各向异性缩放(400% × 25%)的描边:`det = 1`,`√|det A|` 的修正量恰好为零
const ANISOTROPIC_STROKE: &str = r#"{
  "v": "5.9.0", "fr": 60, "ip": 0, "op": 60, "w": 200, "h": 100, "ddd": 0,
  "layers": [
    { "ddd": 0, "ty": 4, "ind": 1, "nm": "squash", "sr": 1, "st": 0, "ip": 0, "op": 60,
      "ks": { "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[100,50]}, "s": {"a":0,"k":[400,25]},
              "r": {"a":0,"k":0}, "o": {"a":0,"k":100} },
      "shapes": [
        { "ty": "gr", "nm": "g", "it": [
          { "ty": "rc", "nm": "r", "p": {"a":0,"k":[0,0]}, "s": {"a":0,"k":[80,40]}, "r": {"a":0,"k":0} },
          { "ty": "st", "nm": "s", "o": {"a":0,"k":100}, "c": {"a":0,"k":[0,0,0]},
            "w": {"a":0,"k":4}, "lc": 1, "lj": 1 },
          { "ty": "tr", "a": {"a":0,"k":[0,0]}, "p": {"a":0,"k":[0,0]},
            "s": {"a":0,"k":[100,100]}, "r": {"a":0,"k":0}, "o": {"a":0,"k":100} }
        ] }
      ] }
  ]
}"#;

/// 200×100 @ 60fps,ip=0 op=60 → 时长恰好 1000ms
const FIXTURE: &str = r#"{
  "v": "5.9.0",
  "nm": "sv-lottie fixture",
  "fr": 60, "ip": 0, "op": 60, "w": 200, "h": 100, "ddd": 0,
  "assets": [],
  "layers": [
    {
      "ddd": 0, "ty": 4, "ind": 1, "nm": "pulse", "sr": 1, "st": 0, "ip": 0, "op": 60,
      "ks": {
        "a": { "a": 0, "k": [0, 0] },
        "p": { "a": 0, "k": [100, 50] },
        "s": { "a": 0, "k": [100, 100] },
        "r": { "a": 0, "k": 0 },
        "o": { "a": 0, "k": 100 }
      },
      "shapes": [
        { "ty": "gr", "nm": "circle", "it": [
          { "ty": "el", "nm": "e", "p": { "a": 0, "k": [0, 0] }, "s": { "a": 0, "k": [30, 30] } },
          { "ty": "fl", "nm": "f", "o": { "a": 0, "k": 100 },
            "c": { "a": 1, "k": [
              { "t": 0,  "s": [1, 0, 0] },
              { "t": 59, "s": [0, 1, 0] }
            ] } },
          { "ty": "tr",
            "a": { "a": 0, "k": [0, 0] }, "p": { "a": 0, "k": [0, 0] },
            "s": { "a": 0, "k": [100, 100] }, "r": { "a": 0, "k": 0 },
            "o": { "a": 0, "k": 100 } }
        ] }
      ]
    },
    {
      "ddd": 0, "ty": 4, "ind": 2, "nm": "slider", "sr": 1, "st": 0, "ip": 0, "op": 60,
      "ks": {
        "a": { "a": 0, "k": [0, 0] },
        "p": { "a": 1, "k": [
          { "t": 0,  "s": [40, 50] },
          { "t": 59, "s": [160, 50] }
        ] },
        "s": { "a": 0, "k": [100, 100] },
        "r": { "a": 0, "k": 0 },
        "o": { "a": 0, "k": 100 }
      },
      "shapes": [
        { "ty": "gr", "nm": "rect", "it": [
          { "ty": "rc", "nm": "r", "p": { "a": 0, "k": [0, 0] },
            "s": { "a": 0, "k": [40, 30] }, "r": { "a": 0, "k": 4 } },
          { "ty": "fl", "nm": "f", "o": { "a": 0, "k": 100 },
            "c": { "a": 0, "k": [0.2, 0.4, 1] } },
          { "ty": "st", "nm": "s", "o": { "a": 0, "k": 100 },
            "c": { "a": 0, "k": [0, 0, 0] }, "w": { "a": 0, "k": 3 },
            "lc": 2, "lj": 2 },
          { "ty": "tr",
            "a": { "a": 0, "k": [0, 0] }, "p": { "a": 0, "k": [0, 0] },
            "s": { "a": 0, "k": [100, 100] }, "r": { "a": 0, "k": 0 },
            "o": { "a": 0, "k": 100 } }
        ] }
      ]
    }
  ]
}"#;

/// 合法 Lottie(规范里图层 transform 的旋转 `r` 本来就可省),但 velato
/// `import/converters.rs:213` 在 `None` 分支写了 `todo!("split rotation")`
const NO_ROTATION_KEY: &str = r#"{
  "v": "5.9.0", "fr": 60, "ip": 0, "op": 60, "w": 100, "h": 100, "ddd": 0,
  "layers": [
    { "ddd": 0, "ty": 4, "ind": 1, "nm": "no-r", "st": 0, "ip": 0, "op": 60,
      "ks": {
        "a": { "a": 0, "k": [0, 0] },
        "p": { "a": 0, "k": [50, 50] },
        "s": { "a": 0, "k": [100, 100] },
        "o": { "a": 0, "k": 100 }
      },
      "shapes": [] }
  ]
}"#;

fn load() -> Lottie {
    Lottie::from_json_str(FIXTURE).expect("固件应当是合法 lottie")
}

fn record(anim: &mut Lottie, frame: f64) -> RecordingSink {
    let mut sink = RecordingSink::default();
    anim.render(frame, Placement::IDENTITY, 1.0, &mut sink);
    sink
}

// ---------------------------------------------------------------------------
// 解析与时间轴
// ---------------------------------------------------------------------------

#[test]
fn parses_and_exposes_timeline_facts() {
    let anim = load();
    assert_eq!(anim.size(), (200.0, 100.0));
    let tl = anim.timeline();
    assert_eq!(tl.frame_rate, 60.0);
    assert_eq!(tl.start_frame, 0.0);
    assert_eq!(tl.end_frame, 60.0);
    assert!(
        (tl.duration_ms() - 1000.0).abs() < 1e-9,
        "60 帧 @ 60fps 应当是 1000ms,实际 {}",
        tl.duration_ms()
    );
}

#[test]
fn malformed_json_is_a_parse_error() {
    let err = Lottie::from_json_str("{ not json }").unwrap_err();
    assert!(matches!(err, Error::Parse(_)), "实际 {err:?}");
}

// ---------------------------------------------------------------------------
// 时间轴边界:环绕 / 钳制 / 半开区间
// ---------------------------------------------------------------------------

fn tl() -> Timeline {
    Timeline {
        start_frame: 0.0,
        end_frame: 60.0,
        frame_rate: 60.0,
    }
}

#[test]
fn timeline_clamps_out_of_range_time() {
    let t = tl();
    assert_eq!(t.frame_at_ms(-1.0, false), 0.0, "t<0 钳制回起点");
    assert_eq!(t.frame_at_ms(-1e9, false), 0.0);
    assert!((t.frame_at_ms(500.0, false) - 30.0).abs() < 1e-6, "中点");
    // t > 时长 → 定格末尾。半开区间:必须严格小于 end_frame(见下一条测试)
    assert!(t.frame_at_ms(1e9, false) < 60.0);
    assert!(t.frame_at_ms(1e9, false) > 59.999);
}

#[test]
fn timeline_wraps_when_looped() {
    let t = tl();
    // 1000ms = 恰好一圈 → 回到 0,而不是停在 60
    assert!(t.frame_at_ms(1000.0, true).abs() < 1e-6);
    // 掉帧不丢相位:2500ms 与 500ms 同相
    assert!((t.frame_at_ms(2500.0, true) - t.frame_at_ms(500.0, true)).abs() < 1e-6);
    // 负时间按 rem_euclid 折回区间尾部
    assert!((t.frame_at_ms(-250.0, true) - 45.0).abs() < 1e-6);
}

#[test]
fn timeline_never_returns_end_frame() {
    // velato 用 `layer.frames.contains(&frame)` 判图层活跃,而 Range 排除上界:
    // 返回正好 60.0 会让整帧一个图层都画不出来。这条是回归卫兵
    let t = tl();
    for &(ms, looped) in &[
        (1000.0, false),
        (1000.0, true),
        (999.999_999, false),
        (1e12, false),
        (f64::INFINITY, false),
        (f64::NAN, true),
    ] {
        let f = t.frame_at_ms(ms, looped);
        assert!(
            (0.0..60.0).contains(&f),
            "frame_at_ms({ms}, {looped}) = {f},越出了 [0, 60)"
        );
    }
    // 定格末尾那一帧必须真的画得出东西
    let mut anim = load();
    let last = anim.timeline().frame_at_ms(1e9, false);
    assert!(
        !record(&mut anim, last).cmds.is_empty(),
        "钳制到末尾的帧不该是空画面"
    );
}

#[test]
fn timeline_half_open_invariant_holds_on_extreme_frame_numbers() {
    // 上一条只沿"时间"这一个轴扫,而不变量真正会破的是**时间轴本身的量级**:
    // 老实现用 `end_frame - frame_span * 1e-9` 收紧上界,当
    // `end_frame / frame_span > 9e6` 时那个减量掉到 end_frame 的半个 ULP 以下,
    // 浮点加法原样吃掉它 → `last == end_frame`,返回值等于 end_frame → 空画面。
    // 现在用的是 `next_down()`,按定义退一个 ULP,这条测试是它的卫兵
    for &(start, end, rate) in &[
        (0.0_f64, 60.0_f64, 60.0_f64),
        (1e7, 1e7 + 5.0, 60.0),
        (1e9, 1e9 + 10.0, 60.0), // 老实现在这里破
        (1e15, 1e15 + 1.0, 24.0),
        (-1e9, -1e9 + 3.0, 30.0),
    ] {
        let t = Timeline {
            start_frame: start,
            end_frame: end,
            frame_rate: rate,
        };
        for &(ms, looped) in &[(1e12, false), (0.0, false), (-1.0, true), (1e12, true)] {
            let f = t.frame_at_ms(ms, looped);
            assert!(
                f >= start && f < end,
                "start={start} end={end} frame_at_ms({ms}, {looped}) = {f},越出了半开区间"
            );
        }
    }
}

#[test]
fn degenerate_timeline_is_not_a_division_by_zero() {
    let bad = Timeline {
        start_frame: 0.0,
        end_frame: 0.0,
        frame_rate: 0.0,
    };
    assert_eq!(bad.duration_ms(), 0.0);
    assert_eq!(bad.frame_at_ms(123.0, true), 0.0);
    assert_eq!(bad.frame_at_ms(123.0, false), 0.0);
}

// ---------------------------------------------------------------------------
// 播放态
// ---------------------------------------------------------------------------

#[test]
fn playback_loop_keeps_phase_after_a_dropped_frame() {
    let mut p = load().playback();
    // 窗口被拖了一下,整整 2.5 圈没推进
    assert!(p.advance(2500.0), "循环动画永不结束");
    assert!(
        (p.time_ms - 500.0).abs() < 1e-9,
        "取模而不是清零,实际 {}",
        p.time_ms
    );
    assert!((p.frame() - 30.0).abs() < 1e-6);
}

#[test]
fn playback_without_loop_stops_at_the_last_pose() {
    let mut p = load().playback().looped(false);
    assert!(p.advance(400.0));
    assert!(!p.advance(5000.0), "走完之后不该再要求续帧");
    assert!(p.finished());
    assert_eq!(p.time_ms, 1000.0, "停在时长上,不是回到 0");
    assert!(
        p.frame() < 60.0 && p.frame() > 59.999,
        "定格在最后一帧的姿势"
    );
    assert!((p.progress() - 1.0).abs() < 1e-6);
    // 停了之后再 advance 不该复活
    assert!(!p.advance(16.0));
}

#[test]
fn playback_seek_respects_loop_mode() {
    let mut p = load().playback();
    p.seek_ms(1500.0);
    assert!((p.time_ms - 500.0).abs() < 1e-9, "循环:取模");
    let mut q = load().playback().looped(false);
    q.seek_ms(1500.0);
    assert_eq!(q.time_ms, 1000.0, "非循环:钳制");
    q.seek_ms(f64::NAN);
    assert_eq!(q.time_ms, 0.0, "NaN 不该污染时间轴");
}

#[test]
fn playback_speed_scales_the_advance() {
    let mut p = load().playback().speed(2.0);
    p.advance(100.0);
    assert!((p.time_ms - 200.0).abs() < 1e-9, "2x:100ms 走 200ms");
    let mut q = load().playback().speed(0.25);
    q.advance(100.0);
    assert!((q.time_ms - 25.0).abs() < 1e-9, "0.25x:100ms 走 25ms");
    // 0x 是合法的"暂停但仍在播"
    let mut z = load().playback().speed(0.0);
    assert!(z.advance(1000.0));
    assert_eq!(z.time_ms, 0.0);
}

#[test]
fn playback_reverse_runs_backwards_and_stops_at_the_start() {
    // 倒放的用法:先 seek 到末尾,再用负速推进
    let mut p = load().playback().looped(false).speed(-1.0);
    p.seek_ms(1000.0);
    assert!(p.advance(300.0), "还没到头");
    assert!((p.time_ms - 700.0).abs() < 1e-9, "实际 {}", p.time_ms);
    assert!((p.frame() - 42.0).abs() < 1e-6, "实际 {}", p.frame());
    // 推过头 → 停在 0 并且"结束"
    assert!(!p.advance(5000.0), "倒放到头同样是结束");
    assert_eq!(p.time_ms, 0.0);
    assert!(p.finished());
    assert_eq!(p.progress(), 0.0);
    // 循环 + 倒放:rem_euclid 折回区间尾部,永不结束
    let mut q = load().playback().speed(-1.0);
    assert!(q.advance(250.0));
    assert!((q.time_ms - 750.0).abs() < 1e-9, "实际 {}", q.time_ms);
    assert!(!q.finished());
}

// ---------------------------------------------------------------------------
// 渲染:动画真的动了
// ---------------------------------------------------------------------------

#[test]
fn first_and_last_frame_draw_differently() {
    let mut anim = load();
    let tl = anim.timeline();
    let first = record(&mut anim, tl.frame_at_ms(0.0, false));
    let last = record(&mut anim, tl.frame_at_ms(tl.duration_ms(), false));

    assert!(!first.cmds.is_empty(), "首帧应当有绘制命令");
    assert_eq!(
        first.cmds.len(),
        last.cmds.len(),
        "结构不变(同样的图层/形状),变的只是几何与颜色"
    );
    assert_ne!(first.cmds, last.cmds, "t=0 与 t=末尾必须画得不一样");

    // 具体到"矩形右移了"与"圆变色了",避免上面那条被某个无关字段偶然满足
    let bbox_of_widest = |s: &RecordingSink| {
        s.cmds
            .iter()
            .filter_map(|c| match c {
                SinkCmd::Stroke { bbox, .. } => Some(*bbox),
                _ => None,
            })
            .next()
            .expect("固件里有一条描边")
    };
    assert!(
        bbox_of_widest(&last).0 > bbox_of_widest(&first).0 + 50,
        "矩形应当明显右移:{:?} → {:?}",
        bbox_of_widest(&first),
        bbox_of_widest(&last)
    );

    let fill_colors = |s: &RecordingSink| {
        s.cmds
            .iter()
            .filter_map(|c| match c {
                SinkCmd::Fill { color, .. } => Some(*color),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let (a, b) = (fill_colors(&first), fill_colors(&last));
    assert!(
        a.iter().any(|c| c.r > 200 && c.g < 60),
        "首帧应当有一块红,实际 {a:?}"
    );
    assert!(
        b.iter().any(|c| c.g > 200 && c.r < 60),
        "末帧应当有一块绿,实际 {b:?}"
    );
}

#[test]
fn render_is_deterministic() {
    let mut anim = load();
    let a = record(&mut anim, 20.0);
    let b = record(&mut anim, 20.0);
    assert_eq!(a.cmds, b.cmds, "同一帧两次绘制的命令流应当逐字相等");
}

#[test]
fn root_canvas_clip_becomes_a_rect_clip_and_stays_balanced() {
    // velato 每帧开头必发一次覆盖整幅合成画布的 push_clip_layer(render.rs:68)。
    // 它是轴对齐矩形,应当落到 Painter 已有的矩形裁剪上,而不是被丢掉
    let mut anim = load();
    let mut sink = RecordingSink::default();
    let stats = anim.render(0.0, Placement::IDENTITY, 1.0, &mut sink);

    assert_eq!(stats.clips_applied, 1, "根裁剪应当被识别成矩形");
    assert_eq!(stats.clips_ignored, 0, "这个固件没有非矩形裁剪");
    assert_eq!(stats.unbalanced_pops, 0, "图层栈必须平衡");
    assert!(matches!(
        sink.cmds.first(),
        Some(SinkCmd::PushClip {
            x: 0,
            y: 0,
            w: 200,
            h: 100
        })
    ));
    assert!(matches!(sink.cmds.last(), Some(SinkCmd::PopClip)));
}

#[test]
fn fixture_triggers_no_silent_degradation() {
    let mut anim = load();
    let mut sink = RecordingSink::default();
    let stats = anim.render(30.0, Placement::IDENTITY, 1.0, &mut sink);

    assert_eq!(stats.fills, 2, "圆一块填充 + 矩形一块填充");
    assert_eq!(stats.strokes, 1);
    // 一句话问完:任何一条降级计数器非零都会让它变 true。
    // 加了新计数器也不会漏掉这条断言
    assert!(!stats.degraded(), "基线固件不该触发任何降级:{stats:?}");
}

// ---------------------------------------------------------------------------
// 降级路径:每一条都必须真的**跑过**,不能只断言它等于 0
// ---------------------------------------------------------------------------

#[test]
fn rect_layer_mask_is_flagged_as_an_unverifiable_intersect() {
    // 固件是 `mode: "s"`(subtract)的矩形遮罩,盖住左半边 → **正解是右半边可见**。
    // velato 的 render.rs:129-133 把 Mask::mode 丢了,只发一个裸的
    // push_clip_layer,于是它落成 intersect 裁剪 —— 画出来的是**左半边**,正好反了。
    //
    // 这一层修不好方向(信息在上游就没了),能做的是**不把它报成零降级**
    let mut anim = Lottie::from_json_str(MASK_SUBTRACT).expect("合法 lottie");
    let mut sink = RecordingSink::default();
    let stats = anim.render(0.0, Placement::IDENTITY, 1.0, &mut sink);

    assert_eq!(stats.clips_applied, 2, "根裁剪 + 遮罩");
    assert_eq!(stats.clips_ignored, 0, "遮罩是矩形,没被忽略");
    assert_eq!(
        stats.mask_clips_intersected, 1,
        "非根的矩形裁剪 = 图层遮罩,mode 已丢失,必须记一笔:{stats:?}"
    );
    assert!(stats.degraded(), "带遮罩的一帧不该被报成零降级");

    // 落到命令流上:遮罩确实被当成 intersect 裁剪压了下去(左半边)
    assert_eq!(
        sink.cmds[1],
        SinkCmd::PushClip {
            x: 0,
            y: 0,
            w: 100,
            h: 100
        },
        "实际命令流 {:?}",
        sink.cmds
    );
    assert_eq!(stats.unbalanced_pops, 0);
    assert_eq!(stats.unclosed_layers, 0, "栈必须被排空");
}

#[test]
fn non_rect_layer_mask_fails_open_and_is_counted() {
    // 三角形遮罩:不是轴对齐矩形 → 整条裁剪忽略,被遮住的部分照常画出来
    let mut anim = Lottie::from_json_str(MASK_TRIANGLE).expect("合法 lottie");
    let mut sink = RecordingSink::default();
    let stats = anim.render(0.0, Placement::IDENTITY, 1.0, &mut sink);

    assert_eq!(stats.clips_ignored, 1, "三角形遮罩被忽略:{stats:?}");
    assert_eq!(stats.clips_applied, 1, "只剩根裁剪");
    assert_eq!(stats.mask_clips_intersected, 0);
    assert!(stats.degraded());
    // fail-open:整块 200×100 都画出来了(遮罩没生效),而不是少画
    let fill_bbox = sink
        .cmds
        .iter()
        .find_map(|c| match c {
            SinkCmd::Fill { bbox, .. } => Some(*bbox),
            _ => None,
        })
        .expect("有一块填充");
    assert_eq!(fill_bbox, (0, 0, 200, 100), "被遮住的部分照常画出来");
    // 被忽略的那一格不能发 pop_clip(会把调用方的裁剪栈带崩)
    assert_eq!(
        sink.cmds
            .iter()
            .filter(|c| matches!(c, SinkCmd::PopClip))
            .count(),
        1,
        "只有根裁剪那一次 pop:{:?}",
        sink.cmds
    );
}

#[test]
fn gradient_fill_is_flattened_to_one_color_and_counted() {
    let mut anim = Lottie::from_json_str(GRADIENT_FILL).expect("合法 lottie");
    let mut sink = RecordingSink::default();
    let stats = anim.render(0.0, Placement::IDENTITY, 1.0, &mut sink);

    assert_eq!(stats.gradient_fallbacks, 1, "{stats:?}");
    assert_eq!(stats.fills, 1);
    assert!(stats.degraded());
    let color = sink
        .cmds
        .iter()
        .find_map(|c| match c {
            SinkCmd::Fill { color, .. } => Some(*color),
            _ => None,
        })
        .expect("有一块填充");
    // 红(0.0)→ 蓝(1.0)的分段线性均值 = 各半
    assert_eq!((color.r, color.g, color.b), (128, 0, 128), "实际 {color:?}");
}

#[test]
fn track_matte_draws_the_matte_layer_as_visible_content_and_is_counted() {
    // 已知缺口的端到端证据:隔离层被拍平,**遮罩图层(白圆)自己被画了出来**。
    // 这正是 README §2 里 `layers_flattened` 那一行描述的后果
    let mut anim = Lottie::from_json_str(TRACK_MATTE).expect("合法 lottie");
    let mut sink = RecordingSink::default();
    let stats = anim.render(0.0, Placement::IDENTITY, 1.0, &mut sink);

    assert_eq!(
        stats.layers_flattened, 2,
        "velato 为轨道遮罩发两次:{stats:?}"
    );
    assert_eq!(stats.unbalanced_pops, 0, "拍平不等于失衡");
    assert_eq!(stats.unclosed_layers, 0);
    assert!(stats.degraded());
    let colors = sink
        .cmds
        .iter()
        .filter_map(|c| match c {
            SinkCmd::Fill { color, .. } => Some((color.r, color.g, color.b)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        colors.contains(&(255, 255, 255)),
        "遮罩图层本身被当成内容画了出来(这是已知缺口,不是意外),实际 {colors:?}"
    );
}

#[test]
fn anisotropic_layer_scale_flags_the_stroke_width() {
    // s = [400%, 25%] → det = 1 → √|det A| = 1.0 = **零修正**。
    // 正解是横向 16px / 纵向 1px,画出来还是 4px
    let mut anim = Lottie::from_json_str(ANISOTROPIC_STROKE).expect("合法 lottie");
    let mut sink = RecordingSink::default();
    let stats = anim.render(0.0, Placement::IDENTITY, 1.0, &mut sink);

    assert_eq!(stats.strokes, 1);
    assert_eq!(
        stats.stroke_width_approximated, 1,
        "各向异性描边必须被报出来:{stats:?}"
    );
    assert!(stats.degraded());
    let (width, bbox) = sink
        .cmds
        .iter()
        .find_map(|c| match c {
            SinkCmd::Stroke { width, bbox, .. } => Some((*width, *bbox)),
            _ => None,
        })
        .expect("有一条描边");
    assert_eq!(width, 4, "补偿量恰好为零 —— 计数器就是为了让这件事不静默");
    // 几何本身是被正确拉伸的(320 × 10),只有描边宽度没跟上
    assert_eq!(bbox, (-60, 45, 260, 55), "实际 {bbox:?}");
}

#[test]
fn zero_alpha_skips_every_draw_and_is_counted() {
    let mut anim = load();
    let mut sink = RecordingSink::default();
    let stats = anim.render(0.0, Placement::IDENTITY, 0.0, &mut sink);

    assert_eq!(stats.fills, 0);
    assert_eq!(stats.strokes, 0);
    assert_eq!(
        stats.transparent_skipped, 3,
        "两块填充 + 一条描边:{stats:?}"
    );
    assert!(stats.degraded());
    // 裁剪照压照弹 —— 栈必须仍然平衡
    assert_eq!(stats.unbalanced_pops, 0);
    assert_eq!(stats.unclosed_layers, 0);
    assert!(
        sink.cmds
            .iter()
            .all(|c| matches!(c, SinkCmd::PushClip { .. } | SinkCmd::PopClip)),
        "只剩裁剪:{:?}",
        sink.cmds
    );
}

#[test]
fn render_alpha_multiplies_into_every_color() {
    let mut anim = load();
    let opaque = record(&mut anim, 0.0);
    let mut half = RecordingSink::default();
    anim.render(0.0, Placement::IDENTITY, 0.5, &mut half);

    let alphas = |s: &RecordingSink| {
        s.cmds
            .iter()
            .filter_map(|c| match c {
                SinkCmd::Fill { color, .. } | SinkCmd::Stroke { color, .. } => Some(color.a),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(alphas(&opaque), vec![255, 255, 255]);
    assert_eq!(
        alphas(&half),
        vec![128, 128, 128],
        "alpha=0.5 透传到每条颜色"
    );
    // 越界的 alpha 被钳制,不是绕回
    let mut over = RecordingSink::default();
    anim.render(0.0, Placement::IDENTITY, 9.0, &mut over);
    assert_eq!(alphas(&over), vec![255, 255, 255], "alpha>1 钳制到 1");
    let mut neg = RecordingSink::default();
    let stats = anim.render(0.0, Placement::IDENTITY, -3.0, &mut neg);
    assert_eq!(stats.transparent_skipped, 3, "alpha<0 钳制到 0");
}

#[test]
fn fit_contain_placement_translates_the_whole_frame() {
    // fit_contain 的结果必须真的喂得进 render:平移 + 缩放都要落到命令流上
    let mut anim = load();
    // 200×100 摆进 x=300 处的 400×400 盒子 → scale=2、tx=300、ty=100
    let place = anim.fit_contain(300.0, 0.0, 400.0, 400.0);
    assert_eq!((place.tx, place.ty, place.scale), (300.0, 100.0, 2.0));

    let mut sink = RecordingSink::default();
    let stats = anim.render(0.0, place, 1.0, &mut sink);
    assert!(!stats.degraded(), "{stats:?}");
    // 根裁剪 = 合成画布经同一变换后的矩形
    assert_eq!(
        sink.cmds.first(),
        Some(&SinkCmd::PushClip {
            x: 300,
            y: 100,
            w: 400,
            h: 200
        }),
        "实际 {:?}",
        sink.cmds.first()
    );
    // 几何跟着平移:首帧矩形左边缘 20 → 300 + 20×2 = 340
    let bbox = sink
        .cmds
        .iter()
        .find_map(|c| match c {
            SinkCmd::Stroke { bbox, .. } => Some(*bbox),
            _ => None,
        })
        .expect("有一条描边");
    assert!(
        bbox.0 >= 330 && bbox.0 <= 340,
        "描边包围盒左边缘 {}",
        bbox.0
    );
    assert!(bbox.1 >= 160, "竖直方向也平移了:{}", bbox.1);
}

#[test]
fn render_with_tolerance_sanitizes_its_input() {
    // 非法容差(0 / 负 / NaN / Inf)必须回落到默认值,而不是产出空路径或 NaN 坐标
    let mut anim = load();
    let baseline = record(&mut anim, 0.0);
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let mut sink = RecordingSink::default();
        let stats = anim.render_with_tolerance(0.0, Placement::IDENTITY, 1.0, bad, &mut sink);
        assert!(!stats.degraded(), "tolerance={bad}:{stats:?}");
        assert_eq!(sink.cmds, baseline.cmds, "tolerance={bad} 应当等价于默认值");
    }
    // 合法的粗容差不会崩,也不会把画面清空
    let mut coarse = RecordingSink::default();
    let stats = anim.render_with_tolerance(0.0, Placement::IDENTITY, 1.0, 8.0, &mut coarse);
    assert_eq!(stats.fills, 2);
    assert_eq!(stats.strokes, 1);
    assert_eq!(stats.empty_paths_skipped, 0);
}

#[test]
fn a_sink_behind_a_reference_or_behind_dyn_works_too() {
    // 调用方手里常见的是 `&mut dyn PathSink`(壳侧的 Painter 桥就是这形状)
    let mut anim = load();
    let direct = record(&mut anim, 0.0);

    let mut inner = RecordingSink::default();
    {
        // S = &mut RecordingSink → 走 `impl PathSink for &mut S`
        let mut by_ref: &mut RecordingSink = &mut inner;
        anim.render(0.0, Placement::IDENTITY, 1.0, &mut by_ref);
    }
    assert_eq!(inner.cmds, direct.cmds, "经一层 &mut 转发后命令流不变");

    let mut erased = RecordingSink::default();
    {
        // S = dyn PathSink → 走 `?Sized` 那条
        let dynamic: &mut dyn PathSink = &mut erased;
        anim.render(0.0, Placement::IDENTITY, 1.0, dynamic);
    }
    assert_eq!(erased.cmds, direct.cmds, "经 dyn 擦除后命令流不变");
}

// ---------------------------------------------------------------------------
// 诊断表面
// ---------------------------------------------------------------------------

#[test]
fn error_display_names_the_two_failure_modes_apart() {
    let parse = Lottie::from_json_str("{ not json }").unwrap_err();
    assert!(parse.to_string().contains("解析失败"), "{parse}");

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let unsupported = Lottie::from_json_str(NO_ROTATION_KEY).unwrap_err();
    std::panic::set_hook(prev);
    let msg = unsupported.to_string();
    assert!(msg.contains("尚未实现"), "{msg}");
    assert!(msg.contains("split rotation"), "要带上 velato 的原文:{msg}");
    assert_ne!(parse, unsupported);
}

#[test]
fn lottie_debug_stays_short_enough_to_read_in_an_assert() {
    // 手写 Debug 的理由:derive 会把整棵图层树连同每条关键帧全打出来
    let anim = load();
    let s = format!("{anim:?}");
    assert!(s.contains("size"), "{s}");
    assert!(s.contains("timeline"), "{s}");
    assert!(s.contains("layers: 2"), "{s}");
    assert!(s.len() < 400, "Debug 输出 {} 字符,太长了:{s}", s.len());
}

#[test]
fn stroke_style_survives_the_round_trip() {
    let mut anim = load();
    let sink = record(&mut anim, 0.0);
    let stroke = sink
        .cmds
        .iter()
        .find_map(|c| match c {
            SinkCmd::Stroke {
                width, cap, join, ..
            } => Some((*width, *cap, *join)),
            _ => None,
        })
        .expect("固件里有一条描边");
    // 固件写的是 w=3 / lc=2(Round) / lj=2(Round)
    assert_eq!(stroke, (3, LineCap::Round, LineJoin::Round));
}

#[test]
fn placement_scales_geometry_and_stroke_width_together() {
    // 变换烘焙进坐标之后描边宽度不再自动缩放,靠 √|det A| 补偿。
    // 各向同性缩放下这个补偿是精确的,这条测试就是它的卫兵
    let mut anim = load();
    let place = Placement {
        tx: 0.0,
        ty: 0.0,
        scale: 3.0,
    };
    let mut sink = RecordingSink::default();
    anim.render(0.0, place, 1.0, &mut sink);
    let width = sink
        .cmds
        .iter()
        .find_map(|c| match c {
            SinkCmd::Stroke { width, .. } => Some(*width),
            _ => None,
        })
        .unwrap();
    assert_eq!(width, 9, "3px 描边放大 3 倍应当是 9px,实际 {width}");
}

#[test]
fn fit_contain_centers_inside_the_box() {
    let anim = load();
    // 200×100 摆进 400×400:等比放大 2 倍(受宽限制),竖直方向居中
    let p = anim.fit_contain(0.0, 0.0, 400.0, 400.0);
    assert_eq!(p.scale, 2.0);
    assert_eq!(p.tx, 0.0);
    assert_eq!(p.ty, 100.0);
}

// ---------------------------------------------------------------------------
// 健壮性:velato 在合法输入上 panic,不能让它崩进程
// ---------------------------------------------------------------------------

#[test]
fn unsupported_lottie_reports_error_instead_of_panicking() {
    // 默认 panic hook 会把 velato 的 todo! 消息打到 stderr,读起来像测试炸了。
    // 这里临时换成空 hook —— 全局状态,并发跑时最多让别的测试少一条 panic 日志
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let got = Lottie::from_json_str(NO_ROTATION_KEY);
    std::panic::set_hook(prev);

    match got {
        Err(Error::Unsupported(msg)) => {
            assert!(
                msg.contains("split rotation"),
                "应当带上 velato 的 todo! 原文,实际 {msg:?}"
            );
        }
        other => panic!("期望 Unsupported,实际 {:?}", other.map(|_| "Ok")),
    }
}

// ---------------------------------------------------------------------------
// 真像素:PathSink 落到 tiny-skia 上
// ---------------------------------------------------------------------------

/// 最小 tiny-skia sink。**只是测试脚手架** —— 真实路径上这一层是
/// `sv_shell::TinySkiaPainter`(它已经有 fill_path / stroke_path / push_clip)。
///
/// 裁剪是**真实现**的(`PathSink` 给了默认空实现,但那样这条唯一的像素测试
/// 就完全不测裁剪了):栈顶存的是已经求过交的矩形,`None` = 空裁剪(什么都不画)
struct PixmapSink<'a> {
    pm: &'a mut tiny_skia::Pixmap,
    /// 每一格都是从根求交下来的结果;`None` = 交出来是空的
    clips: Vec<Option<tiny_skia::Rect>>,
    /// 栈顶裁剪对应的遮罩,随 push/pop 重建
    mask: Option<tiny_skia::Mask>,
}

impl<'a> PixmapSink<'a> {
    fn new(pm: &'a mut tiny_skia::Pixmap) -> Self {
        Self {
            pm,
            clips: Vec::new(),
            mask: None,
        }
    }

    fn rebuild_mask(&mut self) {
        self.mask = match self.clips.last() {
            // 没有裁剪:不需要遮罩
            None => None,
            Some(top) => {
                let mut m =
                    tiny_skia::Mask::new(self.pm.width(), self.pm.height()).expect("画布尺寸非零");
                // 空裁剪就留一张全 0 的遮罩 —— 什么都画不出来,这是正解
                if let Some(rect) = top {
                    let mut pb = tiny_skia::PathBuilder::new();
                    pb.push_rect(*rect);
                    if let Some(p) = pb.finish() {
                        m.fill_path(
                            &p,
                            tiny_skia::FillRule::Winding,
                            true,
                            tiny_skia::Transform::identity(),
                        );
                    }
                }
                Some(m)
            }
        };
    }
}

fn build_path(cmds: &[PathCmd]) -> Option<tiny_skia::Path> {
    let mut pb = tiny_skia::PathBuilder::new();
    for c in cmds {
        match *c {
            PathCmd::MoveTo(x, y) => pb.move_to(x, y),
            PathCmd::LineTo(x, y) => pb.line_to(x, y),
            PathCmd::QuadTo(cx, cy, x, y) => pb.quad_to(cx, cy, x, y),
            PathCmd::CubicTo(a, b, c2, d, x, y) => pb.cubic_to(a, b, c2, d, x, y),
            PathCmd::Close => pb.close(),
        }
    }
    pb.finish()
}

impl PathSink for PixmapSink<'_> {
    fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color) {
        let Some(p) = build_path(path) else { return };
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        paint.anti_alias = true;
        let rule = match fill {
            PathFill::NonZero => tiny_skia::FillRule::Winding,
            PathFill::EvenOdd => tiny_skia::FillRule::EvenOdd,
        };
        self.pm.fill_path(
            &p,
            &paint,
            rule,
            tiny_skia::Transform::identity(),
            self.mask.as_ref(),
        );
    }

    fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color) {
        let Some(p) = build_path(path) else { return };
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(color.r, color.g, color.b, color.a);
        paint.anti_alias = true;
        let stroke = tiny_skia::Stroke {
            width: style.width,
            miter_limit: style.miter_limit,
            line_cap: match style.cap {
                LineCap::Butt => tiny_skia::LineCap::Butt,
                LineCap::Round => tiny_skia::LineCap::Round,
                LineCap::Square => tiny_skia::LineCap::Square,
            },
            line_join: match style.join {
                LineJoin::Miter => tiny_skia::LineJoin::Miter,
                LineJoin::Round => tiny_skia::LineJoin::Round,
                LineJoin::Bevel => tiny_skia::LineJoin::Bevel,
            },
            ..Default::default()
        };
        self.pm.stroke_path(
            &p,
            &paint,
            &stroke,
            tiny_skia::Transform::identity(),
            self.mask.as_ref(),
        );
    }

    fn push_clip_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let (mut l, mut t, mut r, mut b) = (x, y, x + w, y + h);
        if let Some(Some(prev)) = self.clips.last() {
            l = l.max(prev.left());
            t = t.max(prev.top());
            r = r.min(prev.right());
            b = b.min(prev.bottom());
        }
        // 与父裁剪不相交(或父裁剪本来就是空的)→ 这一格是空裁剪
        let empty_parent = matches!(self.clips.last(), Some(None));
        let rect = if empty_parent {
            None
        } else {
            tiny_skia::Rect::from_ltrb(l, t, r, b)
        };
        self.clips.push(rect);
        self.rebuild_mask();
    }

    fn pop_clip(&mut self) {
        self.clips.pop();
        self.rebuild_mask();
    }
}

/// 一帧的像素统计
struct Shot {
    /// 非背景像素数
    ink: usize,
    /// 最靠左的**矩形填充色**像素列(蓝);圆是红→绿,不会混进来
    rect_left: u32,
}

fn rasterize(anim: &mut Lottie, frame: f64) -> Shot {
    let mut pm = tiny_skia::Pixmap::new(200, 100).unwrap();
    pm.fill(tiny_skia::Color::WHITE);
    {
        let mut sink = PixmapSink::new(&mut pm);
        let stats = anim.render(frame, Placement::IDENTITY, 1.0, &mut sink);
        assert!(!stats.degraded(), "像素路径也不该有降级:{stats:?}");
        assert!(sink.clips.is_empty(), "裁剪栈必须被排空:{:?}", sink.clips);
    }
    let mut ink = 0usize;
    let mut rect_left = u32::MAX;
    for (i, px) in pm.pixels().iter().enumerate() {
        if !(px.red() == 255 && px.green() == 255 && px.blue() == 255) {
            ink += 1;
        }
        // 固件里矩形的填充色是 (0.2, 0.4, 1) → 明显偏蓝;圆是红↔绿,黑描边三通道都低。
        // **必须按颜色挑**:只看"最左非背景列"会在末帧挑到**圆**的左边缘(85),
        // 而不是矩形(140),结论碰巧成立但机制不对
        if px.blue() > 180 && px.red() < 120 {
            rect_left = rect_left.min(i as u32 % 200);
        }
    }
    Shot { ink, rect_left }
}

#[test]
fn renders_visible_pixels_and_the_rect_really_moves() {
    let mut anim = load();
    let tl = anim.timeline();
    let first = rasterize(&mut anim, tl.frame_at_ms(0.0, false));
    let last = rasterize(&mut anim, tl.frame_at_ms(tl.duration_ms(), false));

    // 40×30 矩形 + 直径 30 的圆,200×100 画布上怎么也得上千个像素
    assert!(first.ink > 1000, "首帧非背景像素只有 {} 个", first.ink);
    assert!(last.ink > 1000, "末帧非背景像素只有 {} 个", last.ink);
    // 固件:矩形中心 40 → 160,宽 40 → 左边缘 20 → 140
    assert!(
        (18..=24).contains(&first.rect_left),
        "首帧矩形左边缘应当在 20 附近,实际 {}",
        first.rect_left
    );
    assert!(
        (138..=144).contains(&last.rect_left),
        "末帧矩形左边缘应当在 140 附近,实际 {}",
        last.rect_left
    );
}

#[test]
fn a_clip_that_the_sink_honours_really_removes_pixels() {
    // 上一条走的是"根裁剪 = 整幅画布"的恒等情形,裁不裁一个像素都不差 ——
    // 也就是说"PathSink 的裁剪真的被落地了"这件事从来没被像素验证过。
    // 这条用带矩形遮罩的固件(填充 200×100、遮罩只有左半边)把两条路分开
    let mut anim = Lottie::from_json_str(MASK_SUBTRACT).expect("合法 lottie");
    let ink = |pm: &tiny_skia::Pixmap| {
        pm.pixels()
            .iter()
            .filter(|px| !(px.red() == 255 && px.green() == 255 && px.blue() == 255))
            .count()
    };

    let mut clipped = tiny_skia::Pixmap::new(200, 100).unwrap();
    clipped.fill(tiny_skia::Color::WHITE);
    {
        let mut sink = PixmapSink::new(&mut clipped);
        anim.render(0.0, Placement::IDENTITY, 1.0, &mut sink);
        assert!(sink.clips.is_empty(), "裁剪栈必须被排空");
    }
    let with_clip = ink(&clipped);

    // 对照组:同一帧,但 sink 不实现裁剪(走 PathSink 的默认空实现)
    struct NoClip<'a>(PixmapSink<'a>);
    impl PathSink for NoClip<'_> {
        fn fill_path(&mut self, path: &[PathCmd], fill: PathFill, color: Color) {
            self.0.fill_path(path, fill, color);
        }
        fn stroke_path(&mut self, path: &[PathCmd], style: &StrokeStyle, color: Color) {
            self.0.stroke_path(path, style, color);
        }
        // push_clip_rect / pop_clip 故意不实现
    }
    let mut unclipped = tiny_skia::Pixmap::new(200, 100).unwrap();
    unclipped.fill(tiny_skia::Color::WHITE);
    {
        let mut sink = NoClip(PixmapSink::new(&mut unclipped));
        anim.render(0.0, Placement::IDENTITY, 1.0, &mut sink);
    }
    let without_clip = ink(&unclipped);

    assert_eq!(without_clip, 200 * 100, "不裁剪:整幅画布都被填满");
    assert!(
        (9000..=11000).contains(&with_clip),
        "遮罩裁到左半边 ≈ 100×100,实际 {with_clip} 像素(不裁是 {without_clip})"
    );
}
