use crate::art::Art;
use crate::auth;
use crate::discord::{Discord, Headline};
use crate::media::Media;
use crate::player::Player;
use crate::ui;
use crate::update::{self, Plan};
use crate::ytm::{Account, Client, Item, Lyrics, Page, QUICK_PICKS, Section, VIDEO_FRAME_HOST};
use crate::{APP_NAME, VERSION};
use eframe::egui::{self, Key, Modifiers, PointerButton};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const RECENT_LIMIT: usize = 100;
const CACHED_PAGES: usize = 24;
const PAGES_KEPT_ON_TRIM: usize = 8;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(350);
const RESTART_AFTER_MS: u64 = 3000;
const NOTICE_TIME: Duration = Duration::from_secs(6);
const UPDATE_NOTICE_TIME: Duration = Duration::from_secs(20);
const LATEST_RELEASE: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases/latest");
const PLAYED_KEPT: usize = 50;
const PROGRESS_TICK: Duration = Duration::from_millis(250);
const RADIO_MARGIN: usize = 2;
const HOME_SEEDS: usize = 3;
const COVER_LOOKUPS: usize = 2;
const QUEUED_COVERS: usize = 30;
const REMEMBERED_COVERS: usize = 1500;
const WINDOW_KEY: &str = "window";
pub const WINDOW_SIZE: [f32; 2] = [1360.0, 860.0];
pub const WINDOW_MIN: [f32; 2] = [940.0, 600.0];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Route {
    Home,
    Search(String),
    Browse(String),
    Liked,
    Recent,
}

pub enum Load {
    Loading,
    Ready(Box<Page>),
    Failed(String),
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub volume: f32,
    pub liked: Vec<Item>,
    pub signed_out_liked: Option<Vec<Item>>,
    pub recent: Vec<Item>,
    pub pinned: Vec<Item>,
    pub visitor: String,
    pub queue_open: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub account: Option<Account>,
    pub covers: Covers,
    pub queue: Vec<Item>,
    pub current: Option<usize>,
    pub position_ms: u64,
    pub lyrics_open: bool,
    pub discord: bool,
    pub discord_headline: Headline,
    pub discord_name: String,
}

pub enum Words {
    Loading,
    Ready(Lyrics),
    Missing,
    Failed,
}

pub struct Notice {
    pub text: String,
    pub action: Option<(&'static str, Action)>,
    until: Instant,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Covers {
    known: HashMap<String, String>,
    #[serde(skip)]
    wanted: RefCell<Vec<String>>,
}

impl Covers {
    pub fn of<'a>(&'a self, item: &'a Item) -> &'a str {
        if item.video_id.is_empty() || !item.thumb.contains(VIDEO_FRAME_HOST) {
            return &item.thumb;
        }
        match self.known.get(&item.video_id) {
            Some(cover) if !cover.is_empty() => cover,
            Some(_) => &item.thumb,
            None => {
                self.wanted.borrow_mut().push(item.video_id.clone());
                &item.thumb
            }
        }
    }
}

#[derive(Default)]
pub struct SignIn {
    pub cookie: String,
    pub busy: bool,
    pub status: String,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            volume: 0.8,
            liked: Vec::new(),
            signed_out_liked: None,
            recent: Vec::new(),
            pinned: Vec::new(),
            visitor: String::new(),
            queue_open: true,
            shuffle: false,
            repeat: Repeat::Off,
            account: None,
            covers: Covers::default(),
            queue: Vec::new(),
            current: None,
            position_ms: 0,
            lyrics_open: false,
            discord: false,
            discord_headline: Headline::App,
            discord_name: String::new(),
        }
    }
}

impl Saved {
    fn leave_account(&mut self) {
        self.account = None;
        if let Some(liked) = self.signed_out_liked.take() {
            self.liked = liked;
        }
    }
}

#[derive(Clone)]
pub enum Action {
    Go(Route),
    Back,
    Forward,
    Reload,
    Play(Vec<Item>, usize),
    Jump(usize),
    PlayNext(Item),
    Enqueue(Item),
    Remove(usize),
    Toggle,
    Next,
    Previous,
    Seek(u64),
    Volume(f32),
    Like(Item),
    Pin(Item),
    ToggleQueue,
    ToggleShuffle,
    CycleRepeat,
    OpenSignIn,
    CloseSignIn,
    BrowserSignIn,
    CookieSignIn,
    SignOut,
    Undo,
    GetUpdate,
    OpenRelease,
    #[cfg_attr(not(any(windows, unix)), allow(dead_code))]
    Pause(bool),
    Move(usize, usize),
    Expand(bool),
    ShowLyrics(bool),
    ClearHistory,
    RestoreHistory(Vec<Item>),
    ShowOnDiscord(bool),
    DiscordHeadline(Headline),
    DiscordName(String),
}

enum Event {
    Page(Route, Result<Box<Page>, String>),
    Part(u64, usize, Vec<Section>),
    Radio(String, Vec<Item>),
    SignedIn(Result<(Account, String), String>),
    Library(Vec<Item>),
    Liked(Vec<Item>),
    LikeRejected(Item, String),
    Cover(String, Option<String>),
    Release(String),
    Updated(Result<PathBuf, String>),
    Pressed(Action),
    Lyrics(String, Result<Option<Lyrics>, String>),
    Discord(bool),
}

pub struct App {
    ctx: egui::Context,
    yt: Arc<Client>,
    events: Receiver<Event>,
    sender: Sender<Event>,
    radio_seed: String,
    autoplay: Option<String>,
    home_parts: Vec<(usize, Vec<Section>)>,
    home_load: u64,
    geometry: Option<String>,
    cancel_sign_in: Arc<AtomicBool>,
    cover_queue: Vec<String>,
    cover_tried: HashSet<String>,
    cover_lookups: usize,
    media: Option<Media>,
    discord: Option<Discord>,
    replaced: Option<(Vec<Item>, Option<usize>, u64)>,
    curated: bool,
    resume_at: Option<u64>,
    pub sign_in: Option<SignIn>,
    pub library: Vec<Item>,
    pub player: Player,
    pub art: Art,
    pub history: Vec<Route>,
    pub at: usize,
    pub pages: HashMap<Route, Load>,
    pub query: String,
    pub typed: Option<Instant>,
    pub focus_search: bool,
    pub expanded: bool,
    pub lyrics: Option<(String, Words)>,
    pub title_bar: ui::TitleBar,
    pub saved: Saved,
    pub notice: Option<Notice>,
    pub update: Option<String>,
    pub updating: bool,
    pub discord_linked: Option<bool>,
    restart: Option<PathBuf>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, query: Option<String>, autoplay: bool) -> Self {
        ui::install(&cc.egui_ctx);
        let mut saved: Saved = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, eframe::APP_KEY))
            .unwrap_or_default();
        let yt = Arc::new(Client::new(saved.visitor.clone()));
        yt.set_session(auth::load());
        if !yt.signed_in() {
            saved.leave_account();
        }
        saved.current = saved.current.filter(|current| *current < saved.queue.len());
        let ctx = cc.egui_ctx.clone();
        let player = Player::new(yt.clone(), {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        });
        player.set_volume(saved.volume);
        let (sender, events) = mpsc::channel();
        let media = Media::new(cc, {
            let (sender, ctx) = (sender.clone(), ctx.clone());
            move |action| {
                let _ = sender.send(Event::Pressed(action));
                ctx.request_repaint();
            }
        });
        let discord = saved.discord.then(|| link_discord(&sender, &ctx));
        let mut app = Self {
            art: Art::new(ctx.clone(), yt.agent.clone()),
            ctx,
            yt,
            events,
            sender,
            radio_seed: String::new(),
            autoplay: query.clone().filter(|_| autoplay),
            home_parts: Vec::new(),
            home_load: 0,
            geometry: cc.storage.and_then(|storage| storage.get_string(WINDOW_KEY)),
            cancel_sign_in: Arc::default(),
            cover_queue: Vec::new(),
            cover_tried: HashSet::new(),
            cover_lookups: 0,
            media,
            discord,
            replaced: None,
            curated: !saved.queue.is_empty(),
            resume_at: saved.current.map(|_| saved.position_ms),
            sign_in: None,
            library: Vec::new(),
            player,
            history: vec![Route::Home],
            at: 0,
            pages: HashMap::new(),
            query: String::new(),
            typed: None,
            focus_search: false,
            expanded: false,
            lyrics: None,
            title_bar: ui::TitleBar::new(cc),
            saved,
            notice: None,
            update: None,
            updating: false,
            discord_linked: None,
            restart: None,
        };
        match query {
            Some(query) => {
                app.query = query.clone();
                app.go(Route::Search(query));
            }
            None => app.ensure(&Route::Home),
        }
        app.load_library();
        app.background(|yt| {
            yt.refresh_clients();
            let latest = yt.agent.head(LATEST_RELEASE).call().ok()?;
            newer_version(latest.get_url(), VERSION).map(Event::Release)
        });
        app
    }

    fn notify(&mut self, text: impl Into<String>) {
        self.offer(text, None, NOTICE_TIME);
    }

    fn offer(&mut self, text: impl Into<String>, action: Option<(&'static str, Action)>, lasts: Duration) {
        self.replaced = None;
        self.notice = Some(Notice {
            text: text.into(),
            action,
            until: Instant::now() + lasts,
        });
    }

    fn background(&self, work: impl FnOnce(&Client) -> Option<Event> + Send + 'static) {
        let (yt, sender, ctx) = (self.yt.clone(), self.sender.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            if let Some(event) = work(&yt) {
                let _ = sender.send(event);
                ctx.request_repaint();
            }
        });
    }

    fn look_up_lyrics(&mut self) {
        if !self.expanded || !self.saved.lyrics_open {
            return;
        }
        let Some(video_id) = self.now_playing().map(|item| item.video_id.clone()) else {
            return;
        };
        if self.lyrics.as_ref().is_some_and(|(known, _)| *known == video_id) {
            return;
        }
        self.lyrics = Some((video_id.clone(), Words::Loading));
        self.background(move |yt| {
            let found = yt.lyrics(&video_id);
            Some(Event::Lyrics(video_id, found))
        });
    }

    fn look_up_covers(&mut self) {
        for video_id in self.saved.covers.wanted.take() {
            if self.cover_tried.insert(video_id.clone()) {
                self.cover_queue.push(video_id);
            }
        }
        if self.cover_queue.len() > QUEUED_COVERS {
            let skipped = self.cover_queue.remove(0);
            self.cover_tried.remove(&skipped);
        }
        while self.cover_lookups < COVER_LOOKUPS {
            let Some(video_id) = self.cover_queue.pop() else { break };
            self.cover_lookups += 1;
            self.background(move |yt| {
                let cover = yt.cover(&video_id).ok();
                Some(Event::Cover(video_id, cover))
            });
        }
    }

    fn load_library(&self) {
        if self.yt.signed_in() {
            self.background(|yt| yt.library().ok().map(Event::Library));
            self.background(|yt| yt.liked().ok().map(Event::Liked));
        }
    }

    fn refresh(&mut self) {
        self.pages.clear();
        self.library.clear();
        self.ensure(&self.route().clone());
        self.load_library();
    }

    fn start_sign_in(&mut self, status: &str, cookie: impl FnOnce() -> Result<String, String> + Send + 'static) {
        if let Some(sign_in) = &mut self.sign_in {
            sign_in.busy = true;
            sign_in.status = status.to_owned();
        }
        self.background(move |yt| {
            let signed = cookie().and_then(|cookie| Ok((yt.sign_in(cookie.clone())?, cookie)));
            Some(Event::SignedIn(signed))
        });
    }

    fn sign_out(&mut self) {
        self.yt.set_session(None);
        auth::forget();
        self.saved.leave_account();
        self.refresh();
    }

    fn home_page(&self, partial: bool) -> Page {
        let mut sections: Vec<Section> = self.home_parts.iter().flat_map(|(_, sections)| sections.iter().cloned()).collect();
        if let Some(at) = sections.iter().position(|section| section.title == QUICK_PICKS) {
            sections[..=at].rotate_right(1);
        }
        Page {
            header: None,
            sections,
            partial,
        }
    }

    pub fn route(&self) -> &Route {
        &self.history[self.at]
    }

    pub fn now_playing(&self) -> Option<&Item> {
        self.saved.queue.get(self.saved.current?)
    }

    pub fn playing(&self) -> bool {
        self.saved.current.is_some() && !self.player.paused()
    }

    fn ensure(&mut self, route: &Route) {
        if self.pages.contains_key(route) || matches!(route, Route::Liked | Route::Recent) {
            return;
        }
        if self.pages.len() >= CACHED_PAGES {
            let kept = &self.history[self.history.len().saturating_sub(PAGES_KEPT_ON_TRIM)..];
            self.pages.retain(|cached, _| kept.contains(cached));
        }
        self.pages.insert(route.clone(), Load::Loading);
        let mut seeds: Vec<Item> = Vec::new();
        let wanted = if self.yt.signed_in() { 0 } else { HOME_SEEDS };
        for item in self.saved.recent.iter().chain(&self.saved.liked) {
            if seeds.len() < wanted && !seeds.iter().any(|seed| seed.subtitle == item.subtitle) {
                seeds.push(item.clone());
            }
        }
        if *route == Route::Home {
            self.home_load += 1;
            self.home_parts.clear();
        }
        let load = self.home_load;
        let (yt, sender, ctx, route) = (self.yt.clone(), self.sender.clone(), self.ctx.clone(), route.clone());
        std::thread::spawn(move || {
            let emit = |order: usize, sections: Vec<Section>| {
                let _ = sender.send(Event::Part(load, order, sections));
                ctx.request_repaint();
            };
            let result = match &route {
                Route::Home => yt.home(&seeds, &emit).map(|()| Page::default()),
                Route::Search(query) => yt.search(query),
                Route::Browse(id) => yt.browse(id),
                Route::Liked | Route::Recent => return,
            };
            let _ = sender.send(Event::Page(route, result.map(Box::new)));
            ctx.request_repaint();
        });
    }

    fn go(&mut self, route: Route) {
        if *self.route() == route {
            return;
        }
        if matches!((self.route(), &route), (Route::Search(_), Route::Search(_))) {
            self.history[self.at] = route;
        } else {
            self.history.truncate(self.at + 1);
            self.history.push(route);
            self.at += 1;
        }
        self.ensure(&self.route().clone());
    }

    fn step(&mut self, at: usize) {
        self.at = at;
        let route = self.route().clone();
        if let Route::Search(query) = &route {
            self.query = query.clone();
        }
        self.ensure(&route);
    }

    fn play(&mut self, items: Vec<Item>, index: usize) {
        let chosen = items.get(index).map(|item| item.video_id.clone()).unwrap_or_default();
        let songs: Vec<Item> = items.into_iter().filter(Item::is_song).collect();
        let before = std::mem::replace(&mut self.saved.queue, songs);
        let upcoming = self.saved.current.map_or(0, |current| current + 1);
        let same_list = before.len() == self.saved.queue.len()
            && before
                .iter()
                .all(|old| self.saved.queue.iter().any(|new| new.video_id == old.video_id));
        if self.curated && upcoming < before.len() && !same_list {
            let position = self.resume_at.unwrap_or_else(|| self.player.position_ms());
            self.offer("Queue replaced", Some(("Undo", Action::Undo)), NOTICE_TIME);
            self.replaced = Some((before, self.saved.current, position));
        }
        self.curated = self.saved.queue.len() > 1;
        let mut at = self.saved.queue.iter().position(|item| item.video_id == chosen).unwrap_or(0);
        if self.saved.shuffle && !self.saved.queue.is_empty() {
            self.saved.queue.swap(0, at);
            at = 0;
            shuffle(&mut self.saved.queue[1..]);
        }
        self.radio_seed.clear();
        self.play_index(at);
    }

    fn play_index(&mut self, index: usize) {
        if index >= self.saved.queue.len() {
            return;
        }
        let played = if self.saved.repeat == Repeat::All {
            0
        } else {
            index.saturating_sub(PLAYED_KEPT)
        };
        self.saved.queue.drain(..played);
        let index = index - played;
        let item = self.saved.queue[index].clone();
        self.resume_at = None;
        self.saved.current = Some(index);
        self.player.load(&item.video_id);
        if let Some(next) = self.saved.queue.get(index + 1) {
            self.player.preload(&next.video_id);
        }
        if index + RADIO_MARGIN >= self.saved.queue.len() && self.radio_seed != item.video_id {
            self.radio_seed = item.video_id.clone();
            let (yt, sender, ctx, seed) = (self.yt.clone(), self.sender.clone(), self.ctx.clone(), item.video_id.clone());
            std::thread::spawn(move || {
                if let Ok(items) = yt.radio(&seed) {
                    let _ = sender.send(Event::Radio(seed, items));
                    ctx.request_repaint();
                }
            });
        }
        self.saved.recent.retain(|recent| recent.video_id != item.video_id);
        self.saved.recent.insert(0, item);
        self.saved.recent.truncate(RECENT_LIMIT);
    }

    fn next(&mut self, automatic: bool) {
        let Some(current) = self.saved.current else { return };
        if current + 1 < self.saved.queue.len() {
            self.play_index(current + 1);
        } else if self.saved.repeat == Repeat::All {
            self.play_index(0);
        } else if automatic {
            self.player.seek(0);
            self.player.set_paused(true);
        }
    }

    fn previous(&mut self) {
        match self.saved.current {
            Some(current) if current > 0 && self.player.position_ms() < RESTART_AFTER_MS => self.play_index(current - 1),
            Some(_) => self.player.seek(0),
            None => {}
        }
    }

    fn drain(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Part(load, order, sections) if load == self.home_load => {
                    self.home_parts.push((order, sections));
                    self.home_parts.sort_by_key(|(order, _)| *order);
                    self.pages.insert(Route::Home, Load::Ready(Box::new(self.home_page(true))));
                }
                Event::Part(..) => {}
                Event::Page(Route::Home, result) => {
                    let page = self.home_page(false);
                    let load = match result {
                        Err(message) if page.sections.is_empty() => Load::Failed(message),
                        _ => Load::Ready(Box::new(page)),
                    };
                    self.pages.insert(Route::Home, load);
                }
                Event::Page(route, Ok(page)) => {
                    let wanted = matches!(&route, Route::Search(query) if self.autoplay.as_ref() == Some(query));
                    let first = page.songs().into_iter().next().filter(|_| wanted);
                    self.pages.insert(route, Load::Ready(page));
                    if let Some(song) = first {
                        self.autoplay = None;
                        self.play(vec![song], 0);
                    }
                }
                Event::Page(route, Err(message)) => drop(self.pages.insert(route, Load::Failed(message))),
                Event::Radio(seed, items) if seed == self.radio_seed => {
                    let upcoming = self.saved.current.map_or(0, |current| current + 1);
                    let had_next = upcoming < self.saved.queue.len();
                    for item in items {
                        if !self.saved.queue.iter().any(|queued| queued.video_id == item.video_id) {
                            self.saved.queue.push(item);
                        }
                    }
                    if let (false, Some(next)) = (had_next, self.saved.queue.get(upcoming)) {
                        self.player.preload(&next.video_id);
                    }
                }
                Event::Radio(..) => {}
                Event::Library(items) => self.library = items,
                Event::Liked(items) if self.yt.signed_in() => {
                    let shown = std::mem::replace(&mut self.saved.liked, items);
                    self.saved.signed_out_liked.get_or_insert(shown);
                }
                Event::Liked(_) => {}
                Event::LikeRejected(item, message) => {
                    if self.yt.signed_in() {
                        toggle(&mut self.saved.liked, item);
                    }
                    self.notify(message);
                }
                Event::Cover(video_id, cover) => {
                    self.cover_lookups -= 1;
                    if self.saved.covers.known.len() >= REMEMBERED_COVERS {
                        self.saved.covers.known.clear();
                    }
                    if let Some(cover) = cover {
                        self.saved.covers.known.insert(video_id, cover);
                    }
                }
                Event::SignedIn(Ok((account, cookie))) if self.sign_in.is_some() => {
                    if let Err(message) = auth::store(&cookie) {
                        self.notify(message);
                    }
                    self.saved.account = Some(account);
                    self.sign_in = None;
                    self.refresh();
                }
                Event::SignedIn(Ok(_)) => self.yt.set_session(None),
                Event::SignedIn(Err(message)) => {
                    if let Some(sign_in) = &mut self.sign_in {
                        sign_in.busy = false;
                        sign_in.status = message;
                    }
                }
                Event::Pressed(action) => self.apply(action),
                Event::Discord(linked) => self.discord_linked = Some(linked),
                Event::Lyrics(video_id, found) => {
                    if let Some((_, words)) = self.lyrics.as_mut().filter(|(wanted, _)| *wanted == video_id) {
                        *words = match found {
                            Ok(Some(lyrics)) => Words::Ready(lyrics),
                            Ok(None) => Words::Missing,
                            Err(_) => Words::Failed,
                        };
                    }
                }
                Event::Release(version) => {
                    self.offer(
                        format!("{APP_NAME} {version} is out"),
                        Some(("Update", Action::GetUpdate)),
                        UPDATE_NOTICE_TIME,
                    );
                    self.update = Some(version);
                }
                Event::Updated(Ok(program)) => {
                    self.restart = Some(program);
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Event::Updated(Err(reason)) => {
                    self.updating = false;
                    let page = Some(("Download page", Action::OpenRelease));
                    self.offer(format!("The update did not install: {reason}"), page, UPDATE_NOTICE_TIME);
                }
            }
        }
        if self.yt.take_expired() {
            auth::forget();
            self.saved.leave_account();
            self.library.clear();
            self.notify("Your YouTube session ended. Sign in again.");
        }
        if let Some(message) = self.player.take_error() {
            self.player.set_paused(true);
            self.notify(message);
        }
        if self.player.take_ended() {
            if self.saved.repeat == Repeat::One {
                self.player.seek(0);
            } else {
                self.next(true);
            }
        }
        if let Some(typed) = self.typed {
            if typed.elapsed() >= SEARCH_DEBOUNCE {
                self.typed = None;
                let query = self.query.trim().to_owned();
                if !query.is_empty() {
                    self.go(Route::Search(query));
                }
            } else {
                ctx.request_repaint_after(SEARCH_DEBOUNCE - typed.elapsed());
            }
        }
        if let Some(notice) = &self.notice {
            let left = notice.until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.notice = None;
                self.replaced = None;
            } else {
                ctx.request_repaint_after(left);
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) -> Vec<Action> {
        let mut out = Vec::new();
        let free = !ctx.wants_keyboard_input() && ctx.memory(|memory| memory.focused().is_none());
        ctx.input_mut(|input| {
            if input.consume_key(Modifiers::COMMAND, Key::K) || (free && input.consume_key(Modifiers::NONE, Key::Slash)) {
                self.focus_search = true;
            }
            if input.consume_key(Modifiers::ALT, Key::ArrowLeft) || input.pointer.button_pressed(PointerButton::Extra1) {
                out.push(Action::Back);
            }
            if input.consume_key(Modifiers::ALT, Key::ArrowRight) || input.pointer.button_pressed(PointerButton::Extra2) {
                out.push(Action::Forward);
            }
            if input.consume_key(Modifiers::COMMAND, Key::R) || input.consume_key(Modifiers::NONE, Key::F5) {
                out.push(Action::Reload);
            }
            if input.consume_key(Modifiers::COMMAND, Key::ArrowRight) {
                out.push(Action::Next);
            }
            if input.consume_key(Modifiers::COMMAND, Key::ArrowLeft) {
                out.push(Action::Previous);
            }
            if free && input.consume_key(Modifiers::NONE, Key::Space) {
                out.push(Action::Toggle);
            }
            if free && input.consume_key(Modifiers::COMMAND, Key::Z) {
                out.push(Action::Undo);
            }
            if self.expanded && free && input.consume_key(Modifiers::NONE, Key::Escape) {
                out.push(Action::Expand(false));
            }
        });
        out
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Go(route) => {
                self.expanded = false;
                self.go(route);
            }
            Action::Back if self.expanded => self.expanded = false,
            Action::Back if self.at > 0 => self.step(self.at - 1),
            Action::Forward if self.at + 1 < self.history.len() => self.step(self.at + 1),
            Action::Back | Action::Forward => {}
            Action::Reload => {
                let route = self.route().clone();
                self.pages.remove(&route);
                self.ensure(&route);
            }
            Action::Play(items, index) => self.play(items, index),
            Action::Jump(index) => self.play_index(index),
            Action::PlayNext(item) => {
                let at = self.saved.current.map_or(0, |current| current + 1).min(self.saved.queue.len());
                self.curated = true;
                self.saved.queue.insert(at, item);
                if self.saved.current.is_none() {
                    self.play_index(0);
                }
            }
            Action::Enqueue(item) => {
                self.curated = true;
                self.saved.queue.push(item);
                if self.saved.current.is_none() {
                    self.play_index(0);
                }
            }
            Action::Remove(index) => {
                if index < self.saved.queue.len() && self.saved.current != Some(index) {
                    self.saved.queue.remove(index);
                    if let Some(current) = self.saved.current.as_mut().filter(|current| index < **current) {
                        *current -= 1;
                    }
                }
            }
            Action::Toggle => match (self.saved.current, self.resume_at) {
                (Some(current), Some(position)) => {
                    self.play_index(current);
                    self.player.seek(position);
                }
                (Some(_), None) => self.player.set_paused(!self.player.paused()),
                (None, _) => {}
            },
            Action::Next => self.next(false),
            Action::Previous => self.previous(),
            Action::Seek(ms) => self.player.seek(ms),
            Action::Volume(volume) => {
                self.saved.volume = volume.clamp(0.0, 1.0);
                self.player.set_volume(self.saved.volume);
            }
            Action::Like(item) => {
                let liked = !self.saved.liked.iter().any(|known| known.key() == item.key());
                toggle(&mut self.saved.liked, item.clone());
                if self.yt.signed_in() && item.is_song() {
                    self.background(move |yt| {
                        let problem = yt.rate(&item.video_id, liked).err()?;
                        Some(Event::LikeRejected(item, format!("Like not saved. {problem}")))
                    });
                }
            }
            Action::Pin(item) => toggle(&mut self.saved.pinned, item),
            Action::ToggleQueue => self.saved.queue_open ^= true,
            Action::ToggleShuffle => {
                self.saved.shuffle ^= true;
                let upcoming = self.saved.current.map_or(0, |current| current + 1).min(self.saved.queue.len());
                if self.saved.shuffle {
                    shuffle(&mut self.saved.queue[upcoming..]);
                }
            }
            Action::CycleRepeat => {
                self.saved.repeat = match self.saved.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                }
            }
            Action::OpenSignIn => self.sign_in = Some(SignIn::default()),
            Action::CloseSignIn => {
                self.cancel_sign_in.store(true, Relaxed);
                self.sign_in = None;
            }
            Action::BrowserSignIn => {
                self.cancel_sign_in.store(true, Relaxed);
                self.cancel_sign_in = Arc::default();
                let cancel = self.cancel_sign_in.clone();
                self.start_sign_in("Finish signing in in the browser window. It closes by itself.", move || {
                    auth::browser_sign_in(&cancel)
                });
            }
            Action::CookieSignIn => {
                let pasted = self.sign_in.as_ref().map(|sign_in| sign_in.cookie.clone()).unwrap_or_default();
                self.start_sign_in("Checking with YouTube.", move || auth::clean_cookie(&pasted));
            }
            Action::SignOut => self.sign_out(),
            Action::Undo => {
                if let Some((queue, current, position)) = self.replaced.take() {
                    self.saved.queue = queue;
                    self.curated = true;
                    self.notice = None;
                    if let Some(current) = current {
                        self.play_index(current);
                        self.player.seek(position);
                    }
                }
            }
            Action::GetUpdate => {
                let Some(version) = self.update.clone().filter(|_| !self.updating) else {
                    return;
                };
                match update::plan() {
                    Plan::InPlace => {
                        self.updating = true;
                        self.offer(
                            format!("Downloading {APP_NAME} {version}. It restarts by itself."),
                            None,
                            UPDATE_NOTICE_TIME,
                        );
                        self.background(move |yt| Some(Event::Updated(update::install(&yt.agent, &version))));
                    }
                    Plan::PackageManager(command) => {
                        self.offer(
                            format!("{APP_NAME} {version} is out. Update it with: {command}"),
                            None,
                            UPDATE_NOTICE_TIME,
                        );
                    }
                    Plan::DownloadPage => self.apply(Action::OpenRelease),
                }
            }
            Action::OpenRelease => {
                self.notice = None;
                self.ctx.open_url(egui::OpenUrl::new_tab(LATEST_RELEASE));
            }
            Action::Pause(paused) => {
                if paused == self.playing() {
                    self.apply(Action::Toggle);
                }
            }
            Action::Move(from, gap) => {
                let upcoming = self.saved.current.map_or(0, |current| current + 1);
                if reorder(&mut self.saved.queue, upcoming, from, gap) {
                    self.curated = true;
                    if from == upcoming || gap == upcoming {
                        self.player.preload(&self.saved.queue[upcoming].video_id);
                    }
                }
            }
            Action::Expand(open) => self.expanded = open && self.saved.current.is_some(),
            Action::ShowLyrics(open) => {
                self.saved.lyrics_open = open;
                if matches!(self.lyrics, Some((_, Words::Failed))) {
                    self.lyrics = None;
                }
            }
            Action::ClearHistory => {
                let undo = Action::RestoreHistory(std::mem::take(&mut self.saved.recent));
                self.offer("History cleared", Some(("Undo", undo)), NOTICE_TIME);
            }
            Action::RestoreHistory(recent) => {
                self.saved.recent = recent;
                self.notice = None;
            }
            Action::ShowOnDiscord(shown) => {
                self.saved.discord = shown;
                self.discord_linked = None;
                self.discord = shown.then(|| link_discord(&self.sender, &self.ctx));
            }
            Action::DiscordHeadline(headline) => self.saved.discord_headline = headline,
            Action::DiscordName(name) => {
                self.saved.discord_name = name;
                self.saved.discord_headline = Headline::Custom;
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let (size, minimized) = ctx.input(|input| (input.content_rect().size(), input.viewport().minimized == Some(true)));
        if !minimized && (size.x < WINDOW_MIN[0] - 1.0 || size.y < WINDOW_MIN[1] - 1.0) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(WINDOW_SIZE.into()));
        }
        self.art.begin_frame(ctx);
        self.drain(ctx);
        let mut actions = self.shortcuts(ctx);
        actions.extend(ui::draw(self, ctx));
        for action in actions {
            self.apply(action);
        }
        self.look_up_covers();
        self.look_up_lyrics();
        if self.playing() || self.player.loading() {
            ctx.request_repaint_after(PROGRESS_TICK);
        }
        let item = self.saved.current.and_then(|current| self.saved.queue.get(current));
        let playing = item.is_some() && !self.player.paused();
        let (position_ms, duration_ms) = (self.player.position_ms(), self.player.duration_ms());
        if let Some(media) = &mut self.media {
            media.show(item, playing, position_ms, duration_ms);
        }
        if let Some(discord) = self.discord.as_mut().filter(|_| !self.player.loading()) {
            let track = item.filter(|_| playing).map(|item| (item, self.saved.covers.of(item)));
            let (headline, name) = (self.saved.discord_headline, &self.saved.discord_name);
            discord.show(track, headline, name, position_ms, duration_ms);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(program) = &self.restart {
            update::restart(program, &crate::profile_arguments());
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let minimized = self.ctx.input(|input| input.viewport().minimized == Some(true));
        match (&self.geometry, minimized) {
            (Some(geometry), true) => storage.set_string(WINDOW_KEY, geometry.clone()),
            (_, true) => {}
            (_, false) => self.geometry = storage.get_string(WINDOW_KEY),
        }
        self.saved.visitor = self.yt.visitor();
        self.saved.position_ms = self.resume_at.unwrap_or_else(|| self.player.position_ms());
        eframe::set_value(storage, eframe::APP_KEY, &self.saved);
    }
}

fn link_discord(sender: &Sender<Event>, ctx: &egui::Context) -> Discord {
    let (sender, ctx) = (sender.clone(), ctx.clone());
    Discord::start(move |linked| {
        let _ = sender.send(Event::Discord(linked));
        ctx.request_repaint();
    })
}

fn newer_version(release_url: &str, current: &str) -> Option<String> {
    let numbers = |version: &str| version.split('.').map(|part| part.parse().ok()).collect::<Option<Vec<u32>>>();
    let latest = release_url.rsplit_once("/tag/v")?.1;
    (numbers(latest)? > numbers(current)?).then(|| latest.to_owned())
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

fn toggle(list: &mut Vec<Item>, item: Item) {
    match list.iter().position(|existing| existing.key() == item.key()) {
        Some(at) => drop(list.remove(at)),
        None => list.insert(0, item),
    }
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

    #[test]
    fn offers_only_releases_newer_than_this_build() {
        let release = |tag: &str| format!("https://github.com/yigitbozyaka/tubefast/releases/tag/{tag}");
        assert_eq!(newer_version(&release("v0.2.0"), "0.1.9").as_deref(), Some("0.2.0"));
        assert_eq!(newer_version(&release("v0.10.0"), "0.9.1").as_deref(), Some("0.10.0"));
        assert_eq!(newer_version(&release("v0.1.0"), "0.1.0"), None);
        assert_eq!(newer_version(&release("v0.1.0"), "0.2.0"), None);
        assert_eq!(newer_version(&release("nightly"), "0.1.0"), None);
        assert_eq!(newer_version("https://github.com/yigitbozyaka/tubefast/releases", "0.1.0"), None);
    }

    #[test]
    fn reorders_only_the_upcoming_part_of_the_queue() {
        let order = |queue: &[Item]| queue.iter().map(|item| item.video_id.as_str()).collect::<String>();
        let mut queue: Vec<Item> = "abcde"
            .chars()
            .map(|id| Item {
                video_id: id.to_string(),
                ..Item::default()
            })
            .collect();
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
