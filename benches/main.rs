use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::Criterion;
use std::time::Duration;
use xfetch::CacheEntry;

fn xfetch(value: u64) -> Option<u64> {
    let entry = CacheEntry::builder(|| value)
        .with_ttl(|_| Duration::from_secs(120))
        .with_delta(|_| Duration::from_secs(10))
        .build();
    if entry.is_expired() {
        None
    } else {
        Some(*entry.get())
    }
}

fn benchmark(c: &mut Criterion) {
    c.bench_function("xfetch", |b| b.iter(|| xfetch(black_box(20))));
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
