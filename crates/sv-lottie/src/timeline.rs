//! 动画状态:时间轴换算 + 播放态。
//!
//! **这里只做算术,不碰帧循环。** 接进 sv-shell 的 `anim::pump` 是另一件事
//! (要新增一条 `Channel::Media`、要处理"每帧写但不 bump 版本",见
//! `docs/plans/lottie-2-architecture.md` §4),不归本 crate。
//! 本模块提供的是那件事需要的**纯函数底座**:给一个 wall-clock 毫秒,
//! 得到一个可以直接喂给 [`crate::Lottie::render`] 的帧号。

/// 从 `velato::Composition` 抄下来的时间轴事实
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timeline {
    /// 动画活跃区间起点(帧)
    pub start_frame: f64,
    /// 动画活跃区间终点(帧,**开区间**)
    pub end_frame: f64,
    /// 帧率
    pub frame_rate: f64,
}

impl Timeline {
    /// 活跃帧数(end - start)
    pub fn frame_span(&self) -> f64 {
        (self.end_frame - self.start_frame).max(0.0)
    }

    /// 总时长(ms)。帧率非法时返回 0
    pub fn duration_ms(&self) -> f64 {
        if !self.is_usable() {
            return 0.0;
        }
        self.frame_span() / self.frame_rate * 1000.0
    }

    fn is_usable(&self) -> bool {
        self.start_frame.is_finite()
            && self.end_frame.is_finite()
            && self.frame_rate.is_finite()
            && self.frame_rate > 0.0
            && self.frame_span() > 0.0
    }

    /// 按 wall-clock 毫秒求帧号。`looped=true` 环绕,`false` 钳制。
    ///
    /// **返回值恒落在 `[start_frame, end_frame)` —— 半开区间是 velato 的硬约束**:
    /// `Renderer::render_layer` 用 `layer.frames.contains(&frame)` 判图层活跃,
    /// 而 `Range::contains` 排除上界。返回正好等于 `end_frame` 的帧号会让
    /// **整帧一个图层都画不出来(空画面)** —— 这正好砸在 lottie 最该用的场景上
    /// (非循环动画定格在最后一帧的姿势)。
    ///
    /// 边界口径:
    /// - `t < 0`:环绕时按 `rem_euclid` 折回区间尾部(倒放/负延迟也有定义),
    ///   钳制时回到 `start_frame`;
    /// - `t > 时长`:环绕时取模(**不是清零** —— 掉帧不丢相位),钳制时定格末尾;
    /// - `t` 是 NaN/Inf,或时间轴不可用(帧率 ≤ 0、区间为空):返回 `start_frame`。
    pub fn frame_at_ms(&self, t_ms: f64, looped: bool) -> f64 {
        if !self.is_usable() {
            return self.start_frame;
        }
        let dur = self.duration_ms();
        let t = if !t_ms.is_finite() {
            0.0
        } else if looped {
            t_ms.rem_euclid(dur)
        } else {
            t_ms.clamp(0.0, dur)
        };
        let f = self.start_frame + t / 1000.0 * self.frame_rate;
        // 上界收紧到"严格小于 end_frame 的最大 f64",保住半开区间。
        //
        // **不能用 epsilon** —— 无论绝对量还是相对量都会破:相对量
        // `end_frame - frame_span * 1e-9` 在 `end_frame / frame_span > 9e6` 时
        // (例如 start=1e9, end=1e9+10)那个减量掉到 end_frame 的半个 ULP 以下,
        // 浮点加法把它原样吃掉,`last == end_frame`,不变量当场失效。
        // `next_down` 是这条不变量的**构造性**写法:它按定义就退一个 ULP
        let last = self.end_frame.next_down();
        f.clamp(self.start_frame, last.max(self.start_frame))
    }

    /// 进度 0..=1(钳制口径)
    pub fn progress_at_ms(&self, t_ms: f64) -> f32 {
        let dur = self.duration_ms();
        if dur <= 0.0 || !t_ms.is_finite() {
            return 0.0;
        }
        (t_ms / dur).clamp(0.0, 1.0) as f32
    }
}

/// 最小播放态。**真源是 `time_ms`**,帧号是它的纯函数。
///
/// 用法(未来接帧循环时):每帧算出 `dt`,调 [`Playback::advance`],
/// 再用 [`Playback::frame`] 取帧号喂给渲染。`advance` 的返回值就是
/// "还要不要继续续帧" —— 与 sv-shell `anim::pump` 的返回值同语义。
#[derive(Clone, Copy, Debug)]
pub struct Playback {
    pub timeline: Timeline,
    /// 已播放时长(ms;循环时恒被折回 `[0, duration)`)
    pub time_ms: f64,
    /// 播放速度倍率。负数即倒放
    pub speed: f32,
    pub looped: bool,
    pub playing: bool,
}

impl Playback {
    /// 从 0 开始播,循环,1x
    pub fn new(timeline: Timeline) -> Self {
        Self {
            timeline,
            time_ms: 0.0,
            speed: 1.0,
            looped: true,
            playing: true,
        }
    }

    pub fn looped(mut self, looped: bool) -> Self {
        self.looped = looped;
        self
    }

    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// 推进 `dt_ms`(已经是两帧之间的真实墙钟差)。
    ///
    /// 返回**推进后是否仍在播放** —— 调用方拿它决定要不要 `request_redraw`。
    /// 非循环动画走到终点会停在 `duration_ms`(而不是 0),这样
    /// [`Self::frame`] 给出的是"最后一帧的姿势",符合"成功勾选定格"的用法。
    pub fn advance(&mut self, dt_ms: f64) -> bool {
        if !self.playing {
            return false;
        }
        let dur = self.timeline.duration_ms();
        if dur <= 0.0 {
            self.playing = false;
            return false;
        }
        let dt = if dt_ms.is_finite() { dt_ms } else { 0.0 };
        let t = self.time_ms + dt * self.speed as f64;
        if self.looped {
            // 取模而不是清零:掉一帧(比如窗口被拖动了 200ms)之后相位仍然对
            self.time_ms = t.rem_euclid(dur);
            true
        } else if t >= dur {
            self.time_ms = dur;
            self.playing = false;
            false
        } else if t <= 0.0 {
            // 倒放到头同样是"结束"
            self.time_ms = 0.0;
            self.playing = self.speed >= 0.0;
            self.playing
        } else {
            self.time_ms = t;
            true
        }
    }

    /// 当前帧号(可直接喂给 [`crate::Lottie::render`])
    pub fn frame(&self) -> f64 {
        self.timeline.frame_at_ms(self.time_ms, self.looped)
    }

    /// 跳到指定时刻(用户拖动进度条)
    pub fn seek_ms(&mut self, t_ms: f64) {
        let dur = self.timeline.duration_ms();
        self.time_ms = if !t_ms.is_finite() {
            0.0
        } else if self.looped && dur > 0.0 {
            t_ms.rem_euclid(dur)
        } else {
            t_ms.clamp(0.0, dur)
        };
    }

    /// 进度 0..=1
    pub fn progress(&self) -> f32 {
        self.timeline.progress_at_ms(self.time_ms)
    }

    /// 非循环动画是否已走完
    pub fn finished(&self) -> bool {
        !self.looped && !self.playing
    }
}
