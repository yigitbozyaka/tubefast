use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const UPDATED_FLAG: &str = "--updated";
pub const LATEST_RELEASE: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases/latest");
const DOWNLOADS: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases/download");
const BETA_TAG: &str = "beta";
const BETA_MANIFEST: &str = "version.txt";
const BETA_MARKER: &str = "-beta.";
const STABLE_TAG_MARKER: &str = "/tag/v";
const BACKUP_PREFIX: &str = "backup-";
const BACKUPS_KEPT: usize = 3;
const LIVE_STATE_FILE: &str = "live-state.json";
const UNREADABLE_STATE_PREFIX: &str = "unreadable-state-";
const FILES_NOT_BACKED_UP: [&str; 2] = ["crash.log", "session.lock"];
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    base: (u32, u32, u32),
    beta: u32,
}

fn parse_version(text: &str) -> Option<Version> {
    let (base, beta) = match text.split_once(BETA_MARKER) {
        Some((base, number)) => (base, number.parse().ok().filter(|number| *number > 0)?),
        None => (text, 0),
    };
    let mut parts = base.split('.').map(|part| part.parse::<u32>().ok());
    let base = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(Version { base, beta })
}

pub fn is_beta(version: &str) -> bool {
    version.contains(BETA_MARKER)
}

fn newest_offer(current: &str, stable: Option<String>, beta: Option<String>) -> Option<String> {
    let current = parse_version(current)?;
    [stable, beta]
        .into_iter()
        .flatten()
        .filter_map(|candidate| Some((parse_version(&candidate)?, candidate)))
        .filter(|(version, _)| *version > current)
        .max_by_key(|(version, _)| *version)
        .map(|(_, candidate)| candidate)
}

pub fn stable_version(agent: &ureq::Agent) -> Option<String> {
    let latest = agent.head(LATEST_RELEASE).call().ok()?;
    let version = latest.get_url().rsplit_once(STABLE_TAG_MARKER)?.1;
    parse_version(version).filter(|parsed| parsed.beta == 0).map(|_| version.to_owned())
}

fn beta_version(agent: &ureq::Agent) -> Option<String> {
    let manifest = agent.get(&format!("{DOWNLOADS}/{BETA_TAG}/{BETA_MANIFEST}")).call().ok()?;
    let version = manifest.into_string().ok()?.trim().to_owned();
    (is_beta(&version) && parse_version(&version).is_some()).then_some(version)
}

pub fn latest(agent: &ureq::Agent, beta: bool, current: &str) -> Option<String> {
    let stable = stable_version(agent);
    newest_offer(current, stable, beta.then(|| beta_version(agent)).flatten())
}

fn release_folder(version: &str) -> String {
    if is_beta(version) {
        format!("{DOWNLOADS}/{BETA_TAG}")
    } else {
        format!("{DOWNLOADS}/v{version}")
    }
}

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
    download(agent, &release_folder(version), placement.asset, &downloaded)?;
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

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn backup_age(folder: &Path) -> u64 {
    folder
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.rsplit('-').next())
        .and_then(|seconds| seconds.parse().ok())
        .unwrap_or(0)
}

fn private_folder(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn forget_old_backups(storage: &Path) {
    let Ok(entries) = std::fs::read_dir(storage) else { return };
    let mut backups: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(BACKUP_PREFIX))
        })
        .collect();
    backups.sort_by_key(|backup| std::cmp::Reverse(backup_age(backup)));
    for old in backups.into_iter().skip(BACKUPS_KEPT) {
        remove_path(&old);
    }
}

fn back_up_into(storage: &Path, live_state: &[u8], from_version: &str, seconds: u64) -> Result<PathBuf, String> {
    let backup = storage.join(format!("{BACKUP_PREFIX}{from_version}-{seconds}"));
    let failed = |error: std::io::Error| {
        remove_path(&backup);
        format!("your data could not be backed up first ({error})")
    };
    let entries = std::fs::read_dir(storage).map_err(failed)?;
    private_folder(&backup).map_err(failed)?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let skipped =
            name.starts_with(BACKUP_PREFIX) || name.starts_with(UNREADABLE_STATE_PREFIX) || FILES_NOT_BACKED_UP.contains(&name.as_str());
        if entry.path().is_file() && !skipped {
            std::fs::copy(entry.path(), backup.join(&name)).map_err(failed)?;
        }
    }
    std::fs::write(backup.join(LIVE_STATE_FILE), live_state).map_err(failed)?;
    forget_old_backups(storage);
    Ok(backup)
}

pub fn downgrade_to_stable(agent: &ureq::Agent, live_state: &[u8], from_version: &str) -> Result<PathBuf, String> {
    let storage = eframe::storage_dir(crate::app_id()).ok_or("there is no folder to back your data up in")?;
    let version = stable_version(agent).ok_or("the latest stable release could not be found")?;
    back_up_into(&storage, live_state, from_version, now_seconds())?;
    install(agent, &version)
}

pub fn keep_unreadable_state(raw: &str) {
    let Some(storage) = eframe::storage_dir(crate::app_id()) else {
        return;
    };
    let file = storage.join(format!("{UNREADABLE_STATE_PREFIX}{}.ron", now_seconds()));
    if std::fs::create_dir_all(&storage).is_ok() && !file.exists() {
        let _ = std::fs::write(file, raw);
    }
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

    fn some(version: &str) -> Option<String> {
        Some(version.to_owned())
    }

    #[test]
    fn orders_betas_between_the_stable_before_and_the_stable_after() {
        let order = |older: &str, newer: &str| parse_version(older).unwrap() < parse_version(newer).unwrap();
        assert!(order("0.11.0-beta.42", "0.12.0"));
        assert!(order("0.12.0", "0.12.0-beta.43"));
        assert!(order("0.12.0-beta.9", "0.12.0-beta.10"));
        assert!(order("0.12.0-beta.43", "0.12.1"));
        assert!(order("0.9.1", "0.10.0"));
        assert_eq!(parse_version("0.12.0"), parse_version("0.12.0"));
    }

    #[test]
    fn parses_only_whole_versions() {
        assert!(parse_version("0.12.0-beta.43").is_some());
        for broken in [
            "",
            "nightly",
            "0.12",
            "0.12.0.1",
            "0.12.x",
            "0.12.0-beta.",
            "0.12.0-beta.0",
            "0.12.0-beta.x",
            "v0.12.0",
        ] {
            assert_eq!(parse_version(broken), None, "{broken}");
        }
        assert!(is_beta("0.12.0-beta.1") && !is_beta("0.12.0"));
    }

    #[test]
    fn offers_the_newest_candidate_that_beats_this_build() {
        assert_eq!(newest_offer("0.11.0", some("0.12.0"), None).as_deref(), Some("0.12.0"));
        assert_eq!(
            newest_offer("0.11.0", some("0.12.0"), some("0.12.0-beta.7")).as_deref(),
            Some("0.12.0-beta.7")
        );
        assert_eq!(
            newest_offer("0.11.0", some("0.12.0"), some("0.11.0-beta.30")).as_deref(),
            Some("0.12.0")
        );
        assert_eq!(newest_offer("0.12.0-beta.7", some("0.12.0"), some("0.12.0-beta.7")), None);
        assert_eq!(
            newest_offer("0.12.0-beta.7", some("0.12.0"), some("0.12.0-beta.8")).as_deref(),
            Some("0.12.0-beta.8")
        );
        assert_eq!(newest_offer("0.12.0-beta.7", some("0.12.1"), None).as_deref(), Some("0.12.1"));
        assert_eq!(newest_offer("0.11.0", some("garbage"), None), None);
        assert_eq!(newest_offer("0.11.0", None, None), None);
    }

    #[test]
    fn beta_builds_download_from_the_rolling_release() {
        assert!(release_folder("0.12.0-beta.43").ends_with("/releases/download/beta"));
        assert!(release_folder("0.12.0").ends_with("/releases/download/v0.12.0"));
    }

    fn scratch_storage(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tubefast-{name}-{}", std::process::id()));
        remove_path(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn backs_up_every_data_file_and_keeps_only_the_latest_backups() {
        let storage = scratch_storage("backup");
        std::fs::write(storage.join("app.ron"), "state").unwrap();
        std::fs::write(storage.join("session.bin"), "cookie").unwrap();
        std::fs::write(storage.join("crash.log"), "log").unwrap();
        for seconds in [100, 200, 300, 400] {
            let backup = back_up_into(&storage, b"{}", "0.12.0-beta.5", seconds).unwrap();
            assert_eq!(std::fs::read_to_string(backup.join("app.ron")).unwrap(), "state");
            assert_eq!(std::fs::read_to_string(backup.join("session.bin")).unwrap(), "cookie");
            assert_eq!(std::fs::read(backup.join(LIVE_STATE_FILE)).unwrap(), b"{}");
            assert!(!backup.join("crash.log").exists());
        }
        let mut remaining: Vec<String> = std::fs::read_dir(&storage)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(BACKUP_PREFIX))
            .collect();
        remaining.sort();
        assert_eq!(
            remaining,
            ["backup-0.12.0-beta.5-200", "backup-0.12.0-beta.5-300", "backup-0.12.0-beta.5-400"]
        );
        assert_eq!(std::fs::read_to_string(storage.join("app.ron")).unwrap(), "state");
        std::fs::remove_dir_all(&storage).unwrap();
    }

    #[test]
    fn a_failed_backup_leaves_nothing_behind() {
        let storage = scratch_storage("backup-failed");
        let missing = storage.join("nothing-here");
        assert!(back_up_into(&missing, b"{}", "0.12.0", 1).is_err());
        assert!(!missing.exists());
        std::fs::remove_dir_all(&storage).unwrap();
    }
}
