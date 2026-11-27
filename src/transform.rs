//! 붙여넣기 전에 텍스트를 바꾸는 변환들.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    Upper,
    Lower,
    Trim,
    JoinLines,
    Quote,
    RemoveBlankLines,
    TitleCase,
}

impl Transform {
    pub const ALL: [Transform; 7] = [
        Transform::Upper,
        Transform::Lower,
        Transform::TitleCase,
        Transform::Trim,
        Transform::JoinLines,
        Transform::RemoveBlankLines,
        Transform::Quote,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Transform::Upper => "대문자로",
            Transform::Lower => "소문자로",
            Transform::TitleCase => "단어 첫 글자만 대문자로",
            Transform::Trim => "줄마다 앞뒤 공백 제거",
            Transform::JoinLines => "한 줄로 합치기",
            Transform::RemoveBlankLines => "빈 줄 제거",
            Transform::Quote => "인용 (> 붙이기)",
        }
    }

    pub fn apply(self, s: &str) -> String {
        let nl = if s.contains("\r\n") { "\r\n" } else { "\n" };
        let lines = || s.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l));
        match self {
            Transform::Upper => s.to_uppercase(),
            Transform::Lower => s.to_lowercase(),
            Transform::TitleCase => {
                let mut out = String::with_capacity(s.len());
                let mut start = true;
                for c in s.chars() {
                    if c.is_alphanumeric() {
                        if start {
                            out.extend(c.to_uppercase());
                        } else {
                            out.extend(c.to_lowercase());
                        }
                        start = false;
                    } else {
                        out.push(c);
                        start = true;
                    }
                }
                out
            }
            Transform::Trim => lines().map(str::trim).collect::<Vec<_>>().join(nl).trim().to_string(),
            Transform::JoinLines => lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            Transform::RemoveBlankLines => {
                lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join(nl)
            }
            Transform::Quote => lines().map(|l| format!("> {}", l)).collect::<Vec<_>>().join(nl),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Transform::*;

    #[test]
    fn transforms() {
        assert_eq!(Upper.apply("abc 가"), "ABC 가");
        assert_eq!(TitleCase.apply("hello wORLD-foo"), "Hello World-Foo");
        assert_eq!(Trim.apply("  a  \r\n  b \r\n"), "a\r\nb");
        assert_eq!(JoinLines.apply("a\n\n  b\nc  "), "a b c");
        assert_eq!(RemoveBlankLines.apply("a\r\n\r\n  \r\nb"), "a\r\nb");
        assert_eq!(Quote.apply("a\nb"), "> a\n> b");
    }
}
