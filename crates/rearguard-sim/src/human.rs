//! Synthetic closed-loop human aim model, and the same player using a recoil macro.
//!
//! **This is a model, not a human.** Its structure follows standard accounts of
//! aimed movement (a planned primary submovement, signal-dependent motor noise,
//! delayed visual feedback), but none of its parameters are fitted to real players.
//! Validation against real players happens in task 1.10.
//!
//! Every millisecond (a 1000 Hz mouse):
//!
//! 1. **Perception.** The player sees the frame shown `visual_delay` ago, with visual
//!    noise that grows with eccentricity.
//! 2. **Efference copy.** The player knows what it has commanded since that frame
//!    (at the *nominal* sensitivity; it cannot know the secret drift), and subtracts
//!    it: `estimated error = perceived error - own recent commands` (a Smith
//!    predictor). Anything it did not command, such as drift, motor noise or recoil,
//!    shows up only after the visual delay, and is then corrected.
//! 3. **Control.** When a new target is noticed, after a reaction time, the player
//!    plans a minimum-jerk primary movement towards it, with its duration set by
//!    Fitts' law and noise in amplitude and direction. Throughout, a feedback term
//!    `gain × (estimated error - rest of the plan)` corrects what the plan will miss.
//! 4. **Motor noise.** The executed velocity is the command plus noise proportional
//!    to speed, plus a small tremor. The hand's motion becomes whole mouse counts.
//! 5. **Firing.** Flick: once the primary movement is done and the estimated error is
//!    within the player's tolerance, click after a click latency. Spray: press when on
//!    target, pull down against the learned recoil pattern (with noise) while feedback
//!    keeps correcting, and release when the magazine is seen to be empty.

use std::collections::VecDeque;

use crate::seed::SimRng;
use crate::world::{Frame, Input, RECOIL_PATTERN_V1, Scenario, target_angular_radius_deg};

const DT: f64 = 0.001;
/// Fire interval the player expects: 600 rounds per minute.
const SHOT_INTERVAL_US: u64 = 100_000;

/// One player's parameters, drawn per session from population ranges.
#[derive(Clone, Debug)]
pub struct HumanParams {
    /// Mean reaction time to a new target, seconds.
    pub reaction_s: f64,
    /// Visual feedback delay, seconds.
    pub visual_delay_s: f64,
    /// Feedback gain, 1/s.
    pub feedback_gain: f64,
    /// Primary movement amplitude as a fraction of the perceived error (undershoot).
    pub primary_gain: f64,
    /// Standard deviation of primary amplitude error, as a fraction of the amplitude.
    pub amplitude_noise: f64,
    /// Standard deviation of primary sideways error, as a fraction of the amplitude.
    pub direction_noise: f64,
    /// Standard deviation of speed-proportional velocity noise (fraction of speed).
    pub velocity_noise: f64,
    /// Standard deviation of tremor, degrees per second.
    pub tremor_deg_s: f64,
    /// Visual noise: fraction of eccentricity plus a floor in degrees.
    pub visual_noise_frac: f64,
    /// Visual noise floor, degrees.
    pub visual_noise_deg: f64,
    /// Fires when the estimated error is below this fraction of the target radius.
    pub tolerance_frac: f64,
    /// Mean click latency, seconds.
    pub click_latency_s: f64,
    /// Learned recoil compensation gain (1 = the exact pattern).
    pub pattern_gain: f64,
    /// Per-shot noise of the learned compensation, as a fraction of the kick.
    pub pattern_noise: f64,
}

impl HumanParams {
    /// Draws a player from the population ranges.
    pub fn sample(rng: &mut SimRng) -> Self {
        Self {
            reaction_s: rng.range(0.20, 0.26),
            visual_delay_s: rng.range(0.10, 0.14),
            feedback_gain: rng.range(6.0, 10.0),
            primary_gain: rng.range(0.90, 0.97),
            amplitude_noise: rng.range(0.05, 0.09),
            direction_noise: rng.range(0.02, 0.04),
            velocity_noise: rng.range(0.04, 0.08),
            tremor_deg_s: rng.range(0.1, 0.3),
            visual_noise_frac: rng.range(0.01, 0.03),
            visual_noise_deg: rng.range(0.02, 0.05),
            tolerance_frac: rng.range(0.35, 0.55),
            click_latency_s: rng.range(0.05, 0.08),
            pattern_gain: rng.gauss(1.0, 0.08),
            pattern_noise: rng.range(0.10, 0.20),
        }
    }
}

/// A minimum-jerk movement of `displacement` degrees over `duration_us`.
#[derive(Clone, Copy, Debug)]
struct Submove {
    start_us: u64,
    duration_us: u64,
    displacement: [f64; 2],
}

impl Submove {
    fn phase(&self, now_us: u64) -> f64 {
        (now_us.saturating_sub(self.start_us) as f64 / self.duration_us as f64).clamp(0.0, 1.0)
    }

    fn done(&self, now_us: u64) -> bool {
        now_us >= self.start_us + self.duration_us
    }

    /// Velocity, degrees per second.
    fn velocity(&self, now_us: u64) -> [f64; 2] {
        if now_us < self.start_us || self.done(now_us) {
            return [0.0, 0.0];
        }
        let u = self.phase(now_us);
        let speed = 30.0 * u * u * (1.0 - u) * (1.0 - u) / (self.duration_us as f64 * 1e-6);
        [self.displacement[0] * speed, self.displacement[1] * speed]
    }

    /// Displacement still to come.
    fn remaining(&self, now_us: u64) -> [f64; 2] {
        let u = self.phase(now_us);
        let s = u * u * u * (10.0 - 15.0 * u + 6.0 * u * u);
        [
            self.displacement[0] * (1.0 - s),
            self.displacement[1] * (1.0 - s),
        ]
    }
}

/// A scalar Ornstein–Uhlenbeck process with unit standard deviation.
#[derive(Clone, Copy, Debug, Default)]
struct Ou(f64);

impl Ou {
    fn step(&mut self, rng: &mut SimRng, tau_s: f64) {
        let a = DT / tau_s;
        self.0 += -self.0 * a + libm::sqrt(2.0 * a) * rng.normal();
    }
}

/// How the player handles recoil during a spray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoilControl {
    /// Pulls against the learned pattern by hand, with noise, and keeps correcting
    /// by sight.
    Learned,
    /// A fixed-pattern macro injects the exact nominal counts for every shot; the
    /// player holds still for the burst and trusts it.
    Macro,
}

/// The simulated player.
#[derive(Debug)]
pub struct Human {
    p: HumanParams,
    rng: SimRng,
    deg_per_count: f64,
    recoil: RecoilControl,
    tolerance_deg: f64,
    delay_us: u64,
    // Target engagement.
    engaged: Option<u64>,
    plan_at_us: Option<u64>,
    primary: Option<Submove>,
    // Efference copy of everything commanded since the frame the player last saw:
    // (generation time, commanded view change in that millisecond, including
    // disturbances the player expects), and its running sum.
    unseen: VecDeque<(u64, [f64; 2])>,
    unseen_sum: [f64; 2],
    hand: [f64; 2],
    velocity_noise: [Ou; 2],
    tremor: [Ou; 2],
    visual_noise: [Ou; 2],
    // Trigger.
    trigger: bool,
    press_at_us: Option<u64>,
    release_at_us: Option<u64>,
    refractory_until_us: u64,
    // Spray.
    press_time_us: u64,
    pulls: Vec<Submove>,
    expected_kicks: Vec<(u64, [f64; 2])>,
    macro_moves: VecDeque<(u64, [i32; 2])>,
}

impl Human {
    /// A player with parameters `p`, drawing its noise from `rng`.
    #[must_use]
    pub fn new(p: HumanParams, rng: SimRng, deg_per_count: f64, recoil: RecoilControl) -> Self {
        let tolerance_deg = p.tolerance_frac * target_angular_radius_deg();
        let delay_us = (p.visual_delay_s * 1e6) as u64;
        Self {
            p,
            rng,
            deg_per_count,
            recoil,
            tolerance_deg,
            delay_us,
            engaged: None,
            plan_at_us: None,
            primary: None,
            unseen: VecDeque::new(),
            unseen_sum: [0.0, 0.0],
            hand: [0.0, 0.0],
            velocity_noise: [Ou::default(); 2],
            tremor: [Ou::default(); 2],
            visual_noise: [Ou::default(); 2],
            trigger: false,
            press_at_us: None,
            release_at_us: None,
            refractory_until_us: 0,
            press_time_us: 0,
            pulls: Vec::new(),
            expected_kicks: Vec::new(),
            macro_moves: VecDeque::new(),
        }
    }

    /// One millisecond of the player. `frames` are those shown up to `now_us`.
    pub fn step(
        &mut self,
        now_us: u64,
        frames: &[Frame],
        scenario: Scenario,
        out: &mut Vec<Input>,
    ) {
        let seen_at = now_us.saturating_sub(self.delay_us);
        let seen = frames
            .iter()
            .rev()
            .find(|f| f.t_us <= seen_at)
            .unwrap_or(&frames[0]);
        for ou in self.velocity_noise.iter_mut().chain(&mut self.tremor) {
            ou.step(&mut self.rng, 0.05);
        }
        for ou in &mut self.visual_noise {
            ou.step(&mut self.rng, 0.05);
        }

        // A new target: stop, then plan after the rest of the reaction time.
        if self.engaged != Some(seen.target_index) {
            self.engaged = Some(seen.target_index);
            self.primary = None;
            let reaction = self.rng.lognormal(self.p.reaction_s, 0.03);
            let decide = (reaction - self.p.visual_delay_s).max(0.03);
            self.plan_at_us = Some(now_us + (decide * 1e6) as u64);
        }

        // Perceived and estimated error.
        let perceived = [seen.target[0] - seen.view[0], seen.target[1] - seen.view[1]];
        let sd = self.p.visual_noise_frac * norm(perceived) + self.p.visual_noise_deg;
        let mut estimate = [
            perceived[0] + sd * self.visual_noise[0].0,
            perceived[1] + sd * self.visual_noise[1].0,
        ];
        // Commands up to the seen frame are now part of what the player sees.
        while self.unseen.front().is_some_and(|(t, _)| *t <= seen.t_us) {
            if let Some((_, d)) = self.unseen.pop_front() {
                self.unseen_sum[0] -= d[0];
                self.unseen_sum[1] -= d[1];
            }
        }
        if self.unseen.is_empty() {
            // Reset exactly, so rounding in the running sum never accumulates.
            self.unseen_sum = [0.0, 0.0];
        }
        estimate[0] -= self.unseen_sum[0];
        estimate[1] -= self.unseen_sum[1];

        if self.plan_at_us.is_some_and(|t| now_us >= t) {
            self.plan_at_us = None;
            self.primary = Some(self.plan_primary(now_us, estimate));
        }

        // Command: primary plan + feedback on what the plan will miss + recoil pull.
        let holding_for_macro = self.recoil == RecoilControl::Macro && self.trigger;
        let mut command = [0.0, 0.0];
        if let Some(primary) = self.primary
            && !holding_for_macro
        {
            let v = primary.velocity(now_us);
            let rest = primary.remaining(now_us);
            let k = self.p.feedback_gain;
            let mut fb = [k * (estimate[0] - rest[0]), k * (estimate[1] - rest[1])];
            let speed = norm(fb);
            if speed > 300.0 {
                fb = [fb[0] * 300.0 / speed, fb[1] * 300.0 / speed];
            }
            command = [v[0] + fb[0], v[1] + fb[1]];
        }
        for pull in &self.pulls {
            let v = pull.velocity(now_us);
            command[0] += v[0];
            command[1] += v[1];
        }

        // Efference copy of this millisecond, plus any recoil kick the player expects.
        let mut expected = [command[0] * DT, command[1] * DT];
        for (t, kick) in &self.expected_kicks {
            if *t == now_us {
                expected[0] += kick[0];
                expected[1] += kick[1];
            }
        }
        self.unseen.push_back((now_us, expected));
        self.unseen_sum[0] += expected[0];
        self.unseen_sum[1] += expected[1];

        // Execution with motor noise, quantised to whole counts.
        let speed = norm(command);
        let mut counts = [0i32; 2];
        for axis in 0..2 {
            let v = command[axis]
                + speed * self.p.velocity_noise * self.velocity_noise[axis].0
                + self.p.tremor_deg_s * self.tremor[axis].0;
            // Mouse right turns right (yaw down); mouse down looks down.
            self.hand[axis] += -v * DT / self.deg_per_count;
            let whole = self.hand[axis].trunc();
            self.hand[axis] -= whole;
            counts[axis] = whole as i32;
        }
        if counts != [0, 0] {
            out.push(Input::Move {
                dx: counts[0],
                dy: counts[1],
            });
        }
        while self.macro_moves.front().is_some_and(|(t, _)| *t <= now_us) {
            let (_, [dx, dy]) = self.macro_moves.pop_front().unwrap_or_default();
            out.push(Input::Move { dx, dy });
        }

        self.trigger_logic(now_us, seen, scenario, estimate, out);
    }

    fn plan_primary(&mut self, now_us: u64, estimate: [f64; 2]) -> Submove {
        let dist = norm(estimate);
        let (along, across) = if dist > 0.0 {
            (
                [estimate[0] / dist, estimate[1] / dist],
                [-estimate[1] / dist, estimate[0] / dist],
            )
        } else {
            ([1.0, 0.0], [0.0, 1.0])
        };
        let a = dist * (self.p.primary_gain + self.p.amplitude_noise * self.rng.normal());
        let s = dist * self.p.direction_noise * self.rng.normal();
        let fitts = 0.10 + 0.07 * libm::log2(1.0 + dist / target_angular_radius_deg());
        let duration = fitts * self.rng.lognormal(1.0, 0.1);
        Submove {
            start_us: now_us,
            duration_us: ((duration * 1e6) as u64).max(20_000),
            displacement: [along[0] * a + across[0] * s, along[1] * a + across[1] * s],
        }
    }

    fn trigger_logic(
        &mut self,
        now_us: u64,
        seen: &Frame,
        scenario: Scenario,
        estimate: [f64; 2],
        out: &mut Vec<Input>,
    ) {
        if self.press_at_us.is_some_and(|t| now_us >= t) {
            self.press_at_us = None;
            self.trigger = true;
            out.push(Input::Button(true));
            match scenario {
                Scenario::Flick => {
                    // A tap: released before the automatic weapon's next shot
                    // (100 ms), even allowing for a frame of delivery jitter.
                    let hold = self.rng.gauss(0.06, 0.015).clamp(0.03, 0.085);
                    self.release_at_us = Some(now_us + (hold * 1e6) as u64);
                }
                Scenario::Spray => self.start_spray(now_us),
            }
            return;
        }
        if self.trigger {
            let release = match scenario {
                Scenario::Flick => self.release_at_us.is_some_and(|t| now_us >= t),
                // Release once the empty magazine is seen, after a short reaction.
                Scenario::Spray => {
                    if seen.magazine_empty && self.release_at_us.is_none() {
                        let r = self.rng.gauss(0.10, 0.03).max(0.03);
                        self.release_at_us = Some(now_us + (r * 1e6) as u64);
                    }
                    self.release_at_us.is_some_and(|t| now_us >= t)
                }
            };
            if release {
                self.trigger = false;
                self.release_at_us = None;
                self.refractory_until_us = now_us + 150_000;
                self.pulls.clear();
                self.expected_kicks.clear();
                out.push(Input::Button(false));
            }
            return;
        }
        let ready = self.primary.is_some_and(|p| p.done(now_us))
            && self.press_at_us.is_none()
            && now_us >= self.refractory_until_us
            && norm(estimate) < self.tolerance_deg;
        if ready {
            let latency = self.rng.lognormal(self.p.click_latency_s, 0.015);
            self.press_at_us = Some(now_us + (latency * 1e6) as u64);
        }
    }

    fn start_spray(&mut self, now_us: u64) {
        self.press_time_us = now_us;
        self.pulls.clear();
        self.expected_kicks.clear();
        for (k, kick) in RECOIL_PATTERN_V1.iter().enumerate() {
            let shot_us = now_us + k as u64 * SHOT_INTERVAL_US;
            match self.recoil {
                RecoilControl::Learned => {
                    let g = self.p.pattern_gain * (1.0 + self.p.pattern_noise * self.rng.normal());
                    let start = shot_us + (self.rng.gauss(0.03, 0.01).max(0.005) * 1e6) as u64;
                    self.pulls.push(Submove {
                        start_us: start,
                        duration_us: 80_000,
                        displacement: [-kick[0] * g, -kick[1] * g],
                    });
                    // The player expects the kick it has learned, at its own pace.
                    let believed = [kick[0] * self.p.pattern_gain, kick[1] * self.p.pattern_gain];
                    self.expected_kicks.push((shot_us + 8_000, believed));
                }
                RecoilControl::Macro => self.schedule_macro_pull(shot_us + 10_000, *kick),
            }
        }
    }

    /// The macro cancels one nominal kick in five equal 1 ms steps, rounded to whole
    /// counts with the remainder carried.
    fn schedule_macro_pull(&mut self, start_us: u64, kick: [f64; 2]) {
        const STEPS: u64 = 5;
        // A kick up (+pitch) is cancelled by moving the mouse down (+dy), and a kick
        // left (+yaw) by moving it right (+dx).
        let total = [kick[0] / self.deg_per_count, kick[1] / self.deg_per_count];
        let mut sent = [0.0f64; 2];
        for i in 1..=STEPS {
            let f = i as f64 / STEPS as f64;
            let want = [(total[0] * f).round(), (total[1] * f).round()];
            let step = [(want[0] - sent[0]) as i32, (want[1] - sent[1]) as i32];
            sent = want;
            if step != [0, 0] {
                self.macro_moves
                    .push_back((start_us + (i - 1) * 1_000, step));
            }
        }
    }
}

fn norm(v: [f64; 2]) -> f64 {
    libm::sqrt(v[0] * v[0] + v[1] * v[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_jerk_submove_integrates_to_its_displacement() {
        let m = Submove {
            start_us: 1_000,
            duration_us: 250_000,
            displacement: [10.0, -4.0],
        };
        let mut sum = [0.0, 0.0];
        for t in (0..400_000u64).step_by(1_000) {
            let v = m.velocity(t);
            sum[0] += v[0] * DT;
            sum[1] += v[1] * DT;
        }
        assert!(
            (sum[0] - 10.0).abs() < 0.01 && (sum[1] + 4.0).abs() < 0.01,
            "{sum:?}"
        );
        assert_eq!(m.remaining(0), [10.0, -4.0]);
        assert_eq!(m.remaining(251_000), [0.0, -0.0]);
    }

    #[test]
    fn macro_pull_sends_the_exact_nominal_counts() {
        let rng = crate::seed::SimSeed::new(1).rng(0);
        let mut h = Human::new(
            HumanParams::sample(&mut crate::seed::SimSeed::new(1).rng(1)),
            rng,
            0.022,
            RecoilControl::Macro,
        );
        h.schedule_macro_pull(0, [0.35, 1.10]);
        let (mut dx, mut dy) = (0, 0);
        for (_, [x, y]) in &h.macro_moves {
            dx += x;
            dy += y;
        }
        assert_eq!(
            (dx, dy),
            (
                (0.35f64 / 0.022).round() as i32,
                (1.10f64 / 0.022).round() as i32
            )
        );
    }
}
