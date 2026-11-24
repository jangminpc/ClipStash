//! "Ctrl+Shift+V" 같은 문자열을 Windows RegisterHotKey 용 값으로 바꿉니다.

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}

fn key_code(name: &str) -> Option<u32> {
    let upper = name.to_ascii_uppercase();
    let b = upper.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_uppercase() || b[0].is_ascii_digit()) {
        return Some(b[0] as u32);
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    Some(match upper.as_str() {
        "SPACE" => 0x20,
        "ENTER" | "RETURN" => 0x0D,
        "TAB" => 0x09,
        "ESC" | "ESCAPE" => 0x1B,
        "BACKSPACE" => 0x08,
        "INSERT" | "INS" => 0x2D,
        "DELETE" | "DEL" => 0x2E,
        "HOME" => 0x24,
        "END" => 0x23,
        "PAGEUP" | "PGUP" => 0x21,
        "PAGEDOWN" | "PGDN" => 0x22,
        "LEFT" => 0x25,
        "UP" => 0x26,
        "RIGHT" => 0x27,
        "DOWN" => 0x28,
        "`" | "BACKQUOTE" | "GRAVE" => 0xC0,
        "-" | "MINUS" => 0xBD,
        "=" | "EQUALS" | "PLUS" => 0xBB,
        "," | "COMMA" => 0xBC,
        "." | "PERIOD" => 0xBE,
        "/" | "SLASH" => 0xBF,
        ";" | "SEMICOLON" => 0xBA,
        "'" | "QUOTE" => 0xDE,
        "[" => 0xDB,
        "]" => 0xDD,
        "\\" | "BACKSLASH" => 0xDC,
        _ => return None,
    })
}

pub fn parse(s: &str) -> Result<Hotkey, String> {
    let mut modifiers = 0;
    let mut vk = None;
    for part in s.split('+').map(str::trim) {
        if part.is_empty() {
            return Err(format!("단축키 형식이 잘못되었습니다: '{}'", s));
        }
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "alt" => modifiers |= MOD_ALT,
            "shift" => modifiers |= MOD_SHIFT,
            "win" | "windows" | "super" => modifiers |= MOD_WIN,
            _ => {
                if vk.is_some() {
                    return Err(format!("키가 두 개 이상 지정되었습니다: '{}'", s));
                }
                vk = Some(key_code(part).ok_or_else(|| format!("알 수 없는 키: '{}'", part))?);
            }
        }
    }
    let vk = vk.ok_or_else(|| format!("키가 지정되지 않았습니다: '{}'", s))?;
    let is_fkey = (0x70..=0x87).contains(&vk);
    if modifiers == 0 && !is_fkey {
        return Err(format!("Ctrl/Alt/Shift/Win 중 하나 이상과 함께 써야 합니다: '{}'", s));
    }
    Ok(Hotkey { modifiers, vk })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(parse("Ctrl+Shift+V"), Ok(Hotkey { modifiers: MOD_CONTROL | MOD_SHIFT, vk: 'V' as u32 }));
        assert_eq!(parse("alt + c"), Ok(Hotkey { modifiers: MOD_ALT, vk: 'C' as u32 }));
        assert_eq!(parse("Win+`").unwrap().vk, 0xC0);
        assert_eq!(parse("F9"), Ok(Hotkey { modifiers: 0, vk: 0x78 }));
    }

    #[test]
    fn rejects() {
        assert!(parse("V").is_err());
        assert!(parse("Ctrl+").is_err());
        assert!(parse("Ctrl+A+B").is_err());
        assert!(parse("Ctrl+Nope").is_err());
        assert!(parse("Ctrl+Shift").is_err());
    }
}
