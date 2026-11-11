use crate::app::{Action, Repeat, Saved};
use crate::player::Player;
use crate::ytm::{Client, Item};
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const RESTART_AFTER_MS: u64 = 3000;
const PLAYED_KEPT: usize = 50;
const RADIO_MARGIN: usize = 2;

pub enum Signal {
    Changed,
    Pressed(Box<Action>),
    Radio(String, Vec<Item>),
}

pub type Replaced = (Vec<Item>, Option<usize>, u64);

pub struct Playback {
    pub player: Player,
    queue: Vec<Item>,
    current: Option<usize>,
    repeat: Repeat,
    shuffle: bool,
    resume_at: Option<u64>,
    curated: bool,
    radio_seed: String,
    started: Vec<Item>,
    problem: Option<String>,
    version: u64,
    yt: Arc<Client>,
    signals: Sender<Signal>,
}

pub fn run(playback: Playback, signals: Receiver<Signal>, ctx: egui::Context) -> Arc<Mutex<Playback>> {
    let shared = Arc::new(Mutex::new(playback));
    let worker = shared.clone();
    std::thread::spawn(move || {
        while let Ok(signal) = signals.recv() {
            let mut playback = worker.lock().unwrap();
            match signal {
                Signal::Changed => {}
                Signal::Pressed(action) => playback.press(*action),
                Signal::Radio(seed, items) => playback.add_radio(&seed, items),
            }
            playback.follow_player();
            drop(playback);
            ctx.request_repaint();
        }
    });
    shared
}

impl Playback {
    pub fn new(player: Player, yt: Arc<Client>, signals: Sender<Signal>, saved: &Saved) -> Self {
        Self {
            player,
            queue: saved.queue.clone(),
            current: saved.current,
            repeat: saved.repeat,
            shuffle: saved.shuffle,
            resume_at: saved.current.map(|_| saved.position_ms),
            curated: !saved.queue.is_empty(),
            radio_seed: String::new(),
            started: Vec::new(),
            problem: None,
            version: 0,
            yt,
            signals,
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn queue(&self) -> &[Item] {
        &self.queue
    }

    pub fn current(&self) -> Option<usize> {
        self.current
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn position_ms(&self) -> u64 {
        self.resume_at.unwrap_or_else(|| self.player.position_ms())
    }

    pub fn take_started(&mut self) -> Vec<Item> {
        std::mem::take(&mut self.started)
    }

    pub fn take_problem(&mut self) -> Option<String> {
        self.problem.take()
    }

    fn changed(&mut self) {
        self.version += 1;
    }

    fn playing(&self) -> bool {
        self.current.is_some() && !self.player.paused()
    }

    fn follow_player(&mut self) {
        if let Some(message) = self.player.take_error() {
            self.player.set_paused(true);
            self.problem = Some(with_retry_hint(&message));
        }
        if self.player.take_ended() {
            if self.repeat == Repeat::One {
                self.player.seek(0);
            } else {
                self.next(true);
            }
        }
    }

    pub fn press(&mut self, action: Action) {
        match action {
            Action::Toggle => self.toggle(),
            Action::Pause(paused) => {
                if paused == self.playing() {
                    self.toggle();
                }
            }
            Action::Next => self.next(false),
            Action::Previous => self.previous(),
            Action::Seek(ms) => self.player.seek(ms),
            Action::Volume(volume) => self.player.set_volume(volume.clamp(0.0, 1.0)),
            _ => {}
        }
    }

    pub fn play(&mut self, items: Vec<Item>, index: usize) -> Option<Replaced> {
        let chosen = items.get(index).map(|item| item.video_id.clone()).unwrap_or_default();
        let songs: Vec<Item> = items.into_iter().filter(Item::is_song).collect();
        let before = std::mem::replace(&mut self.queue, songs);
        let upcoming = self.current.map_or(0, |current| current + 1);
        let same_list =
            before.len() == self.queue.len() && before.iter().all(|old| self.queue.iter().any(|new| new.video_id == old.video_id));
        let mut replaced = None;
        if self.curated && upcoming < before.len() && !same_list {
            replaced = Some((before, self.current, self.position_ms()));
        }
        self.curated = self.queue.len() > 1;
        let mut at = self.queue.iter().position(|item| item.video_id == chosen).unwrap_or(0);
        if self.shuffle && !self.queue.is_empty() {
            self.queue.swap(0, at);
            at = 0;
            shuffle(&mut self.queue[1..]);
        }
        self.radio_seed.clear();
        self.changed();
        self.play_index(at);
        replaced
    }

    pub fn play_index(&mut self, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        let played = if self.repeat == Repeat::All {
            0
        } else {
            index.saturating_sub(PLAYED_KEPT)
        };
        self.queue.drain(..played);
        let index = index - played;
        let item = self.queue[index].clone();
        self.resume_at = None;
        self.current = Some(index);
        self.changed();
        self.player.load(&item.video_id);
        if let Some(next) = self.queue.get(index + 1) {
            self.player.preload(&next.video_id);
        }
        if index + RADIO_MARGIN >= self.queue.len() && self.radio_seed != item.video_id {
            self.radio_seed = item.video_id.clone();
            let (yt, signals, seed) = (self.yt.clone(), self.signals.clone(), item.video_id.clone());
            std::thread::spawn(move || {
                if let Ok(items) = yt.radio(&seed) {
                    let _ = signals.send(Signal::Radio(seed, items));
                }
            });
        }
        self.started.push(item);
    }

    fn add_radio(&mut self, seed: &str, items: Vec<Item>) {
        if seed != self.radio_seed {
            return;
        }
        let upcoming = self.current.map_or(0, |current| current + 1);
        let had_next = upcoming < self.queue.len();
        for item in items {
            if !self.queue.iter().any(|queued| queued.video_id == item.video_id) {
                self.queue.push(item);
            }
        }
        if let (false, Some(next)) = (had_next, self.queue.get(upcoming)) {
            self.player.preload(&next.video_id);
        }
        self.changed();
    }

    fn next(&mut self, automatic: bool) {
        let Some(current) = self.current else { return };
        if current + 1 < self.queue.len() {
            self.play_index(current + 1);
        } else if self.repeat == Repeat::All {
            self.play_index(0);
        } else if automatic {
            self.player.seek(0);
            self.player.set_paused(true);
        }
    }

    fn previous(&mut self) {
        match self.current {
            Some(current) if current > 0 && self.player.position_ms() < RESTART_AFTER_MS => self.play_index(current - 1),
            Some(_) => self.player.seek(0),
            None => {}
        }
    }

    fn toggle(&mut self) {
        match (self.current, self.resume_at) {
            (Some(_), _) if self.player.loading() => {}
            (Some(current), None) if self.player.failed() => {
                let position = self.player.position_ms();
                self.play_index(current);
                self.player.seek(position);
            }
            (Some(current), Some(position)) => {
                self.play_index(current);
                self.player.seek(position);
            }
            (Some(_), None) => self.player.set_paused(!self.player.paused()),
            (None, _) => {}
        }
    }

    pub fn play_next(&mut self, item: Item) {
        let at = self.current.map_or(0, |current| current + 1).min(self.queue.len());
        self.curated = true;
        self.queue.insert(at, item);
        self.changed();
        if self.current.is_none() {
            self.play_index(0);
        }
    }

    pub fn enqueue(&mut self, item: Item) {
        self.curated = true;
        self.queue.push(item);
        self.changed();
        if self.current.is_none() {
            self.play_index(0);
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.queue.len() && self.current != Some(index) {
            self.queue.remove(index);
            if let Some(current) = self.current.as_mut().filter(|current| index < **current) {
                *current -= 1;
            }
            self.changed();
        }
    }

    pub fn extend(&mut self, fresh: impl IntoIterator<Item = Item>) {
        self.queue.extend(fresh);
        if self.shuffle {
            let upcoming = self.current.map_or(0, |current| current + 1).min(self.queue.len());
            shuffle(&mut self.queue[upcoming..]);
        }
        self.changed();
    }

    pub fn restore(&mut self, (queue, current, position): Replaced) {
        self.queue = queue;
        self.curated = true;
        self.changed();
        if let Some(current) = current {
            self.play_index(current);
            self.player.seek(position);
        }
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle ^= true;
        let upcoming = self.current.map_or(0, |current| current + 1).min(self.queue.len());
        if self.shuffle {
            shuffle(&mut self.queue[upcoming..]);
        }
        self.changed();
    }

    pub fn cycle_repeat(&mut self) {
        self.repeat = match self.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
        self.changed();
    }

    pub fn reorder(&mut self, from: usize, gap: usize) {
        let upcoming = self.current.map_or(0, |current| current + 1);
        if reorder(&mut self.queue, upcoming, from, gap) {
            self.curated = true;
            if from == upcoming || gap == upcoming {
                self.player.preload(&self.queue[upcoming].video_id);
            }
            self.changed();
        }
    }
}

fn with_retry_hint(message: &str) -> String {
    let transient = ["Could not reach", "connection was lost", "timed out", "HTTP 5", "HTTP 429"];
    let message = message.trim_end_matches('.');
    if transient.iter().any(|sign| message.contains(sign)) {
        format!("{message}. Press play to try again.")
    } else {
        format!("{message}.")
    }
}

fn reorder(queue: &mut Vec<Item>, upcoming: usize, from: usize, gap: usize) -> bool {
    let to = if gap > from { gap - 1 } else { gap };
    if from == to || from.min(to) < upcoming || from.max(to) >= queue.len() {
        return false;
    }
    let item = queue.remove(from);
    queue.insert(to, item);
    true
}

fn shuffle(items: &mut [Item]) {
    let mut state = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_nanos() as u64)
        | 1;
    for index in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        items.swap(index, (state % (index as u64 + 1)) as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn song(id: &str) -> Item {
        Item {
            video_id: id.to_owned(),
            ..Item::default()
        }
    }

    #[test]
    fn a_finished_song_repeats_while_the_window_draws_nothing() {
        let yt = Arc::new(Client::new(String::new()));
        let (signals, inbox) = mpsc::channel();
        let wake = signals.clone();
        let player = Player::new(yt.clone(), move || {
            let _ = wake.send(Signal::Changed);
        });
        let saved = Saved {
            queue: vec![song("a")],
            current: Some(0),
            repeat: Repeat::One,
            ..Saved::default()
        };
        let playback = run(Playback::new(player, yt, signals, &saved), inbox, egui::Context::default());
        playback.lock().unwrap().player.end_at(90_000);
        let deadline = Instant::now() + Duration::from_secs(5);
        while playback.lock().unwrap().player.position_ms() != 0 {
            assert!(Instant::now() < deadline, "the ended song was never restarted");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_retry_hint_follows_only_transient_failures() {
        assert_eq!(
            with_retry_hint("Could not reach YouTube. Check your connection."),
            "Could not reach YouTube. Check your connection. Press play to try again."
        );
        assert_eq!(with_retry_hint("Sign in to confirm your age"), "Sign in to confirm your age.");
    }

    #[test]
    fn reorders_only_the_upcoming_part_of_the_queue() {
        let order = |queue: &[Item]| queue.iter().map(|item| item.video_id.as_str()).collect::<String>();
        let mut queue: Vec<Item> = "abcde".chars().map(|id| song(&id.to_string())).collect();
        assert!(reorder(&mut queue, 1, 1, 4));
        assert_eq!(order(&queue), "acdbe");
        assert!(reorder(&mut queue, 1, 4, 1));
        assert_eq!(order(&queue), "aecdb");
        assert!(reorder(&mut queue, 1, 1, 5));
        assert_eq!(order(&queue), "acdbe");
        for (from, gap) in [(2, 2), (2, 3), (0, 3), (3, 0), (9, 2), (2, 9)] {
            assert!(!reorder(&mut queue, 1, from, gap));
        }
        assert_eq!(order(&queue), "acdbe");
    }
}
