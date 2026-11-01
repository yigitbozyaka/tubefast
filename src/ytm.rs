use crate::auth;
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::{BufReader, Read};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MUSIC_API: &str = "https://music.youtube.com/youtubei/v1/";
const PLAYER_API: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
const VISITOR_API: &str = "https://www.youtube.com/youtubei/v1/visitor_id?prettyPrint=false";
const LANGUAGE: &str = "en";
const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const REMIX_CLIENT_ID: &str = "67";
const BUNDLED_CLIENTS: &str = include_str!("../clients.json");
const LATEST_CLIENTS: &str = "https://raw.githubusercontent.com/yigitbozyaka/tubefast/master/clients.json";
const CLIENTS_BYTES: u64 = 16 * 1024;
const CLIENTS_RETRY: Duration = Duration::from_secs(10 * 60);
const WATCH_API: &str = "https://www.youtube.com/youtubei/v1/next?prettyPrint=false";
const MWEB_VERSION: &str = "2.20260708.05.00";
const MWEB_CLIENT_ID: &str = "2";
const MWEB_UA: &str = "Mozilla/5.0 (iPad; CPU OS 16_7_10 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1,gzip(gfe)";
const SONG_CARD_KEY: &str = "\"videoAttributeViewModel\"";
const COVER_HOST: &str = "googleusercontent.com";
const THUMB_WIDTH: u64 = 700;
const COVER_SIZE: &str = "=w544-h544-l90-rj";
pub const VIDEO_FRAME_HOST: &str = "i.ytimg.com";
pub const QUICK_PICKS: &str = "Quick picks";
const LISTED_FRAME_SIDE: u32 = 256;
const SHARP_FRAME: &str = "maxresdefault.jpg";
const LYRICS_TAB: &str = "MPLY";
const LYRICS_CLIENT: &str = "ANDROID_MUSIC";
const LYRICS_VERSION: &str = "7.21.50";
const LYRICS_CLIENT_ID: &str = "21";
const LYRICS_UA: &str = "com.google.android.apps.youtube.music/7.21.50 (Linux; U; Android 14) gzip";
const LRCLIB_API: &str = "https://lrclib.net/api/get";
const LRCLIB_UA: &str = concat!("Tubefast ", env!("CARGO_PKG_VERSION"), " (", env!("CARGO_PKG_REPOSITORY"), ")");
const LRCLIB_WAIT: Duration = Duration::from_secs(6);
const LRCLIB_SOURCE: &str = "Source: lrclib.net";
const LYRICS_PAUSE: &str = "♪";
const AAC_ITAG: u64 = 140;
const RELATED_ITEMS: usize = 18;
const EXPLORE_ORDER: usize = 100;
const HOME_ORDER: usize = 200;
const PERSONAL_PAGES: usize = 3;
const LIKED_SONGS: &str = "VLLM";
const TRACK_PAGES: usize = 50;
const SESSION_REJECTED: &str = "YouTube did not accept this sign-in. Sign in again.";
const UNUSED_KEYS: [&str; 14] = [
    "counterpart",
    "menu",
    "thumbnailOverlay",
    "trackingParams",
    "clickTrackingParams",
    "loggingContext",
    "multiSelectCheckbox",
    "accessibility",
    "accessibilityData",
    "frameworkUpdates",
    "serviceTrackingParams",
    "buttons",
    "badges",
    "playerOverlays",
];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub title: String,
    pub subtitle: String,
    pub extra: String,
    pub duration: String,
    pub thumb: String,
    pub video_id: String,
    pub browse_id: String,
    pub artist_id: String,
    pub album_id: String,
    pub is_artist: bool,
    #[serde(default)]
    pub artists: Vec<(String, String)>,
}

impl Item {
    pub fn is_song(&self) -> bool {
        !self.video_id.is_empty()
    }

    pub fn key(&self) -> &str {
        if self.is_song() { &self.video_id } else { &self.browse_id }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layout {
    Cards,
    Compact,
    Tracks,
    Top,
}

#[derive(Clone, Debug)]
pub struct Section {
    pub title: String,
    pub layout: Layout,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    pub header: Option<Item>,
    pub sections: Vec<Section>,
    pub partial: bool,
}

impl Page {
    pub fn songs(&self) -> Vec<Item> {
        self.sections
            .iter()
            .flat_map(|s| &s.items)
            .filter(|i| i.is_song())
            .cloned()
            .collect()
    }
}

pub struct Stream {
    pub url: String,
    pub len: u64,
    pub duration_ms: u64,
}

#[derive(Debug, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<(u64, String)>,
    pub synced: bool,
    pub source: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub photo: String,
}

#[derive(Deserialize, PartialEq)]
struct Clients {
    remix_version: String,
    player: PlayerClient,
}

#[derive(Deserialize, PartialEq)]
struct PlayerClient {
    id: String,
    user_agent: String,
    client: Value,
}

impl Clients {
    fn parse(json: &str) -> Option<Clients> {
        let clients: Clients = serde_json::from_str(json).ok()?;
        let named = |key: &str| clients.player.client[key].as_str().is_some_and(|value| !value.is_empty());
        let complete = named("clientName") && named("clientVersion");
        let filled = [&clients.remix_version, &clients.player.id, &clients.player.user_agent]
            .iter()
            .all(|value| !value.is_empty());
        (complete && filled).then_some(clients)
    }
}

pub struct Client {
    pub agent: ureq::Agent,
    visitor: Mutex<String>,
    session: Mutex<Option<String>>,
    expired: AtomicBool,
    clients: Mutex<Clients>,
    clients_checked: Mutex<Option<Instant>>,
}

impl Client {
    pub fn new(visitor: String) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(Duration::from_secs(30))
            .user_agent(WEB_UA)
            .build();
        Self {
            agent,
            visitor: Mutex::new(visitor),
            session: Mutex::new(None),
            expired: AtomicBool::new(false),
            clients: Mutex::new(Clients::parse(BUNDLED_CLIENTS).expect("the bundled clients.json should be complete")),
            clients_checked: Mutex::new(None),
        }
    }

    pub fn refresh_clients(&self) -> bool {
        let mut checked = self.clients_checked.lock().unwrap();
        if checked.is_some_and(|at| at.elapsed() < CLIENTS_RETRY) {
            return false;
        }
        *checked = Some(Instant::now());
        let latest = self.agent.get(LATEST_CLIENTS).call().ok().and_then(|response| {
            let mut json = String::new();
            response.into_reader().take(CLIENTS_BYTES).read_to_string(&mut json).ok()?;
            Clients::parse(&json)
        });
        let mut clients = self.clients.lock().unwrap();
        match latest {
            Some(latest) if latest != *clients => {
                *clients = latest;
                true
            }
            _ => false,
        }
    }

    pub fn player_agent(&self) -> String {
        self.clients.lock().unwrap().player.user_agent.clone()
    }

    pub fn visitor(&self) -> String {
        self.visitor.lock().unwrap().clone()
    }

    pub fn set_session(&self, cookie: Option<String>) {
        *self.session.lock().unwrap() = cookie;
    }

    pub fn signed_in(&self) -> bool {
        self.session.lock().unwrap().is_some()
    }

    pub fn take_expired(&self) -> bool {
        self.expired.swap(false, Relaxed)
    }

    pub fn sign_in(&self, cookie: String) -> Result<Account, String> {
        self.set_session(Some(cookie));
        let account = self.account();
        if account.is_err() {
            self.set_session(None);
            self.expired.store(false, Relaxed);
        }
        account
    }

    fn account(&self) -> Result<Account, String> {
        let used = self.session.lock().unwrap().clone().unwrap_or_default();
        let response = self.music("account/account_menu", json!({}), &[])?;
        let Some(header) = find(&response, "activeAccountHeaderRenderer") else {
            self.end_session(&used);
            return Err(SESSION_REJECTED.to_owned());
        };
        Ok(Account {
            name: text(&header["accountName"]),
            photo: thumb(&header["accountPhoto"]),
        })
    }

    fn end_session(&self, rejected: &str) {
        let mut session = self.session.lock().unwrap();
        if session.as_deref() == Some(rejected) {
            *session = None;
            self.expired.store(true, Relaxed);
        }
    }

    pub fn cover(&self, video_id: &str) -> Result<String, String> {
        let body = json!({
            "context": { "client": { "clientName": "MWEB", "clientVersion": MWEB_VERSION, "hl": LANGUAGE } },
            "videoId": video_id,
        });
        let request = self
            .agent
            .post(WATCH_API)
            .set("User-Agent", MWEB_UA)
            .set("X-YouTube-Client-Name", MWEB_CLIENT_ID)
            .set("X-YouTube-Client-Version", MWEB_VERSION);
        let watch_page = request
            .send_json(body)
            .map_err(describe)?
            .into_string()
            .map_err(|e| e.to_string())?;
        Ok(single_song_cover(&watch_page))
    }

    pub fn library(&self) -> Result<Vec<Item>, String> {
        let page = self.browse("FEmusic_liked_playlists", &|_| {})?;
        let saved: Vec<Item> = page
            .sections
            .into_iter()
            .flat_map(|section| section.items)
            .filter(|item| !item.is_song())
            .collect();
        if saved.is_empty() {
            self.account()?;
        }
        Ok(saved)
    }

    fn feed(&self, order: usize, pages: usize, emit: &(dyn Fn(usize, Vec<Section>) + Sync)) -> Result<(), String> {
        let mut response = self.music("browse", json!({ "browseId": "FEmusic_home" }), &[])?;
        let mut seen = HashSet::new();
        for step in 0..pages {
            let mut sections = parse_page(&response).sections;
            sections.retain(|section| seen.insert(section.title.clone()));
            emit(order + step, sections);
            let Some(token) = continuation(&response).filter(|_| step + 1 < pages) else {
                break;
            };
            response = self.more(&token)?;
        }
        Ok(())
    }

    fn more(&self, token: &str) -> Result<Value, String> {
        let params = [("ctoken", token), ("continuation", token), ("type", "next")];
        self.music("browse", json!({ "continuation": token }), &params)
    }

    pub fn liked(&self) -> Result<Vec<Item>, String> {
        let mut response = self.music("browse", json!({ "browseId": LIKED_SONGS }), &[])?;
        let mut token = tracks_continuation(&response);
        let mut songs = parse_page(&response).songs();
        for _ in 1..TRACK_PAGES {
            let Some(next) = token else { break };
            response = self.more(&next)?;
            songs.extend(parse_page(&response).songs());
            token = continuation(&response);
        }
        Ok(songs)
    }

    pub fn rate(&self, video_id: &str, liked: bool) -> Result<(), String> {
        let endpoint = if liked { "like/like" } else { "like/removelike" };
        self.music(endpoint, json!({ "target": { "videoId": video_id } }), &[]).map(drop)
    }

    pub fn home(&self, seeds: &[Item], emit: &(dyn Fn(usize, Vec<Section>) + Sync)) -> Result<(), String> {
        let (explore_order, home_order, pages) = if self.signed_in() {
            (HOME_ORDER, EXPLORE_ORDER, PERSONAL_PAGES)
        } else {
            (EXPLORE_ORDER, HOME_ORDER, 1)
        };
        std::thread::scope(|scope| {
            let related: Vec<_> = seeds
                .iter()
                .enumerate()
                .map(|(order, seed)| {
                    scope.spawn(move || {
                        let radio = self.radio(&seed.video_id)?;
                        let items: Vec<Item> = radio
                            .into_iter()
                            .filter(|item| item.video_id != seed.video_id)
                            .take(RELATED_ITEMS)
                            .collect();
                        if !items.is_empty() {
                            emit(
                                order,
                                vec![Section {
                                    title: format!("More like {}", seed.title),
                                    layout: Layout::Compact,
                                    items,
                                }],
                            );
                        }
                        Ok(())
                    })
                })
                .collect();
            let explore = scope.spawn(move || {
                self.browse("FEmusic_explore", &|_| {})
                    .map(|page| emit(explore_order, page.sections))
            });
            let home = self.feed(home_order, pages, emit);
            let finished = related.into_iter().chain([explore]).map(|task| task.join().unwrap()).chain([home]);
            let mut outcome = Err(String::new());
            for result in finished {
                if outcome.is_err() {
                    outcome = result;
                }
            }
            outcome
        })
    }

    pub fn browse(&self, browse_id: &str, progress: &dyn Fn(&Page)) -> Result<Page, String> {
        let response = self.music("browse", json!({ "browseId": browse_id }), &[])?;
        let mut token = tracks_continuation(&response);
        let mut page = parse_page(&response);
        if let Some(header) = &mut page.header {
            header.browse_id = browse_id.to_owned();
            header.is_artist = browse_id.starts_with("UC");
        }
        let tracks = page.sections.iter().position(|section| section.layout == Layout::Tracks);
        for _ in 1..TRACK_PAGES {
            let (Some(next), Some(at)) = (token.take(), tracks) else { break };
            page.partial = true;
            inherit_header(&mut page);
            progress(&page);
            let Ok(response) = self.more(&next) else { break };
            page.sections[at].items.extend(parse_page(&response).songs());
            token = continuation(&response);
        }
        page.partial = false;
        inherit_header(&mut page);
        Ok(page)
    }

    pub fn search(&self, query: &str) -> Result<Page, String> {
        let response = self.music("search", json!({ "query": query }), &[])?;
        let mut page = parse_page(&response);
        page.header = None;
        for section in page.sections.iter_mut().filter(|section| section.title.is_empty()) {
            section.title = "Results".to_owned();
        }
        Ok(page)
    }

    pub fn radio(&self, video_id: &str) -> Result<Vec<Item>, String> {
        let body = json!({
            "videoId": video_id,
            "playlistId": format!("RDAMVM{video_id}"),
            "isAudioOnly": true,
        });
        Ok(parse_page(&self.music("next", body, &[])?).songs())
    }

    pub fn lyrics(&self, song: &Item, seconds: u64) -> Result<Option<Lyrics>, String> {
        let known = self.youtube_lyrics(&song.video_id);
        if matches!(&known, Ok(Some(lyrics)) if lyrics.synced) {
            return known;
        }
        match self.lrclib_lyrics(song, seconds) {
            Some(found) if found.synced || !matches!(known, Ok(Some(_))) => Ok(Some(found)),
            _ => known,
        }
    }

    fn lrclib_lyrics(&self, song: &Item, seconds: u64) -> Option<Lyrics> {
        let seconds = match seconds {
            0 => self.stream(&song.video_id).ok()?.duration_ms / 1000,
            played => played,
        };
        let artist = match song.artists.first() {
            Some((name, _)) => name,
            None => song.subtitle.split(" • ").next()?,
        };
        let request = self
            .agent
            .get(LRCLIB_API)
            .set("User-Agent", LRCLIB_UA)
            .timeout(LRCLIB_WAIT)
            .query("track_name", &song.title)
            .query("artist_name", artist)
            .query("duration", &seconds.to_string());
        let found = request.clone().call().or_else(|refused| match refused {
            ureq::Error::Status(500.., _) => request.call(),
            other => Err(other),
        });
        lrclib_lyrics(&found.ok()?.into_json().ok()?)
    }

    fn youtube_lyrics(&self, video_id: &str) -> Result<Option<Lyrics>, String> {
        let watch = self.music("next", json!({ "videoId": video_id }), &[])?;
        let tabs = find(&watch, "tabs").and_then(Value::as_array);
        let Some(id) = tabs
            .into_iter()
            .flatten()
            .map(|tab| &tab["tabRenderer"])
            .filter(|tab| tab["unselectable"] != true)
            .filter_map(|tab| tab.pointer("/endpoint/browseEndpoint/browseId")?.as_str())
            .find(|id| id.starts_with(LYRICS_TAB))
        else {
            return Ok(None);
        };
        let body = json!({
            "context": { "client": { "clientName": LYRICS_CLIENT, "clientVersion": LYRICS_VERSION, "hl": LANGUAGE } },
            "browseId": id,
        });
        let request = self
            .agent
            .post(&format!("{MUSIC_API}browse?prettyPrint=false"))
            .set("User-Agent", LYRICS_UA)
            .set("X-YouTube-Client-Name", LYRICS_CLIENT_ID)
            .set("X-YouTube-Client-Version", LYRICS_VERSION);
        if let Some(timed) = send(request, body).ok().as_ref().and_then(timed_lyrics) {
            return Ok(Some(timed));
        }
        Ok(plain_lyrics(&self.music("browse", json!({ "browseId": id }), &[])?))
    }

    pub fn stream(&self, video_id: &str) -> Result<Stream, String> {
        let mut response = self.player(video_id)?;
        if audio(&response).is_none() {
            self.visitor.lock().unwrap().clear();
            response = self.player(video_id)?;
        }
        if audio(&response).is_none() && self.refresh_clients() {
            response = self.player(video_id)?;
        }
        audio(&response).ok_or_else(|| {
            let status = &response["playabilityStatus"];
            let reason = if playable(&response) {
                None
            } else {
                status["reason"].as_str().or(status["status"].as_str())
            };
            reason.unwrap_or("No compatible audio stream for this track").to_owned()
        })
    }

    fn player(&self, video_id: &str) -> Result<Value, String> {
        let visitor = self.ensure_visitor()?;
        let (mut client, id, user_agent) = {
            let player = &self.clients.lock().unwrap().player;
            (player.client.clone(), player.id.clone(), player.user_agent.clone())
        };
        let version = client["clientVersion"].as_str().unwrap_or_default().to_owned();
        client["hl"] = LANGUAGE.into();
        client["visitorData"] = visitor.as_str().into();
        let body = json!({
            "context": { "client": client },
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true,
        });
        let request = self
            .agent
            .post(PLAYER_API)
            .set("User-Agent", &user_agent)
            .set("X-YouTube-Client-Name", &id)
            .set("X-YouTube-Client-Version", &version)
            .set("X-Goog-Visitor-Id", &visitor);
        match request.send_json(body) {
            Ok(response) => read(response),
            Err(ureq::Error::Status(code, _)) => {
                Ok(json!({ "playabilityStatus": { "reason": format!("YouTube answered with HTTP {code}") } }))
            }
            Err(error) => Err(describe(error)),
        }
    }

    fn ensure_visitor(&self) -> Result<String, String> {
        let known = self.visitor();
        if !known.is_empty() {
            return Ok(known);
        }
        let body = json!({ "context": { "client": { "clientName": "WEB", "clientVersion": "2.20260707.00.00", "hl": LANGUAGE } } });
        let response = send(self.agent.post(VISITOR_API), body)?;
        let visitor = response["responseContext"]["visitorData"]
            .as_str()
            .ok_or("YouTube did not issue a visitor id")?
            .to_owned();
        *self.visitor.lock().unwrap() = visitor.clone();
        Ok(visitor)
    }

    fn music(&self, endpoint: &str, mut body: Value, params: &[(&str, &str)]) -> Result<Value, String> {
        let version = self.clients.lock().unwrap().remix_version.clone();
        body["context"] = json!({ "client": { "clientName": "WEB_REMIX", "clientVersion": version, "hl": LANGUAGE } });
        let mut request = self
            .agent
            .post(&format!("{MUSIC_API}{endpoint}?prettyPrint=false"))
            .set("Origin", auth::ORIGIN)
            .set("X-YouTube-Client-Name", REMIX_CLIENT_ID)
            .set("X-YouTube-Client-Version", &version);
        for (name, value) in params {
            request = request.query(name, value);
        }
        let cookie = self.session.lock().unwrap().clone();
        if let Some(cookie) = &cookie {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs());
            let signature = auth::authorization(cookie, now).unwrap_or_default();
            request = request
                .set("Cookie", cookie)
                .set("Authorization", &signature)
                .set("X-Origin", auth::ORIGIN)
                .set("X-Goog-AuthUser", "0");
        }
        let response = match request.send_json(body) {
            Ok(response) => read(response)?,
            Err(ureq::Error::Status(401, _)) if cookie.is_some() => {
                self.end_session(&cookie.unwrap_or_default());
                return Err(SESSION_REJECTED.to_owned());
            }
            Err(error) => return Err(describe(error)),
        };
        if let Some(visitor) = response["responseContext"]["visitorData"].as_str() {
            let mut known = self.visitor.lock().unwrap();
            if known.is_empty() {
                *known = visitor.to_owned();
            }
        }
        Ok(response)
    }
}

fn send(request: ureq::Request, body: Value) -> Result<Value, String> {
    read(request.send_json(body).map_err(describe)?)
}

fn read(response: ureq::Response) -> Result<Value, String> {
    let reader = BufReader::new(response.into_reader());
    Lean.deserialize(&mut serde_json::Deserializer::from_reader(reader))
        .map_err(|e| e.to_string())
}

fn find<'a>(node: &'a Value, key: &str) -> Option<&'a Value> {
    match node {
        Value::Array(list) => list.iter().find_map(|child| find(child, key)),
        Value::Object(map) => map.get(key).or_else(|| map.values().find_map(|child| find(child, key))),
        _ => None,
    }
}

fn timed_lyrics(response: &Value) -> Option<Lyrics> {
    let cues = find(response, "timedLyricsData")?.as_array()?;
    let lines: Vec<(u64, String)> = cues
        .iter()
        .filter_map(|cue| {
            let start = cue.pointer("/cueRange/startTimeMilliseconds")?.as_str()?.parse().ok()?;
            Some((start, cue["lyricLine"].as_str()?.to_owned()))
        })
        .collect();
    let source = find(response, "sourceMessage").and_then(Value::as_str).unwrap_or_default();
    (!lines.is_empty()).then(|| Lyrics {
        lines,
        synced: true,
        source: source.to_owned(),
    })
}

fn lrclib_lyrics(found: &Value) -> Option<Lyrics> {
    let timed = found["syncedLyrics"].as_str().unwrap_or_default().lines().filter_map(|line| {
        let (stamp, words) = line.strip_prefix('[')?.split_once(']')?;
        let (minutes, seconds) = stamp.split_once(':')?;
        let start = minutes.parse::<u64>().ok()? * 60_000 + (seconds.parse::<f64>().ok()? * 1000.0).round() as u64;
        let words = Some(words.trim()).filter(|words| !words.is_empty());
        Some((start, words.unwrap_or(LYRICS_PAUSE).to_owned()))
    });
    let mut lines: Vec<(u64, String)> = timed.collect();
    let synced = !lines.is_empty();
    if !synced {
        let plain = found["plainLyrics"].as_str().unwrap_or_default();
        lines = plain.lines().map(|line| (0, line.trim().to_owned())).collect();
    }
    (!lines.is_empty()).then(|| Lyrics {
        lines,
        synced,
        source: LRCLIB_SOURCE.to_owned(),
    })
}

fn plain_lyrics(response: &Value) -> Option<Lyrics> {
    let shelf = find(response, "musicDescriptionShelfRenderer")?;
    let words = text(&shelf["description"]);
    let lines: Vec<(u64, String)> = words.lines().map(|line| (0, line.trim().to_owned())).collect();
    (!lines.is_empty()).then(|| Lyrics {
        lines,
        synced: false,
        source: text(&shelf["footer"]),
    })
}

fn single_song_cover(watch_page: &str) -> String {
    let mut mentions = watch_page
        .match_indices(SONG_CARD_KEY)
        .map(|(at, key)| &watch_page[at + key.len()..]);
    let song = match (mentions.next(), mentions.next()) {
        (Some(only), None) => only.find('{').map(|open| &only[open..]),
        _ => None,
    };
    let song: Value = song
        .and_then(|json| serde_json::Deserializer::from_str(json).into_iter().next()?.ok())
        .unwrap_or_default();
    let image = song.pointer("/image/sources/0/url").and_then(Value::as_str).unwrap_or_default();
    match (image.contains(COVER_HOST), image.contains('=')) {
        (false, _) => String::new(),
        (true, true) => image.to_owned(),
        (true, false) => format!("{image}{COVER_SIZE}"),
    }
}

fn continuation(response: &Value) -> Option<String> {
    let classic = find(response, "nextContinuationData").map(|data| &data["continuation"]);
    let token = classic.or_else(|| find(response, "continuationCommand").map(|command| &command["token"]))?;
    token.as_str().map(str::to_owned)
}

fn tracks_continuation(playlist: &Value) -> Option<String> {
    find(playlist, "musicPlaylistShelfRenderer").and_then(continuation)
}

fn inherit_header(page: &mut Page) {
    let Some(header) = &page.header else { return };
    let (thumb, artist) = (header.thumb.clone(), header.extra.clone());
    for item in page.sections.iter_mut().flat_map(|s| &mut s.items) {
        if item.thumb.is_empty() {
            item.thumb = thumb.clone();
        }
        if item.subtitle.is_empty() {
            item.subtitle = artist.clone();
        }
    }
}

struct Lean;

impl<'de> DeserializeSeed<'de> for Lean {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Lean {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_f64<E>(self, value: f64) -> Result<Value, E> {
        Ok(value.into())
    }

    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut list = Vec::new();
        while let Some(value) = seq.next_element_seed(Lean)? {
            list.push(value);
        }
        Ok(Value::Array(list))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut object = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if UNUSED_KEYS.contains(&key.as_str()) {
                map.next_value::<IgnoredAny>()?;
            } else {
                object.insert(key, map.next_value_seed(Lean)?);
            }
        }
        Ok(Value::Object(object))
    }
}

fn playable(response: &Value) -> bool {
    response["playabilityStatus"]["status"] == "OK"
}

fn audio(response: &Value) -> Option<Stream> {
    let formats = response["streamingData"]["adaptiveFormats"]
        .as_array()
        .filter(|_| playable(response))?;
    let format = formats.iter().find(|f| f["itag"].as_u64() == Some(AAC_ITAG))?;
    let number = |key: &str| format[key].as_str().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    Some(Stream {
        url: format["url"].as_str()?.to_owned(),
        len: number("contentLength"),
        duration_ms: number("approxDurationMs"),
    })
}

fn describe(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("YouTube answered with HTTP {code}"),
        ureq::Error::Transport(_) => "Could not reach YouTube. Check your connection.".to_owned(),
    }
}

pub fn sized(url: &str, px: u32) -> String {
    let Some(at) = url.rfind('=') else { return url.to_owned() };
    let (base, params) = url.split_at(at);
    if params.starts_with("=w") {
        let rest = params.splitn(3, '-').nth(2).map(|r| format!("-{r}")).unwrap_or_default();
        format!("{base}=w{px}-h{px}{rest}")
    } else if params.starts_with("=s") {
        format!("{base}=s{px}")
    } else {
        url.to_owned()
    }
}

pub fn sharper(url: &str, px: u32) -> Option<String> {
    let id = url.split_once(VIDEO_FRAME_HOST)?.1.strip_prefix("/vi/")?.split('/').next()?;
    (px > LISTED_FRAME_SIDE).then(|| format!("https://{VIDEO_FRAME_HOST}/vi/{id}/{SHARP_FRAME}"))
}

pub fn parse_page(response: &Value) -> Page {
    let mut page = Page::default();
    collect(response, &mut page);
    page.sections.retain(|s| !s.items.is_empty());
    page
}

const SHELVES: [&str; 5] = [
    "musicCarouselShelfRenderer",
    "musicShelfRenderer",
    "musicPlaylistShelfRenderer",
    "musicCardShelfRenderer",
    "gridRenderer",
];
const ITEMS: [&str; 3] = [
    "musicResponsiveListItemRenderer",
    "musicTwoRowItemRenderer",
    "playlistPanelVideoRenderer",
];
const HEADERS: [&str; 4] = [
    "musicResponsiveHeaderRenderer",
    "musicImmersiveHeaderRenderer",
    "musicVisualHeaderRenderer",
    "musicDetailHeaderRenderer",
];

fn collect(node: &Value, page: &mut Page) {
    match node {
        Value::Array(list) => list.iter().for_each(|child| collect(child, page)),
        Value::Object(map) => {
            for (key, child) in map {
                let key = key.as_str();
                if SHELVES.contains(&key) {
                    page.sections.push(shelf(key, child));
                } else if ITEMS.contains(&key) {
                    let loose = matches!(page.sections.last(), Some(s) if s.title.is_empty() && s.layout == Layout::Tracks);
                    if !loose {
                        page.sections.push(Section {
                            title: String::new(),
                            layout: Layout::Tracks,
                            items: Vec::new(),
                        });
                    }
                    page.sections.last_mut().unwrap().items.extend(item(child));
                } else if HEADERS.contains(&key) {
                    page.header = Some(header(child));
                } else {
                    collect(child, page);
                }
            }
        }
        _ => {}
    }
}

fn shelf(kind: &str, node: &Value) -> Section {
    let children = node["contents"].as_array().or(node["items"].as_array());
    let entries = || children.into_iter().flatten().filter_map(Value::as_object).flat_map(|o| o.iter());
    let mut items: Vec<Item> = entries()
        .filter(|(k, _)| ITEMS.contains(&k.as_str()))
        .filter_map(|(_, v)| item(v))
        .collect();
    let cards = entries().next().is_some_and(|(k, _)| k == "musicTwoRowItemRenderer");
    let (title, layout) = match kind {
        "musicCardShelfRenderer" => {
            let top = card(node);
            for item in items.iter_mut().filter(|item| top.is_artist && item.subtitle.is_empty()) {
                item.subtitle = top.title.clone();
            }
            items.insert(0, top);
            ("Top result".to_owned(), Layout::Top)
        }
        "musicCarouselShelfRenderer" | "gridRenderer" => {
            let basic = &node["header"]["musicCarouselShelfBasicHeaderRenderer"]["title"];
            let grid = &node["header"]["gridHeaderRenderer"]["title"];
            (text(basic) + &text(grid), if cards { Layout::Cards } else { Layout::Compact })
        }
        _ => (text(&node["title"]), Layout::Tracks),
    };
    Section { title, layout, items }
}

fn card(node: &Value) -> Item {
    let mut it = Item {
        title: text(&node["title"]),
        subtitle: text(&node["subtitle"]),
        thumb: thumb(&node["thumbnail"]),
        video_id: first_str(
            node,
            &[
                "/onTap/watchEndpoint/videoId",
                "/title/runs/0/navigationEndpoint/watchEndpoint/videoId",
            ],
        ),
        browse_id: first_str(
            node,
            &[
                "/onTap/browseEndpoint/browseId",
                "/title/runs/0/navigationEndpoint/browseEndpoint/browseId",
            ],
        ),
        ..Item::default()
    };
    it.is_artist = !it.is_song() && it.browse_id.starts_with("UC");
    tidy(&mut it);
    it
}

fn tidy(it: &mut Item) {
    let mut parts: Vec<&str> = it.subtitle.split(" • ").collect();
    if parts.first() == Some(&"Song") {
        parts.remove(0);
    }
    let timed = parts
        .last()
        .is_some_and(|last| last.contains(':') && last.chars().all(|c| c.is_ascii_digit() || c == ':'));
    if timed && it.duration.is_empty() {
        it.duration = parts.pop().unwrap_or_default().to_owned();
    }
    let credited;
    if it.is_song()
        && let Some(slot) = parts.iter().position(|part| *part != "Video")
    {
        let (title, featured) = clean_title(&it.title);
        let prefix = format!("{} - ", parts[slot]).to_ascii_lowercase();
        it.title = match title.to_ascii_lowercase().starts_with(&prefix) {
            true if title.len() > prefix.len() => title[prefix.len()..].to_owned(),
            _ => title,
        };
        let first = featured.split([',', '&']).next().unwrap_or_default().trim();
        if !first.is_empty() && !it.subtitle.contains(first) {
            credited = format!("{}, {featured}", parts[slot]);
            parts[slot] = &credited;
        }
    }
    it.subtitle = parts.join(" • ");
}

/// Drops the "(Official Video)" style noise from a song title and returns the
/// featured artists it named separately, so the byline can carry them.
fn clean_title(title: &str) -> (String, String) {
    let credit = |inner: &str| {
        let inner = inner.trim();
        let lower = inner.to_ascii_lowercase();
        ["featuring ", "feat.", "feat ", "ft.", "ft "]
            .into_iter()
            .find(|mark| lower.starts_with(mark))
            .map(|mark| inner[mark.len()..].trim().to_owned())
    };
    let noise = |inner: &str| {
        let lower = inner.to_ascii_lowercase();
        let tagged = ["official", "oficial", "officiel", "lyric"].iter().any(|word| lower.contains(word));
        let words = [
            "audio",
            "video",
            "music",
            "visualizer",
            "visualiser",
            "hd",
            "hq",
            "4k",
            "mv",
            "clip",
            "videoclip",
        ];
        let bare = !lower.trim().is_empty() && lower.split_whitespace().all(|word| words.contains(&word));
        let kept = ["remix", "live", "version", "edit", "cover", "acoustic", "mix"]
            .iter()
            .any(|word| lower.contains(word));
        (tagged || bare) && !kept
    };
    let (mut kept, mut featured, mut rest) = (String::new(), String::new(), title);
    while let Some(open) = rest.find(['(', '[']) {
        let close = if rest.as_bytes()[open] == b'(' { ')' } else { ']' };
        let Some(length) = rest[open..].find(close) else { break };
        let inner = &rest[open + 1..open + length];
        kept.push_str(&rest[..open]);
        match credit(inner) {
            Some(names) if featured.is_empty() => featured = names,
            Some(_) => {}
            None if noise(inner) => {}
            None => kept.push_str(&rest[open..=open + length]),
        }
        rest = &rest[open + length + 1..];
    }
    kept.push_str(rest);
    let lower = kept.to_ascii_lowercase();
    let inline = [" featuring ", " feat. ", " ft. "]
        .into_iter()
        .filter_map(|mark| lower.find(mark).map(|at| (at, mark.len())))
        .min();
    if let Some((at, length)) = inline.filter(|_| featured.is_empty()) {
        let tail = &kept[at + length..];
        let end = tail.find(['(', '[']).unwrap_or(tail.len());
        featured = tail[..end].trim().to_owned();
        kept = format!("{} {}", &kept[..at], &tail[end..]);
    }
    let clean = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = clean.trim_end_matches([' ', '-', '|', '–']).to_owned();
    if clean.is_empty() {
        (title.to_owned(), String::new())
    } else {
        (clean, featured)
    }
}

fn header(node: &Value) -> Item {
    let details = [
        text(&node["subtitle"]),
        text(&node["secondSubtitle"]),
        text(&node["monthlyListenerCount"]),
    ];
    Item {
        title: text(&node["title"]),
        subtitle: details.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" • "),
        extra: text(&node["straplineTextOne"]),
        thumb: thumb(&node["thumbnail"]),
        ..Item::default()
    }
}

fn item(node: &Value) -> Option<Item> {
    let column = |index: usize| text(&node["flexColumns"][index]["musicResponsiveListItemFlexColumnRenderer"]["text"]);
    let title = [text(&node["title"]), column(0)].into_iter().find(|s| !s.is_empty())?;
    let bylines = [
        text(&node["subtitle"]),
        text(&node["shortBylineText"]),
        text(&node["longBylineText"]),
        column(1),
    ];
    let subtitle = bylines.into_iter().find(|s| !s.is_empty());
    let fixed = text(&node["fixedColumns"][0]["musicResponsiveListItemFixedColumnRenderer"]["text"]);
    let mut it = Item {
        title,
        subtitle: subtitle.unwrap_or_default(),
        extra: [column(2), column(3)]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" • "),
        duration: fixed + &text(&node["lengthText"]),
        thumb: thumb(node),
        video_id: first_str(
            node,
            &[
                "/playlistItemData/videoId",
                "/videoId",
                "/navigationEndpoint/watchEndpoint/videoId",
                "/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/watchEndpoint/videoId",
                "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId",
            ],
        ),
        browse_id: first_str(
            node,
            &[
                "/navigationEndpoint/browseEndpoint/browseId",
                "/title/runs/0/navigationEndpoint/browseEndpoint/browseId",
                "/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/browseEndpoint/browseId",
            ],
        ),
        ..Item::default()
    };
    links(node, "", &mut it);
    it.is_artist = !it.is_song() && it.browse_id.starts_with("UC");
    tidy(&mut it);
    (it.is_song() || !it.browse_id.is_empty()).then_some(it)
}

fn links(node: &Value, run_text: &str, it: &mut Item) {
    match node {
        Value::Array(list) => list.iter().for_each(|child| links(child, run_text, it)),
        Value::Object(map) => {
            let run_text = map.get("text").and_then(Value::as_str).unwrap_or(run_text);
            for (key, child) in map {
                if key == "browseEndpoint" {
                    let kind = child.pointer("/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType");
                    let id = child["browseId"].as_str().unwrap_or_default();
                    match kind.and_then(Value::as_str) {
                        Some("MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL") => {
                            if it.artist_id.is_empty() {
                                it.artist_id = id.to_owned();
                            }
                            if !run_text.is_empty() && it.artists.iter().all(|(_, known)| known != id) {
                                it.artists.push((run_text.to_owned(), id.to_owned()));
                            }
                        }
                        Some("MUSIC_PAGE_TYPE_ALBUM") if it.album_id.is_empty() => it.album_id = id.to_owned(),
                        _ => {}
                    }
                } else if key != "menu" {
                    links(child, run_text, it);
                }
            }
        }
        _ => {}
    }
}

fn text(node: &Value) -> String {
    node["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|run| run["text"].as_str())
        .collect()
}

fn first_str(node: &Value, pointers: &[&str]) -> String {
    pointers
        .iter()
        .find_map(|p| node.pointer(p)?.as_str())
        .unwrap_or_default()
        .to_owned()
}

fn thumb(node: &Value) -> String {
    match node {
        Value::Array(list) => list.iter().map(thumb).find(|url| !url.is_empty()).unwrap_or_default(),
        Value::Object(map) => {
            let sizes = map.get("thumbnails").and_then(Value::as_array);
            let enough = |size: &&Value| size["width"].as_u64().is_some_and(|width| width >= THUMB_WIDTH);
            if let Some(url) = sizes
                .and_then(|sizes| sizes.iter().find(enough).or(sizes.last()))
                .and_then(|size| size["url"].as_str())
            {
                return url.to_owned();
            }
            map.iter()
                .filter(|(key, _)| *key != "menu")
                .map(|(_, child)| thumb(child))
                .find(|url| !url.is_empty())
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_only_the_session_that_youtube_rejected() {
        let yt = Client::new(String::new());
        yt.set_session(Some("newer".to_owned()));
        yt.end_session("older");
        assert!(yt.signed_in() && !yt.take_expired());
        yt.end_session("newer");
        assert!(!yt.signed_in() && yt.take_expired());
        yt.end_session("");
        assert!(!yt.take_expired());
    }

    #[test]
    fn resizes_thumbnails_without_touching_other_hosts() {
        assert_eq!(
            sized("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj", 240),
            "https://lh3.googleusercontent.com/abc=w240-h240-l90-rj"
        );
        assert_eq!(
            sized("https://yt3.googleusercontent.com/abc=s192", 512),
            "https://yt3.googleusercontent.com/abc=s512"
        );
        assert_eq!(
            sized("https://i.ytimg.com/vi/x/hq720.jpg?sqp=a&rs=b", 240),
            "https://i.ytimg.com/vi/x/hq720.jpg?sqp=a&rs=b"
        );
    }

    #[test]
    fn reads_timed_lyrics_and_falls_back_to_plain_text() {
        let timed = json!({ "contents": { "timedLyricsModel": { "lyricsData": {
            "timedLyricsData": [
                { "lyricLine": "♪", "cueRange": { "startTimeMilliseconds": "0", "endTimeMilliseconds": "14940" } },
                { "lyricLine": "Our ends were beginnings", "cueRange": { "startTimeMilliseconds": "19180" } },
                { "lyricLine": "no start time" },
            ],
            "sourceMessage": "Source: Musixmatch",
        } } } });
        let lyrics = timed_lyrics(&timed).unwrap();
        assert_eq!(lyrics.lines, [(0, "♪".to_owned()), (19180, "Our ends were beginnings".to_owned())]);
        assert!(lyrics.synced);
        assert_eq!(lyrics.source, "Source: Musixmatch");
        assert_eq!(timed_lyrics(&json!({ "contents": {} })), None);

        let plain = json!({ "musicDescriptionShelfRenderer": {
            "description": { "runs": [{ "text": "First line\r\n\r\nSecond line" }] },
            "footer": { "runs": [{ "text": "Source: LyricFind" }] },
        } });
        let lyrics = plain_lyrics(&plain).unwrap();
        let lines: Vec<&str> = lyrics.lines.iter().map(|(_, line)| line.as_str()).collect();
        assert_eq!(lines, ["First line", "", "Second line"]);
        assert!(!lyrics.synced);
        assert_eq!(plain_lyrics(&json!({ "messageRenderer": {} })), None);
    }

    #[test]
    fn reads_lrclib_lines_by_their_time_stamps_and_plain_text_without_them() {
        let stamped =
            "[ar:Daft Punk]\n[00:31.48] Like the legend of the phoenix\n[01:05.5]\n[62:03.250] All ends with beginnings\nno time stamp";
        let timed = json!({ "plainLyrics": "ignored when there are time stamps", "syncedLyrics": stamped });
        let lyrics = lrclib_lyrics(&timed).unwrap();
        let lines = [
            (31_480, "Like the legend of the phoenix"),
            (65_500, "♪"),
            (3_723_250, "All ends with beginnings"),
        ];
        assert_eq!(lyrics.lines, lines.map(|(start, line)| (start, line.to_owned())));
        assert!(lyrics.synced);
        assert_eq!(lyrics.source, "Source: lrclib.net");

        let plain = json!({ "plainLyrics": "First line\n\nSecond line", "syncedLyrics": null });
        let lyrics = lrclib_lyrics(&plain).unwrap();
        let lines: Vec<&str> = lyrics.lines.iter().map(|(_, line)| line.as_str()).collect();
        assert_eq!(lines, ["First line", "", "Second line"]);
        assert!(!lyrics.synced);
        assert_eq!(
            lrclib_lyrics(&json!({ "instrumental": true, "plainLyrics": null, "syncedLyrics": null })),
            None
        );
        assert_eq!(lrclib_lyrics(&json!({ "code": 404, "name": "TrackNotFound" })), None);
    }

    #[test]
    fn links_a_video_to_its_uploader_when_it_names_no_artist() {
        let byline = |kind: &str| {
            json!({
                "title": { "runs": [{ "text": "Get Lucky (10 min loop)" }] },
                "videoId": "video",
                "subtitle": { "runs": [{ "text": "nTOURIST", "navigationEndpoint": { "browseEndpoint": {
                    "browseId": "UCuploader",
                    "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": kind } },
                } } }] },
            })
        };
        assert_eq!(item(&byline("MUSIC_PAGE_TYPE_USER_CHANNEL")).unwrap().artist_id, "UCuploader");
        assert_eq!(item(&byline("MUSIC_PAGE_TYPE_ARTIST")).unwrap().artist_id, "UCuploader");
        assert_eq!(item(&byline("MUSIC_PAGE_TYPE_PLAYLIST")).unwrap().artist_id, "");
    }

    #[test]
    fn keeps_every_artist_of_a_song_with_its_own_page() {
        let artist = |name: &str, id: &str| {
            json!({ "text": name, "navigationEndpoint": { "browseEndpoint": {
                "browseId": id,
                "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" } },
            } } })
        };
        let runs = json!([artist("ERAY067", "UC1"), { "text": ", " }, artist("MANSUR", "UC2"), { "text": " & " }, artist("Yung Ouzo", "UC3"), { "text": " • " }, { "text": "1.8M plays" }]);
        let song = item(&json!({
            "title": { "runs": [{ "text": "olm was rap mep" }] },
            "videoId": "song",
            "subtitle": { "runs": runs },
            "shortBylineText": { "runs": [artist("ERAY067", "UC1")] },
        }))
        .unwrap();
        let pair = |name: &str, id: &str| (name.to_owned(), id.to_owned());
        assert_eq!(song.subtitle, "ERAY067, MANSUR & Yung Ouzo • 1.8M plays");
        assert_eq!(
            song.artists,
            [pair("ERAY067", "UC1"), pair("MANSUR", "UC2"), pair("Yung Ouzo", "UC3")]
        );
        assert_eq!(song.artist_id, "UC1");
    }

    #[test]
    fn pages_through_a_playlist_by_its_own_token_not_the_suggestions_one() {
        let track = |id: &str| json!({ "musicResponsiveListItemRenderer": { "title": { "runs": [{ "text": id }] }, "videoId": id } });
        let first = json!({ "sectionListRenderer": {
            "continuations": [{ "nextContinuationData": { "continuation": "suggestions" } }],
            "contents": [{ "musicPlaylistShelfRenderer": { "contents": [
                track("a"),
                { "continuationItemRenderer": { "continuationEndpoint": { "continuationCommand": { "token": "tracks" } } } },
            ] } }],
        } });
        assert_eq!(continuation(&first).as_deref(), Some("suggestions"));
        assert_eq!(tracks_continuation(&first).as_deref(), Some("tracks"));

        let last = json!({ "onResponseReceivedActions": [{ "appendContinuationItemsAction": {
            "continuationItems": [track("b"), track("c")],
        } }] });
        let songs: Vec<String> = parse_page(&last).songs().into_iter().map(|song| song.video_id).collect();
        assert_eq!(songs, ["b", "c"]);
        assert_eq!(continuation(&last), None);
    }

    #[test]
    fn queues_a_song_once_when_it_also_has_a_video() {
        let watch = r#"{ "contents": [{ "playlistPanelVideoWrapperRenderer": {
            "primaryRenderer": { "playlistPanelVideoRenderer": {
                "title": { "runs": [{ "text": "RANDEVU" }] }, "videoId": "song"
            } },
            "counterpart": [{ "counterpartRenderer": { "playlistPanelVideoRenderer": {
                "title": { "runs": [{ "text": "MOTIVE - RANDEVU (Official Music Video)" }] }, "videoId": "video"
            } } }]
        } }] }"#;
        let response = Lean.deserialize(&mut serde_json::Deserializer::from_str(watch)).unwrap();
        let queued: Vec<String> = parse_page(&response).songs().into_iter().map(|song| song.video_id).collect();
        assert_eq!(queued, ["song"]);
    }

    #[test]
    fn asks_for_a_sharper_video_frame_only_at_large_sizes() {
        let frame = "https://i.ytimg.com/vi/x/hqdefault.jpg?sqp=a&rs=b";
        assert_eq!(sharper(frame, 512).as_deref(), Some("https://i.ytimg.com/vi/x/maxresdefault.jpg"));
        assert_eq!(sharper(frame, 256), None);
        assert_eq!(sharper("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj", 512), None);
    }

    #[test]
    fn accepts_only_a_complete_client_list() {
        assert!(Clients::parse(BUNDLED_CLIENTS).is_some());
        assert!(Clients::parse("{}").is_none());
        assert!(Clients::parse(&BUNDLED_CLIENTS.replace("\"clientName\"", "\"name\"")).is_none());
        assert!(Clients::parse(&BUNDLED_CLIENTS.replace("\"101\"", "\"\"")).is_none());
    }

    #[test]
    fn takes_a_cover_only_when_youtube_names_exactly_one_song() {
        let song = |url: &str| json!({ "videoAttributeViewModel": { "title": "Loser", "image": { "sources": [{ "url": url }] } } });
        let art = "https://yt3.googleusercontent.com/abc";
        assert_eq!(
            single_song_cover(&json!({ "cards": [song(art)] }).to_string()),
            "https://yt3.googleusercontent.com/abc=w544-h544-l90-rj"
        );
        assert_eq!(
            single_song_cover(&json!({ "cards": [song("https://lh3.googleusercontent.com/abc=w60-h60")] }).to_string()),
            "https://lh3.googleusercontent.com/abc=w60-h60"
        );
        assert_eq!(single_song_cover(&json!({ "cards": [song(art), song(art)] }).to_string()), "");
        assert_eq!(
            single_song_cover(&json!({ "cards": [song("https://www.youtube.com/img/watch/yt_music_channel.jpeg")] }).to_string()),
            ""
        );
        assert_eq!(single_song_cover("{\"cards\":[]}"), "");
    }

    #[test]
    fn parses_shelves_loose_results_and_headers() {
        let song = json!({ "musicResponsiveListItemRenderer": {
            "playlistItemData": { "videoId": "abcdefghijk" },
            "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "small" }, { "url": "large" }] } } },
            "menu": { "items": [{ "browseEndpoint": { "browseId": "UCmenu", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" } } } }] },
            "flexColumns": [
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Kuzu Kuzu" }] } } },
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                    { "text": "Song" }, { "text": " • " },
                    { "text": "Tarkan", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCartist", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" } } } } },
                    { "text": " • " }, { "text": "3:52" }
                ] } } }
            ]
        }});
        let album = json!({ "musicTwoRowItemRenderer": {
            "title": { "runs": [{ "text": "Karma" }] },
            "subtitle": { "runs": [{ "text": "Album" }] },
            "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_album" } }
        }});
        let response = json!({ "contents": [
            { "musicResponsiveHeaderRenderer": { "title": { "runs": [{ "text": "Karma" }] }, "straplineTextOne": { "runs": [{ "text": "Tarkan" }] } } },
            { "musicCarouselShelfRenderer": { "header": { "musicCarouselShelfBasicHeaderRenderer": { "title": { "runs": [{ "text": "Albums" }] } } }, "contents": [album] } },
            { "itemSectionRenderer": { "contents": [song.clone()] } },
            { "itemSectionRenderer": { "contents": [song] } }
        ]});
        let page = parse_page(&response);
        assert_eq!(page.header.as_ref().unwrap().extra, "Tarkan");
        assert_eq!(page.sections.len(), 2);
        assert_eq!(
            (page.sections[0].title.as_str(), page.sections[0].layout),
            ("Albums", Layout::Cards)
        );
        assert_eq!(page.sections[0].items[0].browse_id, "MPREb_album");
        let track = &page.sections[1].items[0];
        assert_eq!(page.sections[1].items.len(), 2);
        assert_eq!(
            (track.title.as_str(), track.subtitle.as_str(), track.duration.as_str()),
            ("Kuzu Kuzu", "Tarkan", "3:52")
        );
        assert_eq!(
            (track.video_id.as_str(), track.artist_id.as_str(), track.thumb.as_str()),
            ("abcdefghijk", "UCartist", "large")
        );
        assert_eq!(page.songs().len(), 2);
    }

    #[test]
    fn strips_title_noise_and_moves_featured_artists_to_the_byline() {
        let song = |title: &str, subtitle: &str| {
            let mut it = Item {
                title: title.to_owned(),
                subtitle: subtitle.to_owned(),
                video_id: "v".to_owned(),
                ..Item::default()
            };
            tidy(&mut it);
            (it.title, it.subtitle)
        };
        let pair = |title: &str, subtitle: &str| (title.to_owned(), subtitle.to_owned());
        assert_eq!(
            song(
                "Get Lucky (Official Audio) (feat. Pharrell Williams & Nile Rodgers)",
                "Video • Daft Punk • 879M views"
            ),
            pair("Get Lucky", "Video • Daft Punk, Pharrell Williams & Nile Rodgers • 879M views")
        );
        assert_eq!(
            song("I Feel It Coming ft. Daft Punk (Official Video)", "The Weeknd • 3:58"),
            pair("I Feel It Coming", "The Weeknd, Daft Punk")
        );
        assert_eq!(song("Daft Punk - Voyager [Lyrics]", "Daft Punk"), pair("Voyager", "Daft Punk"));
        assert_eq!(
            song("Feels (feat. Pharrell Williams)", "Calvin Harris, Pharrell Williams"),
            pair("Feels", "Calvin Harris, Pharrell Williams")
        );
        assert_eq!(
            song("Lady (Official Remix) (Live)", "Modjo"),
            pair("Lady (Official Remix) (Live)", "Modjo")
        );
        assert_eq!(song("(Official Video)", "Someone"), pair("(Official Video)", "Someone"));
        assert_eq!(song("Çağrı (Lyric Video)", "Şehinşah"), pair("Çağrı", "Şehinşah"));
    }
}
