//! 검색: 공백으로 나눈 모든 단어가 포함되어야 일치(AND), 대소문자 무시,
//! 그리고 한글 초성 검색(예: "ㅎㄱ" → "한국")을 지원합니다.

const CHOSUNG: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ', 'ㅌ',
    'ㅍ', 'ㅎ',
];

/// 한글 음절이면 초성(호환용 자모)을 돌려줍니다.
pub fn chosung(c: char) -> Option<char> {
    let code = c as u32;
    if (0xAC00..=0xD7A3).contains(&code) {
        Some(CHOSUNG[((code - 0xAC00) / 588) as usize])
    } else {
        None
    }
}

fn is_chosung_jamo(c: char) -> bool {
    CHOSUNG.contains(&c)
}

fn char_matches(q: char, h: char) -> bool {
    q == h || (is_chosung_jamo(q) && chosung(h) == Some(q))
}

fn lower(s: &str) -> Vec<char> {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

fn contains_term(hay: &[char], term: &[char]) -> bool {
    if term.is_empty() {
        return true;
    }
    if term.len() > hay.len() {
        return false;
    }
    (0..=hay.len() - term.len())
        .any(|start| term.iter().enumerate().all(|(i, q)| char_matches(*q, hay[start + i])))
}

/// 검색어를 미리 준비해 두면 많은 항목을 빠르게 걸러낼 수 있습니다.
#[derive(Debug, Clone)]
pub struct Query {
    terms: Vec<Vec<char>>,
}

impl Query {
    pub fn new(query: &str) -> Self {
        Query { terms: query.split_whitespace().map(lower).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn matches(&self, haystack: &str) -> bool {
        if self.terms.is_empty() {
            return true;
        }
        let hay = lower(haystack);
        self.terms.iter().all(|t| contains_term(&hay, t))
    }
}

pub fn matches(haystack: &str, query: &str) -> bool {
    Query::new(query).matches(haystack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        assert!(matches("anything", ""));
        assert!(matches("", "   "));
    }

    #[test]
    fn case_insensitive_and_terms() {
        assert!(matches("Hello Rust World", "rust hello"));
        assert!(!matches("Hello Rust World", "rust python"));
    }

    #[test]
    fn korean_plain_and_chosung() {
        assert!(matches("대한민국 만세", "민국"));
        assert!(matches("대한민국 만세", "ㄷㅎㅁㄱ"));
        assert!(matches("대한민국 만세", "ㅁㅅ"));
        assert!(!matches("대한민국 만세", "ㅁㄷ"));
        // 초성과 완성형 섞어 쓰기
        assert!(matches("대한민국", "대ㅎ"));
    }

    #[test]
    fn chosung_of_syllables() {
        assert_eq!(chosung('가'), Some('ㄱ'));
        assert_eq!(chosung('힣'), Some('ㅎ'));
        assert_eq!(chosung('까'), Some('ㄲ'));
        assert_eq!(chosung('a'), None);
    }
}
