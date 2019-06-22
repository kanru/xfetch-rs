#![deny(missing_docs)]
//! This crate implements the XFetch probabilistic early expiration algorithm.
//!
//! # Cache Stampede
//!
//! A cache stampede is a type of cascading failure that can occur when
//! massively parallel computing systems with caching mechanisms come under
//! very high load. This behaviour is sometimes also called dog-piling.
//!
//! Under normal load, cache misses will trigger a recomputation to refresh the
//! cache. Other process or thread can continue as before.
//!
//! Under heavy load, cache misses may trigger multipre process / threads trying
//! to refresh content thus add more loading to the resource source which the
//! cache was meant to reduce the loading.
//!
//! Several approaches can be used to mitigate cache stampedes. The algorithm
//! used here is proposed by Vattani, A.; Chierichetti, F.; Lowenstein, K.
//! (2015) in the paper [Optimal Probabilistic Cache Stampede Prevention][vldb].
//!
//! The idea is any worker can volunteer to recompute the value before it
//! expires. With a probability that increases when the cache entry approaches
//! expiration, each worker may recompute the cache by making an independent
//! decision. The effect of the cache stampede is mitigated as fewer workers
//! will expire at the same time.
//!
//! The following is the algorithm pseudo code:
//!
//! ```ignore
//! function XFetch(key, ttl; beta = 1)
//!     value, delta, expiry <- cache_read(key)
//!     if !value or time() - delta * beta * ln(rand()) >= expiry then
//!         start <- time()
//!         value <- recompute_value()
//!         delta <- time() - start
//!         cache_write(key, (value, delta), ttl)
//!     end
//!     return value
//! end
//! ```
//!
//! The parameter **beta** can be set to greater than `1.0` to favor earlier
//! recomputation or lesser to favor later. The default `1.0` is optimal for
//! most use cases.
//!
//! `rand()` is a random number in the range (0, 1].
//!
//! **delta** is the time required for the recomputation. If it takes longer to
//! recompute then the algorithm will also favor earlier recomputation.
//!
//! # Examples
//!
//! Create a single cache entry and test it's expiration:
//!
//! ```rust
//! # struct SomeValue { value: u64, ttl: u64 };
//! # fn expensive_computation() -> SomeValue { SomeValue { value: 42, ttl: 10000 } }
//! use xfetch::CacheEntry;
//! use std::time::Duration;
//!
//! let entry = CacheEntry::new(|| {
//!     expensive_computation()
//! })
//! .with_ttl(|value| {
//!     Duration::from_millis(value.ttl)
//! })
//! .build();
//!
//! assert!(!entry.is_expired());
//! ```
//!
//! The [CacheEntry](struct.CacheEntry.html) can be used with any cache library.
//! For example the `lru` crate:
//!
//! ```rust
//! use lru::LruCache;
//! use xfetch::CacheEntry;
//! use std::time::Duration;
//!
//! struct SomeValue {
//!     value: u64,
//!     ttl: u64
//! };
//!
//! fn recompute_value(n: u64) -> SomeValue {
//!     SomeValue { value: n, ttl: 10000 }
//! }
//!
//! fn main() {
//!     let mut cache = LruCache::new(2);
//!
//!     cache.put("apple", CacheEntry::new(|| recompute_value(3))
//!         .with_ttl(|v| Duration::from_millis(v.ttl))
//!         .build());
//!     cache.put("banana", CacheEntry::new(|| recompute_value(2))
//!         .with_ttl(|v| Duration::from_millis(v.ttl))
//!         .build());
//!
//!     if let Some(entry) = cache.get(&"apple") {
//!         if !entry.is_expired() {
//!             assert_eq!(entry.get().value, 3);
//!         } else {
//!             cache.put("apple", CacheEntry::new(|| recompute_value(3))
//!                 .with_ttl(|v| Duration::from_millis(v.ttl))
//!                 .build());
//!         }
//!     }
//! }
//! ```
//!
//! # References
//!
//! - Wikipedia [Cache Stampede][wikipedia].
//! - Vattani, A.; Chierichetti, F.; Lowenstein, K. (2015), [Optimal
//!   Probabilistic Cache Stampede Prevention][vldb] (PDF), 8 (8), VLDB, pp. 886–897,
//!   ISSN 2150-8097.
//! - Jim Nelson, Internet Archive, [RedisConf17 - Preventing cache stampede with Redis & XFetch][archive].
//!
//! [vldb]: http://www.vldb.org/pvldb/vol8/p886-vattani.pdf
//! [wikipedia]: https://en.wikipedia.org/wiki/Cache_stampede
//! [archive]: https://www.slideshare.net/RedisLabs/redisconf17-internet-archive-preventing-cache-stampede-with-redis-and-xfetch

use rand::{distributions::OpenClosed01, thread_rng, Rng, RngCore};
use std::time::{Duration, Instant};

const DEFAULT_BETA: f32 = 1.0;

/// The builder for building [CacheEntry](struct.CacheEntry.html) with
/// supplied parameters.
pub struct CacheEntryBuilder<T> {
    value: T,
    delta: Duration,
    beta: f32,
    expiry: Option<Instant>,
}

impl<T> CacheEntryBuilder<T> {
    /// Set the beta value.
    ///
    /// Beta value > `1.0` favors more eager early expiration, value < `1.0`
    /// favors lazier early expiration.
    ///
    /// The default value `1.0` is usually the optimal value for most use cases.
    pub fn with_beta(mut self, beta: f32) -> CacheEntryBuilder<T> {
        self.beta = beta;
        self
    }

    /// Set the delta.
    ///
    /// Usually the delta value is mesured from the time took by the
    /// recomputation function. However, if the recomputation function does not
    /// reflect the actual time required (for example, a asynchronous
    /// computation), then the delta value can be set via this method.
    ///
    /// The reference of the value returned by the recomputation function is
    /// passed to the closure.
    pub fn with_delta<F>(mut self, f: F) -> CacheEntryBuilder<T>
    where
        F: FnOnce(&T) -> Duration,
    {
        self.delta = f(&self.value);
        self
    }

    /// Set the ttl.
    ///
    /// The reference of the value returned by the recomputation function is
    /// passed to the closure.
    ///
    /// If the ttl is not set then the cache entry will become a eternal cache
    /// entry that will never expire.
    pub fn with_ttl<F>(mut self, f: F) -> CacheEntryBuilder<T>
    where
        F: FnOnce(&T) -> Duration,
    {
        self.expiry = Some(Instant::now() + f(&self.value));
        self
    }

    /// Return a new [CacheEntry](struct.CacheEntry.html) with the supplied
    /// parameters.
    pub fn build(self) -> CacheEntry<T> {
        CacheEntry {
            value: self.value,
            delta: self.delta,
            beta: self.beta,
            expiry: self.expiry,
        }
    }
}

/// A cache entry that employs probabilistic early expiration
///
/// # Examples
///
/// In this example, you can see how to create a new cache entry. The value of
/// the entry is passed in as a closure so the time required for recomputation
/// can be measured. The time to expiration can be set by chaining the
/// [`with_ttl()`](struct.CacheEntryBuilder.html#method.with_ttl) method.
///
/// ```
/// use std::time::Duration;
/// use xfetch::CacheEntry;
///
/// let entry = CacheEntry::new(|| 42)
///     .with_ttl(|_| Duration::from_secs(10))
///     .build();
/// ```
///
/// See the [module-level documentation](index.html) for more information.
#[derive(Copy, Clone)]
pub struct CacheEntry<T> {
    value: T,
    delta: Duration,
    beta: f32,
    expiry: Option<Instant>,
}

impl<T> CacheEntry<T> {
    /// Return a new [CacheEntryBuilder](struct.CacheEntryBuilder.html).
    ///
    /// This method takes a closure which should return the value to be cached.
    pub fn new<F>(f: F) -> CacheEntryBuilder<T>
    where
        F: FnOnce() -> T,
    {
        let start = Instant::now();
        let value = f();
        let recompute_time = start.elapsed();
        CacheEntryBuilder {
            value,
            delta: recompute_time,
            beta: DEFAULT_BETA,
            expiry: None,
        }
    }

    fn is_expired_with_rng(&self, rng: &mut RngCore) -> bool {
        match self.expiry {
            Some(expiry) => {
                let now = Instant::now();
                let delta = self.delta.as_millis() as f32;
                let rand: f32 = rng.sample(OpenClosed01);
                let xfetch = Duration::from_millis((delta * self.beta * -rand.ln()).round() as u64);
                (now + xfetch) >= expiry
            }
            None => false,
        }
    }

    /// Check whether the cache has expired or not.
    ///
    /// With probabilstic early expiration, this method may return `true` before
    /// the entry is really expired.
    pub fn is_expired(&self) -> bool {
        self.is_expired_with_rng(&mut thread_rng())
    }

    /// Check if the cache entry will never expire.
    ///
    /// If the cache entry is created without setting time to expiration then it
    /// is a eternal cache entry.
    pub fn is_eternal(&self) -> bool {
        self.expiry.is_none()
    }

    /// Returns a reference of the contained value.
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Unwraps the value.
    pub fn into_inner(self) -> T {
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::mock::StepRng;

    #[test]
    fn test_new_entry() {
        let entry = CacheEntry::new(|| ()).build();
        assert_eq!(*entry.get(), ());
        assert_eq!(entry.into_inner(), ());
        assert!(entry.is_eternal());
        assert_eq!(entry.beta, DEFAULT_BETA);
    }

    #[test]
    fn test_new_entry_with_ttl() {
        let entry = CacheEntry::new(|| ())
            .with_ttl(|_| Duration::from_secs(60))
            .build();
        assert_eq!(*entry.get(), ());
        assert!(entry.expiry.is_some());
    }

    #[test]
    fn test_new_entry_with_beta() {
        let entry = CacheEntry::new(|| ()).with_beta(0.9).build();
        assert_eq!(*entry.get(), ());
        assert_eq!(entry.beta, 0.9);
    }

    #[test]
    fn test_early_expiry() {
        let mut zeros = StepRng::new(0, 0);
        let entry = CacheEntry::new(|| ())
            .with_delta(|_| Duration::from_secs(10))
            .with_ttl(|_| Duration::from_secs(120))
            .build();
        assert!(entry.is_expired_with_rng(&mut zeros));
    }

    #[test]
    fn test_no_early_expiry() {
        let mut max = StepRng::new(!0, 0);
        let entry = CacheEntry::new(|| ())
            .with_delta(|_| Duration::from_secs(10))
            .with_ttl(|_| Duration::from_secs(120))
            .build();
        assert!(!entry.is_expired_with_rng(&mut max));
    }
}
