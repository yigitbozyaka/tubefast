use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const UPDATED_FLAG: &str = "--updated";
const DOWNLOADS: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases/download");
const PROGRAM: &str = "tubefast.exe";
const LINUX_PROGRAM: &str = "tubefast-linux-x64";
const LINUX_APPIMAGE: &str = "Tubefast-x86_64.AppImage";
const MACOS_ARCHIVE: &str = "tubefast-macos-universal.zip";
const MACOS_BUNDLE: &str = "Tubefast.app";
const WRITE_PROBE: &str = ".tubefast-write-probe";
const CHECKSUMS: &str = "SHA256SUMS.txt";
const PROGRAM_BYTES: u64 = 64 * 1024 * 1024;
const SCOOP_FOLDER: &str = "\\scoop\\apps\\";
const SCOOP_COMMAND: &str = "scoop update tubefast";

pub enum Plan {
    InPlace,
    PackageManager(&'static str),
    DownloadPage,
}

struct Placement {
    asset: &'static str,
    replace: PathBuf,
    bundle: bool,
}

fn writable(folder: &Path) -> bool {
    let probe = folder.join(WRITE_PROBE);
    let created = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(probe);
    created
}

fn app_bundle(program: &Path) -> Option<&Path> {
    program
        .ancestors()
        .nth(3)
        .filter(|bundle| bundle.extension().is_some_and(|extension| extension == "app"))
}

fn placement() -> Option<Placement> {
    let program = std::env::current_exe().ok()?;
    if cfg!(windows) {
        return Some(Placement {
            asset: PROGRAM,
            replace: program,
            bundle: false,
        });
    }
    let placement = if cfg!(target_os = "macos") {
        Placement {
            asset: MACOS_ARCHIVE,
            replace: app_bundle(&program)?.to_path_buf(),
            bundle: true,
        }
    } else {
        if std::env::consts::ARCH != "x86_64" {
            return None;
        }
        let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|path| path.is_file());
        match appimage {
            Some(appimage) => Placement {
                asset: LINUX_APPIMAGE,
                replace: appimage,
                bundle: false,
            },
            None => Placement {
                asset: LINUX_PROGRAM,
                replace: program,
                bundle: false,
            },
        }
    };
    writable(placement.replace.parent()?).then_some(placement)
}

pub fn plan() -> Plan {
    let program = std::env::current_exe().ok().filter(|_| cfg!(windows));
    match program {
        Some(program) if program.to_string_lossy().to_lowercase().contains(SCOOP_FOLDER) => Plan::PackageManager(SCOOP_COMMAND),
        Some(_) => Plan::InPlace,
        None if placement().is_some() => Plan::InPlace,
        None => Plan::DownloadPage,
    }
}

fn listed_checksum<'a>(checksums: &'a str, file: &str) -> Option<&'a str> {
    checksums.lines().find_map(|line| {
        let (checksum, name) = line.split_once(char::is_whitespace)?;
        let complete = checksum.len() == 64 && checksum.chars().all(|c| c.is_ascii_hexdigit());
        (complete && name.trim().trim_start_matches('*') == file).then_some(checksum)
    })
}

fn download(agent: &ureq::Agent, release: &str, asset: &str, into: &Path) -> Result<(), String> {
    let failed = |error: ureq::Error| format!("the download failed ({error})");
    let checksums = agent.get(&format!("{release}/{CHECKSUMS}")).call().map_err(failed)?;
    let checksums = checksums.into_string().map_err(|e| e.to_string())?;
    let expected = listed_checksum(&checksums, asset).ok_or("the release lists no checksum for the program")?;
    let response = agent.get(&format!("{release}/{asset}")).call().map_err(failed)?;
    let mut body = response.into_reader().take(PROGRAM_BYTES);
    let mut file = std::fs::File::create(into).map_err(|e| format!("this folder cannot be written to ({e})"))?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut block = [0u8; 64 * 1024];
    loop {
        let read = body.read(&mut block).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        digest.update(&block[..read]);
        file.write_all(&block[..read]).map_err(|e| e.to_string())?;
    }
    let actual: String = digest.finish().as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
    if actual.eq_ignore_ascii_case(expected) {
        return Ok(());
    }
    drop(file);
    let _ = std::fs::remove_file(into);
    Err("the download does not match its published checksum".to_owned())
}

fn remove_path(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<(), String> {
    Ok(())
}

fn unpack(archive: &Path, folder: &Path) -> Result<PathBuf, String> {
    remove_path(folder);
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let unpacked = Command::new("ditto").args(["-x", "-k"]).arg(archive).arg(folder).status();
    let bundle = folder.join(MACOS_BUNDLE);
    if unpacked.is_ok_and(|status| status.success()) && bundle.is_dir() {
        Ok(bundle)
    } else {
        Err("the download could not be unpacked".to_owned())
    }
}

fn swap(program: &Path, fresh: &Path) -> Result<(), String> {
    let previous = program.with_extension("old");
    remove_path(&previous);
    std::fs::rename(program, &previous).map_err(|e| format!("the running program could not be moved aside ({e})"))?;
    std::fs::rename(fresh, program).map_err(|error| {
        let _ = std::fs::rename(&previous, program);
        format!("the new program could not be put in place ({error})")
    })
}

pub fn install(agent: &ureq::Agent, version: &str) -> Result<PathBuf, String> {
    let program = std::env::current_exe().map_err(|e| e.to_string())?;
    let placement = placement().ok_or("this installation cannot update itself")?;
    let downloaded = placement.replace.with_extension("new");
    download(agent, &format!("{DOWNLOADS}/v{version}"), placement.asset, &downloaded)?;
    if placement.bundle {
        let unpacked = placement.replace.with_extension("unpacked");
        let fresh = unpack(&downloaded, &unpacked)?;
        swap(&placement.replace, &fresh)?;
        remove_path(&unpacked);
        remove_path(&downloaded);
        return Ok(program);
    }
    make_executable(&downloaded)?;
    swap(&placement.replace, &downloaded)?;
    Ok(placement.replace)
}

pub fn restart(program: &Path, arguments: &[String]) {
    let _ = Command::new(program).args(arguments).arg(UPDATED_FLAG).spawn();
}

pub fn clear_leftovers() {
    if let Some(placement) = placement() {
        for leftover in ["old", "new", "unpacked"] {
            remove_path(&placement.replace.with_extension(leftover));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    const NEW_PROGRAM: &[u8] = b"the next version of the program";
    const WRONG_CHECKSUM: &str = "0b8c9e6c1f1b6f0d3c0f2a3a4b5c6d7e8f90123456789abcdef0123456789abc";

    fn serve(checksum: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut request = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                reader.read_line(&mut request).unwrap();
                while reader.read_line(&mut String::new()).unwrap() > 2 {}
                let body = if request.contains(CHECKSUMS) {
                    format!("{}  other.zip\n{checksum}  {PROGRAM}\n", "a".repeat(64)).into_bytes()
                } else {
                    NEW_PROGRAM.to_vec()
                };
                let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                stream.write_all(head.as_bytes()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        address
    }

    fn sha256(bytes: &[u8]) -> String {
        let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
        digest.as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn reads_only_a_complete_checksum_for_the_named_file() {
        let good = "f".repeat(64);
        let list = format!(
            "{}  tubefast-windows-x64.zip\n{good} *tubefast.exe\nshort  tubefast.exe\n",
            "a".repeat(64)
        );
        assert_eq!(listed_checksum(&list, "tubefast.exe"), Some(good.as_str()));
        assert_eq!(listed_checksum(&list, "missing.exe"), None);
        assert_eq!(listed_checksum("abc  tubefast.exe", "tubefast.exe"), None);
    }

    #[test]
    fn finds_the_app_bundle_only_around_the_program() {
        let inside = Path::new("/Applications/Tubefast.app/Contents/MacOS/tubefast");
        assert_eq!(app_bundle(inside), Some(Path::new("/Applications/Tubefast.app")));
        assert_eq!(app_bundle(Path::new("/home/me/.local/bin/tubefast")), None);
        assert_eq!(app_bundle(Path::new("/Users/me/target/release/tubefast")), None);
    }

    #[test]
    fn installs_a_verified_download_and_refuses_a_tampered_one() {
        let folder = std::env::temp_dir().join(format!("tubefast-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (program, fresh) = (folder.join("Tubefast.exe"), folder.join("Tubefast.new"));
        std::fs::write(&program, b"the running version").unwrap();
        let agent = ureq::agent();

        let tampered = download(&agent, &serve(WRONG_CHECKSUM.to_owned()), PROGRAM, &fresh);
        assert!(tampered.unwrap_err().contains("checksum"));
        assert!(!fresh.exists());
        assert_eq!(std::fs::read(&program).unwrap(), b"the running version");

        download(&agent, &serve(sha256(NEW_PROGRAM)), PROGRAM, &fresh).unwrap();
        swap(&program, &fresh).unwrap();
        assert_eq!(std::fs::read(&program).unwrap(), NEW_PROGRAM);
        assert_eq!(std::fs::read(folder.join("Tubefast.old")).unwrap(), b"the running version");
        assert!(!fresh.exists());
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
