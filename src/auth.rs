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
const BROWSERS: [(&str, &str); 4] = [
    ("ProgramFiles(x86)", r"Microsoft\Edge\Application\msedge.exe"),
    ("ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
    ("ProgramFiles", r"Google\Chrome\Application\chrome.exe"),
    ("LocalAppData", r"Google\Chrome\Application\chrome.exe"),
];
const BROWSERS_ELSEWHERE: [&str; 7] = [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];
pub const SESSION_KEPT: &str = if cfg!(windows) {
    "The session stays on this computer, encrypted with your Windows account."
} else {
    "The session stays on this computer, in a file only your user account can read."
};

fn sapisid(cookie: &str) -> Option<&str> {
    let value = |name: &str| cookie.split("; ").find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='));
    value("__Secure-3PAPISID").or_else(|| value("SAPISID"))
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
    match sapisid(&cookie) {
        Some(_) => Ok(cookie),
        None => Err("That text has no YouTube session in it. Copy the whole cookie value while signed in.".to_owned()),
    }
}

fn session_file() -> Option<PathBuf> {
    Some(eframe::storage_dir(app_id())?.join("session.bin"))
}

pub fn store(cookie: &str) -> Result<(), String> {
    let sealed = seal(cookie.as_bytes(), true).ok_or("This sign-in cannot be kept after a restart on this system.")?;
    let file = session_file().ok_or("There is no folder to keep the sign-in in.")?;
    let mut private = std::fs::OpenOptions::new();
    private.create(true).write(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut private, 0o600);
    std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))
        .and_then(|()| private.open(file)?.write_all(&sealed))
        .map_err(|e| e.to_string())
}

pub fn load() -> Option<String> {
    String::from_utf8(seal(&std::fs::read(session_file()?).ok()?, false)?).ok()
}

pub fn forget() {
    if let Some(file) = session_file() {
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
}

impl Socket {
    fn open(port: u16) -> Option<Socket> {
        let version: Value = ureq::get(&format!("http://127.0.0.1:{port}/json/version"))
            .timeout(Duration::from_secs(2))
            .call()
            .ok()?
            .into_json()
            .ok()?;
        let address = version["webSocketDebuggerUrl"].as_str()?;
        let path = &address[address.find("/devtools/")?..];
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
        head.starts_with(b"HTTP/1.1 101").then_some(Socket { stream, calls: 0 })
    }

    fn call(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.calls += 1;
        let text = json!({ "id": self.calls, "method": method, "params": params }).to_string();
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
        frame.extend([0u8; 4]);
        frame.extend(text.as_bytes());
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
        .filter_map(|cookie| Some(format!("{}={}", cookie["name"].as_str()?, cookie["value"].as_str()?)))
        .collect();
    let cookie = pairs.join("; ");
    sapisid(&cookie).is_some().then_some(cookie)
}

fn launch(profile: &Path, extra: &[&str]) -> Result<(Child, u16), String> {
    let port = TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| listener.local_addr())
        .map_err(|e| e.to_string())?
        .port();
    let installed = BROWSERS
        .iter()
        .filter_map(|(root, path)| Some(PathBuf::from(std::env::var_os(root)?).join(path)));
    let programs: Vec<PathBuf> = if cfg!(windows) {
        installed.collect()
    } else {
        BROWSERS_ELSEWHERE.iter().map(PathBuf::from).collect()
    };
    let _ = std::fs::remove_dir_all(profile);
    programs
        .iter()
        .find_map(|program| {
            Command::new(program)
                .arg(format!("--user-data-dir={}", profile.display()))
                .arg(format!("--remote-debugging-port={port}"))
                .args(["--no-first-run", "--no-default-browser-check"])
                .args(extra)
                .spawn()
                .ok()
        })
        .map(|browser| (browser, port))
        .ok_or_else(|| "Neither Edge nor Chrome was found on this computer.".to_owned())
}

fn watch(browser: &mut Child, port: u16, cancel: &AtomicBool) -> Result<String, String> {
    let started = Instant::now();
    let mut socket: Option<Socket> = None;
    let mut settled = 0;
    loop {
        if cancel.load(Relaxed) {
            return Err(String::new());
        }
        if started.elapsed() > LOGIN_TIMEOUT {
            return Err("Signing in took too long. Try again.".to_owned());
        }
        if matches!(browser.try_wait(), Ok(Some(_))) {
            return Err("The browser window was closed before you signed in.".to_owned());
        }
        std::thread::sleep(POLL_EVERY);
        let Some(connection) = &mut socket else {
            socket = Socket::open(port);
            continue;
        };
        let Ok(reply) = connection.call("Storage.getCookies", json!({})) else {
            socket = None;
            continue;
        };
        let Some(cookie) = youtube_cookie(&reply) else { continue };
        settled += 1;
        if cookie.contains("LOGIN_INFO=") || settled >= SETTLED_POLLS {
            let _ = connection.call("Browser.close", json!({}));
            return Ok(cookie);
        }
    }
}

fn discard(mut browser: Child, profile: &Path) {
    let _ = browser.kill();
    let _ = browser.wait();
    for _ in 0..15 {
        if std::fs::remove_dir_all(profile).is_ok() || !profile.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn browser_sign_in(cancel: &AtomicBool) -> Result<String, String> {
    let profile = eframe::storage_dir(app_id())
        .ok_or("There is no folder for the sign-in window.")?
        .join("signin-profile");
    let app = format!("--app={LOGIN_URL}");
    let (mut browser, port) = launch(&profile, &["--window-size=520,760", &app])?;
    let cookie = watch(&mut browser, port, cancel);
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
        let raw = "SID=a;  SAPISID=k ; junk; LOGIN_INFO=x";
        assert_eq!(clean_cookie(raw).unwrap(), "SID=a; SAPISID=k; LOGIN_INFO=x");
        assert_eq!(
            clean_cookie("accept: */*\r\nCookie: SID=a; SAPISID=k\r\nuser-agent: x").unwrap(),
            "SID=a; SAPISID=k"
        );
        assert_eq!(
            clean_cookie("curl 'https://x' \\\n  -H 'cookie: SID=a; SAPISID=k' \\\n  -H 'x: y'").unwrap(),
            "SID=a; SAPISID=k"
        );
        assert!(clean_cookie("SID=a; HSID=b").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn sealed_sessions_round_trip_and_are_not_plain_text() {
        let sealed = seal(b"SAPISID=secret", true).unwrap();
        assert!(!sealed.windows(6).any(|window| window == b"secret"));
        assert_eq!(seal(&sealed, false).unwrap(), b"SAPISID=secret");
        assert!(seal(b"not a sealed blob", false).is_none());
    }

    #[test]
    #[ignore = "needs Edge or Chrome installed"]
    fn reads_the_session_out_of_a_real_browser() {
        let profile = std::env::temp_dir().join(format!("{}-signin-test-{}", app_id(), std::process::id()));
        let (mut browser, port) = launch(&profile, &["--headless=new", "about:blank"]).unwrap();
        let mut socket = (0..100)
            .find_map(|_| {
                std::thread::sleep(Duration::from_millis(200));
                Socket::open(port)
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
        let found = watch(&mut browser, port, &AtomicBool::new(false));
        discard(browser, &profile);
        let found = found.unwrap();
        assert!(
            found.contains("SAPISID=abc") && found.contains("LOGIN_INFO=xyz") && !found.contains("wrong-site"),
            "{found}"
        );
        assert!(!profile.exists());
    }
}
