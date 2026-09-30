// SPDX-License-Identifier: MIT OR Apache-2.0

//! The simulated aim range: a Rust mirror of the Godot demo's `RangeSession`
//! (`demo/scripts/range_session.gd`, `scenario.gd`), with the input-probe drift applied
//! at the two hook points, and telemetry emitted in the [`rearguard_core::telemetry`]
//! schema.
//!
//! Timing follows the demo run headless with `--fixed-fps 120`: one render frame per
//! physics tick. Input generated between two frames is delivered in a batch at the
//! next frame boundary, stamped a few microseconds apart (as Godot stamps events when
//! it drains the OS queue), and then the physics tick runs.

use rearguard_core::probe::{ProbeGenerator, Stream};
use rearguard_core::telemetry::{self, Button, End, Fire, Header, Move, Recoil, Record, Target};

use crate::seed::SimRng;

/// Physics ticks (and frames) per second.
pub const PHYSICS_HZ: u32 = 120;
/// 600 rounds per minute.
pub const FIRE_INTERVAL_TICKS: u64 = 12;
/// A flick target is replaced after 2.5 s without a hit.
pub const FLICK_TIMEOUT_TICKS: u64 = 300;
/// Eye-to-target distance.
pub const TARGET_DISTANCE_M: f64 = 10.0;
/// Target sphere radius.
pub const TARGET_RADIUS_M: f64 = 0.3;
/// Pitch limit, degrees.
pub const PITCH_LIMIT_DEG: f64 = 89.0;
/// Microseconds between events delivered in the same batch.
const BATCH_SPACING_US: u64 = 4;
/// Client clock at session start: an arbitrary engine uptime.
const START_US: u64 = 5_000_000;
/// Render frame counter at session start.
const START_FRAME: u64 = 600;

/// Identifier of [`RECOIL_PATTERN_V1`] in telemetry.
pub const RECOIL_PATTERN_ID: &str = "v1";
/// The demo's fixed recoil pattern `v1`: per-shot kick `[yaw, pitch]` in degrees.
pub const RECOIL_PATTERN_V1: [[f64; 2]; 30] = [
    [0.00, 0.90],
    [0.05, 1.00],
    [-0.05, 1.10],
    [0.05, 1.10],
    [0.00, 1.00],
    [-0.05, 0.90],
    [0.05, 0.80],
    [0.00, 0.70],
    [0.05, 0.60],
    [0.00, 0.50],
    [0.35, 0.30],
    [0.40, 0.30],
    [0.40, 0.25],
    [0.35, 0.25],
    [0.30, 0.20],
    [0.30, 0.20],
    [0.25, 0.20],
    [0.25, 0.15],
    [0.20, 0.15],
    [0.20, 0.15],
    [-0.40, 0.20],
    [-0.45, 0.20],
    [-0.45, 0.15],
    [-0.40, 0.15],
    [-0.35, 0.15],
    [-0.35, 0.10],
    [-0.30, 0.10],
    [-0.30, 0.10],
    [-0.25, 0.10],
    [-0.25, 0.10],
];

/// Target angular radius, degrees (about 1.72°).
#[must_use]
pub fn target_angular_radius_deg() -> f64 {
    libm::atan(TARGET_RADIUS_M / TARGET_DISTANCE_M).to_degrees()
}

/// Scenario played in a session. (The demo's `tracking` scenario is not simulated.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Scenario {
    /// Static targets one at a time within ±40° yaw, -10° to 20° pitch; a hit or a
    /// 2.5 s timeout brings the next.
    Flick,
    /// A static target near the centre; one trigger hold sprays up to 30 shots with
    /// recoil pattern v1; releasing brings the next target.
    Spray,
}

impl Scenario {
    /// Name used in telemetry and on the command line.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Flick => "flick",
            Self::Spray => "spray",
        }
    }

    fn next_target(self, rng: &mut SimRng) -> [f64; 2] {
        match self {
            Self::Flick => [rng.range(-40.0, 40.0), rng.range(-10.0, 20.0)],
            Self::Spray => [rng.range(-10.0, 10.0), rng.range(0.0, 5.0)],
        }
    }
}

/// One input event from an agent, in generation order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// Raw mouse counts (whole numbers, as a mouse reports them).
    Move {
        /// Horizontal counts; +dx is mouse right.
        dx: i32,
        /// Vertical counts; +dy is mouse down.
        dy: i32,
    },
    /// Fire button.
    Button(bool),
}

/// The game state at one frame boundary, after that frame's input and tick: what the
/// screen showed from then until the next frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Simulation time of the frame boundary, in microseconds from session start.
    pub t_us: u64,
    /// View `[yaw, pitch]`, degrees.
    pub view: [f64; 2],
    /// Current target `[yaw, pitch]`, degrees.
    pub target: [f64; 2],
    /// Current target index.
    pub target_index: u64,
    /// Shots fired in the session so far.
    pub shots: u64,
    /// Shots fired in the current trigger hold.
    pub burst_shot: u32,
    /// Whether the magazine is empty (spray only).
    pub magazine_empty: bool,
}

/// Ground truth for one shot. Not telemetry: it records the drift's effect, which only
/// the simulator (or a server holding the seed) can know.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotTruth {
    /// Session-wide shot index.
    pub shot: u64,
    /// Target index at the shot.
    pub target_index: u64,
    /// Shot index within the trigger hold.
    pub burst_shot: u32,
    /// Aim error `target - view` at the shot, degrees.
    pub error: [f64; 2],
    /// `nominal view - view` at the shot: how much the drift has changed the aim error
    /// since the session started. The nominal view applies the same inputs with every
    /// multiplier equal to 1.
    pub drift_offset: [f64; 2],
    /// `drift_offset` when the current target appeared.
    pub drift_offset_at_target: [f64; 2],
    /// Whether the shot hit.
    pub hit: bool,
}

/// Static session parameters.
#[derive(Clone, Debug)]
pub struct WorldConfig {
    /// Scenario.
    pub scenario: Scenario,
    /// Session length, seconds of game time.
    pub duration_s: f64,
    /// Degrees of view per raw count at multiplier 1.
    pub deg_per_count: f64,
    /// Match identifier for the probe key hierarchy.
    pub match_id: String,
    /// Player identifier for the probe key hierarchy.
    pub player_id: u64,
    /// Seed of the target sequence (recorded in the header).
    pub scenario_seed: u64,
}

/// The simulated game.
#[derive(Debug)]
pub struct World {
    config: WorldConfig,
    probe: ProbeGenerator,
    targets_rng: SimRng,
    records: Vec<Record>,
    truth: Vec<ShotTruth>,
    frames: Vec<Frame>,
    view: [f64; 2],
    nominal: [f64; 2],
    frame: u64,
    tick: u64,
    target: [f64; 2],
    target_index: u64,
    target_since_tick: u64,
    drift_offset_at_target: [f64; 2],
    trigger_down: bool,
    next_fire_tick: u64,
    burst_shot: u32,
    shots: u64,
    hits: u64,
    finished: bool,
}

impl World {
    /// Starts a session: writes the header and the first target.
    #[must_use]
    pub fn new(config: WorldConfig, probe: ProbeGenerator, mut targets_rng: SimRng) -> Self {
        let target = config.scenario.next_target(&mut targets_rng);
        let mut world = Self {
            config,
            probe,
            targets_rng,
            records: Vec::new(),
            truth: Vec::new(),
            frames: Vec::new(),
            view: [0.0, 0.0],
            nominal: [0.0, 0.0],
            frame: START_FRAME,
            tick: 0,
            target,
            target_index: 0,
            target_since_tick: 0,
            drift_offset_at_target: [0.0, 0.0],
            trigger_down: false,
            next_fire_tick: 0,
            burst_shot: 0,
            shots: 0,
            hits: 0,
            finished: false,
        };
        world.records.push(Record::Header(Header {
            ts_us: START_US,
            frame: START_FRAME,
            tick: 0,
            format: telemetry::FORMAT.to_owned(),
            version: telemetry::VERSION,
            match_id: world.config.match_id.clone(),
            player_id: world.config.player_id,
            source: "sim".to_owned(),
            probe_start_us: START_US,
            deg_per_count: world.config.deg_per_count,
            physics_hz: PHYSICS_HZ,
            scenario: world.config.scenario.name().to_owned(),
            scenario_seed: world.config.scenario_seed,
            duration_s: world.config.duration_s,
            recoil_pattern: RECOIL_PATTERN_ID.to_owned(),
            target_distance_m: TARGET_DISTANCE_M,
            target_radius_m: TARGET_RADIUS_M,
        }));
        world.record_target(START_US);
        world.frames.push(world.snapshot(0));
        world
    }

    /// Simulation time of frame boundary `n` (frame 0 is the session start).
    #[must_use]
    pub fn frame_time_us(n: u64) -> u64 {
        n * 1_000_000 / u64::from(PHYSICS_HZ)
    }

    /// Every frame so far; the last is the current one.
    #[must_use]
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// Whether the session has ended.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Delivers one frame's input batch, then runs one physics tick.
    pub fn step_frame(&mut self, inputs: &[Input]) {
        if self.finished {
            return;
        }
        let n = self.frames.len() as u64;
        let boundary = START_US + Self::frame_time_us(n);
        self.frame += 1;
        for (i, input) in inputs.iter().enumerate() {
            let ts = boundary + i as u64 * BATCH_SPACING_US;
            match *input {
                Input::Move { dx, dy } => self.handle_motion(dx, dy, ts),
                Input::Button(pressed) => self.handle_trigger(pressed, ts),
            }
        }
        let ts = boundary + inputs.len() as u64 * BATCH_SPACING_US;
        self.step_tick(ts);
        self.frames.push(self.snapshot(Self::frame_time_us(n)));
    }

    /// Ends the session and returns its telemetry and ground truth.
    #[must_use]
    pub fn into_output(self) -> (Vec<Record>, Vec<ShotTruth>) {
        (self.records, self.truth)
    }

    fn snapshot(&self, t_us: u64) -> Frame {
        Frame {
            t_us,
            view: self.view,
            target: self.target,
            target_index: self.target_index,
            shots: self.shots,
            burst_shot: self.burst_shot,
            magazine_empty: self.magazine_empty(),
        }
    }

    fn multiplier(&mut self, stream: Stream, ts_us: u64) -> f64 {
        let tick = telemetry::probe_tick(ts_us, START_US);
        self.probe.drift(stream, tick).multiplier()
    }

    fn add_view(&mut self, delta: [f64; 2], nominal_delta: [f64; 2]) {
        self.view[0] += delta[0];
        self.view[1] = (self.view[1] + delta[1]).clamp(-PITCH_LIMIT_DEG, PITCH_LIMIT_DEG);
        self.nominal[0] += nominal_delta[0];
        self.nominal[1] =
            (self.nominal[1] + nominal_delta[1]).clamp(-PITCH_LIMIT_DEG, PITCH_LIMIT_DEG);
    }

    fn drift_offset(&self) -> [f64; 2] {
        [
            self.nominal[0] - self.view[0],
            self.nominal[1] - self.view[1],
        ]
    }

    fn handle_motion(&mut self, dx: i32, dy: i32, ts: u64) {
        let m = self.multiplier(Stream::Sensitivity, ts);
        let dpc = self.config.deg_per_count;
        let nominal = [-f64::from(dx) * dpc, -f64::from(dy) * dpc];
        self.add_view([nominal[0] * m, nominal[1] * m], nominal);
        self.records.push(Record::Move(Move {
            ts_us: ts,
            frame: self.frame,
            tick: self.tick,
            dx: f64::from(dx),
            dy: f64::from(dy),
            yaw: self.view[0],
            pitch: self.view[1],
        }));
    }

    fn handle_trigger(&mut self, pressed: bool, ts: u64) {
        if pressed == self.trigger_down {
            return;
        }
        self.trigger_down = pressed;
        self.records.push(Record::Button(Button {
            ts_us: ts,
            frame: self.frame,
            tick: self.tick,
            pressed,
        }));
        if pressed {
            self.try_fire(ts);
            return;
        }
        if self.config.scenario == Scenario::Spray && self.burst_shot > 0 {
            self.advance_target(ts);
        }
        self.burst_shot = 0;
    }

    fn step_tick(&mut self, ts: u64) {
        self.tick += 1;
        if self.trigger_down {
            self.try_fire(ts);
        }
        if self.config.scenario == Scenario::Flick
            && self.tick - self.target_since_tick >= FLICK_TIMEOUT_TICKS
        {
            self.advance_target(ts);
        }
        if self.tick as f64 / f64::from(PHYSICS_HZ) >= self.config.duration_s {
            self.finished = true;
            self.records.push(Record::End(End {
                ts_us: ts,
                frame: self.frame,
                tick: self.tick,
                shots: self.shots,
                hits: self.hits,
                complete: true,
            }));
        }
    }

    fn magazine_empty(&self) -> bool {
        self.config.scenario == Scenario::Spray
            && self.burst_shot as usize >= RECOIL_PATTERN_V1.len()
    }

    fn try_fire(&mut self, ts: u64) {
        if self.tick < self.next_fire_tick || self.magazine_empty() {
            return;
        }
        let hit = angular_distance_deg(self.view, self.target) <= target_angular_radius_deg();
        let shot = self.shots;
        self.records.push(Record::Fire(Fire {
            ts_us: ts,
            frame: self.frame,
            tick: self.tick,
            shot,
            burst_shot: self.burst_shot,
            yaw: self.view[0],
            pitch: self.view[1],
            target: self.target_index,
            target_yaw: self.target[0],
            target_pitch: self.target[1],
            hit,
        }));
        self.truth.push(ShotTruth {
            shot,
            target_index: self.target_index,
            burst_shot: self.burst_shot,
            error: [self.target[0] - self.view[0], self.target[1] - self.view[1]],
            drift_offset: self.drift_offset(),
            drift_offset_at_target: self.drift_offset_at_target,
            hit,
        });
        self.shots += 1;
        if hit {
            self.hits += 1;
        }
        self.next_fire_tick = self.tick + FIRE_INTERVAL_TICKS;
        let burst_shot = self.burst_shot;
        self.burst_shot += 1;
        if self.config.scenario == Scenario::Spray {
            let kick = RECOIL_PATTERN_V1[burst_shot as usize];
            let m = self.multiplier(Stream::Recoil, ts);
            self.add_view([kick[0] * m, kick[1] * m], kick);
            self.records.push(Record::Recoil(Recoil {
                ts_us: ts,
                frame: self.frame,
                tick: self.tick,
                shot,
                burst_shot,
                kick_yaw: kick[0],
                kick_pitch: kick[1],
                yaw: self.view[0],
                pitch: self.view[1],
            }));
        }
        if hit && self.config.scenario == Scenario::Flick {
            self.advance_target(ts);
        }
    }

    fn advance_target(&mut self, ts: u64) {
        self.target_index += 1;
        self.target_since_tick = self.tick;
        self.target = self.config.scenario.next_target(&mut self.targets_rng);
        self.drift_offset_at_target = self.drift_offset();
        self.record_target(ts);
    }

    fn record_target(&mut self, ts: u64) {
        self.records.push(Record::Target(Target {
            ts_us: ts,
            frame: self.frame,
            tick: self.tick,
            target: self.target_index,
            target_yaw: self.target[0],
            target_pitch: self.target[1],
        }));
    }
}

/// Angle in degrees between two view directions `[yaw, pitch]`.
#[must_use]
pub fn angular_distance_deg(a: [f64; 2], b: [f64; 2]) -> f64 {
    let (da, db) = (direction(a), direction(b));
    let dot = da[0] * db[0] + da[1] * db[1] + da[2] * db[2];
    libm::acos(dot.clamp(-1.0, 1.0)).to_degrees()
}

/// Unit view direction for `[yaw, pitch]` in degrees (looking along -Z at 0, 0).
fn direction(angles: [f64; 2]) -> [f64; 3] {
    let (yaw, pitch) = (angles[0].to_radians(), angles[1].to_radians());
    [
        -libm::sin(yaw) * libm::cos(pitch),
        libm::sin(pitch),
        -libm::cos(yaw) * libm::cos(pitch),
    ]
}

#[cfg(test)]
mod tests {
    use rearguard_core::probe::{Amplitude, ProbeConfig, RootSeed};

    use super::*;
    use crate::seed::SimSeed;

    fn world(scenario: Scenario, ppm: u32) -> World {
        let root = RootSeed::from_bytes(&mut [9; 32]);
        let epoch = root.match_key(b"w").player_key(1).epoch_seed(0);
        let config = ProbeConfig {
            amplitude: Amplitude::from_ppm(ppm).unwrap(),
            ..ProbeConfig::default()
        };
        let probe = ProbeGenerator::new(&epoch, &config).unwrap();
        World::new(
            WorldConfig {
                scenario,
                duration_s: 5.0,
                deg_per_count: 0.022,
                match_id: "w".to_owned(),
                player_id: 1,
                scenario_seed: 3,
            },
            probe,
            SimSeed::new(1).rng(0),
        )
    }

    #[test]
    fn without_drift_the_view_is_the_nominal_view() {
        let mut w = world(Scenario::Spray, 0);
        w.step_frame(&[Input::Move { dx: 100, dy: -50 }, Input::Button(true)]);
        for _ in 0..40 {
            w.step_frame(&[Input::Move { dx: 0, dy: 10 }]);
        }
        let (_, truth) = w.into_output();
        assert!(!truth.is_empty());
        assert!(truth.iter().all(|t| t.drift_offset == [0.0, 0.0]));
    }

    #[test]
    fn drift_scales_moves_and_recoil_within_the_amplitude() {
        let mut w = world(Scenario::Spray, 20_000);
        w.step_frame(&[Input::Move { dx: 1000, dy: 0 }, Input::Button(true)]);
        let yaw = w.frames().last().unwrap().view[0];
        let nominal = -1000.0 * 0.022;
        assert!(
            yaw != nominal && ((yaw / nominal) - 1.0).abs() <= 0.02,
            "{yaw}"
        );
        for _ in 0..40 {
            w.step_frame(&[]);
        }
        let (records, _) = w.into_output();
        let kicks: Vec<_> = records
            .iter()
            .filter_map(|r| match r {
                Record::Recoil(k) => Some(k.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            kicks.len(),
            4,
            "shots on the press (tick 0) and at ticks 12, 24 and 36"
        );
        assert_eq!(kicks[1].kick_pitch, 1.00);
    }

    #[test]
    fn flick_hit_advances_the_target_and_timeout_replaces_it() {
        let mut w = world(Scenario::Flick, 5_000);
        let t0 = w.frames()[0].target;
        // Aim exactly (to the count) at the target, then fire.
        let dx = (-t0[0] / 0.022).round() as i32;
        let dy = (-t0[1] / 0.022).round() as i32;
        w.step_frame(&[
            Input::Move { dx, dy },
            Input::Button(true),
            Input::Button(false),
        ]);
        assert_eq!(w.frames().last().unwrap().target_index, 1);
        for _ in 0..FLICK_TIMEOUT_TICKS {
            w.step_frame(&[]);
        }
        assert_eq!(w.frames().last().unwrap().target_index, 2);
    }

    #[test]
    fn telemetry_is_ordered_and_ends_at_the_duration() {
        let mut w = world(Scenario::Flick, 5_000);
        while !w.finished() {
            w.step_frame(&[Input::Move { dx: 1, dy: 1 }, Input::Move { dx: -1, dy: 0 }]);
        }
        assert_eq!(w.frames().len(), 5 * PHYSICS_HZ as usize + 1);
        let (records, _) = w.into_output();
        assert!(matches!(records.first(), Some(Record::Header(_))));
        assert!(matches!(records.last(), Some(Record::End(e)) if e.complete && e.tick == 600));
        assert!(records.windows(2).all(|w| w[0].ts_us() <= w[1].ts_us()));
    }

    #[test]
    fn angular_distance_matches_simple_cases() {
        assert!(angular_distance_deg([0.0, 0.0], [0.0, 0.0]) < 1e-6);
        assert!((angular_distance_deg([0.0, 0.0], [30.0, 0.0]) - 30.0).abs() < 1e-9);
        assert!((angular_distance_deg([10.0, 0.0], [10.0, -20.0]) - 20.0).abs() < 1e-9);
        assert!((target_angular_radius_deg() - 1.718).abs() < 0.001);
    }
}
