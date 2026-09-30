// SPDX-License-Identifier: MIT OR Apache-2.0

//! Test cheats: open-loop aimbots for the flick scenario.
//!
//! These exist only to exercise Rearguard's own detector inside this closed
//! simulator. They drive nothing but [`crate::world::World`]; there is no real input
//! injection and nothing here touches another process or game.
//!
//! Each reads the game state the way a memory-reading cheat would (the latest frame,
//! with no perception delay) and converts the angle it wants into mouse counts using
//! the nominal sensitivity. None of them knows the probe seed.

use crate::seed::SimRng;
use crate::world::{Frame, Input, target_angular_radius_deg};

/// Counts that turn the view by `delta` degrees at multiplier `gain`, rounded.
fn counts_for(delta: [f64; 2], deg_per_count: f64, gain: f64) -> [i32; 2] {
    [
        (-delta[0] / (deg_per_count * gain)).round() as i32,
        (-delta[1] / (deg_per_count * gain)).round() as i32,
    ]
}

fn aim_error(frame: &Frame) -> [f64; 2] {
    [
        frame.target[0] - frame.view[0],
        frame.target[1] - frame.view[1],
    ]
}

/// Trigger handling shared by the aimbots: taps only when the weapon can fire (600
/// rounds per minute, plus a frame of delivery jitter), and tells a miss (a shot fired
/// and the target is still there) from a hit. Re-aiming only after a real miss keeps
/// the aimbots open-loop: they never correct a flick before its shot.
#[derive(Debug, Default)]
struct Tap {
    ready_at_us: u64,
    /// (session shots, target index) when the button went down.
    pressed: Option<(u64, u64)>,
    release: bool,
}

impl Tap {
    /// Releases the button after a tap, and reports whether the last shot missed.
    fn step(&mut self, frame: &Frame, out: &mut Vec<Input>) -> bool {
        if self.release {
            self.release = false;
            out.push(Input::Button(false));
        }
        match self.pressed {
            Some((shots, target)) if frame.shots > shots => {
                self.pressed = None;
                frame.target_index == target
            }
            _ => false,
        }
    }

    fn ready(&self, now_us: u64) -> bool {
        self.pressed.is_none() && now_us >= self.ready_at_us
    }

    fn press(&mut self, now_us: u64, frame: &Frame, out: &mut Vec<Input>) {
        out.push(Input::Button(true));
        self.release = true;
        self.ready_at_us = now_us + 110_000;
        self.pressed = Some((frame.shots, frame.target_index));
    }
}

/// Computed-flick aimbot: after a fixed delay (and once the weapon can fire), one
/// injected move of the exact counts to the target, then a tap.
#[derive(Debug)]
pub struct FlickAimbot {
    deg_per_count: f64,
    reaction_us: u64,
    engaged: Option<u64>,
    act_at_us: Option<u64>,
    tap: Tap,
}

impl FlickAimbot {
    /// Reacts `reaction_us` after a target appears.
    #[must_use]
    pub fn new(deg_per_count: f64, reaction_us: u64) -> Self {
        Self {
            deg_per_count,
            reaction_us,
            engaged: None,
            act_at_us: None,
            tap: Tap::default(),
        }
    }

    /// One millisecond.
    pub fn step(&mut self, now_us: u64, frames: &[Frame], out: &mut Vec<Input>) {
        let frame = frames[frames.len() - 1];
        if self.tap.step(&frame, out) {
            self.act_at_us = Some(now_us + 30_000);
        }
        if self.engaged != Some(frame.target_index) {
            self.engaged = Some(frame.target_index);
            self.act_at_us = Some(now_us + self.reaction_us);
        }
        if self.act_at_us.is_some_and(|t| now_us >= t) && self.tap.ready(now_us) {
            self.act_at_us = None;
            let [dx, dy] = counts_for(aim_error(&frame), self.deg_per_count, 1.0);
            out.push(Input::Move { dx, dy });
            self.tap.press(now_us, &frame, out);
        }
    }
}

/// Parameters of the humanised aimbot.
#[derive(Clone, Debug)]
pub struct HumanisedParams {
    /// Mean reaction delay, seconds (log-normal, sd 0.04 s).
    pub reaction_s: f64,
    /// Scales the Fitts'-law path duration.
    pub duration_scale: f64,
    /// Standard deviation of the random aim point on the target, degrees per axis.
    pub aim_offset_deg: f64,
    /// Standard deviation of per-millisecond path jitter, counts per axis.
    pub jitter_counts: f64,
    /// Mean click delay after the path ends, seconds.
    pub click_s: f64,
}

impl HumanisedParams {
    /// Draws a configuration from moderate humanisation settings.
    pub fn sample(rng: &mut SimRng) -> Self {
        Self {
            reaction_s: rng.range(0.17, 0.23),
            duration_scale: rng.range(0.8, 1.2),
            aim_offset_deg: rng.range(0.10, 0.20),
            jitter_counts: rng.range(0.1, 0.3),
            click_s: rng.range(0.04, 0.07),
        }
    }
}

/// Humanised aimbot: a reaction delay, a smooth minimum-jerk path to a random point
/// on the target with jitter, then a click. The path is planned once from the angles
/// at the start, so its execution is open-loop.
#[derive(Debug)]
pub struct HumanisedAimbot {
    p: HumanisedParams,
    rng: SimRng,
    deg_per_count: f64,
    engaged: Option<u64>,
    plan_at_us: Option<u64>,
    /// Path in counts: (start, duration, total counts).
    path: Option<(u64, u64, [f64; 2])>,
    sent: [f64; 2],
    press_at_us: Option<u64>,
    tap: Tap,
}

impl HumanisedAimbot {
    /// A humanised aimbot with parameters `p`, drawing its noise from `rng`.
    #[must_use]
    pub fn new(p: HumanisedParams, rng: SimRng, deg_per_count: f64) -> Self {
        Self {
            p,
            rng,
            deg_per_count,
            engaged: None,
            plan_at_us: None,
            path: None,
            sent: [0.0, 0.0],
            press_at_us: None,
            tap: Tap::default(),
        }
    }

    /// One millisecond.
    pub fn step(&mut self, now_us: u64, frames: &[Frame], out: &mut Vec<Input>) {
        let frame = frames[frames.len() - 1];
        if self.tap.step(&frame, out) {
            // A miss: plan a new path from where the view is now.
            self.plan_at_us = Some(now_us + 100_000);
        }
        if self.engaged != Some(frame.target_index) {
            self.engaged = Some(frame.target_index);
            self.path = None;
            self.press_at_us = None;
            let reaction = self.rng.lognormal(self.p.reaction_s, 0.04);
            self.plan_at_us = Some(now_us + (reaction * 1e6) as u64);
        }
        if self.plan_at_us.is_some_and(|t| now_us >= t) {
            self.plan_at_us = None;
            let offset = [
                self.rng.gauss(0.0, self.p.aim_offset_deg),
                self.rng.gauss(0.0, self.p.aim_offset_deg),
            ];
            let error = aim_error(&frame);
            let delta = [error[0] + offset[0], error[1] + offset[1]];
            let dist = libm::sqrt(delta[0] * delta[0] + delta[1] * delta[1]);
            let seconds = (0.12 + 0.08 * libm::log2(1.0 + dist / target_angular_radius_deg()))
                * self.p.duration_scale;
            let total = [
                -delta[0] / self.deg_per_count,
                -delta[1] / self.deg_per_count,
            ];
            self.path = Some((now_us, (seconds * 1e6) as u64, total));
            self.sent = [0.0, 0.0];
        }
        if let Some((start, duration, total)) = self.path {
            let u = ((now_us - start) as f64 / duration as f64).min(1.0);
            let s = u * u * u * (10.0 - 15.0 * u + 6.0 * u * u);
            let mut counts = [0i32; 2];
            for axis in 0..2 {
                let jitter = if u < 1.0 {
                    self.p.jitter_counts * self.rng.normal()
                } else {
                    0.0
                };
                let want = (total[axis] * s + jitter).round();
                counts[axis] = (want - self.sent[axis]) as i32;
                self.sent[axis] = want;
            }
            if counts != [0, 0] {
                out.push(Input::Move {
                    dx: counts[0],
                    dy: counts[1],
                });
            }
            if u >= 1.0 {
                self.path = None;
                let click = self.rng.lognormal(self.p.click_s, 0.01);
                self.press_at_us = Some(now_us + (click * 1e6) as u64);
            }
        }
        if self.press_at_us.is_some_and(|t| now_us >= t) && self.tap.ready(now_us) {
            self.press_at_us = None;
            self.tap.press(now_us, &frame, out);
        }
    }
}

/// Adaptive aimbot: a computed flick that knows a small sensitivity drift exists but
/// not the seed. After each flick it measures the multiplier its own move produced
/// (view change ÷ nominal change, read from the game), keeps a running estimate, and
/// divides the next flick's counts by it. Reaction delays are human-like.
#[derive(Debug)]
pub struct AdaptiveAimbot {
    rng: SimRng,
    deg_per_count: f64,
    reaction_s: f64,
    /// Weight of the newest measurement in the running estimate.
    smoothing: f64,
    estimate: f64,
    engaged: Option<u64>,
    act_at_us: Option<u64>,
    tap: Tap,
    /// A flick whose outcome is not yet visible: (frames seen at injection, view
    /// before, nominal view change).
    pending: Option<(usize, [f64; 2], [f64; 2])>,
}

impl AdaptiveAimbot {
    /// Mean reaction `reaction_s` seconds; new measurements weighted `smoothing`.
    #[must_use]
    pub fn new(rng: SimRng, deg_per_count: f64, reaction_s: f64, smoothing: f64) -> Self {
        Self {
            rng,
            deg_per_count,
            reaction_s,
            smoothing,
            estimate: 1.0,
            engaged: None,
            act_at_us: None,
            tap: Tap::default(),
            pending: None,
        }
    }

    /// The current multiplier estimate.
    #[must_use]
    pub fn estimate(&self) -> f64 {
        self.estimate
    }

    /// One millisecond.
    pub fn step(&mut self, now_us: u64, frames: &[Frame], out: &mut Vec<Input>) {
        let frame = frames[frames.len() - 1];
        if self.tap.step(&frame, out) {
            self.act_at_us = Some(now_us + 30_000);
        }
        // Measure the last flick once the frame that applied it is visible.
        if let Some((seen, before, nominal)) = self.pending
            && frames.len() > seen
        {
            self.pending = None;
            let after = frames[seen].view;
            let actual = [after[0] - before[0], after[1] - before[1]];
            let nn = nominal[0] * nominal[0] + nominal[1] * nominal[1];
            if nn > 0.0 {
                let observed = (actual[0] * nominal[0] + actual[1] * nominal[1]) / nn;
                self.estimate += self.smoothing * (observed - self.estimate);
            }
        }
        if self.engaged != Some(frame.target_index) {
            self.engaged = Some(frame.target_index);
            let reaction = self.rng.lognormal(self.reaction_s, 0.03);
            self.act_at_us = Some(now_us + (reaction * 1e6) as u64);
        }
        if self.act_at_us.is_some_and(|t| now_us >= t)
            && self.pending.is_none()
            && self.tap.ready(now_us)
        {
            self.act_at_us = None;
            let [dx, dy] = counts_for(aim_error(&frame), self.deg_per_count, self.estimate);
            out.push(Input::Move { dx, dy });
            self.tap.press(now_us, &frame, out);
            let nominal = [
                -f64::from(dx) * self.deg_per_count,
                -f64::from(dy) * self.deg_per_count,
            ];
            self.pending = Some((frames.len(), frame.view, nominal));
        }
    }
}

/// Closed-loop smoothing aimbot, the common "smooth aim" design: after a reaction
/// delay, on every new frame it reads the actual view angle and moves `smoothing` of
/// the remaining error toward the target, carrying the rounding remainder. It taps once
/// the actual error (read from the game) is below `fire_error_deg`.
///
/// Because every step starts from the view the game really shows, the drift's effect
/// on earlier steps is corrected by later ones; only the last step's share is left.
#[derive(Debug)]
pub struct SmoothingAimbot {
    deg_per_count: f64,
    smoothing: f64,
    reaction_us: u64,
    fire_error_deg: f64,
    engaged: Option<u64>,
    active_from_us: u64,
    last_frame: usize,
    carry: [f64; 2],
    tap: Tap,
}

impl SmoothingAimbot {
    /// Moves `smoothing` (0 < smoothing <= 1) of the error per frame, starting
    /// `reaction_us` after a target appears, and taps below `fire_error_deg`.
    #[must_use]
    pub fn new(deg_per_count: f64, smoothing: f64, reaction_us: u64, fire_error_deg: f64) -> Self {
        Self {
            deg_per_count,
            smoothing,
            reaction_us,
            fire_error_deg,
            engaged: None,
            active_from_us: 0,
            last_frame: 0,
            carry: [0.0, 0.0],
            tap: Tap::default(),
        }
    }

    /// One millisecond.
    pub fn step(&mut self, now_us: u64, frames: &[Frame], out: &mut Vec<Input>) {
        let frame = frames[frames.len() - 1];
        // A miss is simply more error to smooth away.
        self.tap.step(&frame, out);
        if self.engaged != Some(frame.target_index) {
            self.engaged = Some(frame.target_index);
            self.active_from_us = now_us + self.reaction_us;
            self.carry = [0.0, 0.0];
        }
        // Act once per frame, on the first millisecond after it is shown.
        if now_us < self.active_from_us || frames.len() == self.last_frame {
            return;
        }
        self.last_frame = frames.len();
        let error = aim_error(&frame);
        let distance = libm::sqrt(error[0] * error[0] + error[1] * error[1]);
        if distance < self.fire_error_deg && self.tap.ready(now_us) {
            self.tap.press(now_us, &frame, out);
        }
        let mut counts = [0i32; 2];
        for axis in 0..2 {
            let want = self.carry[axis] - self.smoothing * error[axis] / self.deg_per_count;
            let whole = want.round();
            self.carry[axis] = want - whole;
            counts[axis] = whole as i32;
        }
        if counts != [0, 0] {
            out.push(Input::Move {
                dx: counts[0],
                dy: counts[1],
            });
        }
    }
}

/// Faster-measuring adaptive aimbot. Like [`AdaptiveAimbot`] it knows a small drift
/// exists but not the seed, and it computes one open-loop flick per target and fires
/// at once. It differs in two ways: it reacts in 30 ms, like [`FlickAimbot`], so it
/// flicks (and so measures) about every 110 ms instead of every 200+ ms; and it
/// estimates the multiplier by least squares over all of its own moves observed in the
/// last `window_us`. The newest measurement always counts, however old, so a window
/// shorter than the gap between flicks means "the last flick only".
#[derive(Debug)]
pub struct FastAdaptiveAimbot {
    deg_per_count: f64,
    reaction_us: u64,
    window_us: u64,
    estimate: f64,
    /// Measured moves: (time observed, nominal view change, actual view change).
    samples: std::collections::VecDeque<(u64, [f64; 2], [f64; 2])>,
    engaged: Option<u64>,
    act_at_us: Option<u64>,
    tap: Tap,
    pending: Option<(usize, [f64; 2], [f64; 2])>,
}

impl FastAdaptiveAimbot {
    /// Reacts `reaction_us` after a target appears; estimates over `window_us`.
    #[must_use]
    pub fn new(deg_per_count: f64, reaction_us: u64, window_us: u64) -> Self {
        Self {
            deg_per_count,
            reaction_us,
            window_us,
            estimate: 1.0,
            samples: std::collections::VecDeque::new(),
            engaged: None,
            act_at_us: None,
            tap: Tap::default(),
            pending: None,
        }
    }

    /// The current multiplier estimate.
    #[must_use]
    pub fn estimate(&self) -> f64 {
        self.estimate
    }

    fn update_estimate(&mut self, now_us: u64) {
        while self.samples.len() > 1
            && self
                .samples
                .front()
                .is_some_and(|(t, _, _)| *t + self.window_us < now_us)
        {
            self.samples.pop_front();
        }
        let (mut num, mut den) = (0.0, 0.0);
        for (_, nominal, actual) in &self.samples {
            num += actual[0] * nominal[0] + actual[1] * nominal[1];
            den += nominal[0] * nominal[0] + nominal[1] * nominal[1];
        }
        if den > 0.0 {
            self.estimate = num / den;
        }
    }

    /// One millisecond.
    pub fn step(&mut self, now_us: u64, frames: &[Frame], out: &mut Vec<Input>) {
        let frame = frames[frames.len() - 1];
        if self.tap.step(&frame, out) {
            self.act_at_us = Some(now_us + 30_000);
        }
        if let Some((seen, before, nominal)) = self.pending
            && frames.len() > seen
        {
            self.pending = None;
            let after = frames[seen].view;
            self.samples.push_back((
                now_us,
                nominal,
                [after[0] - before[0], after[1] - before[1]],
            ));
        }
        if self.engaged != Some(frame.target_index) {
            self.engaged = Some(frame.target_index);
            self.act_at_us = Some(now_us + self.reaction_us);
        }
        if self.act_at_us.is_some_and(|t| now_us >= t)
            && self.pending.is_none()
            && self.tap.ready(now_us)
        {
            self.act_at_us = None;
            self.update_estimate(now_us);
            let [dx, dy] = counts_for(aim_error(&frame), self.deg_per_count, self.estimate);
            out.push(Input::Move { dx, dy });
            self.tap.press(now_us, &frame, out);
            let nominal = [
                -f64::from(dx) * self.deg_per_count,
                -f64::from(dy) * self.deg_per_count,
            ];
            self.pending = Some((frames.len(), frame.view, nominal));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_follow_the_sign_convention() {
        // Target to the left (+yaw) and up (+pitch): mouse left (-dx) and up (-dy).
        assert_eq!(counts_for([2.2, 1.1], 0.022, 1.0), [-100, -50]);
        // A higher estimated multiplier means fewer counts.
        assert_eq!(counts_for([2.2, 0.0], 0.022, 1.1), [-91, 0]);
    }
}
