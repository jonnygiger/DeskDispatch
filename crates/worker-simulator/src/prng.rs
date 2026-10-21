use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Zero-dependency, ultra-fast thread-safe pseudo-random number generator (SplitMix64).
#[derive(Debug, Default)]
pub struct FastPrng;

static STATE: AtomicU64 = AtomicU64::new(0);

impl FastPrng {
    /// Generates the next 64-bit pseudo-random unsigned integer atomically.
    pub fn next_u64() -> u64 {
        let prev = STATE.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
        let mut z = if prev == 0 {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0xda3e_43cb_9b5e_e64b);
            nanos ^ 0x9e37_79b9_7f4a_7c15
        } else {
            prev.wrapping_add(0x9e37_79b9_7f4a_7c15)
        };

        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Generates a pseudo-random `u32` integer in the range `[min, max)` (inclusive of `min`, exclusive of `max`).
    /// If `min >= max`, returns `min`.
    pub fn gen_range_u32(min: u32, max: u32) -> u32 {
        if min >= max {
            return min;
        }
        let range = (max - min) as u64;
        min + (Self::next_u64() % range) as u32
    }

    /// Generates a pseudo-random `i32` integer in the range `[min, max)` (inclusive of `min`, exclusive of `max`).
    /// Handles negative numbers correctly.
    pub fn gen_range_i32(min: i32, max: i32) -> i32 {
        if min >= max {
            return min;
        }
        let range = (max as i64 - min as i64) as u64;
        let offset = Self::next_u64() % range;
        min + offset as i32
    }

    /// Generates a pseudo-random `usize` integer in the range `[min, max)` (inclusive of `min`, exclusive of `max`).
    /// If `min >= max`, returns `min`.
    pub fn gen_range_usize(min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        let range = (max - min) as u64;
        min + (Self::next_u64() % range) as usize
    }

    /// Generates a pseudo-random `u8` integer in the range `[min, max]` (inclusive of `min` and `max`).
    pub fn gen_range_u8_inclusive(min: u8, max: u8) -> u8 {
        if min >= max {
            return min;
        }
        let range = (max - min) as u64 + 1;
        min + (Self::next_u64() % range) as u8
    }

    /// Generates a pseudo-random `u64` integer in the range `[min, max)` (inclusive of `min`, exclusive of `max`).
    /// If `min >= max`, returns `min`.
    pub fn gen_range_u64(min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        let range = max - min;
        min + (Self::next_u64() % range)
    }

    /// Returns a boolean with probability `p` of being `true`.
    pub fn gen_bool(p: f64) -> bool {
        if p <= 0.0 {
            false
        } else if p >= 1.0 {
            true
        } else {
            let threshold = (p * (u64::MAX as f64)) as u64;
            Self::next_u64() <= threshold
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_prng_range_and_bool() {
        for _ in 0..100 {
            let val_u32 = FastPrng::gen_range_u32(10, 20);
            assert!((10..20).contains(&val_u32));

            let val_i32 = FastPrng::gen_range_i32(-10, 10);
            assert!((-10..10).contains(&val_i32));

            let val_usize = FastPrng::gen_range_usize(100, 200);
            assert!((100..200).contains(&val_usize));

            let val_u8 = FastPrng::gen_range_u8_inclusive(20, 220);
            assert!((20..=220).contains(&val_u8));

            let val_u64 = FastPrng::gen_range_u64(2, 8);
            assert!((2..8).contains(&val_u64));
        }

        assert!(!FastPrng::gen_bool(0.0));
        assert!(FastPrng::gen_bool(1.0));

        // Degenerate ranges
        assert_eq!(FastPrng::gen_range_u32(5, 5), 5);
        assert_eq!(FastPrng::gen_range_u32(10, 5), 10);
        assert_eq!(FastPrng::gen_range_i32(-5, -5), -5);
        assert_eq!(FastPrng::gen_range_i32(-10, -20), -10);
    }
}
