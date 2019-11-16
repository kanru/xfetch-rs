use std::thread;
use std::time::{Duration, Instant};
use xfetch::CacheEntry;

fn main() {
    let entry = CacheEntry::builder(|| {
        thread::sleep(Duration::from_secs(1));
        42
    })
    .with_ttl(|_| Duration::from_secs(60))
    .build();

    let start = Instant::now();
    for _ in 0..120 {
        thread::sleep(Duration::from_millis(500));
        let mut early_expire = 0.0;
        for _ in 0..1000 {
            if entry.is_expired() {
                early_expire += 1.0;
            }
        }
        println!("{} {}", start.elapsed().as_secs(), early_expire / 1000.0);
    }
}
