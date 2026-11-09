use crate::art::Art;
use crate::auth;
use crate::discord::{Discord, Headline};
use crate::hotkeys::{self, Combo, Command, Hotkeys};
use crate::media::Media;
use crate::playback::{self, Playback, Replaced, Signal};
use crate::player::{Player, Speaker, Status};
use crate::tray::{self, Tray};
use crate::ui;
use crate::update::{self, Plan};
use crate::ytm::{Account, Client, Item, Lyrics, Page, QUICK_PICKS, Section, VIDEO_FRAME_HOST};
use crate::{APP_NAME, VERSION};
use eframe::egui::{self, Key, Modifiers, PointerButton};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RECENT_LIMIT: usize = 100;
const CACHED_PAGES: usize = 24;
const PAGES_KEPT_ON_TRIM: usize = 8;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(350);
const NOTICE_TIME: Duration = Duration::from_secs(6);
const UPDATE_NOTICE_TIME: Duration = Duration::from_secs(20);
const RELEASE_NOTES: &str = include_str!("../RELEASE_NOTES.md");
const LATEST_RELEASE: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases/latest");
const PROGRESS_TICK: Duration = Duration::from_millis(250);
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
    Settings,
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
    pub hotkeys: bool,
    pub hide_on_close: bool,
    pub hotkey_combos: [Option<Combo>; 5],
    pub notes_seen: String,
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
            hotkeys: false,
            hide_on_close: false,
            hotkey_combos: hotkeys::standard_combos(),
            notes_seen: String::new(),
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

fn share_link(item: &Item) -> String {
    if item.is_song() {
        return format!("https://music.youtube.com/watch?v={}", item.video_id);
    }
    if !item.browse_id.is_empty() {
        return format!("https://music.youtube.com/browse/{}", item.browse_id);
    }
    if !item.artist_id.is_empty() {
        return format!("https://music.youtube.com/channel/{}", item.artist_id);
    }
    if !item.album_id.is_empty() {
        return format!("https://music.youtube.com/browse/{}", item.album_id);
    }
    String::new()
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
    Share(Item),
    Pin(Item),
    ToggleQueue,
    ToggleShuffle,
    CycleRepeat,
    OpenSignIn,
    CloseSignIn,
    BrowserSignIn,
    CookieSignIn,
    SignOut,
    SwitchAccount(Account),
    Undo,
    GetUpdate,
    OpenRelease,
    CloseNotes,
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
    VolumeBy(f32),
    GlobalHotkeys(bool),
    HideOnClose(bool),
    Record(Option<Command>),
    ResetHotkeys,
}

enum Event {
    Page(Route, Result<Box<Page>, String>),
    Part(u64, usize, Vec<Section>),
    SignedIn(Result<(Account, String), String>),
    Library(Vec<Item>),
    Accounts(Vec<Account>),
    Liked(Vec<Item>),
    LikeRejected(Item, String),
    Cover(String, Option<String>),
    Release(String),
    Updated(Result<PathBuf, String>),
    Lyrics(String, Result<Option<Lyrics>, String>),
    Discord(bool),
    Pressed(Action),
}

pub struct App {
    ctx: egui::Context,
    yt: Arc<Client>,
    events: Receiver<Event>,
    sender: Sender<Event>,
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
    hotkeys: Option<Hotkeys>,
    tray: Option<Tray>,
    window: isize,
    replaced: Option<Replaced>,
    growing: Option<(Route, usize, usize)>,
    playback: Arc<Mutex<Playback>>,
    speaker: Speaker,
    synced: u64,
    pub sign_in: Option<SignIn>,
    pub library: Vec<Item>,
    pub accounts: Vec<Account>,
    pub player: Status,
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
    pub hotkey_taken: Vec<Command>,
    pub recording: Option<Command>,
    pub notes: Vec<String>,
    restart: Option<PathBuf>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, query: Option<String>, autoplay: bool, updated: bool) -> Self {
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
        yt.act_as(saved.account.as_ref().map_or("", |account| &account.page));
        saved.current = saved.current.filter(|current| *current < saved.queue.len());
        let fresh_version = saved.notes_seen != VERSION && (updated || !saved.notes_seen.is_empty());
        saved.notes_seen = VERSION.to_owned();
        let ctx = cc.egui_ctx.clone();
        let (signals, signaled) = mpsc::channel();
        let mut player = Player::new(yt.clone(), {
            let signals = signals.clone();
            move || {
                let _ = signals.send(Signal::Changed);
            }
        });
        player.set_volume(saved.volume);
        let speaker = Speaker::new(&mut player);
        let status = player.status();
        let playback = Playback::new(player, yt.clone(), signals.clone(), &saved);
        let playback = playback::run(playback, signaled, ctx.clone());
        let (sender, events) = mpsc::channel();
        let media = Media::new(cc, move |action| {
            let _ = signals.send(Signal::Pressed(Box::new(action)));
        });
        let discord = saved.discord.then(|| link_discord(&sender, &ctx));
        let mut app = Self {
            art: Art::new(ctx.clone(), yt.agent.clone()),
            ctx,
            yt,
            events,
            sender,
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
            hotkeys: None,
            tray: None,
            window: tray::window_of(cc),
            replaced: None,
            growing: None,
            playback,
            speaker,
            synced: 0,
            sign_in: None,
            library: Vec::new(),
            accounts: Vec::new(),
            player: status,
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
            hotkey_taken: Vec::new(),
            recording: None,
            notes: if fresh_version { release_notes(RELEASE_NOTES) } else { Vec::new() },
            restart: None,
        };
        match query {
            Some(query) => {
                app.query = query.clone();
                app.go(Route::Search(query));
            }
            None => app.ensure(&Route::Home),
        }
        app.restart_hotkeys();
        app.restart_tray();
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
        let Some(song) = self.now_playing() else {
            return;
        };
        if self.lyrics.as_ref().is_some_and(|(known, _)| *known == song.video_id) {
            return;
        }
        if self.player.loading() {
            self.lyrics = None;
            return;
        }
        let (song, seconds) = (song.clone(), self.player.duration_ms() / 1000);
        self.lyrics = Some((song.video_id.clone(), Words::Loading));
        self.background(move |yt| {
            let found = yt.lyrics(&song, seconds);
            Some(Event::Lyrics(song.video_id, found))
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
            self.background(|yt| yt.accounts().ok().map(Event::Accounts));
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
        self.yt.act_as("");
        self.accounts.clear();
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
        if self.pages.contains_key(route) || matches!(route, Route::Liked | Route::Recent | Route::Settings) {
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
                Route::Browse(id) => yt.browse(id, &|page| {
                    let _ = sender.send(Event::Page(route.clone(), Ok(Box::new(page.clone()))));
                    ctx.request_repaint();
                }),
                Route::Liked | Route::Recent | Route::Settings => return,
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

    fn growing_list(&self, items: &[Item]) -> Option<(Route, usize, usize)> {
        let Some(Load::Ready(page)) = self.pages.get(self.route()) else {
            return None;
        };
        let first = items.first().map(|item| &item.video_id);
        let at = page
            .sections
            .iter()
            .position(|section| section.items.first().map(|item| &item.video_id) == first)?;
        page.partial.then(|| (self.route().clone(), at, page.sections[at].items.len()))
    }

    fn grow_queue(&mut self, route: &Route, page: &Page, playback: &mut Playback) {
        let Some((from, at, offered)) = &mut self.growing else { return };
        if from != route {
            return;
        }
        let Some(section) = page.sections.get(*at) else { return };
        let fresh = section.items.iter().skip(*offered).filter(|item| item.is_song()).cloned();
        *offered = section.items.len();
        playback.extend(fresh);
        if !page.partial {
            self.growing = None;
        }
    }

    fn play(&mut self, playback: &mut Playback, items: Vec<Item>, index: usize) {
        self.growing = None;
        if let Some(replaced) = playback.play(items, index) {
            self.offer("Queue replaced", Some(("Undo", Action::Undo)), NOTICE_TIME);
            self.replaced = Some(replaced);
        }
    }

    fn sync(&mut self, playback: &mut Playback) {
        for item in playback.take_started() {
            self.saved.recent.retain(|recent| recent.video_id != item.video_id);
            self.saved.recent.insert(0, item);
        }
        self.saved.recent.truncate(RECENT_LIMIT);
        if let Some(message) = playback.take_problem() {
            self.notify(message);
        }
        if playback.version() != self.synced {
            self.synced = playback.version();
            self.saved.queue = playback.queue().to_vec();
            self.saved.current = playback.current();
            self.saved.repeat = playback.repeat();
            self.saved.shuffle = playback.shuffle();
        }
    }

    fn drain(&mut self, ctx: &egui::Context, playback: &mut Playback) {
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
                    self.grow_queue(&route, &page, playback);
                    self.pages.insert(route, Load::Ready(page));
                    if let Some(song) = first {
                        self.autoplay = None;
                        self.play(playback, vec![song], 0);
                    }
                }
                Event::Page(route, Err(message)) => drop(self.pages.insert(route, Load::Failed(message))),
                Event::Library(items) => self.library = items,
                Event::Accounts(accounts) if self.yt.signed_in() => self.accounts = accounts,
                Event::Accounts(_) => {}
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
                Event::Discord(linked) => self.discord_linked = Some(linked),
                Event::Pressed(action) => self.apply(action, playback),
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
            self.sign_out();
            self.notify("Your YouTube session ended. Sign in again.");
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

    fn restart_hotkeys(&mut self) {
        self.hotkeys = None;
        self.hotkey_taken.clear();
        if !self.saved.hotkeys || self.recording.is_some() {
            return;
        }
        let bindings = Command::ALL
            .into_iter()
            .filter_map(|command| Some((command, self.saved.hotkey_combos[command as usize]?)))
            .collect();
        let (sender, ctx) = (self.sender.clone(), self.ctx.clone());
        let press = move |command: Command| {
            let _ = sender.send(Event::Pressed(command.action()));
            ctx.request_repaint();
        };
        if let Some((hotkeys, taken)) = Hotkeys::start(bindings, press) {
            self.hotkeys = Some(hotkeys);
            self.hotkey_taken = taken;
        }
    }

    fn restart_tray(&mut self) {
        self.tray = None;
        if !self.saved.hide_on_close {
            return;
        }
        let ctx = self.ctx.clone();
        self.tray = Tray::start(self.window, move || ctx.request_repaint());
    }

    fn set_recording(&mut self, command: Option<Command>) {
        if self.recording != command {
            self.recording = command;
            self.restart_hotkeys();
        }
    }

    fn bind(&mut self, command: Command, combo: Option<Combo>) {
        for other in &mut self.saved.hotkey_combos {
            if combo.is_some() && *other == combo {
                *other = None;
            }
        }
        self.saved.hotkey_combos[command as usize] = combo;
        self.restart_hotkeys();
    }

    fn record(&mut self, ctx: &egui::Context) -> bool {
        if *self.route() != Route::Settings {
            self.set_recording(None);
        }
        let Some(command) = self.recording else {
            return false;
        };
        let pressed = ctx.input_mut(|input| {
            let pressed = input.events.iter().find_map(|event| match event {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            });
            input
                .events
                .retain(|event| !matches!(event, egui::Event::Key { .. } | egui::Event::Text(_)));
            pressed
        });
        match pressed {
            Some((Key::Escape, _)) => self.set_recording(None),
            Some((Key::Backspace | Key::Delete, _)) => {
                self.set_recording(None);
                self.bind(command, None);
            }
            Some((key, modifiers)) => {
                if let Some(combo) = Combo::from_input(key, modifiers) {
                    self.set_recording(None);
                    self.bind(command, Some(combo));
                }
            }
            None => {}
        }
        true
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

    fn apply(&mut self, action: Action, playback: &mut Playback) {
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
            Action::Play(items, index) => {
                let growing = self.growing_list(&items);
                self.play(playback, items, index);
                self.growing = growing;
            }
            Action::Jump(index) => playback.play_index(index),
            Action::PlayNext(item) => playback.play_next(item),
            Action::Enqueue(item) => playback.enqueue(item),
            Action::Remove(index) => playback.remove(index),
            Action::Toggle | Action::Next | Action::Previous | Action::Seek(_) | Action::Pause(_) => playback.press(action),
            Action::Volume(volume) => {
                self.saved.volume = volume.clamp(0.0, 1.0);
                playback.player.set_volume(self.saved.volume);
            }
            Action::VolumeBy(step) => self.apply(Action::Volume(self.saved.volume + step), playback),
            Action::GlobalHotkeys(enabled) => {
                self.saved.hotkeys = enabled;
                self.recording = None;
                self.restart_hotkeys();
            }
            Action::HideOnClose(enabled) => {
                self.saved.hide_on_close = enabled;
                self.restart_tray();
            }
            Action::Record(command) => self.set_recording(command),
            Action::ResetHotkeys => {
                self.saved.hotkey_combos = hotkeys::standard_combos();
                self.recording = None;
                self.restart_hotkeys();
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
            Action::Share(item) => {
                let link = share_link(&item);
                if link.is_empty() {
                    return;
                }
                self.ctx.copy_text(link);
                self.notify("Copied share link");
            }
            Action::Pin(item) => toggle(&mut self.saved.pinned, item),
            Action::ToggleQueue => self.saved.queue_open ^= true,
            Action::ToggleShuffle => playback.toggle_shuffle(),
            Action::CycleRepeat => playback.cycle_repeat(),
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
            Action::SwitchAccount(account) => {
                self.yt.act_as(&account.page);
                self.saved.account = Some(account);
                self.refresh();
            }
            Action::Undo => {
                if let Some(replaced) = self.replaced.take() {
                    self.notice = None;
                    playback.restore(replaced);
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
                    Plan::DownloadPage => self.apply(Action::OpenRelease, playback),
                }
            }
            Action::OpenRelease => {
                self.notice = None;
                self.ctx.open_url(egui::OpenUrl::new_tab(LATEST_RELEASE));
            }
            Action::CloseNotes => self.notes.clear(),
            Action::Move(from, gap) => playback.reorder(from, gap),
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
        if ctx.input(|input| input.viewport().close_requested()) && tray::hides_on_close(self.tray.is_some(), tray::quitting()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if let Some(tray) = &self.tray {
                tray.hide();
            }
        }
        let (size, minimized) = ctx.input(|input| (input.content_rect().size(), input.viewport().minimized == Some(true)));
        if !minimized && (size.x < WINDOW_MIN[0] - 1.0 || size.y < WINDOW_MIN[1] - 1.0) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(WINDOW_SIZE.into()));
        }
        self.art.begin_frame(ctx);
        let playback = self.playback.clone();
        let mut playback = playback.lock().unwrap();
        self.speaker.follow_default_device(&mut playback.player);
        self.sync(&mut playback);
        self.drain(ctx, &mut playback);
        let mut actions = if self.record(ctx) { Vec::new() } else { self.shortcuts(ctx) };
        actions.extend(ui::draw(self, ctx));
        for action in actions {
            self.apply(action, &mut playback);
        }
        self.sync(&mut playback);
        drop(playback);
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
        let playback = self.playback.clone();
        let mut playback = playback.lock().unwrap();
        self.sync(&mut playback);
        self.saved.position_ms = playback.position_ms();
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

fn toggle(list: &mut Vec<Item>, item: Item) {
    match list.iter().position(|existing| existing.key() == item.key()) {
        Some(at) => drop(list.remove(at)),
        None => list.insert(0, item),
    }
}

fn release_notes(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim().trim_start_matches(['-', '*']).trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_release_notes_as_plain_lines() {
        let notes = release_notes("# 0.7.2\n\n- Faster search\n* Lyrics stay in sync\n   \n");
        assert_eq!(notes, ["Faster search", "Lyrics stay in sync"]);
    }

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
}
