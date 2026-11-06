use crate::ytm;
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureId, TextureOptions};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const WORKERS: usize = 4;
const MAX_QUEUED_JOBS: usize = 32;
const MAX_FAILED: usize = 128;
const BUDGET_BYTES: usize = 16 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 6 * 1024 * 1024;
const RETRY_AFTER: Duration = Duration::from_secs(20);
// ponytail: the 8 px size is the blur, stretched with linear filtering
const SIZES: [u32; 5] = [8, 64, 128, 256, 512];

type Sized = (String, u32);
type Jobs = Arc<(Mutex<VecDeque<Sized>>, Condvar)>;
type Loaded = (Sized, Option<(ColorImage, Color32)>);

#[derive(Clone, Copy)]
pub struct Picture {
    pub id: TextureId,
    pub tint: Color32,
}

enum Slot {
    Loading,
    Failed(Instant),
    Ready {
        texture: TextureHandle,
        tint: Color32,
        used: u64,
        bytes: usize,
    },
}

#[derive(Default)]
struct Cache {
    slots: HashMap<Sized, Slot>,
    frame: u64,
    bytes: usize,
}

type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct Art {
    cache: RefCell<Cache>,
    jobs: Jobs,
    closed: Arc<AtomicBool>,
    loaded: Receiver<Loaded>,
}

impl Art {
    pub fn new(wake: impl Fn() + Send + Sync + 'static, agent: ureq::Agent) -> Self {
        Self::start(Some((Arc::new(wake), agent)))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn idle() -> Self {
        Self::start(None)
    }

    fn start(workers: Option<(Wake, ureq::Agent)>) -> Self {
        let jobs = Jobs::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (sender, loaded) = mpsc::channel();
        if let Some((wake, agent)) = workers {
            for _ in 0..WORKERS {
                let (wake, agent, jobs, sender, closed) = (wake.clone(), agent.clone(), jobs.clone(), sender.clone(), closed.clone());
                std::thread::spawn(move || work(wake, agent, jobs, sender, closed));
            }
        }
        Self {
            cache: RefCell::default(),
            jobs,
            closed,
            loaded,
        }
    }

    pub fn begin_frame(&self, ctx: &egui::Context) {
        let mut cache = self.cache.borrow_mut();
        cache.frame += 1;
        let frame = cache.frame;
        let mut failed_loaded = false;
        while let Ok((key, result)) = self.loaded.try_recv() {
            let slot = match result {
                Some((image, tint)) => {
                    let bytes = image.width() * image.height() * 4;
                    cache.bytes += bytes;
                    Slot::Ready {
                        texture: ctx.load_texture(&key.0, image, TextureOptions::LINEAR),
                        tint,
                        used: frame,
                        bytes,
                    }
                }
                None => {
                    failed_loaded = true;
                    Slot::Failed(Instant::now())
                }
            };
            cache.slots.insert(key, slot);
        }
        if failed_loaded {
            prune_failed(&mut cache);
        }
        if cache.bytes > BUDGET_BYTES {
            let mut idle: Vec<(u64, usize, Sized)> = cache
                .slots
                .iter()
                .filter_map(|(key, slot)| match slot {
                    Slot::Ready { used, bytes, .. } if used + 1 < frame => Some((*used, *bytes, key.clone())),
                    _ => None,
                })
                .collect();
            idle.sort_unstable();
            for (_, bytes, key) in idle {
                if cache.bytes <= BUDGET_BYTES * 3 / 4 {
                    break;
                }
                cache.slots.remove(&key);
                cache.bytes -= bytes;
            }
        }
    }

    pub fn get(&self, ctx: &egui::Context, url: &str, points: f32) -> Option<Picture> {
        if url.is_empty() {
            return None;
        }
        let wanted = (points * ctx.pixels_per_point()).ceil() as u32;
        let px = SIZES.into_iter().find(|size| *size >= wanted).unwrap_or(SIZES[SIZES.len() - 1]);
        let key = (ytm::sized(url, px), px);
        let mut cache = self.cache.borrow_mut();
        let frame = cache.frame;
        match cache.slots.get_mut(&key) {
            Some(Slot::Ready { texture, tint, used, .. }) => {
                *used = frame;
                return Some(Picture {
                    id: texture.id(),
                    tint: *tint,
                });
            }
            Some(Slot::Loading) => return None,
            Some(Slot::Failed(at)) if at.elapsed() < RETRY_AFTER => return None,
            _ => {}
        }
        cache.slots.insert(key.clone(), Slot::Loading);
        let dropped = {
            let mut queue = self.jobs.0.lock().unwrap();
            let dropped = enqueue(&mut queue, key);
            self.jobs.1.notify_one();
            dropped
        };
        if let Some(dropped) = dropped
            && matches!(cache.slots.get(&dropped), Some(Slot::Loading))
        {
            cache.slots.remove(&dropped);
        }
        None
    }
}

fn enqueue(queue: &mut VecDeque<Sized>, key: Sized) -> Option<Sized> {
    let dropped = (queue.len() >= MAX_QUEUED_JOBS).then(|| queue.pop_front()).flatten();
    queue.push_back(key);
    dropped
}

fn prune_failed(cache: &mut Cache) {
    let now = Instant::now();
    cache
        .slots
        .retain(|_, slot| !matches!(slot, Slot::Failed(at) if now.duration_since(*at) >= RETRY_AFTER));
    let mut failed: Vec<(Instant, Sized)> = cache
        .slots
        .iter()
        .filter_map(|(key, slot)| match slot {
            Slot::Failed(at) => Some((*at, key.clone())),
            _ => None,
        })
        .collect();
    if failed.len() > MAX_FAILED {
        failed.sort_unstable_by_key(|(at, _)| *at);
        let excess = failed.len() - MAX_FAILED;
        for (_, key) in failed.into_iter().take(excess) {
            cache.slots.remove(&key);
        }
    }
}

impl Drop for Art {
    fn drop(&mut self) {
        {
            let _queue = self.jobs.0.lock().unwrap();
            self.closed.store(true, Ordering::SeqCst);
        }
        self.jobs.1.notify_all();
    }
}

fn work(wake: Wake, agent: ureq::Agent, jobs: Jobs, sender: Sender<Loaded>, closed: Arc<AtomicBool>) {
    loop {
        let key = {
            let mut queue = jobs.0.lock().unwrap();
            loop {
                if closed.load(Ordering::SeqCst) {
                    return;
                }
                match queue.pop_back() {
                    Some(job) => break job,
                    None => queue = jobs.1.wait(queue).unwrap(),
                }
            }
        };
        let result = load(&agent, &key.0, key.1);
        if sender.send((key, result)).is_err() {
            return;
        }
        wake();
    }
}

fn fetch(agent: &ureq::Agent, url: &str) -> Option<image::DynamicImage> {
    let mut bytes = Vec::new();
    agent
        .get(url)
        .call()
        .ok()?
        .into_reader()
        .take(MAX_DOWNLOAD_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    image::load_from_memory(&bytes).ok()
}

fn load(agent: &ureq::Agent, url: &str, px: u32) -> Option<(ColorImage, Color32)> {
    let sharp = ytm::sharper(url, px).and_then(|sharp| fetch(agent, &sharp));
    let decoded = sharp.or_else(|| fetch(agent, url))?;
    let side = decoded.width().min(decoded.height());
    let square = decoded.crop_imm((decoded.width() - side) / 2, (decoded.height() - side) / 2, side, side);
    let fitted = if side > px { square.thumbnail_exact(px, px) } else { square };
    let rgba = fitted.to_rgba8();
    let mut sum = [0u64; 3];
    let mut count = 0u64;
    for pixel in rgba.pixels().step_by(5) {
        for channel in 0..3 {
            sum[channel] += pixel[channel] as u64;
        }
        count += 1;
    }
    let mean = sum.map(|total| (total / count.max(1)) as u8);
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some((
        ColorImage::from_rgba_unmultiplied(size, &rgba),
        Color32::from_rgb(mean[0], mean[1], mean[2]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_queue_drops_oldest_when_full() {
        let mut queue: VecDeque<Sized> = (0..MAX_QUEUED_JOBS).map(|index| (index.to_string(), 64)).collect();

        let dropped = enqueue(&mut queue, ("new".to_owned(), 64));

        assert_eq!(dropped, Some(("0".to_owned(), 64)));
        assert_eq!(queue.len(), MAX_QUEUED_JOBS);
        assert_eq!(queue.pop_back(), Some(("new".to_owned(), 64)));
    }

    #[test]
    fn failed_art_entries_are_expired_and_capped() {
        let mut cache = Cache::default();
        for index in 0..=MAX_FAILED {
            cache.slots.insert((index.to_string(), 64), Slot::Failed(Instant::now()));
        }

        prune_failed(&mut cache);

        assert_eq!(cache.slots.len(), MAX_FAILED);

        let expired = Instant::now() - RETRY_AFTER - Duration::from_secs(1);
        cache.slots.insert(("expired".to_owned(), 64), Slot::Failed(expired));
        prune_failed(&mut cache);

        assert_eq!(cache.slots.len(), MAX_FAILED);
        assert!(!cache.slots.contains_key(&("expired".to_owned(), 64)));
    }
}
