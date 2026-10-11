use crate::ytm;
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions};
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::{Time, TimeBase};

const RANGE_BYTES: u64 = 1024 * 1024;
const WINDOW_BYTES: u64 = 8 * 1024 * 1024;
const KEPT_BEHIND: u64 = 1024 * 1024;
const BUFFERED_AHEAD: u64 = WINDOW_BYTES - KEPT_BEHIND - RANGE_BYTES;
const WINDOW_POLL: Duration = Duration::from_millis(200);
const MAX_STALLS: u32 = 5;
const QUEUED_CHUNKS: usize = 24;
const SILENCE_SAMPLES: usize = 512;
const URL_LIFETIME: Duration = Duration::from_secs(3 * 60 * 60);
const URL_CACHE_ENTRIES: usize = 64;
const SELFTEST_FAR_SEEK_MS: u64 = 20 * 60 * 1000;
const DEVICE_POLL: Duration = Duration::from_secs(2);
const NO_OUTPUT: &str = "No audio output device was found";

type Wake = Arc<dyn Fn() + Send + Sync>;
type Chunks = Arc<Mutex<Receiver<Chunk>>>;
type Urls = Arc<Mutex<HashMap<String, (Instant, String, u64, u64)>>>;

#[derive(Default)]
struct Shared {
    generation: AtomicU64,
    epoch: AtomicU64,
    position_ms: AtomicU64,
    duration_ms: AtomicU64,
    buffered: AtomicU32,
    loading: AtomicBool,
    ended: AtomicBool,
    error: Mutex<Option<String>>,
}

struct Chunk {
    samples: Vec<f32>,
    start_ms: u64,
    epoch: u64,
    rate: u32,
    channels: u16,
    end: bool,
}

enum Cmd {
    Load { video_id: String, generation: u64, epoch: u64 },
    Opened(Track),
    Failed { generation: u64, message: String },
    Seek { ms: u64, epoch: u64 },
}

#[derive(Default)]
struct Window {
    start: u64,
    bytes: VecDeque<u8>,
    reader: u64,
}

impl Window {
    fn end(&self) -> u64 {
        self.start + self.bytes.len() as u64
    }
}

struct Fetch {
    window: Mutex<Window>,
    changed: Condvar,
    len: u64,
    failed: AtomicBool,
    generation: u64,
    shared: Arc<Shared>,
}

impl Fetch {
    fn start(yt: &ytm::Client, url: String, len: u64, generation: u64, shared: Arc<Shared>) -> Arc<Fetch> {
        let (agent, user_agent) = (yt.agent.clone(), yt.player_agent());
        let window = Window {
            bytes: VecDeque::with_capacity(len.min(WINDOW_BYTES) as usize),
            ..Window::default()
        };
        let fetch = Arc::new(Fetch {
            window: Mutex::new(window),
            changed: Condvar::new(),
            len,
            failed: AtomicBool::new(false),
            generation,
            shared,
        });
        let worker = fetch.clone();
        std::thread::spawn(move || {
            let complete = worker.download(&agent, &url, &user_agent);
            worker.failed.store(!complete, Relaxed);
            worker.changed.notify_all();
        });
        fetch
    }

    fn current(&self) -> bool {
        self.shared.generation.load(Relaxed) == self.generation
    }

    fn next_range(&self) -> Option<u64> {
        let windowed = self.len > WINDOW_BYTES;
        let mut window = self.window.lock().unwrap();
        while self.current() {
            if windowed {
                if window.reader < window.start || window.reader > window.end() + RANGE_BYTES {
                    window.bytes.clear();
                    window.start = window.reader;
                }
                let played = window
                    .reader
                    .saturating_sub(window.start + KEPT_BEHIND)
                    .min(window.bytes.len() as u64);
                window.bytes.drain(..played as usize);
                window.start += played;
            }
            let end = window.end();
            if end < self.len && (!windowed || end < window.reader + BUFFERED_AHEAD) {
                return Some(end);
            }
            if !windowed {
                return None;
            }
            window = self.changed.wait_timeout(window, WINDOW_POLL).unwrap().0;
        }
        None
    }

    fn download(&self, agent: &ureq::Agent, url: &str, user_agent: &str) -> bool {
        let mut stalls = 0;
        let mut block = [0u8; 16 * 1024];
        while let Some(from) = self.next_range() {
            let mut at = from;
            let to = (from + RANGE_BYTES).min(self.len) - 1;
            let request = agent.get(&format!("{url}&range={from}-{to}")).set("User-Agent", user_agent);
            if let Ok(response) = request.call() {
                let mut body = response.into_reader();
                while let Ok(read) = body.read(&mut block) {
                    if read == 0 || !self.current() {
                        break;
                    }
                    self.window.lock().unwrap().bytes.extend(&block[..read]);
                    at += read as u64;
                    self.changed.notify_all();
                    self.shared.buffered.store((at * 1000 / self.len) as u32, Relaxed);
                }
            }
            if at > from {
                stalls = 0;
                continue;
            }
            stalls += 1;
            if stalls >= MAX_STALLS {
                return false;
            }
            std::thread::sleep(Duration::from_millis(400 * stalls as u64));
        }
        true
    }
}

struct Reader {
    fetch: Arc<Fetch>,
    at: u64,
    seekable: Arc<AtomicBool>,
}

impl Read for Reader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut window = self.fetch.window.lock().unwrap();
        loop {
            window.reader = self.at;
            if self.at >= window.start && self.at < window.end() {
                let offset = (self.at - window.start) as usize;
                let mut count = 0;
                for (slot, byte) in out.iter_mut().zip(window.bytes.range(offset..)) {
                    *slot = *byte;
                    count += 1;
                }
                self.at += count as u64;
                window.reader = self.at;
                drop(window);
                self.fetch.changed.notify_all();
                return Ok(count);
            }
            if !self.fetch.current() {
                return Err(io::Error::other("cancelled"));
            }
            if self.at >= self.fetch.len {
                return Ok(0);
            }
            if self.fetch.failed.load(Relaxed) {
                return Err(io::Error::other("The connection was lost while streaming"));
            }
            self.fetch.changed.notify_all();
            window = self.fetch.changed.wait_timeout(window, WINDOW_POLL).unwrap().0;
        }
    }
}

impl Seek for Reader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::Current(delta) => self.at as i64 + delta,
            SeekFrom::End(delta) => self.fetch.len as i64 + delta,
        };
        self.at = target.clamp(0, self.fetch.len as i64) as u64;
        Ok(self.at)
    }
}

impl MediaSource for Reader {
    fn is_seekable(&self) -> bool {
        self.seekable.load(Relaxed)
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.fetch.len)
    }
}

struct Track {
    video_id: String,
    generation: u64,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    id: u32,
    time_base: Option<TimeBase>,
    finished: bool,
}

impl Track {
    fn open(yt: &ytm::Client, urls: &Urls, video_id: &str, generation: u64, shared: &Arc<Shared>) -> Result<Track, String> {
        let (url, len, duration_ms) = resolve(yt, urls, video_id)?;
        if shared.generation.load(Relaxed) == generation {
            shared.duration_ms.store(duration_ms, Relaxed);
        }
        let seekable = Arc::new(AtomicBool::new(false));
        let fetch = Fetch::start(yt, url, len, generation, shared.clone());
        let reader = Reader {
            fetch,
            at: 0,
            seekable: seekable.clone(),
        };
        let source = MediaSourceStream::new(Box::new(reader), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("m4a");
        let format = symphonia::default::get_probe()
            .format(&hint, source, &FormatOptions::default(), &MetadataOptions::default())
            .map_err(|e| e.to_string())?
            .format;
        seekable.store(true, Relaxed);
        let track = format.default_track().ok_or("This stream has no audio track")?;
        let (id, time_base) = (track.id, track.codec_params.time_base);
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| e.to_string())?;
        Ok(Track {
            video_id: video_id.to_owned(),
            generation,
            format,
            decoder,
            id,
            time_base,
            finished: false,
        })
    }

    fn next_chunk(&mut self, epoch: u64) -> Result<Option<Chunk>, String> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(DecodeError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id() != self.id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(DecodeError::DecodeError(_)) => continue,
                Err(e) => return Err(e.to_string()),
            };
            let spec = *decoded.spec();
            let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            buffer.copy_interleaved_ref(decoded);
            let time = self.time_base.map(|base| base.calc_time(packet.ts()));
            let start_ms = time.map_or(0, |t| t.seconds * 1000 + (t.frac * 1000.0) as u64);
            return Ok(Some(Chunk {
                samples: buffer.samples().to_vec(),
                start_ms,
                epoch,
                rate: spec.rate,
                channels: spec.channels.count() as u16,
                end: false,
            }));
        }
    }

    fn seek(&mut self, ms: u64) {
        let time = Time::new(ms / 1000, (ms % 1000) as f64 / 1000.0);
        if self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.id),
                },
            )
            .is_ok()
        {
            self.decoder.reset();
            self.finished = false;
        }
    }
}

fn resolve(yt: &ytm::Client, urls: &Urls, video_id: &str) -> Result<(String, u64, u64), String> {
    if let Some((at, url, len, duration_ms)) = urls.lock().unwrap().get(video_id)
        && at.elapsed() < URL_LIFETIME
    {
        return Ok((url.clone(), *len, *duration_ms));
    }
    let stream = yt.stream(video_id)?;
    let mut urls = urls.lock().unwrap();
    if urls.len() >= URL_CACHE_ENTRIES {
        urls.clear();
    }
    urls.insert(
        video_id.to_owned(),
        (Instant::now(), stream.url.clone(), stream.len, stream.duration_ms),
    );
    Ok((stream.url, stream.len, stream.duration_ms))
}

struct PcmSource {
    chunks: Chunks,
    samples: Vec<f32>,
    at: usize,
    rate: u32,
    channels: u16,
    shared: Arc<Shared>,
    wake: Wake,
}

impl PcmSource {
    fn new(chunks: Chunks, shared: Arc<Shared>, wake: Wake) -> Self {
        Self {
            chunks,
            samples: vec![0.0; SILENCE_SAMPLES],
            at: 0,
            rate: 44_100,
            channels: 2,
            shared,
            wake,
        }
    }

    fn refill(&mut self) {
        self.at = 0;
        while let Ok(chunk) = self.chunks.lock().unwrap().try_recv() {
            if chunk.epoch != self.shared.epoch.load(Relaxed) {
                continue;
            }
            if chunk.end {
                self.shared.ended.store(true, Relaxed);
                (self.wake)();
                break;
            }
            if chunk.samples.is_empty() {
                continue;
            }
            self.shared.position_ms.store(chunk.start_ms, Relaxed);
            self.rate = chunk.rate;
            self.channels = chunk.channels;
            self.samples = chunk.samples;
            return;
        }
        self.samples.clear();
        self.samples.resize(SILENCE_SAMPLES, 0.0);
    }
}

impl Iterator for PcmSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.at >= self.samples.len() {
            self.refill();
        }
        let sample = self.samples[self.at];
        self.at += 1;
        Some(sample)
    }
}

impl rodio::Source for PcmSource {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.samples.len())
    }

    fn channels(&self) -> rodio::ChannelCount {
        self.channels
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        self.rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

fn decode_loop(
    commands: Receiver<Cmd>,
    replies: Sender<Cmd>,
    pcm: SyncSender<Chunk>,
    shared: Arc<Shared>,
    yt: Arc<ytm::Client>,
    urls: Urls,
    wake: Wake,
) {
    let mut track: Option<Track> = None;
    let mut pending: Option<Chunk> = None;
    let mut resume: Option<u64> = None;
    let mut epoch = 0;
    loop {
        if let Some(chunk) = pending.take() {
            match pcm.try_send(chunk) {
                Ok(()) => {}
                Err(TrySendError::Full(chunk)) => pending = Some(chunk),
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        let decoding = track.as_ref().is_some_and(|t| !t.finished);
        let command = if pending.is_some() {
            match commands.recv_timeout(Duration::from_millis(10)) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else if decoding {
            match commands.try_recv() {
                Ok(command) => Some(command),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            match commands.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        let current = |generation: u64| shared.generation.load(Relaxed) == generation;
        match command {
            Some(Cmd::Load {
                video_id,
                generation,
                epoch: started,
            }) => {
                (track, pending, resume, epoch) = (None, None, None, started);
                let (replies, shared, yt, urls) = (replies.clone(), shared.clone(), yt.clone(), urls.clone());
                std::thread::spawn(move || {
                    let opened = Track::open(&yt, &urls, &video_id, generation, &shared).or_else(|message| {
                        urls.lock().unwrap().remove(&video_id);
                        if shared.generation.load(Relaxed) == generation && yt.refresh_clients() {
                            Track::open(&yt, &urls, &video_id, generation, &shared)
                        } else {
                            Err(message)
                        }
                    });
                    let reply = match opened {
                        Ok(track) => Cmd::Opened(track),
                        Err(message) => {
                            urls.lock().unwrap().remove(&video_id);
                            Cmd::Failed { generation, message }
                        }
                    };
                    let _ = replies.send(reply);
                });
            }
            Some(Cmd::Opened(mut opened)) if current(opened.generation) => {
                if let Some(ms) = resume.take() {
                    opened.seek(ms);
                }
                track = Some(opened);
                shared.loading.store(false, Relaxed);
                wake();
            }
            Some(Cmd::Failed { generation, message }) if current(generation) => {
                *shared.error.lock().unwrap() = Some(message);
                shared.loading.store(false, Relaxed);
                wake();
            }
            Some(Cmd::Seek { ms, epoch: flushed }) => {
                (pending, epoch) = (None, flushed);
                match &mut track {
                    Some(track) => track.seek(ms),
                    None => resume = Some(ms),
                }
            }
            _ => {}
        }
        if pending.is_some() {
            continue;
        }
        let Some(playing) = track.as_mut().filter(|t| !t.finished) else {
            continue;
        };
        match playing.next_chunk(epoch) {
            Ok(Some(chunk)) => pending = Some(chunk),
            Ok(None) => {
                playing.finished = true;
                pending = Some(Chunk {
                    samples: Vec::new(),
                    start_ms: 0,
                    epoch,
                    rate: 0,
                    channels: 0,
                    end: true,
                });
            }
            Err(message) => {
                if current(playing.generation) {
                    urls.lock().unwrap().remove(&playing.video_id);
                    *shared.error.lock().unwrap() = Some(message);
                    wake();
                }
                track = None;
            }
        }
    }
}

fn default_device() -> Option<String> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    rodio::cpal::default_host().default_output_device()?.name().ok()
}

pub struct Player {
    commands: Sender<Cmd>,
    shared: Arc<Shared>,
    output: Option<(rodio::OutputStream, rodio::Sink)>,
    device: Option<String>,
    device_lost: Arc<AtomicBool>,
    device_polled: Instant,
    chunks: Chunks,
    wake: Wake,
    volume: f32,
    yt: Arc<ytm::Client>,
    urls: Urls,
}

impl Player {
    pub fn new(yt: Arc<ytm::Client>, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let wake: Wake = Arc::new(wake);
        let shared = Arc::new(Shared::default());
        let urls = Urls::default();
        let (commands, inbox) = mpsc::channel();
        let (pcm, chunks) = mpsc::sync_channel(QUEUED_CHUNKS);
        let worker = (commands.clone(), shared.clone(), yt.clone(), urls.clone(), wake.clone());
        std::thread::spawn(move || decode_loop(inbox, worker.0, pcm, worker.1, worker.2, worker.3, worker.4));
        let mut player = Self {
            commands,
            shared,
            output: None,
            device: None,
            device_lost: Arc::default(),
            device_polled: Instant::now(),
            chunks: Arc::new(Mutex::new(chunks)),
            wake,
            volume: 1.0,
            yt,
            urls,
        };
        player.open_output();
        if player.output.is_none() {
            *player.shared.error.lock().unwrap() = Some(NO_OUTPUT.to_owned());
        }
        player
    }

    fn open_output(&mut self) {
        self.device_lost.store(false, Relaxed);
        let lost = self.device_lost.clone();
        let notice_loss = move |error| {
            lost.fetch_or(matches!(error, rodio::cpal::StreamError::DeviceNotAvailable), Relaxed);
        };
        let stream = rodio::OutputStreamBuilder::from_default_device()
            .and_then(|device| device.with_error_callback(notice_loss).open_stream_or_fallback())
            .or_else(|_| rodio::OutputStreamBuilder::open_default_stream());
        self.output = stream.ok().map(|mut stream| {
            stream.log_on_drop(false);
            let sink = rodio::Sink::connect_new(stream.mixer());
            sink.pause();
            sink.set_volume(self.volume * self.volume);
            sink.append(PcmSource::new(self.chunks.clone(), self.shared.clone(), self.wake.clone()));
            (stream, sink)
        });
        self.device = default_device().filter(|_| self.output.is_some());
    }

    pub fn follow_default_device(&mut self) {
        if self.device_polled.elapsed() < DEVICE_POLL {
            return;
        }
        self.device_polled = Instant::now();
        if default_device() == self.device && !self.device_lost.load(Relaxed) {
            return;
        }
        let paused = self.paused();
        let played = self.output.take().is_some();
        self.open_output();
        self.set_paused(paused);
        if played && self.output.is_none() {
            *self.shared.error.lock().unwrap() = Some(NO_OUTPUT.to_owned());
        }
    }

    pub fn load(&self, video_id: &str) {
        let generation = self.shared.generation.fetch_add(1, Relaxed) + 1;
        let epoch = self.shared.epoch.fetch_add(1, Relaxed) + 1;
        self.shared.position_ms.store(0, Relaxed);
        self.shared.duration_ms.store(0, Relaxed);
        self.shared.buffered.store(0, Relaxed);
        self.shared.ended.store(false, Relaxed);
        self.shared.loading.store(true, Relaxed);
        self.shared.error.lock().unwrap().take();
        let _ = self.commands.send(Cmd::Load {
            video_id: video_id.to_owned(),
            generation,
            epoch,
        });
        self.set_paused(false);
    }

    pub fn preload(&self, video_id: &str) {
        let (yt, urls, video_id) = (self.yt.clone(), self.urls.clone(), video_id.to_owned());
        std::thread::spawn(move || resolve(&yt, &urls, &video_id));
    }

    pub fn seek(&self, ms: u64) {
        let epoch = self.shared.epoch.fetch_add(1, Relaxed) + 1;
        self.shared.position_ms.store(ms, Relaxed);
        self.shared.ended.store(false, Relaxed);
        let _ = self.commands.send(Cmd::Seek { ms, epoch });
    }

    pub fn set_paused(&self, paused: bool) {
        if let Some((_, sink)) = &self.output {
            if paused { sink.pause() } else { sink.play() }
        }
    }

    pub fn paused(&self) -> bool {
        self.output.as_ref().is_none_or(|(_, sink)| sink.is_paused())
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        if let Some((_, sink)) = &self.output {
            sink.set_volume(volume * volume);
        }
    }

    pub fn position_ms(&self) -> u64 {
        self.shared.position_ms.load(Relaxed)
    }

    pub fn duration_ms(&self) -> u64 {
        self.shared.duration_ms.load(Relaxed)
    }

    pub fn buffered(&self) -> f32 {
        self.shared.buffered.load(Relaxed) as f32 / 1000.0
    }

    pub fn loading(&self) -> bool {
        self.shared.loading.load(Relaxed)
    }

    pub fn take_ended(&self) -> bool {
        self.shared.ended.swap(false, Relaxed)
    }

    pub fn take_error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().take()
    }
}

pub fn selftest(query: &str) -> Result<String, String> {
    let started = Instant::now();
    let yt = ytm::Client::new(String::new());
    let song = yt.search(query)?.songs().into_iter().next().ok_or("search returned no songs")?;
    let searched = started.elapsed();
    let radio = yt.radio(&song.video_id)?;
    let shared = Arc::new(Shared::default());
    let opening = Instant::now();
    let mut track = Track::open(&yt, &Urls::default(), &song.video_id, 0, &shared)?;
    let opened = opening.elapsed();
    let mut frames = 0u64;
    let mut peak = 0f32;
    let mut rate = 0;
    while frames < 3 * 44_100 {
        let chunk = track.next_chunk(0)?.ok_or("stream ended early")?;
        frames += (chunk.samples.len() / chunk.channels.max(1) as usize) as u64;
        peak = chunk.samples.iter().fold(peak, |peak, sample| peak.max(sample.abs()));
        rate = chunk.rate;
    }
    if peak <= 0.001 {
        return Err("decoded audio is silent".to_owned());
    }
    let duration = shared.duration_ms.load(Relaxed);
    let far = if duration > SELFTEST_FAR_SEEK_MS + 60_000 {
        SELFTEST_FAR_SEEK_MS
    } else {
        60_000
    };
    let seeking = Instant::now();
    for target in [60_000, 5_000, far, 5_000] {
        track.seek(target);
        let landed = track.next_chunk(0)?.ok_or("seek ran past the end")?.start_ms;
        if landed.abs_diff(target) > 1000 {
            return Err(format!("seek to {target} ms landed at {landed} ms"));
        }
    }
    Ok(format!(
        "ok: \"{}\" by {} | search {searched:.0?} | open {opened:.0?} | {rate} Hz, peak {peak:.2} | seeks to 60s, 5s, {}s, 5s in {:.0?} | radio {} tracks | duration {duration} ms",
        song.title,
        song.subtitle,
        far / 1000,
        seeking.elapsed(),
        radio.len(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reopened_output_keeps_playing_the_queued_audio() {
        let shared = Arc::new(Shared::default());
        let (pcm, chunks) = mpsc::sync_channel(QUEUED_CHUNKS);
        let chunks: Chunks = Arc::new(Mutex::new(chunks));
        let open = || PcmSource::new(chunks.clone(), shared.clone(), Arc::new(|| {}));
        let queue = |start_ms| {
            let chunk = Chunk {
                samples: vec![0.5; 4],
                start_ms,
                epoch: 0,
                rate: 48_000,
                channels: 2,
                end: false,
            };
            pcm.try_send(chunk).is_ok()
        };
        assert!(queue(1000) && queue(2000));
        let mut first = open();
        assert_eq!(first.by_ref().take(SILENCE_SAMPLES + 4).filter(|sample| *sample == 0.5).count(), 4);
        drop(first);
        let mut second = open();
        assert_eq!(second.by_ref().take(SILENCE_SAMPLES + 4).filter(|sample| *sample == 0.5).count(), 4);
        assert_eq!(shared.position_ms.load(Relaxed), 2000);
        assert!(queue(3000));
    }
}
