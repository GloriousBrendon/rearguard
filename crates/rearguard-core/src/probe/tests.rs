// SPDX-License-Identifier: MIT OR Apache-2.0

use core::num::{NonZeroU32, NonZeroU64};

use proptest::prelude::*;

use super::*;

/// Root seed for golden vectors and statistical tests: public test data, not a secret.
fn test_root() -> RootSeed {
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = i as u8;
    }
    RootSeed::from_bytes(&mut bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn config(shape: SignalShape, amplitude: Amplitude) -> ProbeConfig {
    ProbeConfig {
        amplitude,
        shape,
        epochs: EpochSchedule::SINGLE,
    }
}

// ---------------------------------------------------------------------------
// Golden vectors (acceptance criterion 1)
//
// Computed once and committed. They must pass unchanged on every platform (CI runs them
// on Linux and Windows). A change here is a protocol change: bump the KDF salt, and
// regenerate with `cargo test -p rearguard-core print_golden_vectors -- --ignored
// --nocapture`.
// ---------------------------------------------------------------------------

const GOLDEN_MATCH: &[u8] = b"rearguard-golden";
const GOLDEN_PLAYER: u64 = 42;

const GOLDEN_TICKS: [u64; 11] = [
    0,
    1,
    250,
    499,
    500,
    777,
    12_345,
    1_000_000,
    86_400_000,
    1 << 40,
    u64::MAX,
];

fn golden_cases() -> [(&'static str, ProbeConfig); 4] {
    [
        (
            "noise-default",
            config(SignalShape::DEFAULT_NOISE, Amplitude::DEFAULT),
        ),
        (
            "noise-fast-max",
            config(
                SignalShape::BandLimitedNoise {
                    knot_interval_ticks: NonZeroU32::new(7).unwrap(),
                },
                Amplitude::MAX,
            ),
        ),
        (
            "sines-default",
            config(SignalShape::DEFAULT_SINUSOIDS, Amplitude::DEFAULT),
        ),
        (
            "sines-wide-max",
            config(
                SignalShape::SinusoidSum {
                    components: MAX_SINUSOIDS,
                    min_period_ticks: 2,
                    max_period_ticks: 1_000_000,
                },
                Amplitude::MAX,
            ),
        ),
    ]
}

/// Every golden row as (case, stream, tick, ppb, multiplier bits).
fn compute_golden_rows() -> Vec<(&'static str, Stream, u64, i32, u64)> {
    let seed = test_root()
        .match_key(GOLDEN_MATCH)
        .player_key(GOLDEN_PLAYER)
        .epoch_seed(0);
    let mut rows = Vec::new();
    for (name, config) in golden_cases() {
        let mut probe = ProbeGenerator::new(&seed, &config).unwrap();
        for stream in [Stream::Sensitivity, Stream::Recoil] {
            for tick in GOLDEN_TICKS {
                let drift = probe.drift(stream, tick);
                rows.push((
                    name,
                    stream,
                    tick,
                    drift.ppb(),
                    drift.multiplier().to_bits(),
                ));
            }
        }
    }
    rows
}

#[test]
#[ignore = "prints the golden tables; run by hand after an intentional format change"]
fn print_golden_vectors() {
    let player = test_root()
        .match_key(GOLDEN_MATCH)
        .player_key(GOLDEN_PLAYER);
    println!("const GOLDEN_EPOCH_SEEDS: [&str; 2] = [");
    for epoch in 0..2 {
        println!("    \"{}\",", hex(player.epoch_seed(epoch).expose_secret()));
    }
    println!("];\n");
    println!("const GOLDEN_DRIFT: &[(&str, Stream, u64, i32, u64)] = &[");
    for (name, stream, tick, ppb, bits) in compute_golden_rows() {
        println!("    (\"{name}\", Stream::{stream:?}, {tick}, {ppb}, {bits:#018x}),");
    }
    println!("];");
}

#[test]
fn golden_epoch_seeds() {
    let player = test_root()
        .match_key(GOLDEN_MATCH)
        .player_key(GOLDEN_PLAYER);
    for (epoch, expected) in GOLDEN_EPOCH_SEEDS.iter().enumerate() {
        let seed = player.epoch_seed(epoch as u64);
        assert_eq!(hex(seed.expose_secret()), *expected, "epoch {epoch}");
    }
}

#[test]
fn golden_drift() {
    let rows = compute_golden_rows();
    assert_eq!(rows.len(), GOLDEN_DRIFT.len());
    for (row, expected) in rows.iter().zip(GOLDEN_DRIFT) {
        assert_eq!(row, expected);
    }
}

#[rustfmt::skip]
const GOLDEN_EPOCH_SEEDS: [&str; 2] = [
    "90a6e8a8811eec0eea229215d0c8442573f50f9969b7ab748288ba03f90ad4f8",
    "03cf6a14013a4a77b036d6efe8b4203e506dd54ff184df4780c9f01c4cd95697",
];
const GOLDEN_DRIFT: &[(&str, Stream, u64, i32, u64)] = &[
    (
        "noise-default",
        Stream::Sensitivity,
        0,
        3547080,
        0x3ff00e8762098a6d,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        1,
        3547545,
        0x3ff00e87dedc18f0,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        250,
        2763582,
        0x3ff00b51d364f500,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        499,
        829123,
        0x3ff003656602b5b4,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        500,
        820328,
        0x3ff0035c2d1ee972,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        777,
        -1438218,
        0x3feff437d6973725,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        12345,
        -2624328,
        0x3fefea8062a21cdc,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        1000000,
        2202782,
        0x3ff00905c8ca674a,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        86400000,
        2294701,
        0x3ff009662b1bfc4f,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        1099511627776,
        -994461,
        0x3feff7da76d0e42b,
    ),
    (
        "noise-default",
        Stream::Sensitivity,
        18446744073709551615,
        3416247,
        0x3ff00dfe31d23dae,
    ),
    (
        "noise-default",
        Stream::Recoil,
        0,
        2520102,
        0x3ff00a5284bac2e8,
    ),
    (
        "noise-default",
        Stream::Recoil,
        1,
        2527520,
        0x3ff00a5a4bfbd6fb,
    ),
    (
        "noise-default",
        Stream::Recoil,
        250,
        3753131,
        0x3ff00f5f716e7111,
    ),
    (
        "noise-default",
        Stream::Recoil,
        499,
        3798743,
        0x3ff00f8f454f36ec,
    ),
    (
        "noise-default",
        Stream::Recoil,
        500,
        3796664,
        0x3ff00f8d173b6c23,
    ),
    (
        "noise-default",
        Stream::Recoil,
        777,
        2678810,
        0x3ff00af8ef9579a4,
    ),
    (
        "noise-default",
        Stream::Recoil,
        12345,
        -654533,
        0x3feffaa358457795,
    ),
    (
        "noise-default",
        Stream::Recoil,
        1000000,
        -934104,
        0x3feff8590abbce55,
    ),
    (
        "noise-default",
        Stream::Recoil,
        86400000,
        1484379,
        0x3ff006147bf4286d,
    ),
    (
        "noise-default",
        Stream::Recoil,
        1099511627776,
        4201896,
        0x3ff0113601de6b4a,
    ),
    (
        "noise-default",
        Stream::Recoil,
        18446744073709551615,
        2716211,
        0x3ff00b2027569fe4,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        0,
        14188323,
        0x3ff03a1d88f45244,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        1,
        13993025,
        0x3ff03950c00bf42a,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        250,
        -60002,
        0x3fefff822aabe9ed,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        499,
        14806445,
        0x3ff03ca5aed0b86a,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        500,
        15739542,
        0x3ff040781b224de6,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        777,
        2007518,
        0x3ff0083909027f84,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        12345,
        -2278692,
        0x3fefed553c8c3c28,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        1000000,
        -5026057,
        0x3fefd6d39831d452,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        86400000,
        8211527,
        0x3ff021a268fea4bf,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        1099511627776,
        -3941316,
        0x3fefdfb67615a855,
    ),
    (
        "noise-fast-max",
        Stream::Sensitivity,
        18446744073709551615,
        -6491016,
        0x3fefcad35a51fd70,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        0,
        10080411,
        0x3ff0294a13b9342d,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        1,
        11995580,
        0x3ff0312246fcbead,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        250,
        -4000574,
        0x3fefdf3a30302718,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        499,
        -3248955,
        0x3fefe5627290ec21,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        500,
        -722271,
        0x3feffa1549b5a307,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        777,
        -13627435,
        0x3fef905d328b17a1,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        12345,
        -634497,
        0x3feffacd5d0456c1,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        1000000,
        -7634369,
        0x3fefc175915a388e,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        86400000,
        1614980,
        0x3ff0069d6de48a03,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        1099511627776,
        242114,
        0x3ff000fddffb63f5,
    ),
    (
        "noise-fast-max",
        Stream::Recoil,
        18446744073709551615,
        -9449962,
        0x3fefb295fe485ead,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        0,
        3125144,
        0x3ff00cccf3746797,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        1,
        3123034,
        0x3ff00ccabd0e4f09,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        250,
        2297099,
        0x3ff00968aed14a71,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        499,
        1184398,
        0x3ff004d9ee6acea2,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        500,
        1179990,
        0x3ff004d54f275a8d,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        777,
        144752,
        0x3ff00097c891b24d,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        12345,
        3426102,
        0x3ff00e088740af25,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        1000000,
        244917,
        0x3ff00100d0681571,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        86400000,
        -37441,
        0x3fefffb17b04249f,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        1099511627776,
        2227796,
        0x3ff00920036f6501,
    ),
    (
        "sines-default",
        Stream::Sensitivity,
        18446744073709551615,
        3127242,
        0x3ff00ccf26a1dde9,
    ),
    (
        "sines-default",
        Stream::Recoil,
        0,
        -1394108,
        0x3feff49457f773fc,
    ),
    (
        "sines-default",
        Stream::Recoil,
        1,
        -1401668,
        0x3feff4847d38f6fe,
    ),
    (
        "sines-default",
        Stream::Recoil,
        250,
        -2746888,
        0x3fefe97f5bbbf9a7,
    ),
    (
        "sines-default",
        Stream::Recoil,
        499,
        -2676582,
        0x3fefea12ccfb09bb,
    ),
    (
        "sines-default",
        Stream::Recoil,
        500,
        -2673428,
        0x3fefea196a457f4c,
    ),
    (
        "sines-default",
        Stream::Recoil,
        777,
        -1223362,
        0x3feff5fa6c8700aa,
    ),
    (
        "sines-default",
        Stream::Recoil,
        12345,
        496661,
        0x3ff00208c96c0901,
    ),
    (
        "sines-default",
        Stream::Recoil,
        1000000,
        3865360,
        0x3ff00fd51fac98a5,
    ),
    (
        "sines-default",
        Stream::Recoil,
        86400000,
        492318,
        0x3ff002043b9b5904,
    ),
    (
        "sines-default",
        Stream::Recoil,
        1099511627776,
        -4299703,
        0x3fefdcc6de877088,
    ),
    (
        "sines-default",
        Stream::Recoil,
        18446744073709551615,
        -1386537,
        0x3feff4a4389dc511,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        0,
        7556004,
        0x3ff01ef30b61185a,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        1,
        6074007,
        0x3ff018e10ed6bb16,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        250,
        -7392851,
        0x3fefc370115762b5,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        499,
        -1668750,
        0x3feff25460aa64c3,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        500,
        4093938,
        0x3ff010c4ce1d264e,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        777,
        4054794,
        0x3ff0109bc279f3c8,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        12345,
        1827559,
        0x3ff0077c55a22f26,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        1000000,
        -2343800,
        0x3fefecccb1f4da32,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        86400000,
        2364106,
        0x3ff009aef1df44b8,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        1099511627776,
        -8374221,
        0x3fefbb65fc559de4,
    ),
    (
        "sines-wide-max",
        Stream::Sensitivity,
        18446744073709551615,
        2995539,
        0x3ff00c450ce09f4e,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        0,
        -7262455,
        0x3fefc4818729299a,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        1,
        -45745,
        0x3fefffa010d712cc,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        250,
        272144,
        0x3ff0011d5d1946df,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        499,
        -9839201,
        0x3fefaf65b32f8f31,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        500,
        1930431,
        0x3ff007e8342031e8,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        777,
        8486713,
        0x3ff022c2f6ac9190,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        12345,
        -14665385,
        0x3fef87dc75615606,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        1000000,
        255488,
        0x3ff0010be609ac1e,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        86400000,
        -248268,
        0x3feffdf75821e640,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        1099511627776,
        -7876621,
        0x3fefbf79874cdd4b,
    ),
    (
        "sines-wide-max",
        Stream::Recoil,
        18446744073709551615,
        3095515,
        0x3ff00cade1fb0748,
    ),
];

// ---------------------------------------------------------------------------
// Determinism and API behaviour
// ---------------------------------------------------------------------------

#[test]
fn output_does_not_depend_on_call_order() {
    let seed = test_root().match_key(b"m").player_key(1).epoch_seed(0);
    let ticks: Vec<u64> = (0..2_000u64).map(|i| i * 37).collect();
    for shape in [SignalShape::DEFAULT_NOISE, SignalShape::DEFAULT_SINUSOIDS] {
        let config = config(shape, Amplitude::DEFAULT);
        let mut forward = ProbeGenerator::new(&seed, &config).unwrap();
        let mut backward = ProbeGenerator::new(&seed, &config).unwrap();
        let a: Vec<_> = ticks
            .iter()
            .map(|&t| forward.drift(Stream::Recoil, t))
            .collect();
        let mut b: Vec<_> = ticks
            .iter()
            .rev()
            .map(|&t| {
                // Interleave the other stream to disturb any cached state.
                backward.drift(Stream::Sensitivity, t ^ 0xFFFF);
                backward.drift(Stream::Recoil, t)
            })
            .collect();
        b.reverse();
        assert_eq!(a, b);
    }
}

#[test]
fn signal_actually_varies_and_uses_its_range() {
    let seed = test_root().match_key(b"m").player_key(1).epoch_seed(0);
    for shape in [SignalShape::DEFAULT_NOISE, SignalShape::DEFAULT_SINUSOIDS] {
        let mut probe = ProbeGenerator::new(&seed, &config(shape, Amplitude::DEFAULT)).unwrap();
        let peak = (0..600_000u64)
            .step_by(10)
            .map(|t| probe.drift(Stream::Sensitivity, t).ppb().unsigned_abs())
            .max()
            .unwrap();
        // Over ten minutes the peak should reach well past half the amplitude.
        assert!(peak > 2_500_000, "{shape:?}: peak {peak} ppb");
    }
}

#[test]
fn zero_amplitude_disables_the_probe() {
    let seed = test_root().match_key(b"m").player_key(1).epoch_seed(0);
    let config = config(SignalShape::DEFAULT_NOISE, Amplitude::from_ppm(0).unwrap());
    let mut probe = ProbeGenerator::new(&seed, &config).unwrap();
    for t in (0..10_000).step_by(97) {
        assert_eq!(probe.drift(Stream::Sensitivity, t).multiplier(), 1.0);
    }
}

#[test]
fn invalid_shapes_are_rejected() {
    let seed = test_root().match_key(b"m").player_key(1).epoch_seed(0);
    let sines = |components, min_period_ticks, max_period_ticks| {
        config(
            SignalShape::SinusoidSum {
                components,
                min_period_ticks,
                max_period_ticks,
            },
            Amplitude::DEFAULT,
        )
    };
    let bad = [
        sines(0, 100, 200),
        sines(MAX_SINUSOIDS + 1, 100, 200),
        sines(4, 1, 200),
        sines(4, 300, 200),
    ];
    for config in bad {
        assert!(ProbeGenerator::new(&seed, &config).is_err(), "{config:?}");
    }
    assert!(ProbeGenerator::new(&seed, &sines(1, 2, 2)).is_ok());
}

#[test]
fn epoch_schedule_locates_ticks() {
    assert_eq!(
        EpochSchedule::SINGLE.locate(u64::MAX),
        EpochTime {
            epoch: 0,
            tick: u64::MAX
        }
    );
    let minute = EpochSchedule::fixed(NonZeroU64::new(60_000).unwrap());
    assert_eq!(
        minute.locate(59_999),
        EpochTime {
            epoch: 0,
            tick: 59_999
        }
    );
    assert_eq!(minute.locate(60_000), EpochTime { epoch: 1, tick: 0 });
    assert_eq!(
        minute.locate(185_000),
        EpochTime {
            epoch: 3,
            tick: 5_000
        }
    );
    assert_eq!(ProbeConfig::default().epochs, EpochSchedule::SINGLE);
}

// ---------------------------------------------------------------------------
// Different seeds give uncorrelated signals (acceptance criterion 2)
//
// Test design. For the band-limited noise shape (the default), the value at a tick
// depends only on 4 consecutive knots. Sampling every 4 knot intervals therefore gives
// samples that share no knots: they are exactly independent within one signal, and
// signals from different seeds are independent of each other. Under that null
// hypothesis the Pearson correlation r of N sample pairs has mean 0 and standard
// deviation sigma = 1/sqrt(N) (N = 2000 gives sigma = 0.0224), and N*r^2 is roughly
// chi-squared with one degree of freedom.
//
// Thresholds, over M = 100 pairs of seeds:
// - every |r| < 5 sigma = 0.112 (chance of any pair failing by luck: about 6e-5);
// - mean r within 4 sigma / sqrt(M) = 0.0089 of zero (no systematic bias);
// - mean N*r^2 in [0.5, 1.5], i.e. within about 3.5 standard errors of 1 (the spread
//   is what independence predicts: neither inflated by hidden correlation nor
//   suspiciously small).
// A positive control checks the test would notice a correlated pair.
//
// The sinusoid shape is not tested this way: its samples are not independent and its
// cross-seed correlation is heavy-tailed (see docs/probe-signal-shapes.md).
// ---------------------------------------------------------------------------

const CORR_SAMPLES: usize = 2_000;
const CORR_PAIRS: u64 = 100;

fn noise_samples(seed: &EpochSeed, stream: Stream) -> Vec<f64> {
    let shape = SignalShape::DEFAULT_NOISE;
    let SignalShape::BandLimitedNoise {
        knot_interval_ticks,
    } = shape
    else {
        unreachable!()
    };
    let spacing = 4 * u64::from(knot_interval_ticks.get());
    let mut probe = ProbeGenerator::new(seed, &config(shape, Amplitude::MAX)).unwrap();
    (0..CORR_SAMPLES as u64)
        // Offset inside the segment so samples are not all on knots.
        .map(|i| f64::from(probe.drift(stream, i * spacing + 123).ppb()))
        .collect()
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len() as f64;
    let (ma, mb) = (a.iter().sum::<f64>() / n, b.iter().sum::<f64>() / n);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma) * (x - ma);
        sbb += (y - mb) * (y - mb);
    }
    sab / (saa * sbb).sqrt()
}

fn assert_uncorrelated(label: &str, pairs: &[(Vec<f64>, Vec<f64>)]) {
    let n = CORR_SAMPLES as f64;
    let sigma = 1.0 / n.sqrt();
    let rs: Vec<f64> = pairs.iter().map(|(a, b)| pearson(a, b)).collect();
    let m = rs.len() as f64;
    let max_abs = rs.iter().fold(0.0f64, |acc, r| acc.max(r.abs()));
    let mean = rs.iter().sum::<f64>() / m;
    let mean_scaled_square = rs.iter().map(|r| n * r * r).sum::<f64>() / m;
    assert!(max_abs < 5.0 * sigma, "{label}: max |r| {max_abs}");
    assert!(
        mean.abs() < 4.0 * sigma / m.sqrt(),
        "{label}: mean r {mean}"
    );
    assert!(
        (0.5..=1.5).contains(&mean_scaled_square),
        "{label}: mean N*r^2 {mean_scaled_square}"
    );
}

#[test]
fn different_players_are_uncorrelated() {
    let match_key = test_root().match_key(b"corr");
    let pairs: Vec<_> = (0..CORR_PAIRS)
        .map(|i| {
            let a = match_key.player_key(2 * i).epoch_seed(0);
            let b = match_key.player_key(2 * i + 1).epoch_seed(0);
            (
                noise_samples(&a, Stream::Sensitivity),
                noise_samples(&b, Stream::Sensitivity),
            )
        })
        .collect();
    assert_uncorrelated("players", &pairs);
}

#[test]
fn streams_and_epochs_of_one_player_are_uncorrelated() {
    let match_key = test_root().match_key(b"corr-2");
    let streams: Vec<_> = (0..CORR_PAIRS)
        .map(|i| {
            let seed = match_key.player_key(i).epoch_seed(0);
            (
                noise_samples(&seed, Stream::Sensitivity),
                noise_samples(&seed, Stream::Recoil),
            )
        })
        .collect();
    assert_uncorrelated("streams", &streams);

    let epochs: Vec<_> = (0..CORR_PAIRS)
        .map(|i| {
            let player = match_key.player_key(i);
            (
                noise_samples(&player.epoch_seed(0), Stream::Recoil),
                noise_samples(&player.epoch_seed(1), Stream::Recoil),
            )
        })
        .collect();
    assert_uncorrelated("epochs", &epochs);
}

#[test]
fn correlation_test_has_power() {
    // Positive control: a signal mixed half-and-half with another is correlated ~0.7.
    let match_key = test_root().match_key(b"control");
    let a = noise_samples(&match_key.player_key(0).epoch_seed(0), Stream::Sensitivity);
    let b = noise_samples(&match_key.player_key(1).epoch_seed(0), Stream::Sensitivity);
    let mixed: Vec<f64> = a.iter().zip(&b).map(|(x, y)| x + y).collect();
    let r = pearson(&a, &mixed);
    assert!(r > 0.6, "r {r}");
}

// ---------------------------------------------------------------------------
// Amplitude bound (acceptance criterion 3)
// ---------------------------------------------------------------------------

fn any_shape() -> impl Strategy<Value = SignalShape> {
    prop_oneof![
        (1u32..=1_000_000).prop_map(|k| SignalShape::BandLimitedNoise {
            knot_interval_ticks: NonZeroU32::new(k).unwrap(),
        }),
        (1..=MAX_SINUSOIDS, 2u32..=1_000_000, 0u32..=1_000_000).prop_map(|(n, min, extra)| {
            SignalShape::SinusoidSum {
                components: n,
                min_period_ticks: min,
                max_period_ticks: min.saturating_add(extra),
            }
        }),
    ]
}

proptest! {
    #[test]
    fn drift_never_exceeds_amplitude(
        seed_bytes in any::<[u8; 32]>(),
        ppm in 0..=Amplitude::MAX.ppm(),
        shape in any_shape(),
        start in any::<u64>(),
    ) {
        let mut bytes = seed_bytes;
        let seed = EpochSeed::from_bytes(&mut bytes);
        let amplitude = Amplitude::from_ppm(ppm).unwrap();
        let config = config(shape, amplitude);
        let mut probe = ProbeGenerator::new(&seed, &config).unwrap();
        let bound = i64::from(ppm) * 1_000;
        let a = f64::from(ppm) / 1e6;
        // A run of consecutive ticks, plus the extremes, from a random starting point.
        let ticks = (0..64).map(|i| start.wrapping_add(i)).chain([0, u64::MAX]);
        for tick in ticks {
            for stream in [Stream::Sensitivity, Stream::Recoil] {
                let drift = probe.drift(stream, tick);
                prop_assert!(i64::from(drift.ppb()).abs() <= bound);
                prop_assert!((drift.multiplier() - 1.0).abs() <= a + 1e-12);
            }
        }
    }

    #[test]
    fn amplitude_above_max_is_rejected(ppm in Amplitude::MAX.ppm() + 1..) {
        prop_assert_eq!(
            Amplitude::from_ppm(ppm),
            Err(ProbeError::AmplitudeTooLarge { ppm })
        );
    }
}

// ---------------------------------------------------------------------------
// Secrets never appear in Debug or Display output (acceptance criterion 4)
// ---------------------------------------------------------------------------

/// Renderings of `bytes` that would betray it in formatted output.
fn leak_patterns(bytes: &[u8; 32]) -> Vec<String> {
    vec![
        format!("{bytes:?}"),
        format!("{:?}", &bytes[..4])
            .trim_end_matches(']')
            .to_owned(),
        hex(bytes),
        hex(&bytes[..4]),
        hex(&bytes[..4]).to_uppercase(),
    ]
}

fn assert_no_leak(shown: &str, secrets: &[[u8; 32]]) {
    for secret in secrets {
        for pattern in leak_patterns(secret) {
            assert!(!shown.contains(&pattern), "{shown:?} contains {pattern:?}");
        }
    }
}

#[test]
fn secrets_never_appear_in_debug_or_display() {
    // Distinctive root bytes, so any leak would be unmistakable.
    let root_bytes = [0xC3u8; 32];
    let root = RootSeed::from_bytes(&mut root_bytes.clone());
    let match_key = root.match_key(b"leak");
    let player = match_key.player_key(9);
    let seed = player.epoch_seed(0);
    let mut secrets = vec![root_bytes, *seed.expose_secret()];
    for stream in [Stream::Sensitivity, Stream::Recoil] {
        for shape in [SignalShape::DEFAULT_NOISE, SignalShape::DEFAULT_SINUSOIDS] {
            secrets.push(*seed.stream_key(stream.tag(), shape.tag()).expose_secret());
        }
    }

    let mut shown = vec![
        format!("{root:?}"),
        format!("{root:#?}"),
        format!("{match_key:?}"),
        format!("{player:?}"),
        format!("{seed:?}"),
        format!("{seed:#?}"),
    ];
    for shape in [SignalShape::DEFAULT_NOISE, SignalShape::DEFAULT_SINUSOIDS] {
        let mut probe = ProbeGenerator::new(&seed, &config(shape, Amplitude::DEFAULT)).unwrap();
        probe.drift(Stream::Sensitivity, 1_234);
        shown.push(format!("{probe:?}"));
        shown.push(format!("{probe:#?}"));
    }
    let key = crate::secret::SecretKey::from_bytes(&mut root_bytes.clone());
    shown.push(format!("{key}"));
    shown.push(format!("{key:?}"));

    for text in &shown {
        assert!(text.contains("<redacted>"), "{text}");
        assert_no_leak(text, &secrets);
    }
}
