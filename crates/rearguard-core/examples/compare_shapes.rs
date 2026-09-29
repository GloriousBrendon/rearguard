//! Compares the two candidate signal shapes; the numbers behind
//! `docs/probe-signal-shapes.md`.
//!
//! `cargo run --release -p rearguard-core --example compare_shapes`
//!
//! For each shape, over a 10-minute match at 1 ms ticks sampled every 50 ms:
//! - RMS of the unit signal (peak bound is 1): the power available to a correlation
//!   detector for a given hard amplitude cap;
//! - fastest rate of change, in amplitude units per second (a perceptibility proxy);
//! - the distribution of |r| between signals of different players: what a detector
//!   sees by chance, which sets the false-positive floor;
//! - autocorrelation at a 5 s lag: how much the signal remembers its past.

use rearguard_core::probe::{
    Amplitude, EpochSchedule, ProbeConfig, ProbeGenerator, RootSeed, SignalShape, Stream,
};

const MATCH_TICKS: u64 = 600_000;
const STEP: u64 = 50;
const PAIRS: u64 = 1_000;

fn config(shape: SignalShape) -> ProbeConfig {
    ProbeConfig {
        amplitude: Amplitude::MAX,
        shape,
        epochs: EpochSchedule::SINGLE,
    }
}

fn unit_series(probe: &mut ProbeGenerator, start: u64) -> Vec<f64> {
    let scale = f64::from(Amplitude::MAX.ppm()) * 1_000.0;
    (start..start + MATCH_TICKS)
        .step_by(STEP as usize)
        .map(|t| f64::from(probe.drift(Stream::Sensitivity, t).ppb()) / scale)
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

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn main() {
    let root = RootSeed::from_bytes(&mut [0x11; 32]);
    let match_key = root.match_key(b"compare");
    for (name, shape) in [
        ("band-limited noise", SignalShape::DEFAULT_NOISE),
        ("sum of 4 sinusoids", SignalShape::DEFAULT_SINUSOIDS),
    ] {
        let (mut rms, mut slope, mut lag5) = (0.0, 0.0f64, 0.0);
        let mut rs = Vec::new();
        for i in 0..PAIRS {
            let mut a =
                ProbeGenerator::new(&match_key.player_key(2 * i).epoch_seed(0), &config(shape))
                    .unwrap();
            let mut b = ProbeGenerator::new(
                &match_key.player_key(2 * i + 1).epoch_seed(0),
                &config(shape),
            )
            .unwrap();
            let sa = unit_series(&mut a, 0);
            let sb = unit_series(&mut b, 0);
            rs.push(pearson(&sa, &sb).abs());
            rms += sa.iter().map(|x| x * x).sum::<f64>() / sa.len() as f64;
            for w in sa.windows(2) {
                slope = slope.max((w[1] - w[0]).abs() * (1_000 / STEP) as f64);
            }
            let lag = (5_000 / STEP) as usize;
            lag5 += pearson(&sa[..sa.len() - lag], &sa[lag..]);
        }
        rs.sort_by(f64::total_cmp);
        let m = PAIRS as f64;
        println!("{name}");
        println!("  RMS / peak bound           {:.3}", (rms / m).sqrt());
        println!("  max slope (amplitude/s)    {slope:.3}");
        println!("  autocorrelation at 5 s     {:.3}", lag5 / m);
        println!(
            "  cross-player |r| over 10 min: median {:.3}  p95 {:.3}  p99 {:.3}  max {:.3}",
            percentile(&rs, 0.5),
            percentile(&rs, 0.95),
            percentile(&rs, 0.99),
            rs[rs.len() - 1]
        );
    }
}
