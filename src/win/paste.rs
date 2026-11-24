//! 대상 창으로 돌아가서 붙여넣기 키(Ctrl+V)를 보냅니다.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let extended = matches!(vk, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_INSERT | VK_DELETE);
    let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 },
        },
    }
}

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
}

/// 사용자가 아직 누르고 있는 Shift/Ctrl/Alt/Win 을 떼어 줍니다(안 그러면 Ctrl+Shift+V 처럼 섞임).
fn release_modifiers() {
    let mut v = Vec::new();
    for vk in [VK_LSHIFT, VK_RSHIFT, VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN] {
        if unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0 {
            v.push(key(vk, true));
        }
    }
    send(&v);
}

/// 대상 창을 앞으로 가져옵니다.
pub fn focus(target: HWND) -> bool {
    unsafe {
        if target.is_null() || IsWindow(target) == 0 {
            return false;
        }
        if IsIconic(target) != 0 {
            ShowWindow(target, SW_RESTORE);
        }
        SetForegroundWindow(target);
        // 포커스가 넘어갈 시간을 잠깐 줍니다.
        for _ in 0..20 {
            if GetForegroundWindow() == target {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
        GetForegroundWindow() == target
    }
}

pub fn send_paste(shift_insert: bool, cursor_back: Option<usize>) {
    release_modifiers();
    let mut v = if shift_insert {
        vec![key(VK_SHIFT, false), key(VK_INSERT, false), key(VK_INSERT, true), key(VK_SHIFT, true)]
    } else {
        vec![key(VK_CONTROL, false), key('V' as u16, false), key('V' as u16, true), key(VK_CONTROL, true)]
    };
    send(&v);
    if let Some(n) = cursor_back.filter(|&n| n > 0) {
        // 붙여넣기가 끝날 때까지 잠깐 기다린 뒤 커서를 옮깁니다.
        std::thread::sleep(std::time::Duration::from_millis(60));
        v.clear();
        for _ in 0..n.min(5000) {
            v.push(key(VK_LEFT, false));
            v.push(key(VK_LEFT, true));
        }
        send(&v);
    }
}
