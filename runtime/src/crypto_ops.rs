use crate::*;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const H_INIT: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

fn sha256_compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            block[i * 4],
            block[i * 4 + 1],
            block[i * 4 + 2],
            block[i * 4 + 3],
        ]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;

    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    let mut state = H_INIT;
    let bit_len = (data.len() as u64) * 8;

    let mut i = 0;
    while i + 64 <= data.len() {
        let block: [u8; 64] = data[i..i + 64].try_into().unwrap();
        sha256_compress(&mut state, &block);
        i += 64;
    }

    let remaining = &data[i..];
    let mut last_blocks = vec![0u8; 128];
    last_blocks[..remaining.len()].copy_from_slice(remaining);
    last_blocks[remaining.len()] = 0x80;

    let blocks_needed = if remaining.len() < 56 { 1 } else { 2 };
    let len_offset = blocks_needed * 64 - 8;
    last_blocks[len_offset..len_offset + 8].copy_from_slice(&bit_len.to_be_bytes());

    for b in 0..blocks_needed {
        let block: [u8; 64] = last_blocks[b * 64..(b + 1) * 64].try_into().unwrap();
        sha256_compress(&mut state, &block);
    }

    let mut result = [0u8; 32];
    for (i, &word) in state.iter().enumerate() {
        result[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    result
}

fn bytes_to_hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_to_bytes(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let block_size = 64;
    let mut k = vec![0u8; block_size];

    if key.len() > block_size {
        let hash = sha256_digest(key);
        k[..32].copy_from_slice(&hash);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut i_key_pad = vec![0x36u8; block_size];
    let mut o_key_pad = vec![0x5cu8; block_size];
    for j in 0..block_size {
        i_key_pad[j] ^= k[j];
        o_key_pad[j] ^= k[j];
    }

    let mut inner = i_key_pad;
    inner.extend_from_slice(message);
    let inner_hash = sha256_digest(&inner);

    let mut outer = o_key_pad;
    outer.extend_from_slice(&inner_hash);
    sha256_digest(&outer)
}

const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

const INV_SBOX: [u8; 256] = [
    0x52, 0x09, 0x6a, 0xd5, 0x30, 0x36, 0xa5, 0x38, 0xbf, 0x40, 0xa3, 0x9e, 0x81, 0xf3, 0xd7, 0xfb,
    0x7c, 0xe3, 0x39, 0x82, 0x9b, 0x2f, 0xff, 0x87, 0x34, 0x8e, 0x43, 0x44, 0xc4, 0xde, 0xe9, 0xcb,
    0x54, 0x7b, 0x94, 0x32, 0xa6, 0xc2, 0x23, 0x3d, 0xee, 0x4c, 0x95, 0x0b, 0x42, 0xfa, 0xc3, 0x4e,
    0x08, 0x2e, 0xa1, 0x66, 0x28, 0xd9, 0x24, 0xb2, 0x76, 0x5b, 0xa2, 0x49, 0x6d, 0x8b, 0xd1, 0x25,
    0x72, 0xf8, 0xf6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xd4, 0xa4, 0x5c, 0xcc, 0x5d, 0x65, 0xb6, 0x92,
    0x6c, 0x70, 0x48, 0x50, 0xfd, 0xed, 0xb9, 0xda, 0x5e, 0x15, 0x46, 0x57, 0xa7, 0x8d, 0x9d, 0x84,
    0x90, 0xd8, 0xab, 0x00, 0x8c, 0xbc, 0xd3, 0x0a, 0xf7, 0xe4, 0x58, 0x05, 0xb8, 0xb3, 0x45, 0x06,
    0xd0, 0x2c, 0x1e, 0x8f, 0xca, 0x3f, 0x0f, 0x02, 0xc1, 0xaf, 0xbd, 0x03, 0x01, 0x13, 0x8a, 0x6b,
    0x3a, 0x91, 0x11, 0x41, 0x4f, 0x67, 0xdc, 0xea, 0x97, 0xf2, 0xcf, 0xce, 0xf0, 0xb4, 0xe6, 0x73,
    0x96, 0xac, 0x74, 0x22, 0xe7, 0xad, 0x35, 0x85, 0xe2, 0xf9, 0x37, 0xe8, 0x1c, 0x75, 0xdf, 0x6e,
    0x47, 0xf1, 0x1a, 0x71, 0x1d, 0x29, 0xc5, 0x89, 0x6f, 0xb7, 0x62, 0x0e, 0xaa, 0x18, 0xbe, 0x1b,
    0xfc, 0x56, 0x3e, 0x4b, 0xc6, 0xd2, 0x79, 0x20, 0x9a, 0xdb, 0xc0, 0xfe, 0x78, 0xcd, 0x5a, 0xf4,
    0x1f, 0xdd, 0xa8, 0x33, 0x88, 0x07, 0xc7, 0x31, 0xb1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xec, 0x5f,
    0x60, 0x51, 0x7f, 0xa9, 0x19, 0xb5, 0x4a, 0x0d, 0x2d, 0xe5, 0x7a, 0x9f, 0x93, 0xc9, 0x9c, 0xef,
    0xa0, 0xe0, 0x3b, 0x4d, 0xae, 0x2a, 0xf5, 0xb0, 0xc8, 0xeb, 0xbb, 0x3c, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2b, 0x04, 0x7e, 0xba, 0x77, 0xd6, 0x26, 0xe1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0c, 0x7d,
];

const RCON: [u8; 10] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0x1b, 0x36];

fn gf_mul(mut a: u8, mut b: u8) -> u8 {
    let mut p: u8 = 0;
    for _ in 0..8 {
        if b & 1 != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1b;
        }
        b >>= 1;
    }
    p
}

fn aes256_key_expansion(key: &[u8; 32]) -> [[u8; 16]; 15] {
    let mut round_keys = [[0u8; 16]; 15];
    let nk = 8;
    let nr = 14;
    let mut w = vec![0u32; 4 * (nr + 1)];

    for i in 0..nk {
        w[i] = u32::from_be_bytes([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
    }

    for i in nk..4 * (nr + 1) {
        let mut temp = w[i - 1];
        if i % nk == 0 {
            temp = temp.rotate_left(8);
            let b = temp.to_be_bytes();
            temp = u32::from_be_bytes([
                SBOX[b[0] as usize],
                SBOX[b[1] as usize],
                SBOX[b[2] as usize],
                SBOX[b[3] as usize],
            ]);
            temp ^= (RCON[i / nk - 1] as u32) << 24;
        } else if i % nk == 4 {
            let b = temp.to_be_bytes();
            temp = u32::from_be_bytes([
                SBOX[b[0] as usize],
                SBOX[b[1] as usize],
                SBOX[b[2] as usize],
                SBOX[b[3] as usize],
            ]);
        }
        w[i] = w[i - nk] ^ temp;
    }

    for r in 0..=nr {
        let mut block = [0u8; 16];
        for j in 0..4 {
            block[j * 4..j * 4 + 4].copy_from_slice(&w[r * 4 + j].to_be_bytes());
        }
        round_keys[r] = block;
    }
    round_keys
}

fn aes256_encrypt_block(block: &[u8; 16], round_keys: &[[u8; 16]; 15]) -> [u8; 16] {
    let mut state = *block;

    for i in 0..16 {
        state[i] ^= round_keys[0][i];
    }

    for round in 1..14 {
        for i in 0..16 {
            state[i] = SBOX[state[i] as usize];
        }

        let tmp = state;
        state[1] = tmp[5];
        state[5] = tmp[9];
        state[9] = tmp[13];
        state[13] = tmp[1];
        state[2] = tmp[10];
        state[6] = tmp[14];
        state[10] = tmp[2];
        state[14] = tmp[6];
        state[3] = tmp[15];
        state[7] = tmp[3];
        state[11] = tmp[7];
        state[15] = tmp[11];

        for c in 0..4 {
            let s0 = state[c * 4];
            let s1 = state[c * 4 + 1];
            let s2 = state[c * 4 + 2];
            let s3 = state[c * 4 + 3];
            state[c * 4] = gf_mul(2, s0) ^ gf_mul(3, s1) ^ s2 ^ s3;
            state[c * 4 + 1] = s0 ^ gf_mul(2, s1) ^ gf_mul(3, s2) ^ s3;
            state[c * 4 + 2] = s0 ^ s1 ^ gf_mul(2, s2) ^ gf_mul(3, s3);
            state[c * 4 + 3] = gf_mul(3, s0) ^ s1 ^ s2 ^ gf_mul(2, s3);
        }

        for i in 0..16 {
            state[i] ^= round_keys[round][i];
        }
    }

    for i in 0..16 {
        state[i] = SBOX[state[i] as usize];
    }
    let tmp = state;
    state[1] = tmp[5];
    state[5] = tmp[9];
    state[9] = tmp[13];
    state[13] = tmp[1];
    state[2] = tmp[10];
    state[6] = tmp[14];
    state[10] = tmp[2];
    state[14] = tmp[6];
    state[3] = tmp[15];
    state[7] = tmp[3];
    state[11] = tmp[7];
    state[15] = tmp[11];
    for i in 0..16 {
        state[i] ^= round_keys[14][i];
    }

    state
}

fn aes256_decrypt_block(block: &[u8; 16], round_keys: &[[u8; 16]; 15]) -> [u8; 16] {
    let mut state = *block;

    for i in 0..16 {
        state[i] ^= round_keys[14][i];
    }

    for round in (1..14).rev() {
        let tmp = state;
        state[1] = tmp[13];
        state[5] = tmp[1];
        state[9] = tmp[5];
        state[13] = tmp[9];
        state[2] = tmp[10];
        state[6] = tmp[14];
        state[10] = tmp[2];
        state[14] = tmp[6];
        state[3] = tmp[7];
        state[7] = tmp[11];
        state[11] = tmp[15];
        state[15] = tmp[3];

        for i in 0..16 {
            state[i] = INV_SBOX[state[i] as usize];
        }

        for i in 0..16 {
            state[i] ^= round_keys[round][i];
        }

        for c in 0..4 {
            let s0 = state[c * 4];
            let s1 = state[c * 4 + 1];
            let s2 = state[c * 4 + 2];
            let s3 = state[c * 4 + 3];
            state[c * 4] =
                gf_mul(0x0e, s0) ^ gf_mul(0x0b, s1) ^ gf_mul(0x0d, s2) ^ gf_mul(0x09, s3);
            state[c * 4 + 1] =
                gf_mul(0x09, s0) ^ gf_mul(0x0e, s1) ^ gf_mul(0x0b, s2) ^ gf_mul(0x0d, s3);
            state[c * 4 + 2] =
                gf_mul(0x0d, s0) ^ gf_mul(0x09, s1) ^ gf_mul(0x0e, s2) ^ gf_mul(0x0b, s3);
            state[c * 4 + 3] =
                gf_mul(0x0b, s0) ^ gf_mul(0x0d, s1) ^ gf_mul(0x09, s2) ^ gf_mul(0x0e, s3);
        }
    }

    let tmp = state;
    state[1] = tmp[13];
    state[5] = tmp[1];
    state[9] = tmp[5];
    state[13] = tmp[9];
    state[2] = tmp[10];
    state[6] = tmp[14];
    state[10] = tmp[2];
    state[14] = tmp[6];
    state[3] = tmp[7];
    state[7] = tmp[11];
    state[11] = tmp[15];
    state[15] = tmp[3];
    for i in 0..16 {
        state[i] = INV_SBOX[state[i] as usize];
    }
    for i in 0..16 {
        state[i] ^= round_keys[0][i];
    }

    state
}

fn pkcs7_pad(data: &[u8]) -> Vec<u8> {
    let pad_len = 16 - (data.len() % 16);
    let mut padded = data.to_vec();
    padded.extend(std::iter::repeat(pad_len as u8).take(pad_len));
    padded
}

fn pkcs7_unpad(data: &[u8]) -> Option<Vec<u8>> {
    if data.is_empty() || data.len() % 16 != 0 {
        return None;
    }
    let pad_len = *data.last()? as usize;
    if pad_len == 0 || pad_len > 16 || pad_len > data.len() {
        return None;
    }

    for &b in &data[data.len() - pad_len..] {
        if b as usize != pad_len {
            return None;
        }
    }
    Some(data[..data.len() - pad_len].to_vec())
}

fn aes256_cbc_encrypt(plaintext: &[u8], key: &[u8; 32], iv: &[u8; 16]) -> Vec<u8> {
    let round_keys = aes256_key_expansion(key);
    let padded = pkcs7_pad(plaintext);
    let mut ciphertext = Vec::with_capacity(padded.len());
    let mut prev_block = *iv;

    for chunk in padded.chunks(16) {
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = chunk[i] ^ prev_block[i];
        }
        let encrypted = aes256_encrypt_block(&block, &round_keys);
        ciphertext.extend_from_slice(&encrypted);
        prev_block = encrypted;
    }
    ciphertext
}

fn aes256_cbc_decrypt(ciphertext: &[u8], key: &[u8; 32], iv: &[u8; 16]) -> Option<Vec<u8>> {
    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return None;
    }
    let round_keys = aes256_key_expansion(key);
    let mut plaintext = Vec::with_capacity(ciphertext.len());
    let mut prev_block = *iv;

    for chunk in ciphertext.chunks(16) {
        let block: [u8; 16] = chunk.try_into().ok()?;
        let decrypted = aes256_decrypt_block(&block, &round_keys);
        let mut plain_block = [0u8; 16];
        for i in 0..16 {
            plain_block[i] = decrypted[i] ^ prev_block[i];
        }
        plaintext.extend_from_slice(&plain_block);
        prev_block = block;
    }
    pkcs7_unpad(&plaintext)
}

fn os_random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    #[cfg(unix)]
    {
        use std::io::Read;
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let _ = f.read_exact(&mut buf);
        }
    }
    #[cfg(not(unix))]
    {
        for b in buf.iter_mut() {
            *b = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos()
                & 0xFF) as u8;
        }
    }
    buf
}

fn safe_string(h: ValueHandle) -> Option<String> {
    if h.is_null() {
        return None;
    }
    let val = unsafe { &*h };
    if val.tag != ValueTag::String as u8 {
        return None;
    }
    Some(value_to_rust_string(val))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_sha256(data_handle: ValueHandle) -> ValueHandle {
    let data = match safe_string(data_handle) {
        Some(s) => s,
        None => return coral_make_string_from_rust(""),
    };
    let hash = sha256_digest(data.as_bytes());
    coral_make_string_from_rust(&bytes_to_hex(&hash))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_hmac_sha256(
    key_handle: ValueHandle,
    msg_handle: ValueHandle,
) -> ValueHandle {
    let key_str = match safe_string(key_handle) {
        Some(s) => s,
        None => return coral_make_string_from_rust(""),
    };
    let msg = match safe_string(msg_handle) {
        Some(s) => s,
        None => return coral_make_string_from_rust(""),
    };
    let key_bytes = match hex_to_bytes(&key_str) {
        Some(b) => b,
        None => key_str.as_bytes().to_vec(),
    };
    let mac = hmac_sha256(&key_bytes, msg.as_bytes());
    coral_make_string_from_rust(&bytes_to_hex(&mac))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_aes256_encrypt(
    plaintext_handle: ValueHandle,
    key_handle: ValueHandle,
    iv_handle: ValueHandle,
) -> ValueHandle {
    let plaintext = match safe_string(plaintext_handle) {
        Some(s) => s,
        None => return coral_make_string_from_rust(""),
    };
    let key_hex = match safe_string(key_handle) {
        Some(s) => s,
        None => return coral_make_string_from_rust(""),
    };
    let iv_hex = match safe_string(iv_handle) {
        Some(s) => s,
        None => return coral_make_string_from_rust(""),
    };

    let key_bytes = match hex_to_bytes(&key_hex) {
        Some(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => return coral_make_string_from_rust(""),
    };
    let iv_bytes = match hex_to_bytes(&iv_hex) {
        Some(b) if b.len() == 16 => {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(&b);
            arr
        }
        _ => return coral_make_string_from_rust(""),
    };

    let ciphertext = aes256_cbc_encrypt(plaintext.as_bytes(), &key_bytes, &iv_bytes);
    coral_make_string_from_rust(&bytes_to_hex(&ciphertext))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_aes256_decrypt(
    ciphertext_handle: ValueHandle,
    key_handle: ValueHandle,
    iv_handle: ValueHandle,
) -> ValueHandle {
    let ct_hex = match safe_string(ciphertext_handle) {
        Some(s) => s,
        None => return coral_make_string_from_rust(""),
    };
    let key_hex = match safe_string(key_handle) {
        Some(s) => s,
        None => return coral_make_string_from_rust(""),
    };
    let iv_hex = match safe_string(iv_handle) {
        Some(s) => s,
        None => return coral_make_string_from_rust(""),
    };

    let ct_bytes = match hex_to_bytes(&ct_hex) {
        Some(b) => b,
        None => return coral_make_string_from_rust(""),
    };
    let key_bytes = match hex_to_bytes(&key_hex) {
        Some(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        }
        _ => return coral_make_string_from_rust(""),
    };
    let iv_bytes = match hex_to_bytes(&iv_hex) {
        Some(b) if b.len() == 16 => {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(&b);
            arr
        }
        _ => return coral_make_string_from_rust(""),
    };

    match aes256_cbc_decrypt(&ct_bytes, &key_bytes, &iv_bytes) {
        Some(plaintext) => {
            let text = String::from_utf8_lossy(&plaintext).to_string();
            coral_make_string_from_rust(&text)
        }
        None => coral_make_string_from_rust(""),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_random_bytes(n_handle: ValueHandle) -> ValueHandle {
    let n = if n_handle.is_null() {
        16
    } else {
        let val = unsafe { &*n_handle };
        if val.tag != ValueTag::Number as u8 {
            16
        } else {
            (unsafe { val.payload.number }) as usize
        }
    };
    let n = n.min(1024);
    let bytes = os_random_bytes(n);
    coral_make_string_from_rust(&bytes_to_hex(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_empty_string() {
        let hash = sha256_digest(b"");
        assert_eq!(
            bytes_to_hex(&hash),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_hello() {
        let hash = sha256_digest(b"hello");
        assert_eq!(
            bytes_to_hex(&hash),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn hmac_sha256_rfc4231_test1() {
        let key = b"Jefe";
        let msg = b"what do ya want for nothing?";
        let mac = hmac_sha256(key, msg);
        assert_eq!(
            bytes_to_hex(&mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn aes256_cbc_roundtrip() {
        let key = [0x01u8; 32];
        let iv = [0x02u8; 16];
        let plaintext = b"Hello, AES-256-CBC encryption!";
        let ciphertext = aes256_cbc_encrypt(plaintext, &key, &iv);
        let decrypted = aes256_cbc_decrypt(&ciphertext, &key, &iv).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn aes256_cbc_block_aligned() {
        let key = [0xABu8; 32];
        let iv = [0xCDu8; 16];
        let plaintext = b"exactly16bytes!!";
        let ciphertext = aes256_cbc_encrypt(plaintext, &key, &iv);
        let decrypted = aes256_cbc_decrypt(&ciphertext, &key, &iv).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn random_bytes_non_empty() {
        let bytes = os_random_bytes(32);
        assert_eq!(bytes.len(), 32);

        assert!(bytes.iter().any(|&b| b != 0));
    }

    #[test]
    fn ffi_sha256_hello() {
        let input = coral_make_string_from_rust("hello");
        let result = coral_sha256(input);
        let hash = value_to_rust_string(unsafe { &*result });
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn ffi_random_bytes_produces_hex() {
        let n = coral_make_number(16.0);
        let result = coral_random_bytes(n);
        let hex = value_to_rust_string(unsafe { &*result });
        assert_eq!(hex.len(), 32);
    }
}
