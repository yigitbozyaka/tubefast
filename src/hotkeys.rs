use crate::app::Action;
use eframe::egui::{Key, Modifiers};
use serde::{Deserialize, Serialize};

pub const ALT: u8 = 1;
pub const CTRL: u8 = 2;
pub const SHIFT: u8 = 4;
pub const SUPPORTED: bool = cfg!(windows);
const VOLUME_STEP: f32 = 0.05;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
}

impl Command {
    pub const ALL: [Command; 5] = [
        Command::PlayPause,
        Command::Next,
        Command::Previous,
        Command::VolumeUp,
        Command::VolumeDown,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::PlayPause => "Play / Pause",
            Command::Next => "Next song",
            Command::Previous => "Previous song",
            Command::VolumeUp => "Volume up",
            Command::VolumeDown => "Volume down",
        }
    }

    pub fn action(self) -> Action {
        match self {
            Command::PlayPause => Action::Toggle,
            Command::Next => Action::Next,
            Command::Previous => Action::Previous,
            Command::VolumeUp => Action::VolumeBy(VOLUME_STEP),
            Command::VolumeDown => Action::VolumeBy(-VOLUME_STEP),
        }
    }

    pub fn standard(self) -> Combo {
        let vk = match self {
            Command::PlayPause => 0x20,
            Command::Next => 0x27,
            Command::Previous => 0x25,
            Command::VolumeUp => 0x26,
            Command::VolumeDown => 0x28,
        };
        Combo { mods: CTRL | ALT, vk }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    fn repeats(self) -> bool {
        matches!(self, Command::VolumeUp | Command::VolumeDown)
    }
}

pub fn standard_combos() -> [Option<Combo>; 5] {
    Command::ALL.map(|command| Some(command.standard()))
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Combo {
    pub mods: u8,
    pub vk: u32,
}

impl Combo {
    pub fn from_input(key: Key, modifiers: Modifiers) -> Option<Self> {
        let mods = [(modifiers.alt, ALT), (modifiers.ctrl, CTRL), (modifiers.shift, SHIFT)]
            .into_iter()
            .filter(|(held, _)| *held)
            .fold(0, |mods, (_, bit)| mods | bit);
        (mods != 0).then_some(Self {
            mods,
            vk: virtual_key(key)?,
        })
    }

    pub fn parts(&self) -> Vec<String> {
        let held = [(CTRL, "Ctrl"), (ALT, "Alt"), (SHIFT, "Shift")];
        let mut parts: Vec<String> = held
            .into_iter()
            .filter(|(bit, _)| self.mods & bit != 0)
            .map(|(_, name)| name.to_owned())
            .collect();
        parts.push(key_name(self.vk));
        parts
    }
}

fn virtual_key(key: Key) -> Option<u32> {
    let name = format!("{key:?}");
    let fixed = match name.as_str() {
        "Space" => 0x20,
        "Tab" => 0x09,
        "Enter" => 0x0D,
        "PageUp" => 0x21,
        "PageDown" => 0x22,
        "End" => 0x23,
        "Home" => 0x24,
        "ArrowLeft" => 0x25,
        "ArrowUp" => 0x26,
        "ArrowRight" => 0x27,
        "ArrowDown" => 0x28,
        "Insert" => 0x2D,
        _ => 0,
    };
    if fixed != 0 {
        return Some(fixed);
    }
    if let [letter] = name.as_bytes() {
        return Some(*letter as u32);
    }
    if let Some(digit) = name.strip_prefix("Num").and_then(|digit| digit.parse::<u32>().ok()) {
        return Some(0x30 + digit);
    }
    let function = name.strip_prefix('F')?.parse::<u32>().ok()?;
    (1..=24).contains(&function).then_some(0x6F + function)
}

fn key_name(vk: u32) -> String {
    match vk {
        0x09 => "Tab".into(),
        0x0D => "Enter".into(),
        0x20 => "Space".into(),
        0x21 => "Page Up".into(),
        0x22 => "Page Down".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2D => "Insert".into(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => char::from_u32(vk).map(String::from).unwrap_or_default(),
    }
}

#[cfg(windows)]
pub struct Hotkeys {
    thread: u32,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl Hotkeys {
    pub fn start(bindings: Vec<(Command, Combo)>, press: impl Fn(Command) + Send + 'static) -> Option<(Self, Vec<Command>)> {
        use std::ptr::null_mut;
        use windows_sys::Win32::System::Threading::GetCurrentThreadId;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, WM_HOTKEY};

        let (ready, started) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("hotkeys".into())
            .spawn(move || unsafe {
                let mut message: MSG = std::mem::zeroed();
                PeekMessageW(&mut message, null_mut(), 0, 0, PM_NOREMOVE);
                let (mut held, mut taken) = (Vec::new(), Vec::new());
                for (command, combo) in bindings {
                    let repeat = if command.repeats() { 0 } else { MOD_NOREPEAT };
                    let registered = RegisterHotKey(null_mut(), command as i32, combo.mods as u32 | repeat, combo.vk);
                    if registered != 0 { &mut held } else { &mut taken }.push(command);
                }
                let _ = ready.send((GetCurrentThreadId(), taken));
                while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                    if message.message == WM_HOTKEY
                        && let Some(command) = Command::ALL.get(message.wParam)
                    {
                        press(*command);
                    }
                }
                for command in held {
                    UnregisterHotKey(null_mut(), command as i32);
                }
            })
            .ok()?;
        let (thread, taken) = started.recv().ok()?;
        Some((
            Self {
                thread,
                worker: Some(worker),
            },
            taken,
        ))
    }
}

#[cfg(windows)]
impl Drop for Hotkeys {
    fn drop(&mut self) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(not(windows))]
pub struct Hotkeys;

#[cfg(not(windows))]
impl Hotkeys {
    pub fn start(_: Vec<(Command, Combo)>, _: impl Fn(Command) + Send + 'static) -> Option<(Self, Vec<Command>)> {
        None
    }
}
