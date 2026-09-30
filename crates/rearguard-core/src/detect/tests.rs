use proptest::prelude::*;

use super::*;
use crate::probe::{Amplitude, RootSeed};
use crate::telemetry::{Button, End, Fire, Header, Move, Target};

const DPC: f64 = 0.022;

fn config() -> DetectorConfig {
    DetectorConfig {
        kappa_bound: 2.0,
        error_flag_score: 12.0,
        steps_flag_score: 12.0,
        min_pairs: 20,
        window_ms: 10_000,
        min_step_counts: 4.0,
    }
}

fn seed(player: u64) -> EpochSeed {
    RootSeed::from_bytes(&mut [5; 32])
        .match_key(b"detect-test")
        .player_key(player)
        .epoch_seed(0)
}

fn probe_config(ppm: u32) -> ProbeConfig {
    ProbeConfig {
        amplitude: Amplitude::from_ppm(ppm).unwrap(),
        ..ProbeConfig::default()
    }
}

/// A tiny deterministic generator for test noise (xorshift64*, then Box–Muller).
struct Noise(u64);

impl Noise {
    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }

    fn normal(&mut self) -> f64 {
        let u1 = 1.0 - self.uniform();
        let u2 = self.uniform();
        libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(core::f64::consts::TAU * u2)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Player {
    /// Flicks by the nominal counts and fires: error carries the drift's effect.
    OpenLoop,
    /// Flicks, then corrects from the view it actually got, then fires.
    ClosedLoop,
}

/// Flick telemetry: `targets` targets 150 ms apart, aimed at a random point on the
/// target (`noise_deg` per axis), under the drift of `seed(1)` at `ppm`.
fn flick_session(
    player: Player,
    targets: u64,
    ppm: u32,
    noise_deg: f64,
    noise_seed: u64,
) -> Vec<Record> {
    let mut probe = ProbeGenerator::new(&seed(1), &probe_config(ppm)).unwrap();
    let mut noise = Noise(noise_seed | 1);
    let mut records = vec![Record::Header(Header {
        ts_us: 0,
        frame: 0,
        tick: 0,
        format: telemetry::FORMAT.to_owned(),
        version: telemetry::VERSION,
        match_id: "t".to_owned(),
        player_id: 1,
        source: "sim".to_owned(),
        probe_start_us: 0,
        deg_per_count: DPC,
        physics_hz: 120,
        scenario: "flick".to_owned(),
        scenario_seed: 0,
        duration_s: 0.0,
        recoil_pattern: "v1".to_owned(),
        target_distance_m: 10.0,
        target_radius_m: 0.3,
    })];
    let mut view = [0.0f64, 0.0];
    let mut frame = 0;
    for i in 0..targets {
        let ts = i * 150_000 + 1_000;
        let target = [noise.uniform() * 80.0 - 40.0, noise.uniform() * 30.0 - 10.0];
        records.push(Record::Target(Target {
            ts_us: ts,
            frame,
            tick: 0,
            target: i,
            target_yaw: target[0],
            target_pitch: target[1],
        }));
        let aim = [
            target[0] + noise_deg * noise.normal(),
            target[1] + noise_deg * noise.normal(),
        ];
        let mut moves = vec![(ts + 10_000, aim)];
        if player == Player::ClosedLoop {
            moves.push((ts + 30_000, aim));
        }
        for (t, goal) in moves {
            frame += 1;
            let m = probe.drift(Stream::Sensitivity, t / 1_000).multiplier();
            let dx = (-(goal[0] - view[0]) / DPC).round();
            let dy = (-(goal[1] - view[1]) / DPC).round();
            view[0] += -dx * DPC * m;
            view[1] = (view[1] - dy * DPC * m).clamp(-89.0, 89.0);
            records.push(Record::Move(Move {
                ts_us: t,
                frame,
                tick: 0,
                dx,
                dy,
                yaw: view[0],
                pitch: view[1],
            }));
        }
        let t = ts + 40_000;
        records.push(Record::Button(Button {
            ts_us: t,
            frame,
            tick: 0,
            pressed: true,
        }));
        records.push(Record::Fire(Fire {
            ts_us: t,
            frame,
            tick: 0,
            shot: i,
            burst_shot: 0,
            yaw: view[0],
            pitch: view[1],
            target: i,
            target_yaw: target[0],
            target_pitch: target[1],
            hit: true,
        }));
        records.push(Record::Button(Button {
            ts_us: t + 1,
            frame,
            tick: 0,
            pressed: false,
        }));
    }
    let end = targets * 150_000 + 1_000;
    records.push(Record::End(End {
        ts_us: end,
        frame,
        tick: 0,
        shots: targets,
        hits: targets,
        complete: true,
    }));
    records
}

fn run(records: &[Record], seed_player: u64, ppm: u32) -> Detector {
    let mut d = Detector::new(&seed(seed_player), &probe_config(ppm), config()).unwrap();
    d.feed(records).unwrap();
    d
}

// Acceptance criterion 3: a planted correlation is detected, a null is not.

#[test]
fn planted_open_loop_correlation_is_detected() {
    // Aim noise 0.1° per axis: kappa about 10/deg, five times the bound.
    let records = flick_session(Player::OpenLoop, 800, 5_000, 0.1, 7);
    let report = run(&records, 1, 5_000).session();
    assert_eq!(report.shots, 800);
    assert_eq!(report.engagements, 800);
    assert_eq!(report.angle_mismatches, 0);
    assert!(report.error.flagged, "{:?}", report.error);
    // The whole drift effect is in the error: slope close to 1.
    assert!((report.error.slope - 1.0).abs() < 0.1, "{:?}", report.error);
    assert!(report.error.confidence > 0.999);
}

#[test]
fn closed_loop_null_is_not_detected() {
    let records = flick_session(Player::ClosedLoop, 400, 5_000, 0.2, 7);
    let report = run(&records, 1, 5_000).session();
    assert!(!report.error.flagged, "{:?}", report.error);
    assert!(report.error.score < 0.0, "{:?}", report.error);
    assert!(report.error.slope.abs() < 0.1, "{:?}", report.error);
}

#[test]
fn open_loop_is_not_detected_under_the_wrong_seed() {
    let records = flick_session(Player::OpenLoop, 400, 5_000, 0.2, 7);
    let report = run(&records, 2, 5_000).session();
    assert!(!report.error.flagged, "{:?}", report.error);
    assert!(report.error.r.abs() < 0.15, "{:?}", report.error);
}

#[test]
fn no_drift_means_no_evidence() {
    let records = flick_session(Player::OpenLoop, 200, 0, 0.2, 7);
    let report = run(&records, 1, 0).session();
    assert_eq!(report.error.score, 0.0);
    assert!(!report.flagged());
}

#[test]
fn chunked_feeding_matches_one_feed_and_windows_partition_the_session() {
    let records = flick_session(Player::OpenLoop, 300, 5_000, 0.2, 9);
    let whole = run(&records, 1, 5_000);
    let mut chunked = Detector::new(&seed(1), &probe_config(5_000), config()).unwrap();
    for chunk in records.chunks(7) {
        chunked.feed(chunk).unwrap();
    }
    assert_eq!(whole.session(), chunked.session());
    assert_eq!(whole.windows(), chunked.windows());
    // 300 targets x 150 ms = 45 s: four full 10 s windows, the fifth in progress.
    assert_eq!(whole.windows().len(), 4);
    let pairs: u64 = whole.windows().iter().map(|w| w.error.pairs).sum::<u64>()
        + whole.current_window().error.pairs;
    assert_eq!(pairs, whole.session().error.pairs);
    assert!(
        whole
            .windows()
            .iter()
            .all(|w| w.error.end_ms - w.error.start_ms == 10_000)
    );
}

#[test]
fn rejects_bad_streams_and_configs() {
    let records = flick_session(Player::OpenLoop, 5, 5_000, 0.2, 1);
    let mut d = Detector::new(&seed(1), &probe_config(5_000), config()).unwrap();
    assert_eq!(d.feed(&records[1..2]), Err(DetectError::BadHeader));
    let mut d = Detector::new(&seed(1), &probe_config(5_000), config()).unwrap();
    d.feed(&records[..3]).unwrap();
    assert_eq!(d.feed(&records[..1]), Err(DetectError::OutOfOrder));
    let mut backwards = records.clone();
    backwards.swap(2, 3);
    let mut d = Detector::new(&seed(1), &probe_config(5_000), config()).unwrap();
    assert_eq!(d.feed(&backwards), Err(DetectError::BadTimestamp));
    for bad in [
        DetectorConfig {
            min_pairs: 2,
            ..config()
        },
        DetectorConfig {
            window_ms: 0,
            ..config()
        },
        DetectorConfig {
            kappa_bound: f64::NAN,
            ..config()
        },
        DetectorConfig {
            min_step_counts: 0.5,
            ..config()
        },
    ] {
        assert!(Detector::new(&seed(1), &probe_config(5_000), bad).is_err());
    }
}

#[test]
fn config_is_read_from_json_and_has_no_defaults() {
    let json = r#"{"kappa_bound":2.0,"error_flag_score":12.0,"steps_flag_score":12.0,
        "min_pairs":20,"window_ms":10000,"min_step_counts":4.0}"#;
    let parsed: DetectorConfig = serde_json::from_str(json).unwrap();
    assert_eq!(parsed, config());
    // Every field is required, and unknown fields are refused.
    assert!(serde_json::from_str::<DetectorConfig>(r#"{"kappa_bound":2.0}"#).is_err());
    let extra = json.replace("\"min_pairs\"", "\"extra\":1,\"min_pairs\"");
    assert!(serde_json::from_str::<DetectorConfig>(&extra).is_err());
}

#[test]
fn step_response_sees_a_one_frame_proportional_controller() {
    // A smoothing controller: every frame, 20% of the remaining error, from the view
    // it really has. The step ratio is 1 - 0.2 x multiplier, so it falls as the drift
    // rises. A controller whose steps ignore the view (fixed decay) is the null.
    for (reads_view, expect) in [(true, true), (false, false)] {
        let mut probe = ProbeGenerator::new(&seed(1), &probe_config(20_000)).unwrap();
        let mut records = flick_session(Player::OpenLoop, 0, 20_000, 0.0, 1);
        records.pop();
        let mut view = [0.0f64, 0.0];
        let mut frame = 0u64;
        let mut ts = 1_000u64;
        for i in 0..400u64 {
            let target = [if i % 2 == 0 { 30.0 } else { -30.0 }, 5.0];
            records.push(Record::Target(Target {
                ts_us: ts,
                frame,
                tick: 0,
                target: i,
                target_yaw: target[0],
                target_pitch: target[1],
            }));
            let mut planned = [target[0] - view[0], target[1] - view[1]];
            for _ in 0..25 {
                frame += 1;
                ts += 8_333;
                let error = if reads_view {
                    [target[0] - view[0], target[1] - view[1]]
                } else {
                    planned
                };
                let step = [0.2 * error[0], 0.2 * error[1]];
                planned = [planned[0] * 0.8, planned[1] * 0.8];
                let (dx, dy) = ((-step[0] / DPC).round(), (-step[1] / DPC).round());
                let m = probe.drift(Stream::Sensitivity, ts / 1_000).multiplier();
                view[0] += -dx * DPC * m;
                view[1] += -dy * DPC * m;
                records.push(Record::Move(Move {
                    ts_us: ts,
                    frame,
                    tick: 0,
                    dx,
                    dy,
                    yaw: view[0],
                    pitch: view[1],
                }));
            }
            ts += 8_333;
            frame += 2;
        }
        let report = run(&records, 1, 20_000).session();
        assert_eq!(
            report.steps.flagged, expect,
            "reads view {reads_view}: {:?}",
            report.steps
        );
    }
}

// Acceptance criterion 4, at the level of the statistic: evidence grows with the drift
// amplitude and with observation length, for players beyond the null. The planted
// open-loop player has slope 1, so kappa = 1 / aim noise; noise up to 0.25° keeps
// kappa >= 4/deg, at least twice the bound of 2. (A player at the bound gives no
// growing evidence by design: that is the null.) Tolerance: z has roughly unit
// standard deviation from sampling noise, so a fall of more than 3.0 in z would be a
// real violation rather than noise; the largest setting must also beat the smallest.

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn evidence_grows_with_amplitude(noise_deg in 0.05f64..0.25, noise_seed in any::<u64>()) {
        let z = |ppm| {
            let records = flick_session(Player::OpenLoop, 200, ppm, noise_deg, noise_seed);
            run(&records, 1, ppm).session().error.z
        };
        let zs = [z(2_500), z(5_000), z(10_000), z(20_000)];
        for pair in zs.windows(2) {
            prop_assert!(pair[1] >= pair[0] - 3.0, "{zs:?}");
        }
        prop_assert!(zs[3] > zs[0], "{zs:?}");
    }

    #[test]
    fn evidence_grows_with_observation_time(noise_deg in 0.05f64..0.25, noise_seed in any::<u64>()) {
        let records = flick_session(Player::OpenLoop, 800, 5_000, noise_deg, noise_seed);
        let mut d = Detector::new(&seed(1), &probe_config(5_000), config()).unwrap();
        let mut zs = Vec::new();
        let mut fed = 0;
        for targets in [100u64, 200, 400, 800] {
            // Feed up to the End of the targets-th engagement.
            let upto = records
                .iter()
                .position(|r| matches!(r, Record::Target(t) if t.target == targets))
                .unwrap_or(records.len());
            d.feed(&records[fed..upto]).unwrap();
            fed = upto;
            zs.push(d.session().error.z);
        }
        for pair in zs.windows(2) {
            prop_assert!(pair[1] >= pair[0] - 3.0, "{zs:?}");
        }
        prop_assert!(zs[3] > zs[0], "{zs:?}");
    }
}
