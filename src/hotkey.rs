use anyhow::{anyhow, Result};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};

#[derive(Debug, Clone)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
    pub display: String,
}

impl Hotkey {
    pub fn register_modifiers(&self) -> HOT_KEY_MODIFIERS {
        HOT_KEY_MODIFIERS(self.modifiers | MOD_NOREPEAT.0)
    }
}

pub fn parse_hotkey(input: &str) -> Result<Hotkey> {
    let mut modifiers = 0u32;
    let mut key: Option<(u32, String)> = None;
    let normalized = input.replace('+', " ");

    for token in normalized.split_whitespace() {
        let upper = token.trim().to_ascii_uppercase();
        if upper.is_empty() {
            continue;
        }

        match upper.as_str() {
            "CTRL" | "CONTROL" => modifiers |= MOD_CONTROL.0,
            "ALT" => modifiers |= MOD_ALT.0,
            "SHIFT" => modifiers |= MOD_SHIFT.0,
            "WIN" | "WINDOWS" | "META" | "CMD" => modifiers |= MOD_WIN.0,
            _ => {
                if key.is_some() {
                    return Err(anyhow!("热键只能有一个主按键"));
                }
                key = Some(parse_key(&upper)?);
            }
        }
    }

    let Some((vk, key_name)) = key else {
        return Err(anyhow!("请输入一个按键，例如 Ctrl+Alt+B 或 F8"));
    };

    let mut parts = Vec::new();
    if modifiers & MOD_CONTROL.0 != 0 {
        parts.push("Ctrl".to_string());
    }
    if modifiers & MOD_ALT.0 != 0 {
        parts.push("Alt".to_string());
    }
    if modifiers & MOD_SHIFT.0 != 0 {
        parts.push("Shift".to_string());
    }
    if modifiers & MOD_WIN.0 != 0 {
        parts.push("Win".to_string());
    }
    parts.push(key_name);

    Ok(Hotkey {
        modifiers,
        vk,
        display: parts.join("+"),
    })
}

fn parse_key(token: &str) -> Result<(u32, String)> {
    if token.len() == 1 {
        let ch = token.as_bytes()[0];
        if ch.is_ascii_alphanumeric() {
            return Ok((ch as u32, token.to_string()));
        }
    }

    if let Some(num) = token.strip_prefix('F') {
        if let Ok(n) = num.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Ok((111 + n, format!("F{}", n)));
            }
        }
    }

    let (vk, name) = match token {
        "ESC" | "ESCAPE" => (0x1B, "Esc"),
        "ENTER" | "RETURN" => (0x0D, "Enter"),
        "SPACE" => (0x20, "Space"),
        "TAB" => (0x09, "Tab"),
        "BACKSPACE" | "BKSP" => (0x08, "Backspace"),
        "DELETE" | "DEL" => (0x2E, "Delete"),
        "INSERT" | "INS" => (0x2D, "Insert"),
        "HOME" => (0x24, "Home"),
        "END" => (0x23, "End"),
        "PAGEUP" | "PGUP" => (0x21, "PageUp"),
        "PAGEDOWN" | "PGDN" => (0x22, "PageDown"),
        "LEFT" => (0x25, "Left"),
        "UP" => (0x26, "Up"),
        "RIGHT" => (0x27, "Right"),
        "DOWN" => (0x28, "Down"),
        "PRINTSCREEN" | "PRTSC" | "PRTSCR" | "SNAPSHOT" => (0x2C, "PrintScreen"),
        "PAUSE" => (0x13, "Pause"),
        _ => return Err(anyhow!("不支持的按键: {}", token)),
    };

    Ok((vk, name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::parse_hotkey;
    use windows::Win32::UI::Input::KeyboardAndMouse::{MOD_ALT, MOD_CONTROL};

    #[test]
    fn parses_combo_hotkey() {
        let hotkey = parse_hotkey("Ctrl+Alt+B").unwrap();
        assert_eq!(hotkey.modifiers, MOD_CONTROL.0 | MOD_ALT.0);
        assert_eq!(hotkey.vk, b'B' as u32);
        assert_eq!(hotkey.display, "Ctrl+Alt+B");
    }

    #[test]
    fn parses_single_key_hotkey() {
        let hotkey = parse_hotkey("F8").unwrap();
        assert_eq!(hotkey.modifiers, 0);
        assert_eq!(hotkey.vk, 0x77);
        assert_eq!(hotkey.display, "F8");
    }

    #[test]
    fn rejects_multiple_main_keys() {
        assert!(parse_hotkey("Ctrl+A+B").is_err());
    }
}
