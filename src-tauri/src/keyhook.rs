//! Windows only: swallow the key-up (and auto-repeat key-downs) of the global
//! shortcut's main key so it does not reach the window behind PopSearch.

pub const MOD_CTRL: u32 = 1;
pub const MOD_SHIFT: u32 = 2;
pub const MOD_ALT: u32 = 4;
pub const MOD_WIN: u32 = 8;

/// Parse an accelerator string into (virtual-key code, modifier mask).
pub fn accel_to_vk(accel: &str) -> Option<(u32, u32)> {
    let mut mods = 0u32;
    let mut vk: Option<u32> = None;
    for part in accel.split('+') {
        let p = part.trim();
        match p.to_ascii_lowercase().as_str() {
            "commandorcontrol" | "commandorctrl" | "cmdorctrl" | "ctrl" | "control" => {
                mods |= MOD_CTRL
            }
            "shift" => mods |= MOD_SHIFT,
            "alt" | "option" => mods |= MOD_ALT,
            "super" | "win" | "meta" => mods |= MOD_WIN,
            _ => {
                if vk.is_some() {
                    return None;
                }
                vk = Some(key_to_vk(p)?);
            }
        }
    }
    Some((vk?, mods))
}

fn key_to_vk(key: &str) -> Option<u32> {
    let rest = key
        .strip_prefix("Key")
        .or_else(|| key.strip_prefix("Digit"))
        .unwrap_or(key);
    if rest.len() == 1 && (key.starts_with("Key") || key.len() == 1) {
        let c = rest.as_bytes()[0].to_ascii_uppercase();
        if c.is_ascii_uppercase() {
            return Some(c as u32);
        }
    }
    if rest.len() == 1 && (key.starts_with("Digit") || key.len() == 1) {
        let c = rest.as_bytes()[0];
        if c.is_ascii_digit() {
            return Some(c as u32);
        }
    }
    if let Some(n) = key.strip_prefix('F').and_then(|s| s.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x6F + n);
        }
    }
    Some(match key.to_ascii_lowercase().as_str() {
        "space" => 0x20,
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "backquote" => 0xC0,
        "minus" => 0xBD,
        "equal" => 0xBB,
        "bracketleft" => 0xDB,
        "bracketright" => 0xDD,
        "backslash" => 0xDC,
        "semicolon" => 0xBA,
        "quote" => 0xDE,
        "comma" => 0xBC,
        "period" => 0xBE,
        "slash" => 0xBF,
        "arrowleft" | "left" => 0x25,
        "arrowup" | "up" => 0x26,
        "arrowright" | "right" => 0x27,
        "arrowdown" | "down" => 0x28,
        _ => return None,
    })
}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, KBDLLHOOKSTRUCT,SetWindowsHookExW, WH_KEYBOARD_LL,
        WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    static HOTKEY_VK: AtomicU32 = AtomicU32::new(0);
    static HOTKEY_MODS: AtomicU32 = AtomicU32::new(0);
    static SWALLOW_VK: AtomicU32 = AtomicU32::new(0);

    pub fn set_hotkey(accel: &str) {
        SWALLOW_VK.store(0, Ordering::SeqCst);
        match super::accel_to_vk(accel) {
            Some((vk, mods)) => {
                HOTKEY_MODS.store(mods, Ordering::SeqCst);
                HOTKEY_VK.store(vk, Ordering::SeqCst);
            }
            None => {
                HOTKEY_VK.store(0, Ordering::SeqCst);
                HOTKEY_MODS.store(0, Ordering::SeqCst);
            }
        }
    }

    pub fn disable() {
        HOTKEY_VK.store(0, Ordering::SeqCst);
        SWALLOW_VK.store(0, Ordering::SeqCst);
    }

    pub fn install() {
        unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), std::ptr::null_mut(), 0);
        }
    }

    fn is_modifier(vk: u32) -> bool {
        matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C)
    }

    unsafe fn down(vk: i32) -> bool {
        (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
    }

    unsafe fn current_mods() -> u32 {
        let mut m = 0;
        if unsafe { down(0x11) } {
            m |= super::MOD_CTRL;
        }
        if unsafe { down(0x10) } {
            m |= super::MOD_SHIFT;
        }
        if unsafe { down(0x12) } {
            m |= super::MOD_ALT;
        }
        if unsafe { down(0x5B) || down(0x5C) } {
            m |= super::MOD_WIN;
        }
        m
    }

    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let info = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let vk = info.vkCode;
            let msg = wparam as u32;
            let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            let swallow = SWALLOW_VK.load(Ordering::SeqCst);
            if swallow != 0 && vk == swallow {
                if is_down {
                    return 1;
                }
                if is_up {
                    SWALLOW_VK.store(0, Ordering::SeqCst);
                    return 1;
                }
            } else if is_down {
                if swallow != 0 && !is_modifier(vk) {
                    SWALLOW_VK.store(0, Ordering::SeqCst);
                }
                let hk = HOTKEY_VK.load(Ordering::SeqCst);
                if hk != 0
                    && vk == hk
                    && unsafe { current_mods() } == HOTKEY_MODS.load(Ordering::SeqCst)
                {
                    SWALLOW_VK.store(vk, Ordering::SeqCst);
                }
            }
        }
        unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
    }
}

#[cfg(windows)]
pub use imp::{disable, install, set_hotkey};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accel_ctrl_shift_space() {
        assert_eq!(
            accel_to_vk("CommandOrControl+Shift+Space"),
            Some((0x20, MOD_CTRL | MOD_SHIFT))
        );
    }

    #[test]
    fn accel_alt_keyq() {
        assert_eq!(accel_to_vk("Alt+KeyQ"), Some((0x51, MOD_ALT)));
    }

    #[test]
    fn accel_ctrl_alt_f8() {
        assert_eq!(accel_to_vk("Ctrl+Alt+F8"), Some((0x77, MOD_CTRL | MOD_ALT)));
    }

    #[test]
    fn accel_super_digit1() {
        assert_eq!(accel_to_vk("Super+Digit1"), Some((0x31, MOD_WIN)));
    }

    #[test]
    fn accel_unknown_key() {
        assert_eq!(accel_to_vk("Ctrl+Foo"), None);
    }
}
