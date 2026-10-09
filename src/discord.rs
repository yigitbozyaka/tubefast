use crate::ytm::{self, Item};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const APPLICATION_ID: &str = "1558230893519306802";
const PIPES: u8 = 10;
const HANDSHAKE: u32 = 0;
const FRAME: u32 = 1;
const CLOSE: u32 = 2;
const PING: u32 = 3;
const LISTENING: u8 = 2;
const CHECK_EVERY: Duration = Duration::from_secs(15);
const SEEK_TOLERANCE_MS: u64 = 2000;
const COVER_SIDE: u32 = 512;
const SHORTEST_LINE: usize = 2;
const LONGEST_LINE: usize = 128;

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Headline {
    #[default]
    App,
    Artist,
    Song,
}

pub struct Discord {
    updates: Sender<Option<Value>>,
    shown: Option<(String, String, Headline, u64)>,
}

impl Discord {
    pub fn start(report: impl Fn(bool) + Send + 'static) -> Self {
        let (updates, inbox) = mpsc::channel();
        std::thread::spawn(move || run(inbox, report));
        Self { updates, shown: None }
    }

    pub fn show(&mut self, track: Option<(&Item, &str)>, headline: Headline, position_ms: u64, duration_ms: u64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        let started = now.saturating_sub(position_ms);
        let track = track.filter(|_| duration_ms > 0);
        let unchanged = match (&self.shown, track) {
            (Some((video_id, art, named, shown)), Some((item, cover))) => {
                *video_id == item.video_id && art == cover && *named == headline && shown.abs_diff(started) < SEEK_TOLERANCE_MS
            }
            (None, None) => true,
            _ => false,
        };
        if unchanged {
            return;
        }
        self.shown = track.map(|(item, cover)| (item.video_id.clone(), cover.to_owned(), headline, started));
        let _ = self
            .updates
            .send(track.map(|(item, cover)| activity(item, cover, headline, started, duration_ms)));
    }
}

fn run(updates: Receiver<Option<Value>>, report: impl Fn(bool)) {
    let (mut pipe, mut activity, mut delivered, mut nonce) = (None, None, false, 0u64);
    let mut linked = None;
    loop {
        if pipe.is_none() {
            (pipe, delivered) = (connect(), false);
        }
        if let Some(open) = &mut pipe {
            let sent = if delivered {
                exchange(open, PING, &json!({}))
            } else {
                nonce += 1;
                let args = json!({ "pid": std::process::id(), "activity": activity });
                exchange(
                    open,
                    FRAME,
                    &json!({ "cmd": "SET_ACTIVITY", "args": args, "nonce": nonce.to_string() }),
                )
            };
            delivered = sent.is_ok();
            if !delivered {
                pipe = None;
            }
        }
        if linked != Some(pipe.is_some()) {
            linked = Some(pipe.is_some());
            report(pipe.is_some());
        }
        match updates.recv_timeout(CHECK_EVERY) {
            Ok(update) => (activity, delivered) = (updates.try_iter().last().unwrap_or(update), false),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn connect() -> Option<Pipe> {
    (0..PIPES).find_map(|number| {
        let mut pipe = open(number)?;
        exchange(&mut pipe, HANDSHAKE, &json!({ "v": 1, "client_id": APPLICATION_ID })).ok()?;
        Some(pipe)
    })
}

#[cfg(windows)]
type Pipe = std::fs::File;

#[cfg(windows)]
fn open(number: u8) -> Option<Pipe> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!(r"\\.\pipe\discord-ipc-{number}"))
        .ok()
}

#[cfg(unix)]
type Pipe = std::os::unix::net::UnixStream;

#[cfg(unix)]
const SANDBOXES: [&str; 3] = ["", "app/com.discordapp.Discord", "snap.discord"];

#[cfg(unix)]
fn open(number: u8) -> Option<Pipe> {
    let folder = ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"]
        .into_iter()
        .find_map(std::env::var_os)
        .unwrap_or_else(|| "/tmp".into());
    let folder = std::path::Path::new(&folder);
    SANDBOXES
        .iter()
        .find_map(|sandbox| Pipe::connect(folder.join(sandbox).join(format!("discord-ipc-{number}"))).ok())
}

fn frame(opcode: u32, payload: &Value) -> Vec<u8> {
    let body = payload.to_string();
    [&opcode.to_le_bytes()[..], &(body.len() as u32).to_le_bytes(), body.as_bytes()].concat()
}

fn exchange(pipe: &mut (impl Read + Write), opcode: u32, payload: &Value) -> io::Result<()> {
    pipe.write_all(&frame(opcode, payload))?;
    let mut word = [0; 4];
    pipe.read_exact(&mut word)?;
    let answer = u32::from_le_bytes(word);
    pipe.read_exact(&mut word)?;
    let length = u64::from(u32::from_le_bytes(word));
    let skipped = io::copy(&mut pipe.take(length), &mut io::sink())?;
    if answer == CLOSE || skipped < length {
        return Err(io::ErrorKind::ConnectionAborted.into());
    }
    Ok(())
}

fn activity(item: &Item, cover: &str, headline: Headline, started: u64, duration_ms: u64) -> Value {
    let mut activity = json!({
        "type": LISTENING,
        "status_display_type": headline as u8,
        "details": line(&item.title),
        "state": line(artist(item)),
        "timestamps": { "start": started, "end": started + duration_ms },
    });
    if cover.starts_with("https://") {
        activity["assets"] = json!({ "large_image": ytm::sized(cover, COVER_SIDE) });
    }
    activity
}

fn artist(item: &Item) -> &str {
    let mut parts = item.subtitle.split(" • ");
    let named = item
        .artists
        .first()
        .and_then(|(name, _)| parts.clone().find(|part| part.contains(name.as_str())));
    named.or_else(|| parts.next()).unwrap_or_default()
}

fn line(text: &str) -> Option<String> {
    let mut units = 0;
    let line: String = text
        .trim()
        .chars()
        .take_while(|letter| {
            units += letter.len_utf16();
            units <= LONGEST_LINE
        })
        .collect();
    (!line.is_empty()).then(|| format!("{line:<SHORTEST_LINE$}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakePipe {
        answer: io::Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl Read for FakePipe {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.answer.read(buffer)
        }
    }

    impl Write for FakePipe {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.written.write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn frames_are_an_opcode_a_length_and_json() {
        let talk = |answer: Vec<u8>| {
            let mut pipe = FakePipe {
                answer: io::Cursor::new(answer),
                written: Vec::new(),
            };
            let accepted = exchange(&mut pipe, HANDSHAKE, &json!({ "v": 1 })).is_ok();
            (accepted, pipe.written, pipe.answer.position())
        };
        let ready = frame(FRAME, &json!({ "evt": "READY" }));
        let (accepted, written, read) = talk(ready.clone());
        assert!(accepted);
        assert_eq!(written, b"\0\0\0\0\x07\0\0\0{\"v\":1}");
        assert_eq!(read, ready.len() as u64);
        assert!(!talk(frame(CLOSE, &json!({ "code": 4000 }))).0);
        assert!(!talk(ready[..ready.len() - 1].to_vec()).0);
        assert!(!talk(Vec::new()).0);
    }

    #[test]
    fn activity_names_the_song_its_artist_and_its_time_bar() {
        let song = |title: &str, subtitle: &str, artists: &[&str]| Item {
            title: title.to_owned(),
            subtitle: subtitle.to_owned(),
            artists: artists.iter().map(|name| (name.to_string(), String::new())).collect(),
            ..Item::default()
        };
        let cover = "https://lh3.googleusercontent.com/abc=w120-h120-l90-rj";
        let lucky = song("Get Lucky", "Daft Punk • Random Access Memories", &[]);
        let shown = activity(&lucky, cover, Headline::App, 1000, 248_000);
        assert_eq!(shown["type"], 2);
        assert_eq!(shown["status_display_type"], 0);
        assert_eq!(activity(&lucky, cover, Headline::Artist, 0, 1)["status_display_type"], 1);
        assert_eq!(activity(&lucky, cover, Headline::Song, 0, 1)["status_display_type"], 2);
        assert_eq!(shown["details"], "Get Lucky");
        assert_eq!(shown["state"], "Daft Punk");
        assert_eq!(shown["timestamps"], json!({ "start": 1000, "end": 249_000 }));
        assert_eq!(
            shown["assets"]["large_image"],
            "https://lh3.googleusercontent.com/abc=w512-h512-l90-rj"
        );

        let video = activity(&song("7", "Video • Prince • 12M views", &["Prince"]), "", Headline::App, 0, 1);
        assert_eq!(video["details"], "7 ");
        assert_eq!(video["state"], "Prince");
        assert!(video["assets"].is_null());

        let unnamed = activity(&song(&"é😀".repeat(100), "", &[]), "", Headline::App, 0, 1);
        assert_eq!(unnamed["details"].as_str().unwrap().encode_utf16().count(), 127);
        assert!(unnamed["state"].is_null());

        let (updates, inbox) = mpsc::channel();
        let mut discord = Discord { updates, shown: None };
        discord.show(Some((&lucky, cover)), Headline::App, 0, 248_000);
        discord.show(Some((&lucky, cover)), Headline::App, 0, 248_000);
        discord.show(Some((&lucky, cover)), Headline::Artist, 0, 248_000);
        let sent: Vec<_> = inbox
            .try_iter()
            .flatten()
            .map(|shown| shown["status_display_type"].clone())
            .collect();
        assert_eq!(sent, [0, 1]);
    }
}
