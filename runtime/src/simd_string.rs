use std::arch::x86_64::*;

#[cfg(target_arch = "x86_64")]
pub fn simd_contains_byte(haystack: &[u8], needle: u8) -> bool {
    if !is_x86_feature_detected!("avx2") || haystack.len() < 32 {
        return haystack.contains(&needle);
    }
    unsafe { simd_contains_byte_avx2(haystack, needle) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn simd_contains_byte_avx2(haystack: &[u8], needle: u8) -> bool {
    let needle_vec = unsafe { _mm256_set1_epi8(needle as i8) };
    let chunks = haystack.len() / 32;

    for i in 0..chunks {
        let block = unsafe { _mm256_loadu_si256(haystack.as_ptr().add(i * 32) as *const __m256i) };
        let cmp = unsafe { _mm256_cmpeq_epi8(block, needle_vec) };
        let mask = unsafe { _mm256_movemask_epi8(cmp) };
        if mask != 0 {
            return true;
        }
    }

    let remainder = &haystack[chunks * 32..];
    remainder.contains(&needle)
}

#[cfg(target_arch = "x86_64")]
pub fn simd_to_lowercase(input: &[u8]) -> Vec<u8> {
    if !is_x86_feature_detected!("avx2") || input.len() < 32 {
        return input.iter().map(|b| b.to_ascii_lowercase()).collect();
    }
    unsafe { simd_to_lowercase_avx2(input) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn simd_to_lowercase_avx2(input: &[u8]) -> Vec<u8> {
    let mut output = vec![0u8; input.len()];
    let a_val = unsafe { _mm256_set1_epi8(b'A' as i8) };
    let z_val = unsafe { _mm256_set1_epi8(b'Z' as i8) };
    let diff = unsafe { _mm256_set1_epi8(32) };

    let chunks = input.len() / 32;
    for i in 0..chunks {
        let block = unsafe { _mm256_loadu_si256(input.as_ptr().add(i * 32) as *const __m256i) };
        let ge_a = unsafe { _mm256_cmpgt_epi8(block, _mm256_sub_epi8(a_val, _mm256_set1_epi8(1))) };
        let le_z = unsafe { _mm256_cmpgt_epi8(_mm256_add_epi8(z_val, _mm256_set1_epi8(1)), block) };
        let is_upper = unsafe { _mm256_and_si256(ge_a, le_z) };
        let lowered = unsafe { _mm256_add_epi8(block, _mm256_and_si256(is_upper, diff)) };
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(i * 32) as *mut __m256i, lowered);
        }
    }

    let start = chunks * 32;
    for j in start..input.len() {
        output[j] = input[j].to_ascii_lowercase();
    }
    output
}

#[cfg(target_arch = "x86_64")]
pub fn simd_to_uppercase(input: &[u8]) -> Vec<u8> {
    if !is_x86_feature_detected!("avx2") || input.len() < 32 {
        return input.iter().map(|b| b.to_ascii_uppercase()).collect();
    }
    unsafe { simd_to_uppercase_avx2(input) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn simd_to_uppercase_avx2(input: &[u8]) -> Vec<u8> {
    let mut output = vec![0u8; input.len()];
    let a_val = unsafe { _mm256_set1_epi8(b'a' as i8) };
    let z_val = unsafe { _mm256_set1_epi8(b'z' as i8) };
    let diff = unsafe { _mm256_set1_epi8(32) };

    let chunks = input.len() / 32;
    for i in 0..chunks {
        let block = unsafe { _mm256_loadu_si256(input.as_ptr().add(i * 32) as *const __m256i) };
        let ge_a = unsafe { _mm256_cmpgt_epi8(block, _mm256_sub_epi8(a_val, _mm256_set1_epi8(1))) };
        let le_z = unsafe { _mm256_cmpgt_epi8(_mm256_add_epi8(z_val, _mm256_set1_epi8(1)), block) };
        let is_lower = unsafe { _mm256_and_si256(ge_a, le_z) };
        let uppered = unsafe { _mm256_sub_epi8(block, _mm256_and_si256(is_lower, diff)) };
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(i * 32) as *mut __m256i, uppered);
        }
    }

    let start = chunks * 32;
    for j in start..input.len() {
        output[j] = input[j].to_ascii_uppercase();
    }
    output
}

#[cfg(target_arch = "x86_64")]
pub fn simd_count_byte(haystack: &[u8], needle: u8) -> usize {
    if !is_x86_feature_detected!("avx2") || haystack.len() < 32 {
        return haystack.iter().filter(|&&b| b == needle).count();
    }
    unsafe { simd_count_byte_avx2(haystack, needle) }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn simd_count_byte_avx2(haystack: &[u8], needle: u8) -> usize {
    let needle_vec = unsafe { _mm256_set1_epi8(needle as i8) };
    let chunks = haystack.len() / 32;
    let mut total: usize = 0;

    for i in 0..chunks {
        let block = unsafe { _mm256_loadu_si256(haystack.as_ptr().add(i * 32) as *const __m256i) };
        let cmp = unsafe { _mm256_cmpeq_epi8(block, needle_vec) };
        let mask = unsafe { _mm256_movemask_epi8(cmp) } as u32;
        total += mask.count_ones() as usize;
    }

    let remainder = &haystack[chunks * 32..];
    total + remainder.iter().filter(|&&b| b == needle).count()
}

#[cfg(not(target_arch = "x86_64"))]
pub fn simd_contains_byte(haystack: &[u8], needle: u8) -> bool {
    haystack.contains(&needle)
}

#[cfg(not(target_arch = "x86_64"))]
pub fn simd_to_lowercase(input: &[u8]) -> Vec<u8> {
    input.iter().map(|b| b.to_ascii_lowercase()).collect()
}

#[cfg(not(target_arch = "x86_64"))]
pub fn simd_to_uppercase(input: &[u8]) -> Vec<u8> {
    input.iter().map(|b| b.to_ascii_uppercase()).collect()
}

#[cfg(not(target_arch = "x86_64"))]
pub fn simd_count_byte(haystack: &[u8], needle: u8) -> usize {
    haystack.iter().filter(|&&b| b == needle).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_byte_found() {
        let data = b"hello world, this is a test string for simd operations!";
        assert!(simd_contains_byte(data, b'w'));
        assert!(simd_contains_byte(data, b'!'));
        assert!(!simd_contains_byte(data, b'Z'));
    }

    #[test]
    fn to_lowercase_ascii() {
        let input = b"HELLO WORLD 123 ABC xyz";
        let result = simd_to_lowercase(input);
        assert_eq!(result, b"hello world 123 abc xyz");
    }

    #[test]
    fn to_uppercase_ascii() {
        let input = b"hello world 123 abc XYZ";
        let result = simd_to_uppercase(input);
        assert_eq!(result, b"HELLO WORLD 123 ABC XYZ");
    }

    #[test]
    fn count_byte() {
        let data = b"aababacaaabababaa";
        assert_eq!(simd_count_byte(data, b'a'), 11);
        assert_eq!(simd_count_byte(data, b'b'), 5);
        assert_eq!(simd_count_byte(data, b'z'), 0);
    }

    #[test]
    fn large_string_lowercase() {
        let input: Vec<u8> = (0..256)
            .map(|i| if i % 2 == 0 { b'A' } else { b'b' })
            .collect();
        let result = simd_to_lowercase(&input);
        for (i, &b) in result.iter().enumerate() {
            if i % 2 == 0 {
                assert_eq!(b, b'a');
            } else {
                assert_eq!(b, b'b');
            }
        }
    }
}
