//! 중복 검사용 해시. Rust 버전이 바뀌어도 값이 변하지 않도록 FNV-1a 를 직접 구현합니다.

const OFFSET: u64 = 0xcbf29ce484222325;
const PRIME: u64 = 0x100000001b3;

pub fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_continue(OFFSET, bytes)
}

pub fn fnv1a64_continue(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// 종류 태그 + 내용으로 16진수 해시 문자열을 만듭니다.
pub fn content_hash(kind: &str, bytes: &[u8]) -> String {
    let h = fnv1a64(kind.as_bytes());
    let h = fnv1a64_continue(h, &[0]);
    format!("{:016x}", fnv1a64_continue(h, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vector() {
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
    }

    #[test]
    fn kind_changes_hash() {
        assert_ne!(content_hash("text", b"x"), content_hash("files", b"x"));
    }
}
