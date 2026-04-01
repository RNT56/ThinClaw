use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use thinclaw::safety::{LeakDetector, Sanitizer, Validator};

fn bench_safety_pipeline(c: &mut Criterion) {
    let sanitizer = Sanitizer::new();
    let validator = Validator::new();
    let leak_detector = LeakDetector::new();

    let inputs = vec![
        ("100B", "x".repeat(100)),
        ("1KB", "x".repeat(1_000)),
        ("10KB", "x".repeat(10_000)),
    ];

    let mut group = c.benchmark_group("safety_pipeline");
    for (label, input) in &inputs {
        group.bench_with_input(
            BenchmarkId::from_parameter(label),
            input,
            |b, input| {
                b.iter(|| {
                    // Full pipeline: validate -> sanitize -> leak detect
                    let _v = black_box(validator.validate(input));
                    let _s = black_box(sanitizer.sanitize(input));
                    let _l = black_box(leak_detector.scan(input));
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_safety_pipeline);
criterion_main!(benches);
