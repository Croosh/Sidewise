use sidewise_core::types::MediaControl;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE,
    VK_MEDIA_PREV_TRACK,
};

pub struct WindowsMediaControl;

impl Default for WindowsMediaControl {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsMediaControl {
    pub fn new() -> Self {
        Self
    }

    fn send_key(vk: VIRTUAL_KEY) {
        unsafe {
            let inputs = [
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: vk,
                            wScan: 0,
                            dwFlags: KEYEVENTF_EXTENDEDKEY,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: vk,
                            wScan: 0,
                            dwFlags: KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
            ];

            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }
}

impl MediaControl for WindowsMediaControl {
    fn next(&self) {
        Self::send_key(VK_MEDIA_NEXT_TRACK);
    }

    fn prev(&self) {
        Self::send_key(VK_MEDIA_PREV_TRACK);
    }

    fn play_pause(&self) {
        Self::send_key(VK_MEDIA_PLAY_PAUSE);
    }
}
