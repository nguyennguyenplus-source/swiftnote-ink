use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use swiftnote_core::{
    default_pen_slots, ContourMesh, RawInputSample, StabilizerPreset, StrokePoint,
    StrokeStabilizer, StrokeTessellator,
};

fn bench_core(c: &mut Criterion) {
    let style = default_pen_slots()[0];
    let points: Vec<StrokePoint> = (0u32..500)
        .map(|i| {
            let t = f64::from(i) * 0.1;
            StrokePoint::new(
                t * 12.0,
                (t * 0.7).sin() * 40.0 + 100.0,
                2.0 + (t * 0.3).cos().abs() as f32 * 2.0,
                0.0,
                0.0,
            )
        })
        .collect();

    c.bench_function("tessellate_500_pts", |b| {
        let mut tess = StrokeTessellator::new();
        let mut mesh = ContourMesh::with_capacity(8192, 16384);
        b.iter(|| {
            tess.tessellate_stroke(black_box(&points), style, &mut mesh);
            black_box(mesh.indices.len())
        });
    });

    let samples: Vec<RawInputSample> = (0u32..1000)
        .map(|i| {
            let t = f64::from(i);
            RawInputSample::new(
                t * 1.5,
                (t * 0.05).sin() * 25.0,
                0.6,
                0.0,
                0.0,
                u64::from(i) * 4,
            )
        })
        .collect();

    c.bench_function("stabilizer_1000_samples", |b| {
        let mut stab = StrokeStabilizer::new(StabilizerPreset::Light);
        let mut out = Vec::with_capacity(2048);
        b.iter(|| {
            out.clear();
            stab.begin(samples[0], style, &mut out);
            for &s in &samples[1..] {
                stab.push(s, style, &mut out);
            }
            if let Some(&last) = samples.last() {
                stab.finish(last, style, &mut out);
            }
            black_box(out.len())
        });
    });
}

criterion_group!(benches, bench_core);
criterion_main!(benches);
