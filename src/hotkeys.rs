use crate::app::Action;
use eframe::egui::{Key, Modifiers};
use serde::de::{Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const ALT: u8 = 1;
pub const CTRL: u8 = 2;
pub const SHIFT: u8 = 4;
pub const COUNT: usize = 6;
const VOLUME_STEP: f32 = 0.05;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
    Overlay,
}

impl Command {
    pub const ALL: [Command; COUNT] = [
        Command::PlayPause,
        Command::Next,
        Command::Previous,
        Command::VolumeUp,
        Command::VolumeDown,
        Command::Overlay,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::PlayPause => "Play / Pause",
            Command::Next => "Next song",
            Command::Previous => "Previous song",
            Command::VolumeUp => "Volume up",
            Command::VolumeDown => "Volume down",
            Command::Overlay => "Show / hide overlay",
        }
    }

    pub fn action(self) -> Action {
        match self {
            Command::PlayPause => Action::Toggle,
            Command::Next => Action::Next,
            Command::Previous => Action::Previous,
            Command::VolumeUp => Action::VolumeBy(VOLUME_STEP),
            Command::VolumeDown => Action::VolumeBy(-VOLUME_STEP),
            Command::Overlay => Action::ToggleOverlay,
        }
    }

    pub fn standard(self) -> Combo {
        let vk = match self {
            Command::PlayPause => 0x20,
            Command::Next => 0x27,
            Command::Previous => 0x25,
            Command::VolumeUp => 0x26,
            Command::VolumeDown => 0x28,
            Command::Overlay => 0x4F,
        };
        Combo { mods: CTRL | ALT, vk }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    fn repeats(self) -> bool {
        matches!(self, Command::VolumeUp | Command::VolumeDown)
    }
}

pub fn standard_combos() -> [Option<Combo>; COUNT] {
    Command::ALL.map(|command| Some(command.standard()))
}

struct PaddedCombos;

impl<'de> Visitor<'de> for PaddedCombos {
    type Value = [Option<Combo>; COUNT];

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a list of hotkey combos")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut combos = standard_combos();
        for slot in &mut combos {
            match seq.next_element()? {
                Some(stored) => *slot = stored,
                None => break,
            }
        }
        Ok(combos)
    }
}

pub fn padded_combos<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[Option<Combo>; COUNT], D::Error> {
    deserializer.deserialize_tuple(COUNT, PaddedCombos)
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

#[cfg(all(unix, not(target_os = "macos")))]
mod portal;

#[cfg(not(windows))]
pub enum Hotkeys {
    Grabbed {
        manager: global_hotkey::GlobalHotKeyManager,
        registered: Vec<global_hotkey::hotkey::HotKey>,
    },
    #[cfg(all(unix, not(target_os = "macos")))]
    Portal(#[allow(dead_code)] async_channel::Sender<()>),
}

#[cfg(not(windows))]
fn code_name(vk: u32) -> Option<String> {
    let name = match vk {
        0x09 => "Tab".to_owned(),
        0x0D => "Enter".to_owned(),
        0x20 => "Space".to_owned(),
        0x21 => "PageUp".to_owned(),
        0x22 => "PageDown".to_owned(),
        0x23 => "End".to_owned(),
        0x24 => "Home".to_owned(),
        0x25 => "ArrowLeft".to_owned(),
        0x26 => "ArrowUp".to_owned(),
        0x27 => "ArrowRight".to_owned(),
        0x28 => "ArrowDown".to_owned(),
        0x2D => "Insert".to_owned(),
        0x30..=0x39 => format!("Digit{}", vk - 0x30),
        0x41..=0x5A => format!("Key{}", char::from_u32(vk)?),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => return None,
    };
    Some(name)
}

#[cfg(not(windows))]
fn hotkey_of(combo: Combo) -> Option<global_hotkey::hotkey::HotKey> {
    use global_hotkey::hotkey::{Code, HotKey, Modifiers as Held};
    let held = [(ALT, Held::ALT), (CTRL, Held::CONTROL), (SHIFT, Held::SHIFT)]
        .into_iter()
        .filter(|(bit, _)| combo.mods & bit != 0)
        .fold(Held::empty(), |held, (_, modifier)| held | modifier);
    Some(HotKey::new(Some(held), code_name(combo.vk)?.parse::<Code>().ok()?))
}

#[cfg(not(windows))]
fn wayland_session() -> bool {
    let forced_x11 = std::env::var("WINIT_UNIX_BACKEND").is_ok_and(|backend| backend == "x11");
    std::env::var_os("WAYLAND_DISPLAY").is_some() && !forced_x11
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn through_portal() -> bool {
    wayland_session() && !portal::refused()
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn through_portal() -> bool {
    false
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn portal_trigger(command: Command) -> Option<String> {
    portal::trigger(command)
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn portal_trigger(_: Command) -> Option<String> {
    None
}

pub fn listed() -> Vec<Command> {
    Command::ALL
        .into_iter()
        .filter(|command| !(through_portal() && *command == Command::Overlay))
        .collect()
}

#[cfg(not(windows))]
pub fn unavailable() -> Option<&'static str> {
    if cfg!(target_os = "macos") || through_portal() {
        None
    } else if wayland_session() {
        Some("This desktop's global shortcuts portal is missing or refused Tubefast, so Wayland cannot listen for keys")
    } else if std::env::var_os("DISPLAY").is_none() {
        Some("No X11 display was found")
    } else {
        None
    }
}

#[cfg(windows)]
pub fn unavailable() -> Option<&'static str> {
    None
}

#[cfg(not(windows))]
impl Hotkeys {
    pub fn start(bindings: Vec<(Command, Combo)>, press: impl Fn(Command) + Send + 'static) -> Option<(Self, Vec<Command>)> {
        use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

        if unavailable().is_some() {
            return None;
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        if through_portal() {
            return Some((Self::Portal(portal::start(bindings, press)?), Vec::new()));
        }
        let manager = GlobalHotKeyManager::new().ok()?;
        let (mut registered, mut taken, mut commands) = (Vec::new(), Vec::new(), Vec::new());
        for (command, combo) in bindings {
            match hotkey_of(combo) {
                Some(hotkey) if manager.register(hotkey).is_ok() => {
                    commands.push((hotkey.id(), command));
                    registered.push(hotkey);
                }
                _ => taken.push(command),
            }
        }
        let press = std::sync::Mutex::new(press);
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() != HotKeyState::Pressed {
                return;
            }
            if let Some((_, command)) = commands.iter().find(|(id, _)| *id == event.id())
                && let Ok(press) = press.lock()
            {
                press(*command);
            }
        }));
        Some((Self::Grabbed { manager, registered }, taken))
    }
}

#[cfg(not(windows))]
impl Drop for Hotkeys {
    fn drop(&mut self) {
        match self {
            Self::Grabbed { manager, registered } => {
                global_hotkey::GlobalHotKeyEvent::set_event_handler(None::<fn(global_hotkey::GlobalHotKeyEvent)>);
                let _ = manager.unregister_all(registered);
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            Self::Portal(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Stored {
        #[serde(deserialize_with = "padded_combos")]
        combos: [Option<Combo>; COUNT],
    }

    #[test]
    fn overlay_command_toggles_the_overlay() {
        assert!(matches!(Command::Overlay.action(), Action::ToggleOverlay));
        assert!(Command::ALL[Command::Overlay as usize] == Command::Overlay);
    }

    #[test]
    fn standard_combos_never_collide() {
        let combos = standard_combos();
        for (at, combo) in combos.iter().enumerate() {
            assert!(!combos[at + 1..].contains(combo));
        }
    }

    #[test]
    fn saves_from_before_the_overlay_keep_their_combos_and_get_the_standard_one() {
        let old = r#"{"combos":[null,{"mods":2,"vk":65},null,null,null]}"#;
        let stored: Stored = serde_json::from_str(old).unwrap();
        assert!(stored.combos[0].is_none());
        assert!(stored.combos[1] == Some(Combo { mods: CTRL, vk: 65 }));
        assert!(stored.combos[Command::Overlay as usize] == Some(Command::Overlay.standard()));
    }
}
