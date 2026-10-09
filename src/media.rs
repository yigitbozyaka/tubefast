use crate::app::Action;
use crate::ytm::Item;

#[cfg(windows)]
pub struct Media {
    controls: windows::Media::SystemMediaTransportControls,
    track: String,
    playing: bool,
}

#[cfg(windows)]
impl Media {
    pub fn new(cc: &eframe::CreationContext<'_>, press: impl Fn(Action) + Send + 'static) -> Option<Self> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows::Foundation::TypedEventHandler;
        use windows::Media::{
            SystemMediaTransportControls, SystemMediaTransportControlsButton as Button,
            SystemMediaTransportControlsButtonPressedEventArgs as Pressed,
        };
        use windows::Win32::Foundation::HWND;
        use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop as Interop;
        use windows::core::{Ref, factory};

        let RawWindowHandle::Win32(window) = cc.window_handle().ok()?.as_raw() else {
            return None;
        };
        let interop = factory::<SystemMediaTransportControls, Interop>().ok()?;
        let controls: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(window.hwnd.get() as *mut _)) }.ok()?;
        controls.SetIsEnabled(true).ok()?;
        controls.SetIsPlayEnabled(true).ok()?;
        controls.SetIsPauseEnabled(true).ok()?;
        controls.SetIsNextEnabled(true).ok()?;
        controls.SetIsPreviousEnabled(true).ok()?;
        let pressed = TypedEventHandler::new(move |_, pressed: Ref<Pressed>| {
            let button = pressed.ok()?.Button()?;
            if button == Button::Play {
                press(Action::Pause(false));
            } else if button == Button::Pause || button == Button::Stop {
                press(Action::Pause(true));
            } else if button == Button::Next {
                press(Action::Next);
            } else if button == Button::Previous {
                press(Action::Previous);
            }
            Ok(())
        });
        controls.ButtonPressed(&pressed).ok()?;
        Some(Self {
            controls,
            track: String::new(),
            playing: false,
        })
    }

    pub fn show(&mut self, item: Option<&Item>, playing: bool, _position_ms: u64, _duration_ms: u64) {
        let track = item.map_or("", |item| item.video_id.as_str());
        if self.track == track && self.playing == playing {
            return;
        }
        (self.track, self.playing) = (track.to_owned(), playing);
        let _ = self.publish(item, playing);
    }

    fn publish(&self, item: Option<&Item>, playing: bool) -> windows::core::Result<()> {
        use windows::Media::{MediaPlaybackStatus as Status, MediaPlaybackType};
        use windows::core::HSTRING;

        let Some(item) = item else {
            return self.controls.SetPlaybackStatus(Status::Closed);
        };
        self.controls
            .SetPlaybackStatus(if playing { Status::Playing } else { Status::Paused })?;
        let display = self.controls.DisplayUpdater()?;
        display.SetType(MediaPlaybackType::Music)?;
        let music = display.MusicProperties()?;
        music.SetTitle(&HSTRING::from(item.title.as_str()))?;
        music.SetArtist(&HSTRING::from(item.subtitle.as_str()))?;
        display.Update()
    }
}

#[cfg(not(windows))]
const SEEK_SLACK_MS: u64 = 1500;
#[cfg(not(windows))]
const COVER_PX: u32 = 600;

#[cfg(target_os = "macos")]
type Cover = std::sync::Arc<std::sync::Mutex<(String, Option<Vec<u8>>)>>;

#[cfg(target_os = "macos")]
pub struct Media {
    ctx: eframe::egui::Context,
    cover: Cover,
    shown: (String, bool, u64, bool),
    at: (std::time::Instant, u64),
}

#[cfg(target_os = "macos")]
impl Media {
    pub fn new(cc: &eframe::CreationContext<'_>, press: impl Fn(Action) + Send + 'static) -> Option<Self> {
        use block2::RcBlock;
        use objc2_media_player::{
            MPChangePlaybackPositionCommandEvent as Scrubbed, MPRemoteCommand, MPRemoteCommandCenter, MPRemoteCommandEvent,
            MPRemoteCommandHandlerStatus,
        };

        let press = std::rc::Rc::new(press);
        let on = |command: &MPRemoteCommand, action: fn(&MPRemoteCommandEvent) -> Action| {
            let press = press.clone();
            let handler = RcBlock::new(move |event: std::ptr::NonNull<MPRemoteCommandEvent>| {
                press(action(unsafe { event.as_ref() }));
                MPRemoteCommandHandlerStatus::Success
            });
            unsafe { command.addTargetWithHandler(&handler) };
        };
        unsafe {
            let center = MPRemoteCommandCenter::sharedCommandCenter();
            on(&center.playCommand(), |_| Action::Pause(false));
            on(&center.pauseCommand(), |_| Action::Pause(true));
            on(&center.togglePlayPauseCommand(), |_| Action::Toggle);
            on(&center.nextTrackCommand(), |_| Action::Next);
            on(&center.previousTrackCommand(), |_| Action::Previous);
            on(&center.changePlaybackPositionCommand(), |event| {
                let seconds = event.downcast_ref::<Scrubbed>().map_or(0.0, |event| event.positionTime());
                Action::Seek((seconds * 1000.0) as u64)
            });
        }
        Some(Self {
            ctx: cc.egui_ctx.clone(),
            cover: Cover::default(),
            shown: Default::default(),
            at: (std::time::Instant::now(), 0),
        })
    }

    pub fn show(&mut self, item: Option<&Item>, playing: bool, position_ms: u64, duration_ms: u64) {
        let track = item.map_or("", |item| item.video_id.as_str());
        if self.shown.0 != track {
            self.fetch_cover(item.map_or("", |item| item.thumb.as_str()));
        }
        let cover = self.cover.lock().unwrap().1.clone();
        let expected = self.at.1 + if self.shown.1 { self.at.0.elapsed().as_millis() as u64 } else { 0 };
        let shown = (track.to_owned(), playing, duration_ms, cover.is_some());
        if self.shown == shown && expected.abs_diff(position_ms) < SEEK_SLACK_MS {
            return;
        }
        (self.shown, self.at) = (shown, (std::time::Instant::now(), position_ms));
        publish(item, playing, position_ms, duration_ms, cover);
    }

    fn fetch_cover(&self, thumb: &str) {
        let url = crate::ytm::sized(thumb, COVER_PX);
        *self.cover.lock().unwrap() = (url.clone(), None);
        if url.is_empty() {
            return;
        }
        let (cover, ctx) = (self.cover.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let Ok(response) = ureq::get(&url).call() else { return };
            if std::io::Read::read_to_end(&mut response.into_reader(), &mut bytes).is_err() {
                return;
            }
            let mut cover = cover.lock().unwrap();
            if cover.0 == url {
                cover.1 = Some(bytes);
                ctx.request_repaint();
            }
        });
    }
}

#[cfg(target_os = "macos")]
fn publish(item: Option<&Item>, playing: bool, position_ms: u64, duration_ms: u64, cover: Option<Vec<u8>>) {
    use block2::RcBlock;
    use objc2::AnyThread;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::NSImage;
    use objc2_foundation::{NSData, NSDictionary, NSNumber, NSSize, NSString};
    use objc2_media_player::{
        MPMediaItemArtwork, MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
        MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoPropertyElapsedPlaybackTime,
        MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState as State,
    };

    unsafe {
        let center = MPNowPlayingInfoCenter::defaultCenter();
        let Some(item) = item else {
            center.setNowPlayingInfo(None);
            return center.setPlaybackState(State::Stopped);
        };
        let (title, artist) = (NSString::from_str(&item.title), NSString::from_str(&item.subtitle));
        let elapsed = NSNumber::new_f64(position_ms as f64 / 1000.0);
        let duration = NSNumber::new_f64(duration_ms as f64 / 1000.0);
        let rate = NSNumber::new_f64(if playing { 1.0 } else { 0.0 });
        let artwork = cover.and_then(|bytes| {
            let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&bytes))?;
            let size = image.size();
            let handler = RcBlock::new(move |_: NSSize| std::ptr::NonNull::from(&*image));
            Some(MPMediaItemArtwork::initWithBoundsSize_requestHandler(
                MPMediaItemArtwork::alloc(),
                size,
                &handler,
            ))
        });
        let mut keys = vec![
            MPMediaItemPropertyTitle,
            MPMediaItemPropertyArtist,
            MPNowPlayingInfoPropertyElapsedPlaybackTime,
            MPNowPlayingInfoPropertyPlaybackRate,
        ];
        let mut values: Vec<&AnyObject> = vec![title.as_ref(), artist.as_ref(), elapsed.as_ref(), rate.as_ref()];
        if duration_ms > 0 {
            keys.push(MPMediaItemPropertyPlaybackDuration);
            values.push(duration.as_ref());
        }
        if let Some(artwork) = &artwork {
            keys.push(MPMediaItemPropertyArtwork);
            values.push(artwork.as_ref());
        }
        center.setNowPlayingInfo(Some(&NSDictionary::from_slices(&keys, &values)));
        center.setPlaybackState(if playing { State::Playing } else { State::Paused });
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
const POSITION_EVERY_MS: u64 = 1000;

#[cfg(all(unix, not(target_os = "macos")))]
struct Update {
    item: Option<Item>,
    playing: bool,
    position_ms: u64,
    duration_ms: u64,
    seeked: bool,
}

#[cfg(all(unix, not(target_os = "macos")))]
pub struct Media {
    updates: async_channel::Sender<Update>,
    shown: (String, bool, u64),
    at: (std::time::Instant, u64),
}

#[cfg(all(unix, not(target_os = "macos")))]
impl Media {
    pub fn new(_cc: &eframe::CreationContext<'_>, press: impl Fn(Action) + Send + 'static) -> Option<Self> {
        let (updates, received) = async_channel::unbounded();
        std::thread::Builder::new()
            .name("mpris".to_owned())
            .spawn(move || {
                if let Err(error) = futures_lite::future::block_on(serve(press, received)) {
                    eprintln!("Media keys are off, MPRIS is not available: {error}");
                }
            })
            .ok()?;
        Some(Self {
            updates,
            shown: Default::default(),
            at: (std::time::Instant::now(), 0),
        })
    }

    pub fn show(&mut self, item: Option<&Item>, playing: bool, position_ms: u64, duration_ms: u64) {
        let track = item.map_or("", |item| item.video_id.as_str());
        let elapsed = self.at.0.elapsed().as_millis() as u64;
        let expected = self.at.1 + if self.shown.1 { elapsed } else { 0 };
        let jumped = expected.abs_diff(position_ms) >= SEEK_SLACK_MS;
        let shown = (track.to_owned(), playing, duration_ms);
        if self.shown == shown && !jumped && (!playing || elapsed < POSITION_EVERY_MS) {
            return;
        }
        let seeked = self.shown.0 == shown.0 && jumped;
        (self.shown, self.at) = (shown, (std::time::Instant::now(), position_ms));
        let _ = self.updates.try_send(Update {
            item: item.cloned(),
            playing,
            position_ms,
            duration_ms,
            seeked,
        });
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn serve(press: impl Fn(Action) + 'static, updates: async_channel::Receiver<Update>) -> mpris_server::zbus::Result<()> {
    use mpris_server::{Player, Time};

    let player = Player::builder(&format!("tubefast.instance{}", std::process::id()))
        .identity(crate::APP_NAME)
        .desktop_entry("tubefast")
        .can_play(true)
        .can_pause(true)
        .can_go_next(true)
        .can_go_previous(true)
        .can_seek(true)
        .build()
        .await?;
    let press = std::rc::Rc::new(press);
    let on = |action: fn() -> Action| {
        let press = press.clone();
        move |_: &Player| press(action())
    };
    player.connect_play(on(|| Action::Pause(false)));
    player.connect_pause(on(|| Action::Pause(true)));
    player.connect_stop(on(|| Action::Pause(true)));
    player.connect_play_pause(on(|| Action::Toggle));
    player.connect_next(on(|| Action::Next));
    player.connect_previous(on(|| Action::Previous));
    let seek_to = |to: Time| Action::Seek(to.as_millis().max(0) as u64);
    player.connect_set_position({
        let press = press.clone();
        move |_, _, to| press(seek_to(to))
    });
    player.connect_seek({
        let press = press.clone();
        move |player, by| press(seek_to(player.position() + by))
    });

    let follow = async {
        let mut track = (String::new(), 0);
        while let Ok(update) = updates.recv().await {
            follow(&player, &mut track, update).await;
        }
    };
    futures_lite::future::or(player.run(), follow).await;
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn follow(player: &mpris_server::Player, track: &mut (String, u64), update: Update) {
    use mpris_server::{Metadata, PlaybackStatus as Status, Time, TrackId};

    let id = update.item.as_ref().map_or("", |item| item.video_id.as_str());
    if (id, update.duration_ms) != (track.0.as_str(), track.1) {
        *track = (id.to_owned(), update.duration_ms);
        let mut metadata = Metadata::new();
        if let Some(item) = &update.item {
            let path = format!("/dev/tubefast/track/{}", item.video_id.replace('-', "_"));
            metadata.set_trackid(TrackId::try_from(path).ok());
            metadata.set_title(Some(&item.title));
            metadata.set_artist(Some([&item.subtitle]));
            let cover = crate::ytm::sized(&item.thumb, COVER_PX);
            metadata.set_art_url(Some(cover).filter(|cover| !cover.is_empty()));
            metadata.set_length(Some(Time::from_millis(update.duration_ms as i64)).filter(|_| update.duration_ms > 0));
        }
        let _ = player.set_metadata(metadata).await;
    }
    let status = match (&update.item, update.playing) {
        (None, _) => Status::Stopped,
        (Some(_), true) => Status::Playing,
        (Some(_), false) => Status::Paused,
    };
    if player.playback_status() != status {
        let _ = player.set_playback_status(status).await;
    }
    let at = Time::from_millis(update.position_ms as i64);
    player.set_position(at);
    if update.seeked {
        let _ = player.seeked(at).await;
    }
}

#[cfg(not(any(windows, unix)))]
pub struct Media;

#[cfg(not(any(windows, unix)))]
impl Media {
    pub fn new(_cc: &eframe::CreationContext<'_>, _press: impl Fn(Action) + Send + 'static) -> Option<Self> {
        None
    }

    pub fn show(&mut self, _item: Option<&Item>, _playing: bool, _position_ms: u64, _duration_ms: u64) {}
}
