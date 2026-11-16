use crate::app::{App, Saved};
use crate::playback::{Playback, Snapshot};
use crate::wake::{self, Next};
use eframe::egui;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

const STORED_FILE: &str = "app.ron";
const STATE_FILE: &str = "parked.json";
const PLAYBACK_FILE: &str = "parked-playback.json";
const FLUSH_EVERY: Duration = Duration::from_secs(30);
const UNSEEN_TICK: Duration = Duration::from_secs(1);
const OVERLAY_TICK: Duration = Duration::from_millis(250);
const LOCK_PATIENCE: Duration = Duration::from_millis(300);
const WINDOW_ATTEMPTS: u32 = 3;

pub type Slot = Rc<RefCell<Option<Box<App>>>>;

pub struct Hosted {
    app: Option<Box<App>>,
    slot: Slot,
}

impl Hosted {
    pub fn new(app: Box<App>, slot: Slot) -> Self {
        Self { app: Some(app), slot }
    }

    fn app(&mut self) -> &mut App {
        self.app.as_mut().expect("the app stays inside until the window is gone")
    }
}

impl eframe::App for Hosted {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.app.as_ref().map_or([0.0; 4], |app| app.clear_color(visuals))
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.app().update(ctx, frame);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.app().on_exit(gl);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.app().save(storage);
    }
}

impl Drop for Hosted {
    fn drop(&mut self) {
        if let Some(app) = self.app.take().filter(|app| app.parking()) {
            *self.slot.borrow_mut() = Some(app);
        }
    }
}

pub enum Outcome {
    Show,
    Quit,
}

pub fn keep_alive(app: &mut App) -> Outcome {
    let mut flush = Flush::new(Instant::now());
    let mut woken_by_ring = true;
    loop {
        let (playing, overlay_shown) = app.step_unseen();
        if crate::tray::quitting() {
            return Outcome::Quit;
        }
        if woken_by_ring || playing {
            flush.touch();
        }
        if flush.due_in(Instant::now()) == Some(Duration::ZERO) {
            app.keep_state();
            flush.done(Instant::now());
        }
        let wait = earliest(tick_while_unseen(playing, overlay_shown), flush.due_in(Instant::now()));
        let (next, rung) = wake::wait_parked(wait);
        woken_by_ring = rung;
        match next {
            Next::Stay => {}
            Next::Show => return Outcome::Show,
            Next::Quit => return Outcome::Quit,
        }
    }
}

pub fn tick_while_unseen(playing: bool, overlay_shown: bool) -> Option<Duration> {
    match (playing, overlay_shown) {
        (false, _) => None,
        (true, true) => Some(OVERLAY_TICK),
        (true, false) => Some(UNSEEN_TICK),
    }
}

pub fn earliest(first: Option<Duration>, second: Option<Duration>) -> Option<Duration> {
    match (first, second) {
        (Some(first), Some(second)) => Some(first.min(second)),
        (first, second) => first.or(second),
    }
}

pub struct Flush {
    last: Instant,
    dirty: bool,
}

impl Flush {
    pub fn new(now: Instant) -> Self {
        Self { last: now, dirty: false }
    }

    pub fn touch(&mut self) {
        self.dirty = true;
    }

    pub fn due_in(&self, now: Instant) -> Option<Duration> {
        self.dirty.then(|| (self.last + FLUSH_EVERY).saturating_duration_since(now))
    }

    pub fn done(&mut self, now: Instant) {
        self.last = now;
        self.dirty = false;
    }
}

pub fn may_try_a_window_again(failed_attempts: u32, tray_host_present: bool) -> bool {
    tray_host_present && failed_attempts < WINDOW_ATTEMPTS
}

pub struct Files {
    stored: PathBuf,
    state: PathBuf,
    playback: PathBuf,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

fn written_after(file: Option<SystemTime>, stored: Option<SystemTime>) -> bool {
    match (file, stored) {
        (None, _) => false,
        (Some(_), None) => true,
        (Some(file), Some(stored)) => file > stored,
    }
}

impl Files {
    pub fn in_folder(folder: &Path) -> Self {
        Self {
            stored: folder.join(STORED_FILE),
            state: folder.join(STATE_FILE),
            playback: folder.join(PLAYBACK_FILE),
        }
    }

    pub fn of_profile() -> Option<Self> {
        eframe::storage_dir(crate::app_id()).map(|folder| Self::in_folder(&folder))
    }

    pub fn write_state(&self, saved: &Saved) {
        if write_atomically(&self.state, &serde_json::to_vec(saved).unwrap_or_default()) {
            let _ = std::fs::remove_file(&self.playback);
        }
    }

    pub fn write_playback(&self, snapshot: &Snapshot) {
        write_atomically(&self.playback, &serde_json::to_vec(snapshot).unwrap_or_default());
    }

    pub fn read_state(&self) -> Option<Saved> {
        serde_json::from_slice(&std::fs::read(&self.state).ok()?).ok()
    }

    pub fn read_playback(&self) -> Option<Snapshot> {
        serde_json::from_slice(&std::fs::read(&self.playback).ok()?).ok()
    }

    pub fn recover(&self, stored: Option<Saved>) -> Saved {
        let stored_at = stored.as_ref().and_then(|_| modified(&self.stored));
        let newer = |path: &Path| written_after(modified(path), stored_at);
        let state = self.read_state().filter(|_| newer(&self.state));
        let playback = self.read_playback().filter(|_| newer(&self.playback));
        newest(stored, state, playback)
    }

    pub fn forget_superseded(&self) {
        let Some(stored_at) = modified(&self.stored) else { return };
        for path in [&self.state, &self.playback] {
            if modified(path).is_some_and(|at| at <= stored_at) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> bool {
    use std::io::Write;
    let partial = path.with_extension("partial");
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    std::fs::File::create(&partial)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| std::fs::rename(&partial, path))
        .is_ok()
}

pub fn newest(stored: Option<Saved>, parked: Option<Saved>, playback: Option<Snapshot>) -> Saved {
    let mut saved = parked.or(stored).unwrap_or_default();
    if let Some(playback) = playback {
        playback.apply_to(&mut saved);
    }
    saved
}

pub fn recover(stored: Option<Saved>) -> Saved {
    match Files::of_profile() {
        Some(files) => files.recover(stored),
        None => newest(stored, None, None),
    }
}

pub fn forget_superseded() {
    if let Some(files) = Files::of_profile() {
        files.forget_superseded();
    }
}

pub fn write_playback_of(playback: &Arc<Mutex<Playback>>) {
    let deadline = Instant::now() + LOCK_PATIENCE;
    loop {
        if let Ok(playback) = playback.try_lock() {
            if let Some(files) = Files::of_profile() {
                files.write_playback(&playback.snapshot());
            }
            return;
        }
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Repeat;
    use crate::ytm::Item;

    fn song(id: &str) -> Item {
        Item {
            video_id: id.into(),
            title: id.into(),
            ..Item::default()
        }
    }

    fn written_at(path: &Path, seconds: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)).unwrap();
    }

    fn at_position(position_ms: u64) -> Saved {
        Saved {
            position_ms,
            ..Saved::default()
        }
    }

    fn scratch_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tubefast-parked-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn a_parked_state_survives_the_round_trip_and_a_newer_one_replaces_it() {
        let folder = scratch_folder("round-trip");
        let files = Files::in_folder(&folder);
        assert!(files.read_state().is_none());
        let mut saved = Saved {
            queue: vec![song("a"), song("b")],
            current: Some(1),
            position_ms: 42_000,
            volume: 0.3,
            shuffle: true,
            repeat: Repeat::All,
            hide_on_close: true,
            ..Saved::default()
        };
        files.write_state(&saved);
        let read = files.read_state().unwrap();
        assert_eq!(read.queue.len(), 2);
        assert_eq!(read.current, Some(1));
        assert_eq!(read.position_ms, 42_000);
        assert!(read.shuffle && read.hide_on_close);
        assert!(read.repeat == Repeat::All);
        saved.position_ms = 99_000;
        files.write_state(&saved);
        assert_eq!(files.read_state().unwrap().position_ms, 99_000);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_parked_file_beats_the_stored_value_and_nothing_gives_the_defaults() {
        let stored = Saved {
            position_ms: 1,
            ..Saved::default()
        };
        let parked = Saved {
            position_ms: 2,
            ..Saved::default()
        };
        assert_eq!(newest(Some(stored), Some(parked), None).position_ms, 2);
        let stored = Saved {
            position_ms: 1,
            ..Saved::default()
        };
        assert_eq!(newest(Some(stored), None, None).position_ms, 1);
        assert_eq!(newest(None, None, None).position_ms, Saved::default().position_ms);
    }

    #[test]
    fn a_playback_snapshot_is_newer_than_the_state_it_is_laid_over() {
        let folder = scratch_folder("snapshot");
        let files = Files::in_folder(&folder);
        let snapshot = Snapshot::for_test(vec![song("x"), song("y"), song("z")], Some(2), 7_000);
        files.write_playback(&snapshot);
        let stored = Saved {
            queue: vec![song("old")],
            current: Some(0),
            position_ms: 1,
            ..Saved::default()
        };
        let saved = newest(Some(stored), None, files.read_playback());
        assert_eq!(saved.queue.len(), 3);
        assert_eq!(saved.current, Some(2));
        assert_eq!(saved.position_ms, 7_000);
        files.write_state(&saved);
        assert!(files.read_playback().is_none());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_file_counts_as_newer_only_when_written_after_eframes_file_or_that_is_missing() {
        let at = |seconds| Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds));
        let (early, late) = (at(10), at(20));
        assert!(written_after(late, early));
        assert!(!written_after(early, late));
        assert!(!written_after(early, early));
        assert!(written_after(early, None));
        assert!(!written_after(None, early));
        assert!(!written_after(None, None));
    }

    #[test]
    fn the_parked_state_wins_only_while_it_is_newer_than_eframes_file() {
        let folder = scratch_folder("newer-wins");
        let files = Files::in_folder(&folder);
        files.write_state(&at_position(2));
        std::fs::write(folder.join(STORED_FILE), b"stored").unwrap();
        written_at(&folder.join(STATE_FILE), 100);
        written_at(&folder.join(STORED_FILE), 50);
        assert_eq!(files.recover(Some(at_position(1))).position_ms, 2);
        written_at(&folder.join(STORED_FILE), 200);
        assert_eq!(files.recover(Some(at_position(1))).position_ms, 1);
        assert_eq!(files.recover(None).position_ms, 2);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_snapshot_older_than_eframes_file_is_not_laid_over_it() {
        let folder = scratch_folder("old-snapshot");
        let files = Files::in_folder(&folder);
        files.write_playback(&Snapshot::for_test(vec![song("x")], Some(0), 7_000));
        std::fs::write(folder.join(STORED_FILE), b"stored").unwrap();
        written_at(&folder.join(PLAYBACK_FILE), 100);
        written_at(&folder.join(STORED_FILE), 50);
        assert_eq!(files.recover(Some(at_position(1))).position_ms, 7_000);
        written_at(&folder.join(STORED_FILE), 200);
        assert_eq!(files.recover(Some(at_position(1))).position_ms, 1);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn parked_files_are_deleted_only_once_eframes_file_is_newer() {
        let folder = scratch_folder("superseded");
        let files = Files::in_folder(&folder);
        files.forget_superseded();
        files.write_state(&at_position(2));
        files.write_playback(&Snapshot::for_test(vec![song("x")], Some(0), 7_000));
        files.forget_superseded();
        assert!(files.read_state().is_some() && files.read_playback().is_some());
        std::fs::write(folder.join(STORED_FILE), b"stored").unwrap();
        written_at(&folder.join(STATE_FILE), 100);
        written_at(&folder.join(PLAYBACK_FILE), 100);
        written_at(&folder.join(STORED_FILE), 50);
        files.forget_superseded();
        assert!(files.read_state().is_some() && files.read_playback().is_some());
        written_at(&folder.join(STORED_FILE), 150);
        files.forget_superseded();
        assert!(files.read_state().is_none() && files.read_playback().is_none());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_window_that_fails_to_open_is_retried_only_with_a_tray_host_and_a_few_times() {
        assert!(may_try_a_window_again(1, true));
        assert!(may_try_a_window_again(WINDOW_ATTEMPTS - 1, true));
        assert!(!may_try_a_window_again(WINDOW_ATTEMPTS, true));
        assert!(!may_try_a_window_again(1, false));
    }

    #[test]
    fn a_broken_file_is_ignored() {
        let folder = scratch_folder("broken");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(STATE_FILE), b"{not json").unwrap();
        let files = Files::in_folder(&folder);
        assert!(files.read_state().is_none());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn ticks_run_only_while_something_plays_and_the_overlay_ticks_faster() {
        assert_eq!(tick_while_unseen(false, true), None);
        assert_eq!(tick_while_unseen(true, false), Some(UNSEEN_TICK));
        assert_eq!(tick_while_unseen(true, true), Some(OVERLAY_TICK));
        assert!(OVERLAY_TICK < UNSEEN_TICK);
    }

    #[test]
    fn the_nearest_deadline_wins_and_no_deadline_waits_forever() {
        let (short, long) = (Some(Duration::from_millis(5)), Some(Duration::from_secs(5)));
        assert_eq!(earliest(short, long), short);
        assert_eq!(earliest(long, short), short);
        assert_eq!(earliest(None, long), long);
        assert_eq!(earliest(short, None), short);
        assert_eq!(earliest(None, None), None);
    }

    #[test]
    fn state_is_flushed_when_changed_and_due_but_not_while_quiet() {
        let start = Instant::now();
        let mut flush = Flush::new(start);
        assert_eq!(flush.due_in(start), None);
        flush.touch();
        assert_eq!(flush.due_in(start), Some(FLUSH_EVERY));
        assert_eq!(flush.due_in(start + FLUSH_EVERY), Some(Duration::ZERO));
        assert_eq!(flush.due_in(start + FLUSH_EVERY * 2), Some(Duration::ZERO));
        flush.done(start + FLUSH_EVERY);
        assert_eq!(flush.due_in(start + FLUSH_EVERY), None);
        flush.touch();
        assert_eq!(flush.due_in(start + FLUSH_EVERY * 2), Some(Duration::ZERO));
    }
}
