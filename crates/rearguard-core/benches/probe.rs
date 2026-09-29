//! Probe generator benchmarks: `cargo bench -p rearguard-core --bench probe`.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use rearguard_core::probe::{
    Amplitude, EpochSchedule, ProbeConfig, ProbeGenerator, RootSeed, SignalShape, Stream,
};

fn config(shape: SignalShape) -> ProbeConfig {
    ProbeConfig {
        amplitude: Amplitude::DEFAULT,
        shape,
        epochs: EpochSchedule::SINGLE,
    }
}

fn bench(c: &mut Criterion) {
    let root = RootSeed::from_bytes(&mut [0x5A; 32]);
    let seed = root.match_key(b"bench").player_key(1).epoch_seed(0);

    c.bench_function("derive root -> epoch seed", |b| {
        b.iter(|| {
            root.match_key(black_box(b"bench"))
                .player_key(black_box(1))
                .epoch_seed(black_box(0))
        })
    });

    for (name, shape) in [
        ("noise", SignalShape::DEFAULT_NOISE),
        ("sinusoids", SignalShape::DEFAULT_SINUSOIDS),
    ] {
        c.bench_function(&format!("{name}: new generator"), |b| {
            b.iter(|| ProbeGenerator::new(black_box(&seed), &config(shape)).unwrap())
        });

        let mut probe = ProbeGenerator::new(&seed, &config(shape)).unwrap();
        // One call per millisecond tick, as a client sampling every input event would.
        let mut tick = 0u64;
        c.bench_function(&format!("{name}: drift, sequential ticks"), |b| {
            b.iter(|| {
                tick += 1;
                probe.drift(Stream::Sensitivity, black_box(tick))
            })
        });
        // Ticks far apart, as a server judging scattered events would: no cache hits.
        let mut tick = 0u64;
        c.bench_function(&format!("{name}: drift, scattered ticks"), |b| {
            b.iter(|| {
                tick = tick.wrapping_add(7_919_993);
                probe.drift(Stream::Sensitivity, black_box(tick))
            })
        });
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
