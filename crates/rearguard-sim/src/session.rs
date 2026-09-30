//! Player classes and running one simulated session.

use rearguard_core::probe::{Amplitude, EpochSeed, ProbeConfig, ProbeGenerator, RootSeed};
use rearguard_core::telemetry::Record;

use crate::aimbot::{
    AdaptiveAimbot, FastAdaptiveAimbot, FlickAimbot, HumanisedAimbot, HumanisedParams,
    SmoothingAimbot,
};
use crate::human::{Human, HumanParams, RecoilControl};
use crate::seed::SimSeed;
use crate::world::{Input, Scenario, ShotTruth, World, WorldConfig, target_angular_radius_deg};

/// A kind of simulated player.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    /// Closed-loop human model ([`crate::human`]). Plays flick and spray.
    Human,
    /// The human model with a fixed-pattern recoil macro. Plays spray.
    RecoilMacro,
    /// Computed-flick aimbot: exact angle, one injected move. Plays flick.
    FlickAimbot,
    /// Humanised aimbot: delay, smoothed path, noise. Plays flick.
    HumanisedAimbot,
    /// Adaptive aimbot: estimates the drift from its own moves. Plays flick.
    AdaptiveAimbot,
    /// Closed-loop smoothing aimbot: each frame, a fraction of the actual remaining
    /// error. Plays flick.
    SmoothingAimbot,
    /// Faster-measuring adaptive aimbot: 30 ms reactions, least-squares drift
    /// estimate over a time window. Plays flick.
    FastAdaptiveAimbot,
}

impl Class {
    /// Every class, in output order. New classes go at the end: a class's position
    /// numbers its random streams, so appending keeps existing sessions unchanged.
    pub const ALL: [Self; 7] = [
        Self::Human,
        Self::RecoilMacro,
        Self::FlickAimbot,
        Self::HumanisedAimbot,
        Self::AdaptiveAimbot,
        Self::SmoothingAimbot,
        Self::FastAdaptiveAimbot,
    ];

    /// Name used on the command line and in the manifest.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::RecoilMacro => "recoil-macro",
            Self::FlickAimbot => "flick-aimbot",
            Self::HumanisedAimbot => "humanised-aimbot",
            Self::AdaptiveAimbot => "adaptive-aimbot",
            Self::SmoothingAimbot => "smoothing-aimbot",
            Self::FastAdaptiveAimbot => "fast-adaptive-aimbot",
        }
    }

    /// Parses a [`Class::name`].
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }

    /// Scenarios this class plays.
    #[must_use]
    pub fn scenarios(self) -> &'static [Scenario] {
        match self {
            Self::Human => &[Scenario::Flick, Scenario::Spray],
            Self::RecoilMacro => &[Scenario::Spray],
            Self::FlickAimbot
            | Self::HumanisedAimbot
            | Self::AdaptiveAimbot
            | Self::SmoothingAimbot
            | Self::FastAdaptiveAimbot => &[Scenario::Flick],
        }
    }

    /// Whether the class aims open-loop (and so should follow the drift).
    #[must_use]
    pub fn open_loop(self) -> bool {
        !matches!(self, Self::Human | Self::SmoothingAimbot)
    }
}

/// What to simulate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SessionSpec {
    /// Player class.
    pub class: Class,
    /// Scenario.
    pub scenario: Scenario,
    /// Index of the session within its class and scenario, from 0.
    pub index: u32,
    /// Length in seconds of game time.
    pub duration_s: f64,
    /// Probe drift amplitude.
    pub amplitude: Amplitude,
    /// Tuning of the configurable cheat models.
    pub models: ModelParams,
}

/// Tuning of the configurable cheat models. Other classes ignore it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelParams {
    /// [`Class::SmoothingAimbot`]: fraction of the remaining error moved per frame,
    /// in (0, 1].
    pub smoothing: f64,
    /// [`Class::FastAdaptiveAimbot`]: estimation window, milliseconds.
    pub estimation_window_ms: u32,
}

impl ModelParams {
    /// Smoothing 0.2 per frame (at 120 frames per second, a time constant of about
    /// 40 ms); estimation window 250 ms (the last two or three flicks).
    pub const DEFAULT: Self = Self {
        smoothing: 0.2,
        estimation_window_ms: 250,
    };
}

impl Default for ModelParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// One simulated session.
#[derive(Clone, Debug)]
pub struct Session {
    /// What was simulated.
    pub spec: SessionSpec,
    /// Match identifier in the header.
    pub match_id: String,
    /// Player identifier in the header.
    pub player_id: u64,
    /// Telemetry, as the client would stream it.
    pub records: Vec<Record>,
    /// Ground truth per shot (never written to telemetry).
    pub truth: Vec<ShotTruth>,
}

/// Random streams per session, by purpose.
#[derive(Clone, Copy)]
enum Purpose {
    Ids = 0,
    Targets = 1,
    Params = 2,
    Noise = 3,
}

fn stream(spec: &SessionSpec, purpose: Purpose) -> u64 {
    let class = Class::ALL
        .iter()
        .position(|c| *c == spec.class)
        .unwrap_or(0) as u64;
    let scenario = spec.scenario as u64;
    ((class << 40 | scenario << 32 | u64::from(spec.index)) << 2) | purpose as u64
}

enum Agent {
    Human(Box<Human>),
    Flick(FlickAimbot),
    Humanised(Box<HumanisedAimbot>),
    Adaptive(Box<AdaptiveAimbot>),
    Smoothing(SmoothingAimbot),
    FastAdaptive(Box<FastAdaptiveAimbot>),
}

/// Simulates one session. `root` must be `seed.probe_root()`; it is passed in so
/// that a batch derives it once.
#[must_use]
pub fn run_session(seed: &SimSeed, root: &RootSeed, spec: SessionSpec) -> Session {
    let mut ids = seed.rng(stream(&spec, Purpose::Ids));
    let match_id = format!("sim-{:016x}", ids.next_u64());
    let player_id = ids.next_u64() >> 32;
    let scenario_seed = ids.next_u64();
    let epoch = root
        .match_key(match_id.as_bytes())
        .player_key(player_id)
        .epoch_seed(0);
    simulate(seed, spec, match_id, player_id, scenario_seed, &epoch)
}

/// Simulates one session under a probe seed and identifiers issued elsewhere (for
/// example by `rearguard-server`), as a real client would receive them. The player's
/// behaviour, targets and noise still come from `seed` and `spec`.
#[must_use]
pub fn run_session_as(
    seed: &SimSeed,
    epoch: &EpochSeed,
    match_id: &str,
    player_id: u64,
    spec: SessionSpec,
) -> Session {
    let mut ids = seed.rng(stream(&spec, Purpose::Ids));
    let _ = (ids.next_u64(), ids.next_u64());
    let scenario_seed = ids.next_u64();
    simulate(
        seed,
        spec,
        match_id.to_owned(),
        player_id,
        scenario_seed,
        epoch,
    )
}

fn simulate(
    seed: &SimSeed,
    spec: SessionSpec,
    match_id: String,
    player_id: u64,
    scenario_seed: u64,
    epoch: &EpochSeed,
) -> Session {
    let mut params = seed.rng(stream(&spec, Purpose::Params));
    // Players choose their own sensitivity: 0.011 to 0.044 degrees per count.
    let deg_per_count = 0.022 * libm::exp2(params.range(-1.0, 1.0));
    let noise = seed.rng(stream(&spec, Purpose::Noise));
    let mut agent = match spec.class {
        Class::Human | Class::RecoilMacro => {
            let control = if spec.class == Class::Human {
                RecoilControl::Learned
            } else {
                RecoilControl::Macro
            };
            let p = HumanParams::sample(&mut params);
            Agent::Human(Box::new(Human::new(p, noise, deg_per_count, control)))
        }
        Class::FlickAimbot => Agent::Flick(FlickAimbot::new(deg_per_count, 30_000)),
        Class::HumanisedAimbot => {
            let p = HumanisedParams::sample(&mut params);
            Agent::Humanised(Box::new(HumanisedAimbot::new(p, noise, deg_per_count)))
        }
        Class::AdaptiveAimbot => Agent::Adaptive(Box::new(AdaptiveAimbot::new(
            noise,
            deg_per_count,
            0.15,
            0.5,
        ))),
        Class::SmoothingAimbot => Agent::Smoothing(SmoothingAimbot::new(
            deg_per_count,
            spec.models.smoothing,
            30_000,
            0.25 * target_angular_radius_deg(),
        )),
        Class::FastAdaptiveAimbot => Agent::FastAdaptive(Box::new(FastAdaptiveAimbot::new(
            deg_per_count,
            30_000,
            u64::from(spec.models.estimation_window_ms) * 1_000,
        ))),
    };

    let config = ProbeConfig {
        amplitude: spec.amplitude,
        ..ProbeConfig::default()
    };
    let probe = ProbeGenerator::new(epoch, &config).expect("default shape is valid");
    let mut world = World::new(
        WorldConfig {
            scenario: spec.scenario,
            duration_s: spec.duration_s,
            deg_per_count,
            match_id: match_id.clone(),
            player_id,
            scenario_seed,
        },
        probe,
        seed.rng(stream(&spec, Purpose::Targets)),
    );

    let mut batch: Vec<Input> = Vec::new();
    let mut n = 1u64;
    while !world.finished() {
        batch.clear();
        let (start, end) = (World::frame_time_us(n - 1), World::frame_time_us(n));
        for ms in start / 1_000 + 1..=end / 1_000 {
            let now = ms * 1_000;
            let frames = world.frames();
            match &mut agent {
                Agent::Human(h) => h.step(now, frames, spec.scenario, &mut batch),
                Agent::Flick(a) => a.step(now, frames, &mut batch),
                Agent::Humanised(a) => a.step(now, frames, &mut batch),
                Agent::Adaptive(a) => a.step(now, frames, &mut batch),
                Agent::Smoothing(a) => a.step(now, frames, &mut batch),
                Agent::FastAdaptive(a) => a.step(now, frames, &mut batch),
            }
        }
        world.step_frame(&batch);
        n += 1;
    }
    let (records, truth) = world.into_output();
    Session {
        spec,
        match_id,
        player_id,
        records,
        truth,
    }
}
