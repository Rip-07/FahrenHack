//! Global hotkeys polled with GetAsyncKeyState (work while the game has focus).

use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    vk: i32,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

fn down(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16) & 0x8000 != 0 }
}

impl Hotkey {
    /// "F1", "CTRL+F5", "SHIFT+NUM3", "ALT+K" …
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut hk = Hotkey { vk: 0, ctrl: false, shift: false, alt: false };
        for part in s.to_uppercase().split('+').map(str::trim) {
            match part {
                "CTRL" | "CONTROL" => hk.ctrl = true,
                "SHIFT" => hk.shift = true,
                "ALT" => hk.alt = true,
                "" => {}
                p => {
                    if hk.vk != 0 {
                        return None;
                    }
                    hk.vk = if let Some(n) = p.strip_prefix('F').and_then(|n| n.parse::<i32>().ok()) {
                        if !(1..=24).contains(&n) {
                            return None;
                        }
                        0x6F + n
                    } else if let Some(n) = p.strip_prefix("NUM").and_then(|n| n.parse::<i32>().ok()) {
                        if !(0..=9).contains(&n) {
                            return None;
                        }
                        0x60 + n
                    } else if p.len() == 1 && p.chars().all(|c| c.is_ascii_alphanumeric()) {
                        p.as_bytes()[0] as i32
                    } else {
                        return None;
                    };
                }
            }
        }
        (hk.vk != 0).then_some(hk)
    }

    pub fn pressed(&self) -> bool {
        down(self.vk) && down(0x11) == self.ctrl && down(0x10) == self.shift && down(0x12) == self.alt
    }
}

#[cfg(test)]
mod tests {
    use super::Hotkey;

    #[test]
    fn parse_keys() {
        assert!(Hotkey::parse("F1").is_some());
        assert!(Hotkey::parse("ctrl+F12").is_some());
        assert!(Hotkey::parse("SHIFT+NUM5").is_some());
        assert!(Hotkey::parse("ALT+K").is_some());
        assert!(Hotkey::parse("F25").is_none());
        assert!(Hotkey::parse("F1+F2").is_none());
        assert!(Hotkey::parse("CTRL").is_none());
    }
}
