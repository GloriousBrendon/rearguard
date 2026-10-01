// SPDX-License-Identifier: MIT OR Apache-2.0

//! Input-probe detector: does a player's aim follow the secret drift?
//!
//! Server-side only. It needs the player's [`EpochSeed`], and its configuration
//! (thresholds included) must never be sent to a client.
//!
//! # What it measures
//!
//! The detector replays the untrusted telemetry itself: every raw delta and recoil kick,
//! once with the probe's multipliers (the view the player really had) and once without
//! (the nominal view). Their difference is the drift's exact effect on the aim. It never
//! trusts a reported angle; it only counts disagreements ([`Evidence::angle_mismatches`]).
//!
//! **Fire-time statistic ([`Report::error`]).** For each shot, the aim error
//! `target - view` is paired with the drift's effect on it: since the target appeared
//! (flick), or over one whole spray burst (last shot minus first, one pair per burst, so
//! pairs stay independent). Yaw and pitch give one pair each. An open-loop player never
//! sees the drift, so its error contains the drift's effect; a closed-loop player
//! corrects it.
//!
//! Correlation alone cannot be the test: a simulated human that taps once "close enough"
//! passes about a quarter of the most recent drift through (task 1.2), which becomes
//! significant with enough shots. So the null hypothesis is a *strength* bound. With
//! `error = slope × drift effect + residual`, the quantity
//! `kappa = slope / residual sd` (drift gain per degree of unexplained error, 1/deg) does
//! not depend on the drift amplitude, and closed-loop players are assumed to have
//! `kappa <= kappa_bound`. For the observed spread of the drift effect `s`, that bound
//! implies a correlation `rho0 = kappa_bound·s / sqrt(1 + kappa_bound²·s²)`. The
//! statistic is Fisher's `z = (atanh r - atanh rho0) · sqrt(n - 3)`.
//!
//! **Step-response statistic ([`Report::steps`]).** Per render frame, the player's raw
//! step (sum of the frame's deltas, in counts). For consecutive frames of one engagement,
//! both at least `min_step_counts` long, it pairs the drift at the first step with the
//! log ratio of the two step lengths. A controller that re-reads the view every frame and
//! moves a fixed share of the remaining error (a smoothing aimbot) makes that ratio
//! `1 - share × multiplier`, so it falls when the drift rises: a response within one
//! frame, faster than any visual reaction. Its null is zero correlation; the statistic
//! is `z = -atanh(r) · sqrt(n - 3)`, positive when the ratio falls with the drift.
//!
//! **Drift-change statistic ([`Report::change`], flick, task 1.3a).** A cheat that
//! knows a drift exists can measure the multiplier its own last move produced and
//! divide the next move by it. Its error is then not the drift's whole effect but the
//! drift's *change* since that measurement. For each shot, with `N` the nominal view
//! change and `D` the drift's effect since the previous shot, and `d_prev` the drift
//! (multiplier - 1) averaged along the path of the moves before the previous shot,
//! the error is paired with `D + N·d_prev`: what a drift-learner that measured the last
//! movement would still get wrong. The null is again a kappa bound, with its own value.
//!
//! **Per-shot spray statistic ([`Report::spray`], task 1.3a).** Within a spray burst,
//! each shot's change in error since the previous shot is paired with what a
//! fixed-pattern recoil macro would get wrong over that interval: the nominal kick
//! times (sensitivity multiplier - recoil multiplier). A macro that cancels each kick
//! by the exact nominal counts has exactly that error, with only tremor and rounding
//! beside it; a human's learned, per-shot correction is much noisier. That gives about
//! 29 pairs per axis per burst, where the fire-time statistic has one. The regressor
//! uses only the recoil records and the probe, never the player's own moves: the
//! drift's effect on a noisy pull would put the same noise into both sides of the fit.
//! The recoil pattern is the same in every burst, so the fit also removes a separate
//! mean per axis and burst shot: a player's habitual error on one kick (or a macro's
//! rounding) repeats every burst and carries no drift. The null is a kappa bound with
//! its own value.
//!
//! **Scores.** Each statistic gives a signed generalised log-likelihood ratio,
//! `score = sign(z) · z² / 2` (the Gaussian GLR for a one-sided shift), and a
//! confidence `Φ(z)`. Evidence is kept per window of telemetry time and for the whole
//! session, and updated as each chunk of records is fed in.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer};

use crate::probe::{EpochSeed, ProbeConfig, ProbeError, ProbeGenerator, Stream};
use crate::telemetry::{self, Record};

/// Detector thresholds. There are deliberately no defaults: every value is chosen by
/// the operator (for example, from a calibration run) and kept on the server.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetectorConfig {
    /// Largest drift gain per degree of residual error a closed-loop player is
    /// assumed to show, 1/deg. The fire-time null hypothesis.
    pub kappa_bound: f64,
    /// Fire-time score at or above which evidence is flagged.
    pub error_flag_score: f64,
    /// Step-response score at or above which evidence is flagged.
    pub steps_flag_score: f64,
    /// Pairs needed before a statistic reports anything but a neutral score.
    pub min_pairs: u64,
    /// Window length for per-window evidence, milliseconds of telemetry time.
    pub window_ms: u64,
    /// Shortest per-frame step, in counts, that the step-response statistic uses.
    pub min_step_counts: f64,
    /// Drift-change statistic (flick). Absent: not computed.
    #[serde(default)]
    pub change: Option<KappaTest>,
    /// Per-shot spray statistic. Absent: not computed.
    #[serde(default)]
    pub spray: Option<KappaTest>,
}

/// Null hypothesis and flag score of a kappa-bound statistic.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KappaTest {
    /// Largest drift gain per degree of residual error a closed-loop player is assumed
    /// to show on this statistic's pairs, 1/deg.
    pub kappa_bound: f64,
    /// Score at or above which evidence is flagged.
    pub flag_score: f64,
}

impl KappaTest {
    fn valid(&self) -> bool {
        self.kappa_bound.is_finite() && self.kappa_bound >= 0.0 && self.flag_score.is_finite()
    }
}

impl DetectorConfig {
    fn validate(&self) -> Result<(), DetectError> {
        let ok = self.kappa_bound.is_finite()
            && self.kappa_bound >= 0.0
            && self.error_flag_score.is_finite()
            && self.steps_flag_score.is_finite()
            && self.min_pairs >= 4
            && self.window_ms > 0
            && self.min_step_counts.is_finite()
            && self.min_step_counts >= 1.0
            && self.change.is_none_or(|t| t.valid())
            && self.spray.is_none_or(|t| t.valid());
        if ok { Ok(()) } else { Err(DetectError::Config) }
    }
}

/// The detector configurations a server runs: one for every scenario, or one per
/// scenario (task 1.12). Thresholds calibrated on flick play do not fit spray play, so
/// a calibration gives a set per scenario.
///
/// In JSON, either a plain [`DetectorConfig`] object, or
/// `{"scenarios": {"flick": {...}, "spray": {...}}}` with a [`DetectorConfig`] per
/// scenario name (the telemetry header's `scenario`).
#[derive(Clone, Debug, PartialEq)]
pub enum DetectorSet {
    /// The same configuration for every scenario.
    Single(DetectorConfig),
    /// One configuration per scenario name. A scenario without one has none: there is
    /// no fallback, so every threshold in use is one somebody chose.
    PerScenario(BTreeMap<String, DetectorConfig>),
}

impl DetectorSet {
    /// The configuration for a session of `scenario`, if there is one.
    #[must_use]
    pub fn for_scenario(&self, scenario: &str) -> Option<&DetectorConfig> {
        match self {
            Self::Single(config) => Some(config),
            Self::PerScenario(configs) => configs.get(scenario),
        }
    }

    /// Checks every configuration, as [`Detector::new`] would.
    ///
    /// # Errors
    /// [`DetectError::Config`] if a configuration is out of range, or if there is none.
    pub fn validate(&self) -> Result<(), DetectError> {
        match self {
            Self::Single(config) => config.validate(),
            Self::PerScenario(configs) if configs.is_empty() => Err(DetectError::Config),
            Self::PerScenario(configs) => configs.values().try_for_each(DetectorConfig::validate),
        }
    }
}

impl<'de> Deserialize<'de> for DetectorSet {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PerScenario {
            scenarios: BTreeMap<String, DetectorConfig>,
        }
        // Chosen by the presence of `scenarios`, so a mistake inside either form is
        // reported as that form's error, not as "matches no variant".
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("scenarios").is_some() {
            serde_json::from_value::<PerScenario>(value).map(|p| Self::PerScenario(p.scenarios))
        } else {
            serde_json::from_value(value).map(Self::Single)
        }
        .map_err(serde::de::Error::custom)
    }
}

/// Why the detector refused a configuration or a record stream.
#[derive(Clone, Debug, PartialEq)]
pub enum DetectError {
    /// The configuration is out of range.
    Config,
    /// The probe configuration is invalid.
    Probe(ProbeError),
    /// The first record is not a header of the supported format and version.
    BadHeader,
    /// A record arrived before the header, or a second header arrived.
    OutOfOrder,
    /// A timestamp went backwards or preceded the probe start.
    BadTimestamp,
}

impl core::fmt::Display for DetectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Config => f.write_str("detector configuration out of range"),
            Self::Probe(e) => write!(f, "probe configuration: {e}"),
            Self::BadHeader => f.write_str("telemetry header missing or unsupported"),
            Self::OutOfOrder => f.write_str("telemetry records out of order"),
            Self::BadTimestamp => f.write_str("telemetry timestamp went backwards"),
        }
    }
}

impl std::error::Error for DetectError {}

/// Running sums for a least-squares fit of `y` on `x`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Moments {
    n: u64,
    sx: f64,
    sy: f64,
    sxx: f64,
    syy: f64,
    sxy: f64,
}

impl Moments {
    fn add(&mut self, x: f64, y: f64) {
        self.n += 1;
        self.sx += x;
        self.sy += y;
        self.sxx += x * x;
        self.syy += y * y;
        self.sxy += x * y;
    }

    /// (variance of x, variance of y, covariance), divided by n.
    fn central(&self) -> (f64, f64, f64) {
        let n = self.n as f64;
        let (mx, my) = (self.sx / n, self.sy / n);
        (
            (self.sxx / n - mx * mx).max(0.0),
            (self.syy / n - my * my).max(0.0),
            self.sxy / n - mx * my,
        )
    }
}

/// Groups per axis for the per-shot spray statistic: one per burst shot, with every
/// shot from the last group on pooled into it.
const SPRAY_GROUPS: usize = 32;

/// Running sums for a least-squares fit of `y` on `x` with a separate mean per group
/// (a fixed-effects fit): per axis and burst shot, for the per-shot spray statistic.
/// Removing each group's mean removes anything the same in every burst, such as a
/// player's habitual over- or under-compensation of one kick, or a macro's rounding.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Grouped([[Moments; SPRAY_GROUPS]; 2]);

impl Grouped {
    fn add(&mut self, axis: usize, group: usize, x: f64, y: f64) {
        self.0[axis][group.min(SPRAY_GROUPS - 1)].add(x, y);
    }

    fn evidence(&self, null: Null, min_pairs: u64, span: (u64, u64)) -> Evidence {
        let (mut pairs, mut groups) = (0u64, 0u64);
        let (mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0);
        for m in self.0.iter().flatten().filter(|m| m.n > 0) {
            let (vx, vy, cxy) = m.central();
            let n = m.n as f64;
            pairs += m.n;
            groups += 1;
            sxx += vx * n;
            syy += vy * n;
            sxy += cxy * n;
        }
        // One degree of freedom per group mean beyond the first.
        let n = pairs as f64 - groups.saturating_sub(1) as f64;
        let central = if n > 0.0 {
            (sxx / n, syy / n, sxy / n)
        } else {
            (0.0, 0.0, 0.0)
        };
        Evidence::from_central(pairs, n, central, null, min_pairs, span)
    }
}

/// Which null hypothesis a statistic tests, and its flag score.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Null {
    /// Kappa at most `bound`, alternative "larger".
    KappaBound { bound: f64, flag: f64 },
    /// Zero correlation, alternative "negative".
    NoResponse { flag: f64 },
}

/// Evidence from one statistic over one span of telemetry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Evidence {
    /// Telemetry time covered, from probe start, milliseconds: `[start, end)`.
    pub start_ms: u64,
    /// End of the span, exclusive.
    pub end_ms: u64,
    /// Pairs used.
    pub pairs: u64,
    /// Pearson correlation of the pairs (0 when undefined).
    pub r: f64,
    /// Least-squares slope of y on x (for the fire-time statistic: share of the drift's
    /// effect found in the error).
    pub slope: f64,
    /// Standard error of the slope.
    pub slope_se: f64,
    /// Residual standard deviation of y after the fit.
    pub residual_sd: f64,
    /// `slope / residual_sd` (fire-time statistic: 1/deg).
    pub kappa: f64,
    /// Test statistic against the null (positive: towards "cheat").
    pub z: f64,
    /// Signed generalised log-likelihood ratio, `sign(z)·z²/2`.
    pub score: f64,
    /// `Φ(z)`: how confidently the evidence exceeds the null.
    pub confidence: f64,
    /// Whether `score` reached the configured flag score.
    pub flagged: bool,
}

impl Evidence {
    fn from_moments(m: &Moments, null: Null, min_pairs: u64, start_ms: u64, end_ms: u64) -> Self {
        let span = (start_ms, end_ms);
        Self::from_central(m.n, m.n as f64, m.central(), null, min_pairs, span)
    }

    /// Evidence from `pairs` pairs with central moments (variance of x, variance of y,
    /// covariance) divided by `n`, the sample size the fit's degrees of freedom count
    /// (`pairs` for one group; fewer after removing group means).
    fn from_central(
        pairs: u64,
        n: f64,
        (vx, vy, cxy): (f64, f64, f64),
        null: Null,
        min_pairs: u64,
        (start_ms, end_ms): (u64, u64),
    ) -> Self {
        let mut e = Self {
            start_ms,
            end_ms,
            pairs,
            r: 0.0,
            slope: 0.0,
            slope_se: 0.0,
            residual_sd: 0.0,
            kappa: 0.0,
            z: 0.0,
            score: 0.0,
            confidence: 0.5,
            flagged: false,
        };
        if pairs < min_pairs.max(4) || n < 4.0 || vx <= 0.0 || vy <= 0.0 {
            return e;
        }
        let r = (cxy / libm::sqrt(vx * vy)).clamp(-0.999_999, 0.999_999);
        let slope = cxy / vx;
        let residual_var = (vy - slope * cxy).max(0.0) * n / (n - 2.0);
        let residual_sd = libm::sqrt(residual_var);
        e.r = r;
        e.slope = slope;
        e.slope_se = libm::sqrt(residual_var / (vx * n));
        e.residual_sd = residual_sd;
        e.kappa = if residual_sd > 0.0 {
            slope / residual_sd
        } else {
            f64::INFINITY
        };
        let root = libm::sqrt(n - 3.0);
        let flag = match null {
            Null::KappaBound { bound, flag } => {
                let k = bound * libm::sqrt(vx);
                let rho0 = k / libm::sqrt(1.0 + k * k);
                e.z = (libm::atanh(r) - libm::atanh(rho0.min(0.999_999))) * root;
                flag
            }
            Null::NoResponse { flag } => {
                e.z = -libm::atanh(r) * root;
                flag
            }
        };
        e.score = e.z.signum() * e.z * e.z / 2.0;
        e.confidence = normal_cdf(e.z);
        e.flagged = e.score >= flag;
        e
    }
}

/// Standard normal cumulative distribution.
fn normal_cdf(z: f64) -> f64 {
    0.5 * libm::erfc(-z / core::f64::consts::SQRT_2)
}

/// Every statistic over one span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Report {
    /// Fire-time drift correlation.
    pub error: Evidence,
    /// Per-frame step response.
    pub steps: Evidence,
    /// Fire-time drift change (flick). Neutral when not configured.
    pub change: Evidence,
    /// Per-shot spray response. Neutral when not configured.
    pub spray: Evidence,
    /// Shots seen.
    pub shots: u64,
    /// Engagements seen: flick targets, or spray bursts.
    pub engagements: u64,
    /// Shots whose reported view disagreed with the replay by more than 1e-6°.
    pub angle_mismatches: u64,
}

impl Report {
    /// Whether any statistic is flagged.
    #[must_use]
    pub fn flagged(&self) -> bool {
        self.error.flagged || self.steps.flagged || self.change.flagged || self.spray.flagged
    }

    /// Every statistic's evidence, with its name: `error`, `steps`, `change`, `spray`.
    #[must_use]
    pub fn statistics(&self) -> [(&'static str, &Evidence); 4] {
        [
            ("error", &self.error),
            ("steps", &self.steps),
            ("change", &self.change),
            ("spray", &self.spray),
        ]
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Span {
    error: Moments,
    steps: Moments,
    change: Moments,
    spray: Grouped,
    shots: u64,
    engagements: u64,
    angle_mismatches: u64,
}

impl Span {
    fn report(&self, config: &DetectorConfig, start_ms: u64, end_ms: u64) -> Report {
        let evidence =
            |m: &Moments, null| Evidence::from_moments(m, null, config.min_pairs, start_ms, end_ms);
        let kappa = |t: Option<KappaTest>| {
            let t = t.unwrap_or(KappaTest {
                kappa_bound: 0.0,
                flag_score: f64::INFINITY,
            });
            Null::KappaBound {
                bound: t.kappa_bound,
                flag: t.flag_score,
            }
        };
        Report {
            error: evidence(
                &self.error,
                Null::KappaBound {
                    bound: config.kappa_bound,
                    flag: config.error_flag_score,
                },
            ),
            steps: evidence(
                &self.steps,
                Null::NoResponse {
                    flag: config.steps_flag_score,
                },
            ),
            change: evidence(&self.change, kappa(config.change)),
            spray: self
                .spray
                .evidence(kappa(config.spray), config.min_pairs, (start_ms, end_ms)),
            shots: self.shots,
            engagements: self.engagements,
            angle_mismatches: self.angle_mismatches,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    Flick,
    Spray,
    Other,
}

/// Header fields the detector uses.
#[derive(Clone, Copy, Debug)]
struct Session {
    scenario: Scenario,
    deg_per_count: f64,
    probe_start_us: u64,
}

/// The raw step of one render frame.
#[derive(Clone, Copy, Debug)]
struct FrameStep {
    frame: u64,
    counts: [f64; 2],
    /// Sensitivity drift (multiplier - 1) at the frame's last move.
    drift: f64,
}

/// Streaming detector for one player's session under one epoch seed.
///
/// Feed telemetry in chunks with [`Detector::feed`]; read the running session evidence
/// with [`Detector::session`] and completed windows with [`Detector::windows`].
#[derive(Debug)]
pub struct Detector {
    config: DetectorConfig,
    probe: ProbeGenerator,
    session: Option<Session>,
    last_ts_us: u64,
    // Replay.
    view: [f64; 2],
    nominal: [f64; 2],
    offset_at_engagement: [f64; 2],
    burst_first: Option<([f64; 2], [f64; 2])>,
    burst_last: Option<([f64; 2], [f64; 2])>,
    // Drift change: nominal view and drift effect at the previous shot, the path-
    // weighted drift of the moves since then (sum of weight x drift, sum of weight),
    // and that average over the moves before the previous shot.
    segment_start: ([f64; 2], [f64; 2]),
    segment_drift: (f64, f64),
    previous_segment_drift: Option<f64>,
    // Per-shot spray: (burst shot, error) of the burst's previous shot, and the
    // macro-error regressor accumulated since then.
    spray_last: Option<(u32, [f64; 2])>,
    spray_drift: [f64; 2],
    // Step response.
    open_frame: Option<FrameStep>,
    previous_step: Option<FrameStep>,
    // Evidence.
    total: Span,
    window: Span,
    window_index: u64,
    windows: Vec<Report>,
}

impl Detector {
    /// A detector for the session seeded by `seed`, with the probe configuration the
    /// client used.
    ///
    /// # Errors
    /// [`DetectError::Config`] or [`DetectError::Probe`] for invalid configurations.
    pub fn new(
        seed: &EpochSeed,
        probe_config: &ProbeConfig,
        config: DetectorConfig,
    ) -> Result<Self, DetectError> {
        config.validate()?;
        let probe = ProbeGenerator::new(seed, probe_config).map_err(DetectError::Probe)?;
        Ok(Self {
            config,
            probe,
            session: None,
            last_ts_us: 0,
            view: [0.0, 0.0],
            nominal: [0.0, 0.0],
            offset_at_engagement: [0.0, 0.0],
            burst_first: None,
            burst_last: None,
            segment_start: ([0.0, 0.0], [0.0, 0.0]),
            segment_drift: (0.0, 0.0),
            previous_segment_drift: None,
            spray_last: None,
            spray_drift: [0.0, 0.0],
            open_frame: None,
            previous_step: None,
            total: Span::default(),
            window: Span::default(),
            window_index: 0,
            windows: Vec::new(),
        })
    }

    /// Evidence for the whole session so far.
    #[must_use]
    pub fn session(&self) -> Report {
        let end = self.elapsed_ms();
        self.total.report(&self.config, 0, end)
    }

    /// Evidence for every completed window, oldest first.
    #[must_use]
    pub fn windows(&self) -> &[Report] {
        &self.windows
    }

    /// Evidence for the window in progress.
    #[must_use]
    pub fn current_window(&self) -> Report {
        let start = self.window_index * self.config.window_ms;
        self.window
            .report(&self.config, start, self.elapsed_ms().max(start))
    }

    fn elapsed_ms(&self) -> u64 {
        self.session.map_or(0, |s| {
            self.last_ts_us.saturating_sub(s.probe_start_us) / 1_000
        })
    }

    /// Feeds the next chunk of records, in stream order.
    ///
    /// # Errors
    /// A missing or unsupported header, records out of order, or a timestamp going
    /// backwards. The detector keeps what it had before the bad record.
    pub fn feed(&mut self, records: &[Record]) -> Result<(), DetectError> {
        for record in records {
            self.feed_one(record)?;
        }
        Ok(())
    }

    fn feed_one(&mut self, record: &Record) -> Result<(), DetectError> {
        let Some(session) = self.session else {
            let Record::Header(h) = record else {
                return Err(DetectError::BadHeader);
            };
            if h.format != telemetry::FORMAT
                || h.version != telemetry::VERSION
                || !(h.deg_per_count.is_finite() && h.deg_per_count > 0.0)
            {
                return Err(DetectError::BadHeader);
            }
            self.session = Some(Session {
                scenario: match h.scenario.as_str() {
                    "flick" => Scenario::Flick,
                    "spray" => Scenario::Spray,
                    _ => Scenario::Other,
                },
                deg_per_count: h.deg_per_count,
                probe_start_us: h.probe_start_us,
            });
            self.last_ts_us = h.ts_us.max(h.probe_start_us);
            return Ok(());
        };
        if matches!(record, Record::Header(_)) {
            return Err(DetectError::OutOfOrder);
        }
        let ts = record.ts_us();
        if ts < self.last_ts_us || ts < session.probe_start_us {
            return Err(DetectError::BadTimestamp);
        }
        self.last_ts_us = ts;
        self.roll_window(ts, session);
        match record {
            Record::Header(_) => {}
            Record::Move(m) => {
                let k = self.multiplier(Stream::Sensitivity, ts, session);
                let delta = [-m.dx * session.deg_per_count, -m.dy * session.deg_per_count];
                self.apply(delta, k);
                let weight = libm::sqrt(delta[0] * delta[0] + delta[1] * delta[1]);
                self.segment_drift.0 += weight * (k - 1.0);
                self.segment_drift.1 += weight;
                self.track_step(m.frame, [m.dx, m.dy], k - 1.0);
            }
            Record::Recoil(r) => {
                let k = self.multiplier(Stream::Recoil, ts, session);
                self.apply([r.kick_yaw, r.kick_pitch], k);
                if self.config.spray.is_some() {
                    let ks = self.multiplier(Stream::Sensitivity, ts, session);
                    self.spray_drift[0] += r.kick_yaw * (ks - k);
                    self.spray_drift[1] += r.kick_pitch * (ks - k);
                }
            }
            Record::Target(_) => {
                self.close_burst(session);
                self.close_frame();
                self.previous_step = None;
                self.offset_at_engagement = self.offset();
                if session.scenario == Scenario::Flick {
                    self.count_engagement();
                }
            }
            Record::Button(b) => {
                if !b.pressed {
                    self.close_burst(session);
                }
            }
            Record::Fire(f) => self.fire(f, session),
            Record::End(_) => {
                self.close_burst(session);
                self.close_frame();
            }
        }
        Ok(())
    }

    fn multiplier(&mut self, stream: Stream, ts_us: u64, session: Session) -> f64 {
        let tick = telemetry::probe_tick(ts_us, session.probe_start_us);
        self.probe.drift(stream, tick).multiplier()
    }

    fn apply(&mut self, delta: [f64; 2], k: f64) {
        const LIMIT: f64 = 89.0;
        self.view[0] += delta[0] * k;
        self.view[1] = (self.view[1] + delta[1] * k).clamp(-LIMIT, LIMIT);
        self.nominal[0] += delta[0];
        self.nominal[1] = (self.nominal[1] + delta[1]).clamp(-LIMIT, LIMIT);
    }

    /// `nominal - view`: the drift's cumulative effect on the aim error.
    fn offset(&self) -> [f64; 2] {
        [
            self.nominal[0] - self.view[0],
            self.nominal[1] - self.view[1],
        ]
    }

    fn roll_window(&mut self, ts_us: u64, session: Session) {
        let index = (ts_us - session.probe_start_us) / 1_000 / self.config.window_ms;
        while index > self.window_index {
            let start = self.window_index * self.config.window_ms;
            let report = self
                .window
                .report(&self.config, start, start + self.config.window_ms);
            self.windows.push(report);
            self.window = Span::default();
            self.window_index += 1;
        }
    }

    fn count_engagement(&mut self) {
        self.total.engagements += 1;
        self.window.engagements += 1;
    }

    fn add_error_pair(&mut self, drift: [f64; 2], error: [f64; 2]) {
        for axis in 0..2 {
            self.total.error.add(drift[axis], error[axis]);
            self.window.error.add(drift[axis], error[axis]);
        }
    }

    fn fire(&mut self, f: &telemetry::Fire, session: Session) {
        self.total.shots += 1;
        self.window.shots += 1;
        if (f.yaw - self.view[0]).abs() > 1e-6 || (f.pitch - self.view[1]).abs() > 1e-6 {
            self.total.angle_mismatches += 1;
            self.window.angle_mismatches += 1;
        }
        let error = [f.target_yaw - self.view[0], f.target_pitch - self.view[1]];
        let offset = self.offset();
        match session.scenario {
            Scenario::Flick => {
                let drift = [
                    offset[0] - self.offset_at_engagement[0],
                    offset[1] - self.offset_at_engagement[1],
                ];
                self.add_error_pair(drift, error);
                if self.config.change.is_some() {
                    self.drift_change(error, offset);
                }
            }
            Scenario::Spray => {
                if f.burst_shot == 0 {
                    self.close_burst(session);
                    self.burst_first = Some((error, offset));
                    self.count_engagement();
                }
                self.burst_last = Some((error, offset));
                if self.config.spray.is_some() {
                    if let Some((shot, e0)) = self.spray_last
                        && shot + 1 == f.burst_shot
                    {
                        let group = f.burst_shot as usize;
                        for axis in 0..2 {
                            let x = self.spray_drift[axis];
                            let y = error[axis] - e0[axis];
                            self.total.spray.add(axis, group, x, y);
                            self.window.spray.add(axis, group, x, y);
                        }
                    }
                    self.spray_last = Some((f.burst_shot, error));
                    self.spray_drift = [0.0, 0.0];
                }
            }
            Scenario::Other => {}
        }
    }

    /// One drift-change pair per axis for this shot, then starts the next segment.
    fn drift_change(&mut self, error: [f64; 2], offset: [f64; 2]) {
        let (start_nominal, start_offset) = self.segment_start;
        if let Some(previous) = self.previous_segment_drift {
            for axis in 0..2 {
                let nominal = self.nominal[axis] - start_nominal[axis];
                let effect = offset[axis] - start_offset[axis];
                let x = effect + nominal * previous;
                self.total.change.add(x, error[axis]);
                self.window.change.add(x, error[axis]);
            }
        }
        let (sum, weight) = self.segment_drift;
        if weight > 0.0 {
            self.previous_segment_drift = Some(sum / weight);
        }
        self.segment_start = (self.nominal, offset);
        self.segment_drift = (0.0, 0.0);
    }

    /// One pair per spray burst: change of error against change of drift effect.
    fn close_burst(&mut self, session: Session) {
        if session.scenario != Scenario::Spray {
            return;
        }
        if let (Some((e0, g0)), Some((e1, g1))) = (self.burst_first, self.burst_last)
            && (g1 != g0 || e1 != e0)
        {
            self.add_error_pair(
                [g1[0] - g0[0], g1[1] - g0[1]],
                [e1[0] - e0[0], e1[1] - e0[1]],
            );
        }
        self.burst_first = None;
        self.burst_last = None;
        self.spray_last = None;
        self.spray_drift = [0.0, 0.0];
    }

    fn track_step(&mut self, frame: u64, counts: [f64; 2], drift: f64) {
        match &mut self.open_frame {
            Some(open) if open.frame == frame => {
                open.counts[0] += counts[0];
                open.counts[1] += counts[1];
                open.drift = drift;
            }
            _ => {
                self.close_frame();
                self.open_frame = Some(FrameStep {
                    frame,
                    counts,
                    drift,
                });
            }
        }
    }

    fn close_frame(&mut self) {
        let Some(step) = self.open_frame.take() else {
            return;
        };
        let length = libm::sqrt(step.counts[0] * step.counts[0] + step.counts[1] * step.counts[1]);
        let long_enough = length >= self.config.min_step_counts;
        if let Some(prev) = self.previous_step
            && long_enough
            && prev.frame + 1 == step.frame
        {
            let prev_length =
                libm::sqrt(prev.counts[0] * prev.counts[0] + prev.counts[1] * prev.counts[1]);
            let ratio = libm::log(length / prev_length);
            self.total.steps.add(prev.drift, ratio);
            self.window.steps.add(prev.drift, ratio);
        }
        self.previous_step = long_enough.then_some(step);
    }
}

#[cfg(test)]
mod tests;
