use crate::art::Art;
use crate::auth;
use crate::discord::{Discord, Headline};
use crate::hotkeys::{self, Combo, Command, Hotkeys};
use crate::media::Media;
use crate::playback::{self, Playback, Replaced, Signal};
use crate::player::{Player, Speaker, Status};
use crate::tray::{self, Closing, Tray};
use crate::ui;
use crate::update::{self, LATEST_RELEASE, Plan};
use crate::wake;
use crate::ytm::{Account, Client, Item, Lyrics, Page, QUICK_PICKS, Section, Target, VIDEO_FRAME_HOST};
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
const PROGRESS_TICK: Duration = Duration::from_millis(250);
const HOME_SEEDS: usize = 3;
const COVER_LOOKUPS: usize = 2;
const QUEUED_COVERS: usize = 30;
const REMEMBERED_COVERS: usize = 1500;
pub const OVERLAY_OPACITY_MIN: f32 = 0.3;
const OVERLAY_OPACITY_DEFAULT: f32 = 0.9;
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
    pub unshuffled: Vec<Item>,
    pub current: Option<usize>,
    pub position_ms: u64,
    pub lyrics_open: bool,
    pub lyrics_romanized: bool,
    pub discord: bool,
    pub discord_headline: Headline,
    pub discord_name: String,
    pub hotkeys: bool,
    pub hide_on_close: bool,
    #[serde(deserialize_with = "hotkeys::padded_combos")]
    pub hotkey_combos: [Option<Combo>; hotkeys::COUNT],
    pub overlay: bool,
    pub overlay_opacity: f32,
    pub overlay_position: Option<[f32; 2]>,
    pub overlay_controls: bool,
    pub overlay_next_up: bool,
    pub notes_seen: String,
    pub browser: String,
    pub beta_updates: bool,
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
    pub browsers: Vec<auth::Installed>,
    pub chosen: usize,
    pub picking: bool,
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
            unshuffled: Vec::new(),
            current: None,
            position_ms: 0,
            lyrics_open: false,
            lyrics_romanized: false,
            discord: false,
            discord_headline: Headline::App,
            discord_name: String::new(),
            hotkeys: false,
            hide_on_close: false,
            hotkey_combos: hotkeys::standard_combos(),
            overlay: false,
            overlay_opacity: OVERLAY_OPACITY_DEFAULT,
            overlay_position: None,
            overlay_controls: true,
            overlay_next_up: true,
            notes_seen: String::new(),
            browser: String::new(),
            beta_updates: false,
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
    ShowAll(usize),
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
    PickBrowser(usize),
    ClosePicker,
    SignOut,
    SwitchAccount(Account),
    Undo,
    GetUpdate,
    BetaUpdates(bool),
    StayOnBeta,
    LeaveBeta,
    OpenRelease,
    CloseNotes,
    #[cfg_attr(not(any(windows, unix)), allow(dead_code))]
    Pause(bool),
    Move(usize, usize),
    Expand(bool),
    ShowLyrics(bool),
    RomanizeLyrics(bool),
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
    ToggleOverlay,
    OverlayEnabled(bool),
    OverlayOpacity(f32),
    OverlayMoved([f32; 2]),
    OverlayResetPosition,
    OverlaySnapped,
    OverlayControls(bool),
    OverlayNextUp(bool),
}

enum Event {
    Page(Route, Result<Box<Page>, String>),
    Part(u64, usize, Vec<Section>),
    FullSection(Route, usize, Target, Vec<Item>),
    SignedIn(Result<(Account, String), String>),
    AccountRestored(String, Result<Account, String>),
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
    restoring_account: bool,
    cover_queue: Vec<String>,
    cover_tried: HashSet<String>,
    pub fetching_all: HashSet<(Route, usize)>,
    cover_lookups: usize,
    media: Option<Media>,
    discord: Option<Discord>,
    hotkeys: Option<Hotkeys>,
    tray: Option<Tray>,
    window: isize,
    fitted: Option<egui::Vec2>,
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
    pub beta_supported: bool,
    pub beta_choice: bool,
    pub discord_linked: Option<bool>,
    pub hotkey_taken: Vec<Command>,
    pub recording: Option<Command>,
    pub overlay_open: bool,
    pub overlay_origin: Option<[f32; 2]>,
    pub overlay_snapping: bool,
    #[cfg(all(unix, not(target_os = "macos")))]
    pub layer_overlay: std::cell::OnceCell<Option<crate::overlay_wayland::Handle>>,
    pub notes: Vec<String>,
    restart: Option<PathBuf>,
    #[cfg(all(unix, not(target_os = "macos")))]
    parking: bool,
    #[cfg(all(unix, not(target_os = "macos")))]
    fresh_window: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, query: Option<String>, autoplay: bool, updated: bool) -> Self {
        ui::install(&cc.egui_ctx);
        wake::point_at_window(cc.egui_ctx.clone());
        let mut saved = starting_state(cc);
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
        let playback = playback::run(playback, signaled);
        #[cfg(all(unix, not(target_os = "macos")))]
        tray::save_before_forced_exit({
            let playback = playback.clone();
            move || crate::parked::write_playback_of(&playback)
        });
        let (sender, events) = mpsc::channel();
        let media = Media::new(cc, move |action| {
            let _ = signals.send(Signal::Pressed(Box::new(action)));
        });
        let discord = saved.discord.then(|| link_discord(&sender));
        let mut app = Self {
            art: Art::new(wake::wake, yt.agent.clone()),
            ctx,
            yt,
            events,
            sender,
            autoplay: query.clone().filter(|_| autoplay),
            home_parts: Vec::new(),
            home_load: 0,
            geometry: cc.storage.and_then(|storage| storage.get_string(WINDOW_KEY)),
            cancel_sign_in: Arc::default(),
            restoring_account: false,
            cover_queue: Vec::new(),
            cover_tried: HashSet::new(),
            fetching_all: HashSet::new(),
            cover_lookups: 0,
            media,
            discord,
            hotkeys: None,
            tray: None,
            window: tray::window_of(cc),
            fitted: None,
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
            beta_supported: matches!(update::plan(), Plan::InPlace),
            beta_choice: false,
            discord_linked: None,
            hotkey_taken: Vec::new(),
            recording: None,
            overlay_open: false,
            overlay_origin: None,
            overlay_snapping: false,
            #[cfg(all(unix, not(target_os = "macos")))]
            layer_overlay: std::cell::OnceCell::new(),
            notes: if fresh_version { release_notes(RELEASE_NOTES) } else { Vec::new() },
            restart: None,
            #[cfg(all(unix, not(target_os = "macos")))]
            parking: false,
            #[cfg(all(unix, not(target_os = "macos")))]
            fresh_window: false,
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
        auth::secure_session_storage();
        auth::remove_leftover_sign_in_profiles();
        app.restore_account();
        app.check_for_update(true);
        app
    }

    fn check_for_update(&self, refresh_clients: bool) {
        let beta = self.saved.beta_updates && self.beta_supported;
        self.background(move |yt| {
            if refresh_clients {
                yt.refresh_clients();
            }
            update::latest(&yt.agent, beta, VERSION).map(Event::Release)
        });
    }

    fn collect_saved(&mut self) {
        self.saved.visitor = self.yt.visitor();
        let playback = self.playback.clone();
        let mut playback = playback.lock().unwrap();
        self.sync(&mut playback);
        self.saved.position_ms = playback.position_ms();
    }

    fn publish_presence(&mut self) {
        let item = self.saved.current.and_then(|current| self.saved.queue.get(current));
        let playing = item.is_some() && !self.player.paused();
        let (position_ms, duration_ms) = (self.player.position_ms(), self.player.duration_ms());
        if let Some(media) = &mut self.media {
            #[cfg(all(unix, not(target_os = "macos")))]
            media.set_volume(self.saved.volume);
            media.show(item, playing, position_ms, duration_ms);
        }
        if let Some(discord) = self.discord.as_mut().filter(|_| !self.player.loading()) {
            let track = item.filter(|_| playing).map(|item| (item, self.saved.covers.of(item)));
            let (headline, name) = (self.saved.discord_headline, &self.saved.discord_name);
            discord.show(track, headline, name, position_ms, duration_ms);
        }
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
        let (yt, sender) = (self.yt.clone(), self.sender.clone());
        std::thread::spawn(move || {
            if let Some(event) = work(&yt) {
                let _ = sender.send(event);
                wake::wake();
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

    fn restore_account(&mut self) {
        let Some(session) = self
            .yt
            .session()
            .filter(|_| self.saved.account.is_none() && !self.restoring_account)
        else {
            return;
        };
        self.restoring_account = true;
        self.background(move |yt| Some(Event::AccountRestored(session, yt.current_account())));
    }

    fn refresh(&mut self) {
        self.pages.clear();
        self.library.clear();
        self.ensure(&self.route().clone());
        self.load_library();
        self.restore_account();
    }

    fn start_sign_in(&mut self, status: &str, cookie: impl FnOnce() -> Result<String, String> + Send + 'static) {
        if let Some(sign_in) = &mut self.sign_in {
            sign_in.busy = true;
            sign_in.status = status.to_owned();
        }
        self.background(move |yt| {
            let signed = cookie().and_then(|cookie| Ok((yt.sign_in(cookie.clone())?, yt.session().unwrap_or(cookie))));
            Some(Event::SignedIn(signed))
        });
    }

    fn launch_browser_sign_in(&mut self) {
        self.cancel_sign_in.store(true, Relaxed);
        self.cancel_sign_in = Arc::default();
        let cancel = self.cancel_sign_in.clone();
        let browser = self
            .sign_in
            .as_ref()
            .and_then(|sign_in| sign_in.browsers.get(sign_in.chosen).cloned());
        self.start_sign_in("Finish signing in in the browser window. It closes by itself.", move || {
            auth::browser_sign_in(&cancel, &browser.ok_or(auth::NO_BROWSER)?)
        });
    }

    fn sign_out(&mut self) {
        auth::forget();
        self.leave_session();
    }

    fn leave_session(&mut self) {
        self.yt.set_session(None);
        self.yt.act_as("");
        self.accounts.clear();
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

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn http_agent(&self) -> ureq::Agent {
        self.yt.agent.clone()
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
        let (yt, sender, route) = (self.yt.clone(), self.sender.clone(), route.clone());
        std::thread::spawn(move || {
            let emit = |order: usize, sections: Vec<Section>| {
                let _ = sender.send(Event::Part(load, order, sections));
                wake::wake();
            };
            let result = match &route {
                Route::Home => yt.home(&seeds, &emit).map(|()| Page::default()),
                Route::Search(query) => yt.search(query),
                Route::Browse(id) => yt.browse(id, &|page| {
                    let _ = sender.send(Event::Page(route.clone(), Ok(Box::new(page.clone()))));
                    wake::wake();
                }),
                Route::Liked | Route::Recent | Route::Settings => return,
            };
            let _ = sender.send(Event::Page(route, result.map(Box::new)));
            wake::wake();
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
        self.saved.volume = playback.player.volume();
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
            self.saved.unshuffled = playback.unshuffled().to_vec();
            self.saved.current = playback.current();
            self.saved.repeat = playback.repeat();
            self.saved.shuffle = playback.shuffle();
        }
    }

    fn drain(&mut self, playback: &mut Playback) -> Option<Duration> {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Part(load, order, sections) if load == self.home_load => {
                    self.home_parts.push((order, sections));
                    self.home_parts.sort_by_key(|(order, _)| *order);
                    self.pages.insert(Route::Home, Load::Ready(Box::new(self.home_page(true))));
                }
                Event::Part(..) => {}
                Event::FullSection(route, at, target, items) => {
                    if let Some(Load::Ready(page)) = self.pages.get_mut(&route) {
                        settle_section(page, at, &target, items);
                    }
                    self.fetching_all.remove(&(route, at));
                }
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
                Event::AccountRestored(asked_with, restored) => {
                    self.restoring_account = false;
                    let same_session = self.yt.session().is_some_and(|now| auth::same_login(&now, &asked_with));
                    match restored {
                        Ok(account) if same_session && self.saved.account.is_none() => self.saved.account = Some(account),
                        Err(message) if same_session => eprintln!("Could not confirm the saved YouTube sign-in: {message}"),
                        _ => {}
                    }
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
                Event::Release(version) if update::is_beta(&version) && !self.saved.beta_updates => {}
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
                    tray::begin_quit();
                    self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Event::Updated(Err(reason)) => {
                    self.updating = false;
                    let page = Some(("Download page", Action::OpenRelease));
                    self.offer(format!("The update did not install: {reason}"), page, UPDATE_NOTICE_TIME);
                }
            }
        }
        if let Some(rejected) = self.yt.take_expired() {
            auth::forget_if_stored(&rejected);
            self.leave_session();
            self.notify("Your YouTube session ended. Sign in again.");
        }
        let mut repaint_after = None;
        if let Some(typed) = self.typed {
            if typed.elapsed() >= SEARCH_DEBOUNCE {
                self.typed = None;
                let query = self.query.trim().to_owned();
                if !query.is_empty() {
                    self.go(Route::Search(query));
                }
            } else {
                repaint_after = Some(SEARCH_DEBOUNCE - typed.elapsed());
            }
        }
        if let Some(notice) = &self.notice {
            let left = notice.until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                self.notice = None;
                self.replaced = None;
            } else {
                repaint_after = Some(repaint_after.map_or(left, |sooner| sooner.min(left)));
            }
        }
        repaint_after
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
        let sender = self.sender.clone();
        let press = move |command: Command| {
            let _ = sender.send(Event::Pressed(command.action()));
            wake::wake();
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
        self.tray = Tray::start(self.window, self.ctx.clone());
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
            Action::ShowAll(at) => {
                let route = self.route().clone();
                let target = match self.pages.get(&route) {
                    Some(Load::Ready(page)) => page.sections.get(at).and_then(|section| section.all.clone()),
                    _ => None,
                };
                if let Some(target) = target
                    && self.fetching_all.insert((route.clone(), at))
                {
                    self.background(move |yt| {
                        let items = yt.full_section(&target).unwrap_or_default();
                        Some(Event::FullSection(route, at, target, items))
                    });
                }
            }
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
            Action::ToggleOverlay => {
                self.overlay_open = overlay_after_toggle(self.saved.overlay, self.overlay_open);
                self.overlay_origin = self.saved.overlay_position;
            }
            Action::OverlayEnabled(enabled) => {
                self.saved.overlay = enabled;
                self.overlay_open = false;
            }
            Action::OverlayOpacity(opacity) => self.saved.overlay_opacity = usable_opacity(opacity),
            Action::OverlayMoved(position) => self.saved.overlay_position = Some(position),
            Action::OverlayResetPosition => {
                self.saved.overlay_position = None;
                self.overlay_origin = None;
                self.overlay_snapping = self.overlay_open;
            }
            Action::OverlaySnapped => self.overlay_snapping = false,
            Action::OverlayControls(shown) => self.saved.overlay_controls = shown,
            Action::OverlayNextUp(shown) => self.saved.overlay_next_up = shown,
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
            Action::OpenSignIn => {
                let browsers = auth::installed_browsers();
                let chosen = auth::chosen_index(&browsers, &self.saved.browser);
                self.sign_in = Some(SignIn {
                    browsers,
                    chosen,
                    ..SignIn::default()
                });
            }
            Action::PickBrowser(at) => {
                if let Some(sign_in) = &mut self.sign_in
                    && let Some(browser) = sign_in.browsers.get(at)
                {
                    sign_in.chosen = at;
                    sign_in.picking = false;
                    self.saved.browser = browser.name.to_owned();
                    self.launch_browser_sign_in();
                }
            }
            Action::ClosePicker => {
                if let Some(sign_in) = &mut self.sign_in {
                    sign_in.picking = false;
                }
            }
            Action::CloseSignIn => {
                self.cancel_sign_in.store(true, Relaxed);
                self.sign_in = None;
            }
            Action::BrowserSignIn => match &mut self.sign_in {
                Some(sign_in) if sign_in.browsers.len() > 1 => sign_in.picking = true,
                _ => self.launch_browser_sign_in(),
            },
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
            Action::BetaUpdates(enabled) => {
                self.saved.beta_updates = enabled;
                self.beta_choice = !enabled && update::is_beta(VERSION);
                if self.update.as_deref().is_some_and(update::is_beta) {
                    self.update = None;
                }
                self.check_for_update(false);
            }
            Action::StayOnBeta => self.beta_choice = false,
            Action::LeaveBeta => {
                self.beta_choice = false;
                if self.updating {
                    return;
                }
                self.updating = true;
                self.offer(
                    format!("Going back to the latest stable release. Your data is backed up first. {APP_NAME} restarts by itself."),
                    None,
                    UPDATE_NOTICE_TIME,
                );
                let live_state = serde_json::to_vec(&self.saved).unwrap_or_default();
                self.background(move |yt| Some(Event::Updated(update::downgrade_to_stable(&yt.agent, &live_state, VERSION))));
            }
            Action::OpenRelease => {
                self.notice = None;
                self.ctx.open_url(egui::OpenUrl::new_tab(LATEST_RELEASE));
            }
            Action::CloseNotes => self.notes.clear(),
            Action::Move(from, gap) => playback.reorder(from, gap),
            Action::Expand(open) => self.expanded = open && self.saved.current.is_some(),
            Action::RomanizeLyrics(romanized) => self.saved.lyrics_romanized = romanized,
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
                self.discord = shown.then(|| link_discord(&self.sender));
            }
            Action::DiscordHeadline(headline) => self.saved.discord_headline = headline,
            Action::DiscordName(name) => {
                self.saved.discord_name = name;
                self.saved.discord_headline = Headline::Custom;
            }
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
impl App {
    pub fn parking(&self) -> bool {
        self.parking
    }

    fn start_parking(&mut self) {
        self.parking = true;
        wake::point_at_parked();
    }

    fn let_go_of_the_window(&mut self) {
        self.art = Art::idle();
        self.ctx = egui::Context::default();
        self.set_recording(None);
        self.overlay_open &= hotkeys::layer_overlay();
        self.fitted = None;
    }

    pub fn attach(&mut self, cc: &eframe::CreationContext<'_>) {
        ui::install(&cc.egui_ctx);
        self.ctx = cc.egui_ctx.clone();
        wake::point_at_window(self.ctx.clone());
        self.art = Art::new(wake::wake, self.yt.agent.clone());
        self.title_bar = ui::TitleBar::new(cc);
        self.window = tray::window_of(cc);
        self.geometry = cc.storage.and_then(|storage| storage.get_string(WINDOW_KEY));
        self.parking = false;
        self.fresh_window = true;
    }

    pub fn step_unseen(&mut self) -> (bool, bool) {
        let playback = self.playback.clone();
        let mut playback = playback.lock().unwrap();
        self.speaker.follow_default_device(&mut playback.player);
        self.sync(&mut playback);
        self.drain(&mut playback);
        let layered = hotkeys::layer_overlay();
        let shown = layered && self.saved.overlay && self.overlay_open;
        let actions = if layered { ui::layer_overlay(self, shown) } else { Vec::new() };
        for action in actions {
            self.apply(action, &mut playback);
        }
        self.sync(&mut playback);
        drop(playback);
        self.look_up_covers();
        self.publish_presence();
        (self.playing() || self.player.loading(), shown)
    }

    pub fn keep_state(&mut self) {
        self.collect_saved();
        if let Some(files) = crate::parked::Files::of_profile() {
            files.write_state(&self.saved);
        }
    }

    pub fn finish_unseen(&mut self) {
        self.keep_state();
        if let Some(program) = &self.restart {
            update::restart(program, &crate::profile_arguments());
        }
    }
}

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        if cfg!(windows) {
            egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32()
        } else {
            egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        wake::frame_drawn();
        #[cfg(all(unix, not(target_os = "macos")))]
        if std::mem::take(&mut self.fresh_window) {
            tray::present_new_window(ctx);
        }
        if ctx.input(|input| input.viewport().close_requested()) {
            match tray::on_close_request(self.tray.is_some()) {
                Closing::Quit => {}
                Closing::Hide => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                    if let Some(tray) = &self.tray {
                        tray.hide();
                    }
                }
                Closing::Park => {
                    #[cfg(all(unix, not(target_os = "macos")))]
                    self.start_parking();
                }
            }
        } else if cfg!(all(unix, not(target_os = "macos"))) && tray::quitting() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let (size, minimized, monitor, resizable) = ctx.input(|input| {
            let viewport = input.viewport();
            (
                input.content_rect().size(),
                viewport.minimized == Some(true),
                viewport.monitor_size,
                viewport.maximized != Some(true) && viewport.fullscreen != Some(true),
            )
        });
        let fit = |target: [f32; 2]| match monitor {
            Some(monitor) => egui::vec2(target[0].min(monitor.x * 0.95), target[1].min(monitor.y * 0.9)),
            None => egui::vec2(target[0], target[1]),
        };
        let (wanted, floor) = (fit(WINDOW_SIZE), fit(WINDOW_MIN));
        let overflows = monitor.is_some_and(|monitor| size.x > monitor.x + 1.0 || size.y > monitor.y + 1.0);
        if !overflows {
            self.fitted = None;
        }
        if !minimized && resizable && overflows {
            if self.fitted != Some(wanted) {
                self.fitted = Some(wanted);
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(floor));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(wanted));
            }
        } else if !minimized && (size.x < floor.x - 1.0 || size.y < floor.y - 1.0) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(wanted));
        }
        self.art.begin_frame(ctx);
        let playback = self.playback.clone();
        let mut playback = playback.lock().unwrap();
        self.speaker.follow_default_device(&mut playback.player);
        self.sync(&mut playback);
        if let Some(after) = self.drain(&mut playback) {
            ctx.request_repaint_after(after);
        }
        let mut actions = if self.record(ctx) { Vec::new() } else { self.shortcuts(ctx) };
        actions.extend(ui::draw(self, ctx));
        actions.extend(ui::overlay(self, ctx));
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
        self.publish_presence();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        tray::release_activation();
        #[cfg(all(unix, not(target_os = "macos")))]
        if self.parking {
            self.let_go_of_the_window();
        }
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
        self.collect_saved();
        eframe::set_value(storage, eframe::APP_KEY, &self.saved);
        #[cfg(all(unix, not(target_os = "macos")))]
        crate::parked::forget_superseded();
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn starting_state(cc: &eframe::CreationContext<'_>) -> Saved {
    crate::parked::recover(stored_state(cc))
}

fn stored_state(cc: &eframe::CreationContext<'_>) -> Option<Saved> {
    let storage = cc.storage?;
    let state = eframe::get_value(storage, eframe::APP_KEY);
    if state.is_none()
        && let Some(raw) = storage.get_string(eframe::APP_KEY)
    {
        update::keep_unreadable_state(&raw);
    }
    state
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn starting_state(cc: &eframe::CreationContext<'_>) -> Saved {
    stored_state(cc).unwrap_or_default()
}

impl Drop for App {
    fn drop(&mut self) {
        tray::release_activation();
    }
}

fn link_discord(sender: &Sender<Event>) -> Discord {
    let sender = sender.clone();
    Discord::start(move |linked| {
        let _ = sender.send(Event::Discord(linked));
        wake::wake();
    })
}

fn overlay_after_toggle(enabled: bool, open: bool) -> bool {
    enabled && !open
}

pub fn usable_opacity(opacity: f32) -> f32 {
    if opacity.is_nan() {
        OVERLAY_OPACITY_DEFAULT
    } else {
        opacity.clamp(OVERLAY_OPACITY_MIN, 1.0)
    }
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

fn settle_section(page: &mut Page, at: usize, target: &Target, items: Vec<Item>) {
    if let Some(section) = page.sections.get_mut(at).filter(|section| section.all.as_ref() == Some(target)) {
        section.all = None;
        if !items.is_empty() {
            section.brief = Some(section.items.len());
            section.items = items;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ytm::Layout;

    #[test]
    fn full_list_replaces_only_its_own_section_and_failure_keeps_items() {
        let song = |title: &str| Item {
            title: title.into(),
            ..Item::default()
        };
        let target = Target {
            browse_id: "VLOLAK".into(),
            params: String::new(),
        };
        let section = |title: &str| Section {
            title: title.into(),
            layout: Layout::Tracks,
            items: vec![song("shown")],
            all: Some(target.clone()),
            brief: None,
        };
        let mut page = Page {
            sections: vec![section("Songs"), section("Albums")],
            ..Page::default()
        };
        let moved = Target {
            browse_id: "MPAD".into(),
            params: "x".into(),
        };
        settle_section(&mut page, 1, &moved, vec![song("a")]);
        assert_eq!(page.sections[1].items.len(), 1);
        assert!(page.sections[1].all.is_some());
        settle_section(&mut page, 1, &target, vec![song("a"), song("b")]);
        assert_eq!(page.sections[1].items.len(), 2);
        assert_eq!(page.sections[1].brief, Some(1));
        assert!(page.sections[1].all.is_none());
        assert_eq!(page.sections[0].items.len(), 1);
        assert!(page.sections[0].all.is_some());
        settle_section(&mut page, 0, &target, Vec::new());
        assert_eq!(page.sections[0].items.len(), 1);
        assert_eq!(page.sections[0].brief, None);
        assert!(page.sections[0].all.is_none());
    }

    #[test]
    fn old_saves_show_every_overlay_part() {
        let saved: Saved = serde_json::from_str(r#"{"volume":0.5,"overlay":true}"#).unwrap();
        assert!(saved.overlay_controls && saved.overlay_next_up);
        let saved: Saved = serde_json::from_str(r#"{"overlay_controls":false}"#).unwrap();
        assert!(!saved.overlay_controls && saved.overlay_next_up);
    }

    #[test]
    fn the_overlay_only_opens_when_enabled() {
        assert!(!overlay_after_toggle(false, false));
        assert!(overlay_after_toggle(true, false));
        assert!(!overlay_after_toggle(true, true));
        assert!(!overlay_after_toggle(false, true));
    }

    #[test]
    fn broken_opacity_falls_back_into_range() {
        assert_eq!(usable_opacity(f32::NAN), OVERLAY_OPACITY_DEFAULT);
        assert_eq!(usable_opacity(-4.0), OVERLAY_OPACITY_MIN);
        assert_eq!(usable_opacity(7.0), 1.0);
        assert_eq!(usable_opacity(f32::INFINITY), 1.0);
        assert_eq!(usable_opacity(0.5), 0.5);
    }

    #[test]
    fn reads_release_notes_as_plain_lines() {
        let notes = release_notes("# 0.7.2\n\n- Faster search\n* Lyrics stay in sync\n   \n");
        assert_eq!(notes, ["Faster search", "Lyrics stay in sync"]);
    }

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }

        fn flush(&mut self) {}
    }

    fn stored(ron: &str) -> MemoryStorage {
        let mut storage = MemoryStorage::default();
        eframe::Storage::set_string(&mut storage, eframe::APP_KEY, ron.to_owned());
        storage
    }

    #[test]
    fn a_save_from_a_newer_build_with_extra_fields_still_loads() {
        let storage = stored("(volume: 0.25, shuffle: true, a_setting_from_the_future: 7, beta_updates: true)");
        let saved: Saved = eframe::get_value(&storage, eframe::APP_KEY).unwrap();
        assert_eq!((saved.volume, saved.shuffle, saved.beta_updates), (0.25, true, true));
    }

    #[test]
    fn a_save_from_before_beta_updates_loads_with_them_off() {
        let storage = stored("(volume: 0.25)");
        let saved: Saved = eframe::get_value(&storage, eframe::APP_KEY).unwrap();
        assert!(!saved.beta_updates);
    }

    #[test]
    fn an_unreadable_save_is_reported_as_unreadable_instead_of_empty() {
        let storage = stored("(volume: not_a_number");
        assert!(eframe::get_value::<Saved>(&storage, eframe::APP_KEY).is_none());
        assert!(eframe::Storage::get_string(&storage, eframe::APP_KEY).is_some());
    }
}
