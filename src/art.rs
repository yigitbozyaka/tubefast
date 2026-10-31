use crate::ytm;
use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureId, TextureOptions};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const WORKERS: usize = 4;
const BUDGET_BYTES: usize = 16 * 1024 * 1024;
const MAX_DOWNLOAD_BYTES: u64 = 6 * 1024 * 1024;
const RETRY_AFTER: Duration = Duration::from_secs(20);
// ponytail: the 8 px size is the blur, stretched with linear filtering
const SIZES: [u32; 5] = [8, 64, 128, 256, 512];

type Sized = (String, u32);
type Jobs = Arc<(Mutex<Vec<Sized>>, Condvar)>;
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

pub struct Art {
    cache: RefCell<Cache>,
    jobs: Jobs,
    loaded: Receiver<Loaded>,
}

impl Art {
    pub fn new(ctx: egui::Context, agent: ureq::Agent) -> Self {
        let jobs = Jobs::default();
        let (sender, loaded) = mpsc::channel();
        for _ in 0..WORKERS {
            let (ctx, agent, jobs, sender) = (ctx.clone(), agent.clone(), jobs.clone(), sender.clone());
            std::thread::spawn(move || work(ctx, agent, jobs, sender));
        }
        Self {
            cache: RefCell::default(),
            jobs,
            loaded,
        }
    }

    pub fn begin_frame(&self, ctx: &egui::Context) {
        let mut cache = self.cache.borrow_mut();
        cache.frame += 1;
        let frame = cache.frame;
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
                None => Slot::Failed(Instant::now()),
            };
            cache.slots.insert(key, slot);
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
        self.jobs.0.lock().unwrap().push(key);
        self.jobs.1.notify_one();
        None
    }
}

fn work(ctx: egui::Context, agent: ureq::Agent, jobs: Jobs, sender: Sender<Loaded>) {
    loop {
        let key = {
            let mut queue = jobs.0.lock().unwrap();
            loop {
                match queue.pop() {
                    Some(job) => break job,
                    None => queue = jobs.1.wait(queue).unwrap(),
                }
            }
        };
        let result = load(&agent, &key.0, key.1);
        if sender.send((key, result)).is_err() {
            return;
        }
        ctx.request_repaint();
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
