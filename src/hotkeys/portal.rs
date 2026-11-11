use super::{ALT, CTRL, Combo, Command, SHIFT};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_lite::StreamExt;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static REFUSED: AtomicBool = AtomicBool::new(false);
static TRIGGERS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

pub fn refused() -> bool {
    REFUSED.load(Ordering::SeqCst)
}

pub fn trigger(command: Command) -> Option<String> {
    let id = id_of(command);
    let triggers = TRIGGERS.lock().ok()?;
    triggers.iter().find(|(known, _)| *known == id).map(|(_, trigger)| trigger.clone())
}

fn id_of(command: Command) -> String {
    (command as usize).to_string()
}

fn key_symbol(vk: u32) -> Option<String> {
    let symbol = match vk {
        0x09 => "Tab".to_owned(),
        0x0D => "Return".to_owned(),
        0x20 => "space".to_owned(),
        0x23 => "End".to_owned(),
        0x24 => "Home".to_owned(),
        0x25 => "Left".to_owned(),
        0x26 => "Up".to_owned(),
        0x27 => "Right".to_owned(),
        0x28 => "Down".to_owned(),
        0x2D => "Insert".to_owned(),
        0x30..=0x39 => char::from_u32(vk)?.to_string(),
        0x41..=0x5A => char::from_u32(vk)?.to_ascii_lowercase().to_string(),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => return None,
    };
    Some(symbol)
}

fn preferred_trigger(combo: Combo) -> Option<String> {
    let mut parts: Vec<String> = [(CTRL, "CTRL"), (ALT, "ALT"), (SHIFT, "SHIFT")]
        .into_iter()
        .filter(|(bit, _)| combo.mods & bit != 0)
        .map(|(_, name)| name.to_owned())
        .collect();
    parts.push(key_symbol(combo.vk)?);
    Some(parts.join("+"))
}

async fn run(bindings: Vec<(Command, Combo)>, press: impl Fn(Command), stop: async_channel::Receiver<()>) -> Result<(), ashpd::Error> {
    let shortcuts = GlobalShortcuts::new().await?;
    let session = shortcuts.create_session().await?;
    let mut activated = std::pin::pin!(shortcuts.receive_activated().await?);
    let wanted: Vec<NewShortcut> = bindings
        .iter()
        .filter(|(command, _)| *command != Command::Overlay || super::layer_overlay())
        .map(|(command, combo)| NewShortcut::new(id_of(*command), command.name()).preferred_trigger(preferred_trigger(*combo).as_deref()))
        .collect();
    let bound = shortcuts.bind_shortcuts(&session, &wanted, None).await?.response()?;
    if let Ok(mut triggers) = TRIGGERS.lock() {
        *triggers = bound
            .shortcuts()
            .iter()
            .map(|shortcut| (shortcut.id().to_owned(), shortcut.trigger_description().to_owned()))
            .collect();
    }
    loop {
        let next = futures_lite::future::or(activated.next(), async {
            let _ = stop.recv().await;
            None
        })
        .await;
        let Some(activation) = next else { break };
        if let Some(command) = Command::ALL.into_iter().find(|command| id_of(*command) == activation.shortcut_id()) {
            press(command);
        }
    }
    let _ = session.close().await;
    Ok(())
}

pub fn start(bindings: Vec<(Command, Combo)>, press: impl Fn(Command) + Send + 'static) -> Option<async_channel::Sender<()>> {
    let (keeper, stop) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("hotkey-portal".to_owned())
        .spawn(move || {
            if futures_lite::future::block_on(run(bindings, press, stop)).is_err() {
                REFUSED.store(true, Ordering::SeqCst);
            }
        })
        .ok()?;
    Some(keeper)
}
