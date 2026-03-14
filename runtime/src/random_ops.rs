use crate::*;
use std::sync::Mutex;

static PRNG_STATE: Mutex<[u64; 4]> = Mutex::new([0; 4]);
static PRNG_SEEDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn ensure_seeded() {
    if !PRNG_SEEDED.load(std::sync::atomic::Ordering::Relaxed) {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(42);
        seed_prng(seed);
    }
}

fn seed_prng(seed: u64) {
    let mut state = PRNG_STATE.lock().unwrap();

    let mut s = seed;
    for i in 0..4 {
        s = s.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = s;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        state[i] = z ^ (z >> 31);
    }

    if state.iter().all(|&x| x == 0) {
        state[0] = 1;
    }
    PRNG_SEEDED.store(true, std::sync::atomic::Ordering::Relaxed);
}

fn next_u64() -> u64 {
    ensure_seeded();
    let mut state = PRNG_STATE.lock().unwrap();
    let result = (state[1].wrapping_mul(5)).rotate_left(7).wrapping_mul(9);
    let t = state[1] << 17;
    state[2] ^= state[0];
    state[3] ^= state[1];
    state[1] ^= state[2];
    state[0] ^= state[3];
    state[2] ^= t;
    state[3] = state[3].rotate_left(45);
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_random() -> ValueHandle {
    let bits = next_u64();

    let f = (bits >> 11) as f64 / (1u64 << 53) as f64;
    coral_make_number(f)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_random_int(min_val: ValueHandle, max_val: ValueHandle) -> ValueHandle {
    let min_f = if min_val.is_null() {
        0.0
    } else {
        let v = unsafe { &*min_val };
        if v.tag == ValueTag::Number as u8 {
            unsafe { v.payload.number }
        } else {
            0.0
        }
    };
    let max_f = if max_val.is_null() {
        100.0
    } else {
        let v = unsafe { &*max_val };
        if v.tag == ValueTag::Number as u8 {
            unsafe { v.payload.number }
        } else {
            100.0
        }
    };
    let min = min_f as i64;
    let max = max_f as i64;
    if min >= max {
        return coral_make_number(min as f64);
    }
    let range = (max - min + 1) as u64;
    let r = next_u64() % range;
    coral_make_number((min + r as i64) as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_random_seed(seed_val: ValueHandle) -> ValueHandle {
    let seed = if seed_val.is_null() {
        42u64
    } else {
        let v = unsafe { &*seed_val };
        if v.tag == ValueTag::Number as u8 {
            let n = unsafe { v.payload.number };
            n as u64
        } else {
            42u64
        }
    };
    seed_prng(seed);
    coral_make_unit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_produces_values_in_range() {
        seed_prng(12345);
        for _ in 0..100 {
            let handle = coral_random();
            let v = unsafe { &*handle };
            let f = unsafe { v.payload.number };
            assert!(f >= 0.0 && f < 1.0, "random() out of range: {}", f);
        }
    }

    #[test]
    fn test_random_seed_reproducibility() {
        seed_prng(42);
        let a1 = next_u64();
        let a2 = next_u64();
        seed_prng(42);
        let b1 = next_u64();
        let b2 = next_u64();
        assert_eq!(a1, b1);
        assert_eq!(a2, b2);
    }
}
