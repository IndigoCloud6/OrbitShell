use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

pub fn encode(key: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let name = key.key.as_str();
    let modifiers = key.modifiers;
    if modifiers.control && !modifiers.alt && matches!(name, "b" | "f" | "j" | "," | "tab")
        || modifiers.control && modifiers.shift && matches!(name, "t" | "w")
    {
        return None;
    }

    let modifier = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let cursor = match name {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        _ => None,
    };
    if let Some(cursor) = cursor {
        return Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{cursor}")
            } else if mode.contains(TermMode::APP_CURSOR) {
                format!("\x1bO{cursor}")
            } else {
                format!("\x1b[{cursor}")
            }
            .into_bytes(),
        );
    }
    let function = match name {
        "insert" => Some(2),
        "delete" => Some(3),
        "pageup" => Some(5),
        "pagedown" => Some(6),
        "f5" => Some(15),
        "f6" => Some(17),
        "f7" => Some(18),
        "f8" => Some(19),
        "f9" => Some(20),
        "f10" => Some(21),
        "f11" => Some(23),
        "f12" => Some(24),
        _ => None,
    };
    if let Some(function) = function {
        return Some(
            if modifier > 1 {
                format!("\x1b[{function};{modifier}~")
            } else {
                format!("\x1b[{function}~")
            }
            .into_bytes(),
        );
    }
    let control = match name {
        "enter" => b"\r".as_slice(),
        "backspace" => {
            if modifiers.control {
                b"\x08"
            } else {
                b"\x7f"
            }
        }
        "tab" if modifiers.shift => b"\x1b[Z",
        "tab" => b"\t",
        "escape" => b"\x1b",
        "f1" => b"\x1bOP",
        "f2" => b"\x1bOQ",
        "f3" => b"\x1bOR",
        "f4" => b"\x1bOS",
        _ => &[],
    };
    if !control.is_empty() {
        return Some(control.to_vec());
    }

    if modifiers.control && !modifiers.alt {
        let byte = match name.as_bytes() {
            [letter @ b'a'..=b'z'] => letter - b'a' + 1,
            [b'@' | b' '] => 0,
            [b'[' | b'\\' | b']' | b'^' | b'_'] => name.as_bytes()[0] & 0x1f,
            _ => return None,
        };
        return Some(vec![byte]);
    }
    if modifiers.alt
        && !modifiers.control
        && let Some(text) = key
            .key_char
            .as_deref()
            .or_else(|| (name == "space").then_some(" "))
    {
        let mut bytes = vec![0x1b];
        bytes.extend_from_slice(text.as_bytes());
        return Some(bytes);
    }
    None
}
