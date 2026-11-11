use crate::app_id;
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};

pub const ORIGIN: &str = "https://music.youtube.com";
const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fmusic.youtube.com%2F";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const POLL_EVERY: Duration = Duration::from_millis(1000);
const SETTLED_POLLS: u32 = 4;
const MAX_MESSAGE_BYTES: u64 = 64 * 1024 * 1024;
#[derive(Clone, Copy, PartialEq)]
pub enum Family {
    Chromium,
    Firefox,
}
const BROWSERS: [(Family, &str, &str, &str); 14] = [
    (
        Family::Chromium,
        "Edge",
        "ProgramFiles(x86)",
        r"Microsoft\Edge\Application\msedge.exe",
    ),
    (Family::Chromium, "Edge", "ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
    (Family::Chromium, "Chrome", "ProgramFiles", r"Google\Chrome\Application\chrome.exe"),
    (Family::Chromium, "Chrome", "LocalAppData", r"Google\Chrome\Application\chrome.exe"),
    (
        Family::Chromium,
        "Brave",
        "ProgramFiles",
        r"BraveSoftware\Brave-Browser\Application\brave.exe",
    ),
    (
        Family::Chromium,
        "Brave",
        "LocalAppData",
        r"BraveSoftware\Brave-Browser\Application\brave.exe",
    ),
    (Family::Chromium, "Opera GX", "LocalAppData", r"Programs\Opera GX\opera.exe"),
    (Family::Firefox, "Firefox", "ProgramFiles", r"Mozilla Firefox\firefox.exe"),
    (Family::Firefox, "Firefox", "ProgramFiles(x86)", r"Mozilla Firefox\firefox.exe"),
    (Family::Firefox, "LibreWolf", "ProgramFiles", r"LibreWolf\librewolf.exe"),
    (Family::Firefox, "Zen", "ProgramFiles", r"Zen Browser\zen.exe"),
    (Family::Firefox, "Waterfox", "ProgramFiles", r"Waterfox\waterfox.exe"),
    (Family::Firefox, "Floorp", "ProgramFiles", r"Floorp\floorp.exe"),
    (Family::Firefox, "Firefox", "LocalAppData", r"Mozilla Firefox\firefox.exe"),
];
const BROWSERS_ELSEWHERE: [(Family, &str, &str); 23] = [
    (Family::Chromium, "Chrome", "google-chrome"),
    (Family::Chromium, "Chrome", "google-chrome-stable"),
    (Family::Chromium, "Chromium", "chromium"),
    (Family::Chromium, "Chromium", "chromium-browser"),
    (Family::Chromium, "Edge", "microsoft-edge"),
    (Family::Chromium, "Brave", "brave-browser"),
    (Family::Chromium, "Brave", "brave"),
    (Family::Chromium, "Brave Origin", "brave-origin"),
    (Family::Chromium, "Brave Origin", "brave-origin-stable"),
    (Family::Chromium, "Helium", "helium"),
    (Family::Chromium, "Helium", "helium-browser"),
    (
        Family::Chromium,
        "Chrome",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ),
    (
        Family::Chromium,
        "Edge",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ),
    (Family::Firefox, "Firefox", "firefox"),
    (Family::Firefox, "LibreWolf", "librewolf"),
    (Family::Firefox, "Zen", "zen-browser"),
    (Family::Firefox, "Waterfox", "waterfox"),
    (Family::Firefox, "Floorp", "floorp"),
    (Family::Firefox, "Firefox", "/Applications/Firefox.app/Contents/MacOS/firefox"),
    (Family::Firefox, "LibreWolf", "/Applications/LibreWolf.app/Contents/MacOS/librewolf"),
    (Family::Firefox, "Zen", "/Applications/Zen.app/Contents/MacOS/zen"),
    (Family::Firefox, "Waterfox", "/Applications/Waterfox.app/Contents/MacOS/waterfox"),
    (Family::Firefox, "Floorp", "/Applications/Floorp.app/Contents/MacOS/floorp"),
];
pub const NO_BROWSER: &str = "No supported browser was found. Install Chrome, Edge, Brave, Firefox or another Chromium or Firefox browser.";

#[derive(Clone)]
pub struct Installed {
    pub name: &'static str,
    family: Family,
    program: PathBuf,
}

fn candidates() -> Vec<(Family, &'static str, PathBuf)> {
    if cfg!(windows) {
        BROWSERS
            .iter()
            .filter_map(|(family, name, root, path)| Some((*family, *name, PathBuf::from(std::env::var_os(root)?).join(path))))
            .collect()
    } else {
        BROWSERS_ELSEWHERE
            .iter()
            .map(|(family, name, program)| (*family, *name, PathBuf::from(program)))
            .collect()
    }
}

fn read_head(path: &Path) -> Option<String> {
    let mut buffer = [0u8; 512];
    let read = std::fs::File::open(path).ok()?.read(&mut buffer).ok()?;
    Some(String::from_utf8_lossy(&buffer[..read]).into_owned())
}

fn snap_name(program: &Path, head: impl Fn(&Path) -> Option<String>) -> Option<String> {
    if let Ok(name) = program.strip_prefix("/snap/bin") {
        return Some(name.to_str()?.to_owned());
    }
    let real = program.canonicalize().unwrap_or_else(|_| program.to_owned());
    if let Ok(inside) = real.strip_prefix("/snap/")
        && let Some(first) = inside.components().next().and_then(|part| part.as_os_str().to_str())
        && first != "bin"
    {
        return Some(first.to_owned());
    }
    let file = program.file_name()?.to_str()?;
    let name = if file == "chromium-browser" { "chromium" } else { file };
    head(&Path::new("/snap/bin").join(name))?;
    let wrapper = head(program)?;
    (wrapper.starts_with("#!") && wrapper.contains("snap")).then(|| name.to_owned())
}

fn profile_leaf() -> String {
    format!("tubefast-signin-{}", app_id())
}

fn sign_in_profile(program: &Path, home: Option<PathBuf>, default: PathBuf, head: impl Fn(&Path) -> Option<String>) -> PathBuf {
    match (snap_name(program, head), home) {
        (Some(name), Some(home)) => home.join("snap").join(name).join("common").join(profile_leaf()),
        _ => default,
    }
}

pub fn remove_leftover_sign_in_profiles() {
    let Some(default) = eframe::storage_dir(app_id()).map(|dir| dir.join("signin-profile")) else {
        return;
    };
    let mut leftovers = vec![default];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(snaps) = std::fs::read_dir(home.join("snap"))
    {
        leftovers.extend(snaps.flatten().map(|snap| snap.path().join("common").join(profile_leaf())));
    }
    for leftover in leftovers {
        if leftover.is_dir() {
            let _ = std::fs::remove_dir_all(leftover);
        }
    }
}

fn find_program(program: &Path) -> Option<PathBuf> {
    if program.components().count() > 1 {
        return program.is_file().then(|| program.to_owned());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(program))
        .find(|found| found.is_file())
}

fn detect(
    candidates: impl IntoIterator<Item = (Family, &'static str, PathBuf)>,
    find: impl Fn(&Path) -> Option<PathBuf>,
) -> Vec<Installed> {
    let mut found: Vec<Installed> = Vec::new();
    for (family, name, program) in candidates {
        if found.iter().all(|browser| browser.name != name)
            && let Some(program) = find(&program)
        {
            found.push(Installed { name, family, program });
        }
    }
    found
}

pub fn installed_browsers() -> Vec<Installed> {
    detect(candidates(), find_program)
}

pub fn chosen_index(installed: &[Installed], remembered: &str) -> usize {
    installed.iter().position(|browser| browser.name == remembered).unwrap_or(0)
}
pub const SESSION_KEPT: &str = if cfg!(windows) {
    "The session stays on this computer, encrypted with your Windows account."
} else {
    "The session stays on this computer, in a file only your user account can read."
};

const INCOMPLETE_COPY: &str =
    "That copy is missing the HttpOnly session cookies. Copy it from the request headers, or sign in with the browser button instead.";

fn pair_value<'a>(cookie: &'a str, name: &str) -> Option<&'a str> {
    cookie.split("; ").find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

fn sapisid(cookie: &str) -> Option<&str> {
    pair_value(cookie, "__Secure-3PAPISID").or_else(|| pair_value(cookie, "SAPISID"))
}

pub fn same_login(first: &str, second: &str) -> bool {
    sapisid(first).is_some() && sapisid(first) == sapisid(second)
}

fn days_since_epoch(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    era * 146_097 + year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year - 719_468
}

fn expires_at(text: &str) -> Option<i64> {
    let parts: Vec<&str> = text.split([' ', ',', '-', ':']).filter(|part| !part.is_empty()).collect();
    let weekday = usize::from(parts.first().is_some_and(|part| part.parse::<u32>().is_err()));
    let [day, month, year, hour, minute, second, ..] = parts[weekday..] else {
        return None;
    };
    let month = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter()
        .position(|name| month.to_ascii_lowercase().starts_with(name))? as i64
        + 1;
    let year: i64 = year.parse().ok()?;
    let year = if year < 70 {
        year + 2000
    } else if year < 100 {
        year + 1900
    } else {
        year
    };
    let clock = hour.parse::<i64>().ok()? * 3600 + minute.parse::<i64>().ok()? * 60 + second.parse::<i64>().ok()?;
    Some(days_since_epoch(year, month, day.parse().ok()?) * 86_400 + clock)
}

fn live_update(header: &str, now: i64) -> Option<(String, String)> {
    let mut parts = header.split(';');
    let (name, value) = parts.next()?.split_once('=')?;
    let (name, value) = (name.trim(), value.trim());
    if name.is_empty() || value.is_empty() {
        return None;
    }
    for attribute in parts {
        let (key, text) = attribute
            .split_once('=')
            .map_or((attribute.trim(), ""), |(key, text)| (key.trim(), text.trim()));
        match key.to_ascii_lowercase().as_str() {
            "domain" => {
                let domain = text.trim_start_matches('.').to_ascii_lowercase();
                if domain != "youtube.com" && !domain.ends_with(".youtube.com") {
                    return None;
                }
            }
            "max-age" if text.parse::<i64>().is_ok_and(|seconds| seconds <= 0) => return None,
            "expires" if expires_at(text).is_some_and(|at| at <= now) => return None,
            _ => {}
        }
    }
    Some((name.to_owned(), value.to_owned()))
}

pub fn merge_set_cookies(cookie: &str, set_cookies: &[&str], now: i64) -> Option<String> {
    let mut pairs: Vec<(String, String)> = cookie
        .split("; ")
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect();
    let mut changed = false;
    for (name, value) in set_cookies.iter().filter_map(|header| live_update(header, now)) {
        match pairs.iter_mut().find(|(known, _)| *known == name) {
            Some((_, old)) if *old == value => {}
            Some((_, old)) => {
                *old = value;
                changed = true;
            }
            None => {
                pairs.push((name, value));
                changed = true;
            }
        }
    }
    changed.then(|| {
        pairs
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

pub fn authorization(cookie: &str, unix_seconds: u64) -> Option<String> {
    let input = format!("{unix_seconds} {} {ORIGIN}", sapisid(cookie)?);
    let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, input.as_bytes());
    let hex: String = digest.as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!("SAPISIDHASH {unix_seconds}_{hex}"))
}

pub fn clean_cookie(pasted: &str) -> Result<String, String> {
    let header = pasted
        .to_ascii_lowercase()
        .find("cookie:")
        .map(|at| pasted[at + 7..].lines().next().unwrap_or_default());
    let raw = header.map_or_else(|| pasted.replace(['\r', '\n'], ""), str::to_owned);
    let raw = raw.trim().trim_end_matches('\\').trim().trim_matches(['\'', '"']);
    let cookie = raw
        .split(';')
        .map(str::trim)
        .filter(|pair| pair.contains('='))
        .collect::<Vec<_>>()
        .join("; ");
    if sapisid(&cookie).is_none() {
        return Err("That text has no YouTube session in it. Copy the whole cookie value while signed in.".to_owned());
    }
    match pair_value(&cookie, "__Secure-3PSID").or_else(|| pair_value(&cookie, "__Secure-1PSID")) {
        Some(_) => Ok(cookie),
        None => Err(INCOMPLETE_COPY.to_owned()),
    }
}

fn rotate_stored(file: &Path, used: &str, set_cookies: &[&str], now: i64) {
    let Some(_lock) = lock_session(file) else { return };
    let Some(stored) = read_session(file).filter(|stored| same_login(stored, used)) else {
        return;
    };
    if let Some(merged) = merge_set_cookies(&stored, set_cookies, now) {
        let _ = write_session(file, &merged);
    }
}

pub fn persist_rotation(used: &str, set_cookies: &[&str], now: i64) {
    if let Some(file) = session_file() {
        rotate_stored(&file, used, set_cookies, now);
    }
}

fn session_file() -> Option<PathBuf> {
    Some(eframe::storage_dir(app_id())?.join("session.bin"))
}

fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir)?.permissions().mode();
        if mode & 0o077 != 0 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode & 0o700))?;
        }
    }
    Ok(())
}

fn lock_session(file: &Path) -> Option<std::fs::File> {
    private_dir(file.parent()?).ok()?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let lock = options.open(file.with_extension("lock")).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = lock.metadata().ok()?.permissions().mode();
        if mode & 0o077 != 0 {
            lock.set_permissions(std::fs::Permissions::from_mode(mode & 0o700)).ok()?;
        }
    }
    lock.lock().ok()?;
    Some(lock)
}

pub fn secure_session_storage() {
    if let Some(file) = session_file() {
        drop(lock_session(&file));
    }
}

fn read_session(file: &Path) -> Option<String> {
    String::from_utf8(seal(&std::fs::read(file).ok()?, false)?).ok()
}

fn write_session(file: &Path, cookie: &str) -> Result<(), String> {
    let sealed = seal(cookie.as_bytes(), true).ok_or("This sign-in cannot be kept after a restart on this system.")?;
    let staged = file.with_extension("tmp");
    let mut private = std::fs::OpenOptions::new();
    private.create_new(true).write(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut private, 0o600);
    let _ = std::fs::remove_file(&staged);
    private_dir(file.parent().unwrap_or(Path::new(".")))
        .and_then(|()| private.open(&staged)?.write_all(&sealed))
        .and_then(|()| std::fs::rename(&staged, file))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&staged);
        })
        .map_err(|e| e.to_string())
}

pub fn store(cookie: &str) -> Result<(), String> {
    let file = session_file().ok_or("There is no folder to keep the sign-in in.")?;
    let _lock = lock_session(&file);
    write_session(&file, cookie)
}

pub fn load() -> Option<String> {
    read_session(&session_file()?)
}

pub fn forget() {
    if let Some(file) = session_file() {
        let _lock = lock_session(&file);
        let _ = std::fs::remove_file(file);
    }
}

pub fn forget_if_stored(cookie: &str) {
    let Some(file) = session_file() else { return };
    let _lock = lock_session(&file);
    if read_session(&file).as_deref() == Some(cookie) {
        let _ = std::fs::remove_file(file);
    }
}

#[cfg(windows)]
fn seal(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData};
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    unsafe {
        let done = if protect {
            CryptProtectData(&input, null(), null(), null(), null(), 0, &mut output)
        } else {
            CryptUnprotectData(&input, null_mut(), null(), null(), null(), 0, &mut output)
        };
        if done == 0 || output.pbData.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData.cast());
        Some(bytes)
    }
}

#[cfg(not(windows))]
fn seal(data: &[u8], _protect: bool) -> Option<Vec<u8>> {
    Some(data.to_vec())
}

struct Socket {
    stream: TcpStream,
    calls: u64,
    family: Family,
}

impl Socket {
    fn open(port: u16, family: Family) -> Option<Socket> {
        let path = match family {
            Family::Firefox => "/session".to_owned(),
            Family::Chromium => {
                let version: Value = ureq::get(&format!("http://127.0.0.1:{port}/json/version"))
                    .timeout(Duration::from_secs(2))
                    .call()
                    .ok()?
                    .into_json()
                    .ok()?;
                let address = version["webSocketDebuggerUrl"].as_str()?;
                address[address.find("/devtools/")?..].to_owned()
            }
        };
        let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).ok()?;
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).ok()?;
            head.push(byte[0]);
        }
        let mut socket = head.starts_with(b"HTTP/1.1 101").then_some(Socket { stream, calls: 0, family })?;
        if family == Family::Firefox {
            let created = socket.call("session.new", json!({ "capabilities": {} })).ok()?;
            created["result"]["sessionId"].is_string().then_some(())?;
        }
        Some(socket)
    }

    fn cookies(&mut self) -> io::Result<Value> {
        match self.family {
            Family::Chromium => self.call("Storage.getCookies", json!({})),
            Family::Firefox => self.call("storage.getCookies", json!({})),
        }
    }

    fn close_browser(&mut self) {
        let _ = match self.family {
            Family::Chromium => self.call("Browser.close", json!({})),
            Family::Firefox => self.call("browser.close", json!({})),
        };
    }

    fn call(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.calls += 1;
        let text = json!({ "id": self.calls, "method": method, "params": params }).to_string();
        let mut mask = [0u8; 4];
        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut mask).map_err(|_| io::Error::other("no randomness"))?;
        let frame = masked_frame(&text, mask);
        self.stream.write_all(&frame)?;
        loop {
            let reply: Value = serde_json::from_slice(&self.message()?).unwrap_or_default();
            if reply["id"] == self.calls {
                return Ok(reply);
            }
        }
    }

    fn message(&mut self) -> io::Result<Vec<u8>> {
        let mut message = Vec::new();
        loop {
            let mut head = [0u8; 2];
            self.stream.read_exact(&mut head)?;
            let length = match head[1] & 0x7F {
                126 => {
                    let mut wide = [0u8; 2];
                    self.stream.read_exact(&mut wide)?;
                    u16::from_be_bytes(wide) as u64
                }
                127 => {
                    let mut wide = [0u8; 8];
                    self.stream.read_exact(&mut wide)?;
                    u64::from_be_bytes(wide)
                }
                short => short as u64,
            };
            if length > MAX_MESSAGE_BYTES {
                return Err(io::ErrorKind::InvalidData.into());
            }
            let mut payload = vec![0u8; length as usize];
            self.stream.read_exact(&mut payload)?;
            match head[0] & 0x0F {
                0x0..=0x2 => message.extend(payload),
                0x8 => return Err(io::ErrorKind::ConnectionAborted.into()),
                _ => continue,
            }
            if head[0] & 0x80 != 0 {
                return Ok(message);
            }
        }
    }
}

fn cookie_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    let inner = value["value"].as_str()?;
    if value["type"] == "base64" {
        use base64::Engine;
        return String::from_utf8(base64::engine::general_purpose::STANDARD.decode(inner).ok()?).ok();
    }
    Some(inner.to_owned())
}

fn masked_frame(text: &str, mask: [u8; 4]) -> Vec<u8> {
    let mut frame = vec![0x81];
    match text.len() {
        length @ 0..=125 => frame.push(0x80 | length as u8),
        length @ 126..=65535 => {
            frame.push(0x80 | 126);
            frame.extend((length as u16).to_be_bytes());
        }
        length => {
            frame.push(0x80 | 127);
            frame.extend((length as u64).to_be_bytes());
        }
    }
    frame.extend(mask);
    frame.extend(text.bytes().enumerate().map(|(at, byte)| byte ^ mask[at % 4]));
    frame
}

fn youtube_cookie(reply: &Value) -> Option<String> {
    let on_youtube = |cookie: &&Value| {
        cookie["domain"]
            .as_str()
            .is_some_and(|domain| domain == "youtube.com" || domain.ends_with(".youtube.com"))
    };
    let pairs: Vec<String> = reply["result"]["cookies"]
        .as_array()?
        .iter()
        .filter(on_youtube)
        .filter_map(|cookie| Some(format!("{}={}", cookie["name"].as_str()?, cookie_value(&cookie["value"])?)))
        .collect();
    let cookie = pairs.join("; ");
    sapisid(&cookie).is_some().then_some(cookie)
}

fn launch_command(browser: &Installed, profile: &Path, url: &str, visible: bool, port: u16) -> Command {
    let mut command = Command::new(&browser.program);
    match browser.family {
        Family::Chromium => {
            command
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg(format!("--remote-debugging-port={port}"))
                .args(["--no-first-run", "--no-default-browser-check"])
                .args(if visible {
                    vec!["--window-size=520,760".to_owned(), format!("--app={url}")]
                } else {
                    vec!["--headless=new".to_owned(), url.to_owned()]
                });
        }
        Family::Firefox => {
            command
                .arg(format!("--remote-debugging-port={port}"))
                .arg("--profile")
                .arg(profile)
                .arg("--no-remote")
                .arg("--wait-for-browser")
                .args(if visible {
                    vec!["--width", "520", "--height", "760"]
                } else {
                    vec!["--headless"]
                })
                .arg(url);
        }
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command
}

fn launch(profile: &Path, url: &str, visible: bool, browser: &Installed) -> Result<(Child, u16), String> {
    let port = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map_err(|e| e.to_string())?
        .port();
    let _ = std::fs::remove_dir_all(profile);
    if browser.family == Family::Firefox {
        std::fs::create_dir_all(profile).map_err(|e| e.to_string())?;
    }
    let child = launch_command(browser, profile, url, visible, port)
        .spawn()
        .map_err(|_| format!("{} could not be started.", browser.name))?;
    Ok((child, port))
}

fn watch(browser: &mut Child, port: u16, family: Family, cancel: &AtomicBool) -> Result<String, String> {
    let started = Instant::now();
    let mut socket: Option<Socket> = None;
    let mut connected = false;
    let mut settled = 0;
    loop {
        if cancel.load(Relaxed) {
            return Err(String::new());
        }
        if started.elapsed() > LOGIN_TIMEOUT {
            return Err("Signing in took too long. Try again.".to_owned());
        }
        if matches!(browser.try_wait(), Ok(Some(_))) {
            return Err(if connected {
                "The browser window was closed before you signed in.".to_owned()
            } else {
                "The browser stopped right after it started. Close any other window of it and try again.".to_owned()
            });
        }
        std::thread::sleep(POLL_EVERY);
        let Some(connection) = &mut socket else {
            socket = Socket::open(port, family);
            connected |= socket.is_some();
            continue;
        };
        let Ok(reply) = connection.cookies() else {
            socket = None;
            continue;
        };
        let Some(cookie) = youtube_cookie(&reply) else { continue };
        settled += 1;
        if cookie.contains("LOGIN_INFO=") || settled >= SETTLED_POLLS {
            connection.close_browser();
            return Ok(cookie);
        }
    }
}

fn discard(mut browser: Child, profile: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/PID", &browser.id().to_string(), "/T", "/F"])
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", browser.id())])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = browser.kill();
    let _ = browser.wait();
    for _ in 0..15 {
        if std::fs::remove_dir_all(profile).is_ok() || !profile.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn browser_sign_in(cancel: &AtomicBool, chosen: &Installed) -> Result<String, String> {
    let default = eframe::storage_dir(app_id())
        .ok_or("There is no folder for the sign-in window.")?
        .join("signin-profile");
    let profile = sign_in_profile(&chosen.program, std::env::var_os("HOME").map(PathBuf::from), default, read_head);
    let (mut browser, port) = launch(&profile, LOGIN_URL, true, chosen)?;
    let cookie = watch(&mut browser, port, chosen.family, cancel);
    discard(browser, &profile);
    cookie
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_requests_the_way_youtube_expects() {
        let cookie = "SID=a; SAPISID=first; __Secure-3PAPISID=abc/123; LOGIN_INFO=x";
        assert_eq!(sapisid(cookie), Some("abc/123"));
        assert_eq!(sapisid("SID=a; SAPISIDX=no"), None);
        let expected = "SAPISIDHASH 1700000000_149e3c5c81746e107a88a2e0103de8a4b63040a3";
        assert_eq!(authorization(cookie, 1_700_000_000).as_deref(), Some(expected));
        assert_eq!(authorization("SID=a", 1_700_000_000), None);
    }

    #[test]
    fn accepts_a_raw_cookie_a_header_line_or_a_curl_command() {
        let raw = "SID=a;  SAPISID=k ; __Secure-1PSID=p; junk; LOGIN_INFO=x";
        assert_eq!(clean_cookie(raw).unwrap(), "SID=a; SAPISID=k; __Secure-1PSID=p; LOGIN_INFO=x");
        assert_eq!(
            clean_cookie("accept: */*\r\nCookie: SID=a; SAPISID=k; __Secure-3PSID=p\r\nuser-agent: x").unwrap(),
            "SID=a; SAPISID=k; __Secure-3PSID=p"
        );
        assert_eq!(
            clean_cookie("curl 'https://x' \\\n  -H 'cookie: SID=a; SAPISID=k; __Secure-3PSID=p' \\\n  -H 'x: y'").unwrap(),
            "SID=a; SAPISID=k; __Secure-3PSID=p"
        );
        assert!(clean_cookie("SID=a; HSID=b").is_err());
        assert_eq!(clean_cookie("SID=a; SAPISID=k").unwrap_err(), INCOMPLETE_COPY);
    }

    #[test]
    fn reads_cookies_from_a_webdriver_bidi_reply() {
        let reply = json!({ "type": "success", "id": 2, "result": { "cookies": [
            { "name": "SAPISID", "domain": ".youtube.com", "path": "/", "secure": true, "httpOnly": false, "sameSite": "none",
              "value": { "type": "string", "value": "abc" } },
            { "name": "LOGIN_INFO", "domain": "music.youtube.com", "path": "/", "secure": true, "httpOnly": true, "sameSite": "lax",
              "expiry": 1900000000, "value": { "type": "base64", "value": "eHl6" } },
            { "name": "SAPISID", "domain": ".example.com", "path": "/", "value": { "type": "string", "value": "wrong-site" } },
        ] } });
        assert_eq!(youtube_cookie(&reply).as_deref(), Some("SAPISID=abc; LOGIN_INFO=xyz"));
        assert_eq!(youtube_cookie(&json!({ "result": { "cookies": [] } })), None);
    }

    #[test]
    fn lists_each_installed_browser_once_and_falls_back_to_the_first() {
        let candidates = [
            (Family::Chromium, "Edge", PathBuf::from("/missing/edge")),
            (Family::Chromium, "Chrome", PathBuf::from("/ok/chrome-a")),
            (Family::Chromium, "Chrome", PathBuf::from("/ok/chrome-b")),
            (Family::Firefox, "Firefox", PathBuf::from("/ok/firefox")),
        ];
        let found = detect(candidates, |program| program.starts_with("/ok").then(|| program.to_owned()));
        let names: Vec<&str> = found.iter().map(|browser| browser.name).collect();
        assert_eq!(names, ["Chrome", "Firefox"]);
        assert_eq!(found[0].program, PathBuf::from("/ok/chrome-a"));
        assert_eq!(chosen_index(&found, "Firefox"), 1);
        assert_eq!(chosen_index(&found, "Edge"), 0);
        assert_eq!(chosen_index(&found, ""), 0);
    }

    #[test]
    fn merges_rotated_cookies_without_ever_removing_any() {
        let now = 1_800_000_000;
        let session = "SAPISID=k; __Secure-1PSIDTS=old; SID=a";
        let rotated = merge_set_cookies(
            session,
            &[
                "__Secure-1PSIDTS=new; Domain=.youtube.com; Path=/; Secure; HttpOnly",
                "SIDCC=fresh; Path=/; Max-Age=31536000",
                "SID=; Max-Age=0; Domain=.youtube.com",
                "SAPISID=gone; Expires=Thu, 01-Jan-1970 00:00:00 GMT",
                "SID=other; Domain=.example.com",
                "__Secure-3PSIDTS=ok; Expires=Fri, 01 Jan 2100 00:00:00 GMT",
            ],
            now,
        );
        assert_eq!(
            rotated.as_deref(),
            Some("SAPISID=k; __Secure-1PSIDTS=new; SID=a; SIDCC=fresh; __Secure-3PSIDTS=ok")
        );
        assert_eq!(merge_set_cookies(session, &["__Secure-1PSIDTS=old", "SID=a; Path=/"], now), None);
        assert_eq!(merge_set_cookies(session, &["SID=b; Expires=01 Jan 1970 00:00:00 GMT"], now), None);
        assert_eq!(
            merge_set_cookies(
                session,
                &["SID=b; Expires=01-Jan-2100 00:00:00 GMT", "SIDCC=c; Expires=garbage"],
                now
            )
            .as_deref(),
            Some("SAPISID=k; __Secure-1PSIDTS=old; SID=b; SIDCC=c")
        );
        assert_eq!(merge_set_cookies(session, &[], now), None);
        assert!(same_login(session, "SAPISID=k; SID=z"));
        assert!(!same_login(session, "SAPISID=other"));
    }

    #[cfg(windows)]
    #[test]
    fn sealed_sessions_round_trip_and_are_not_plain_text() {
        let sealed = seal(b"SAPISID=secret", true).unwrap();
        assert!(!sealed.windows(6).any(|window| window == b"secret"));
        assert_eq!(seal(&sealed, false).unwrap(), b"SAPISID=secret");
        assert!(seal(b"not a sealed blob", false).is_none());
    }

    fn first_of(family: Family) -> Installed {
        installed_browsers().into_iter().find(|browser| browser.family == family).unwrap()
    }

    #[test]
    #[ignore = "needs Edge, Chrome or another Chromium browser installed"]
    fn reads_the_session_out_of_a_real_browser() {
        let profile = std::env::temp_dir().join(format!("{}-signin-test-{}", app_id(), std::process::id()));
        let chosen = first_of(Family::Chromium);
        let family = chosen.family;
        let (mut browser, port) = launch(&profile, "about:blank", false, &chosen).unwrap();
        let mut socket = (0..100)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(200));
                Socket::open(port, family)
            })
            .expect("the browser never opened its control port");
        let cookie =
            |name: &str, value: &str, domain: &str| json!({ "name": name, "value": value, "domain": domain, "path": "/", "secure": true });
        let cookies = json!({ "cookies": [
            cookie("SAPISID", "abc", ".youtube.com"),
            cookie("LOGIN_INFO", "xyz", ".youtube.com"),
            cookie("SAPISID", "wrong-site", ".example.com"),
        ] });
        assert!(socket.call("Storage.setCookies", cookies).unwrap()["error"].is_null());
        let found = watch(&mut browser, port, family, &AtomicBool::new(false));
        discard(browser, &profile);
        let found = found.unwrap();
        assert!(
            found.contains("SAPISID=abc") && found.contains("LOGIN_INFO=xyz") && !found.contains("wrong-site"),
            "{found}"
        );
        assert!(!profile.exists());
    }

    #[test]
    #[ignore = "needs Firefox installed"]
    fn talks_to_a_real_firefox_over_bidi() {
        let profile = std::env::temp_dir().join(format!("{}-firefox-test-{}", app_id(), std::process::id()));
        let chosen = first_of(Family::Firefox);
        let (browser, port) = launch(&profile, "about:blank", false, &chosen).unwrap();
        let socket = (0..100).find_map(|_| {
            std::thread::sleep(Duration::from_millis(200));
            Socket::open(port, Family::Firefox)
        });
        let reply = socket.map(|mut socket| {
            let reply = socket.cookies();
            socket.close_browser();
            reply
        });
        discard(browser, &profile);
        let reply = reply.expect("Firefox never opened its BiDi port").unwrap();
        assert_eq!(reply["type"], "success");
        assert!(reply["result"]["cookies"].is_array());
    }

    #[test]
    #[ignore = "needs Firefox installed"]
    fn keeps_waiting_on_a_visible_firefox_until_it_is_closed() {
        let profile = std::env::temp_dir().join(format!("{}-firefox-wait-test-{}", app_id(), std::process::id()));
        let chosen = first_of(Family::Firefox);
        let (mut browser, port) = launch(&profile, "about:blank", true, &chosen).unwrap();
        let cancel = AtomicBool::new(false);
        let (still_waiting, cancelled) = std::thread::scope(|scope| {
            let watcher = scope.spawn(|| watch(&mut browser, port, Family::Firefox, &cancel));
            std::thread::sleep(Duration::from_secs(8));
            let still_waiting = !watcher.is_finished();
            cancel.store(true, Relaxed);
            (still_waiting, watcher.join().unwrap())
        });
        let _ = browser.kill();
        let _ = browser.wait();
        let closed = watch(&mut browser, port, Family::Firefox, &AtomicBool::new(false));
        discard(browser, &profile);
        assert!(still_waiting, "the watch ended while the window was open");
        assert_eq!(cancelled, Err(String::new()));
        assert_eq!(closed, Err("The browser window was closed before you signed in.".to_owned()));
    }

    #[test]
    fn masks_every_frame_with_its_own_key() {
        let frame = masked_frame(r#"{"id":1}"#, [1, 2, 3, 4]);
        let unmasked: Vec<u8> = frame[6..]
            .iter()
            .enumerate()
            .map(|(at, byte)| byte ^ [1, 2, 3, 4][at % 4])
            .collect();
        assert_eq!(&frame[2..6], &[1, 2, 3, 4]);
        assert_eq!(unmasked, br#"{"id":1}"#);
        let keys: Vec<[u8; 4]> = (0..2)
            .map(|_| {
                let mut key = [0u8; 4];
                ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut key).unwrap();
                key
            })
            .collect();
        assert_ne!(keys[0], keys[1]);
    }

    #[test]
    fn rotation_only_touches_a_live_session_of_the_same_login() {
        let dir = std::env::temp_dir().join(format!("{}-rotation-test-{}", app_id(), std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("session.bin");
        let update = ["__Secure-1PSIDTS=new; Domain=.youtube.com"];
        rotate_stored(&file, "SAPISID=k", &update, 0);
        assert!(!file.exists());
        write_session(&file, "SAPISID=k; OTHER=kept; __Secure-1PSIDTS=old").unwrap();
        rotate_stored(&file, "SAPISID=different", &update, 0);
        assert_eq!(read_session(&file).unwrap(), "SAPISID=k; OTHER=kept; __Secure-1PSIDTS=old");
        rotate_stored(&file, "SAPISID=k; OTHER=stale", &update, 0);
        assert_eq!(read_session(&file).unwrap(), "SAPISID=k; OTHER=kept; __Secure-1PSIDTS=new");
        assert!(!file.with_extension("tmp").exists());
        std::fs::write(file.with_extension("tmp"), b"stale").unwrap();
        write_session(&file, "SAPISID=k; after=stale-tmp").unwrap();
        assert_eq!(read_session(&file).unwrap(), "SAPISID=k; after=stale-tmp");
        let _ = std::fs::remove_file(file.with_extension("lock"));
        std::fs::create_dir(file.with_extension("lock")).unwrap();
        rotate_stored(&file, "SAPISID=k", &update, 0);
        assert_eq!(read_session(&file).unwrap(), "SAPISID=k; after=stale-tmp");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_snap_browsers_profile_inside_their_own_folder() {
        let home = || Some(PathBuf::from("/home/me"));
        let default = || PathBuf::from("/home/me/.local/share/tubefast/signin-profile");
        let leaf = profile_leaf();
        let script = "#!/bin/sh
exec /snap/bin/firefox \"$@\"";
        let files = |path: &Path| match path.to_str()?.replace('\\', "/").as_str() {
            "/snap/bin/firefox" | "/snap/bin/chromium" | "/opt/firefox/firefox" => Some("ELF".to_owned()),
            "/usr/bin/firefox" | "/usr/bin/chromium-browser" => Some(script.to_owned()),
            _ => None,
        };
        let snap = sign_in_profile(Path::new("/usr/bin/firefox"), home(), default(), files);
        assert_eq!(snap, PathBuf::from("/home/me/snap/firefox/common").join(&leaf));
        let direct = sign_in_profile(Path::new("/snap/bin/firefox"), home(), default(), files);
        assert_eq!(direct, snap);
        let real_binary = sign_in_profile(Path::new("/opt/firefox/firefox"), home(), default(), files);
        assert_eq!(real_binary, default());
        let plain = sign_in_profile(Path::new("/usr/bin/google-chrome"), home(), default(), files);
        assert_eq!(plain, default());
        let wrapper = sign_in_profile(Path::new("/usr/bin/chromium-browser"), home(), default(), files);
        assert_eq!(wrapper, PathBuf::from("/home/me/snap/chromium/common").join(&leaf));
    }

    #[test]
    fn builds_the_launch_arguments_for_each_family() {
        let browser = |family| Installed {
            name: "Test",
            family,
            program: PathBuf::from("test"),
        };
        let args = |family, visible| -> Vec<String> {
            launch_command(&browser(family), Path::new("profile"), "https://x", visible, 9222)
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect()
        };
        let firefox = args(Family::Firefox, false);
        assert_eq!(
            firefox,
            [
                "--remote-debugging-port=9222",
                "--profile",
                "profile",
                "--no-remote",
                "--wait-for-browser",
                "--headless",
                "https://x"
            ]
        );
        assert!(args(Family::Firefox, true).contains(&"--width".to_owned()));
        let chromium = args(Family::Chromium, true);
        assert!(chromium.contains(&"--user-data-dir=profile".to_owned()) && chromium.contains(&"--app=https://x".to_owned()));
        assert!(args(Family::Chromium, false).contains(&"--headless=new".to_owned()));
    }
}
