use crate::APP_NAME;
use crate::app::{Action, App, Covers, Load, Repeat, Route, Words};
use crate::art::Art;
use crate::discord::Headline;
use crate::ytm::{Item, Layout};
use eframe::egui::ecolor::Hsva;
use eframe::egui::epaint::{RectShape, Shadow};
use eframe::egui::style::ScrollStyle;
use eframe::egui::text::{CCursor, LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{
    self, Align, Align2, Color32, FontData, FontDefinitions, FontFamily, FontId, Frame, Galley, Id, Key, Margin, Pos2, Rect, Response,
    RichText, Sense, Shape, Stroke, StrokeKind, TextStyle, Ui, UiBuilder, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;
use std::ops::Range;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

const BASE: Color32 = rgb(0x0E0F12);
const PANEL: Color32 = rgb(0x0A0B0D);
const RAISED: Color32 = rgb(0x1A1B20);
const TEXT: Color32 = rgb(0xECEDEF);
const MUTED: Color32 = Color32::from_rgba_premultiplied(166, 166, 166, 166);
const FAINT: Color32 = Color32::from_rgba_premultiplied(122, 122, 122, 122);
const ACCENT: Color32 = rgb(0xF4623F);
const ON_ACCENT: Color32 = rgb(0x1A0A05);
const FLOATING: Color32 = rgb(0x1B1C21);

const SIDEBAR_WIDTH: f32 = 244.0;
const QUEUE_WIDTH: f32 = 336.0;
const QUEUE_MIN_WINDOW: f32 = 1140.0;
const BAR_HEIGHT: f32 = 84.0;
const ACCOUNT_HEIGHT: f32 = 68.0;
const UPDATE_HEIGHT: f32 = 50.0;
const TOPBAR_HEIGHT: f32 = 68.0;
const AMBIENT_HEIGHT: f32 = 380.0;
const HERO_ART: f32 = 216.0;
const BAR_ART: f32 = 52.0;
const ROW_HEIGHT: f32 = 56.0;
const ROW_RADIUS: f32 = 8.0;
const ART_RADIUS: f32 = 8.0;
const CARD_MIN: f32 = 164.0;
const CARD_GAP: f32 = 20.0;
const CARD_TEXT: f32 = 62.0;
const COMPACT_MIN: f32 = 320.0;
const COMPACT_ROWS: usize = 3;
const PAGE_MARGIN: i8 = 32;
const HOVER_TIME: f32 = 0.1;
const MENU_WIDTH: f32 = 248.0;
const QUEUE_ROW: f32 = 52.0;
const STICKY_HEIGHT: f32 = 60.0;
const STAGE_ART: f32 = 512.0;
const STAGE_LUMINANCE: f32 = 0.14;
const PAGE_FADE: f64 = 0.12;

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn veil(opacity: f32) -> Color32 {
    Color32::from_white_alpha((opacity.clamp(0.0, 1.0) * 255.0) as u8)
}

fn shade(opacity: f32) -> Color32 {
    Color32::from_black_alpha((opacity.clamp(0.0, 1.0) * 255.0) as u8)
}

static FAMILIES: LazyLock<[FontFamily; 5]> =
    LazyLock::new(|| ["medium", "bold", "display", "icons", "icons-fill"].map(|name| FontFamily::Name(name.into())));

fn sans(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

fn medium(size: f32) -> FontId {
    FontId::new(size, FAMILIES[0].clone())
}

fn bold(size: f32) -> FontId {
    FontId::new(size, FAMILIES[1].clone())
}

fn display(size: f32) -> FontId {
    FontId::new(size, FAMILIES[2].clone())
}

fn glyph(size: f32) -> FontId {
    FontId::new(size, FAMILIES[3].clone())
}

fn solid(size: f32) -> FontId {
    FontId::new(size, FAMILIES[4].clone())
}

fn system_font(file: &str) -> Option<&'static [u8]> {
    let fonts = if cfg!(target_os = "macos") {
        std::path::PathBuf::from("/System/Library/Fonts")
    } else {
        std::path::Path::new(&std::env::var_os("WINDIR")?).join("Fonts")
    };
    mapped(&fonts.join(file))
}

fn mapped(path: &std::path::Path) -> Option<&'static [u8]> {
    let file = std::fs::File::open(path).ok()?;
    let map = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    Some(&Box::leak(Box::new(map))[..])
}

/// Fonts each system ships for the scripts the fonts above lack, in fallback order. Every Noto font follows them, the sans ones first.
const FALLBACK_FONTS: [&str; 43] = [
    // Linux
    "NotoSansCJK-Regular.ttc",
    "NotoSansCJK-VF.ttc",
    "DroidSansFallbackFull.ttf",
    "wqy-microhei.ttc",
    "NanumGothic.ttf",
    "Loma.ttf",
    "Lohit-Devanagari.ttf",
    "DejaVuSans.ttf",
    "FreeSans.ttf",
    // macOS
    "SFArabic.ttf",
    "GeezaPro.ttc",
    "SFHebrew.ttf",
    "SFArmenian.ttf",
    "SFGeorgian.ttf",
    "KohinoorBangla.ttc",
    "KohinoorGujarati.ttc",
    "KohinoorTelugu.ttc",
    "MuktaMahee.ttc",
    "Tamil Sangam MN.ttc",
    "Kannada Sangam MN.ttc",
    "Malayalam Sangam MN.ttc",
    "Oriya Sangam MN.ttc",
    "Sinhala Sangam MN.ttc",
    "Khmer Sangam MN.ttf",
    "Lao Sangam MN.ttf",
    "Myanmar Sangam MN.ttc",
    "KefaIII.ttf",
    "Kailasa.ttc",
    "Arial Unicode.ttf",
    // Windows
    "msjh.ttc",
    "YuGothR.ttc",
    "ebrima.ttf",
    "mmrtext.ttf",
    "himalaya.ttf",
    "monbaiti.ttf",
    "gadugi.ttf",
    "javatext.ttf",
    "msyi.ttf",
    "taile.ttf",
    "ntailu.ttf",
    "phagspa.ttf",
    "seguihis.ttf",
    "simsunb.ttf",
];

fn fallback_fonts() -> Vec<(String, &'static [u8])> {
    let mut dirs: Vec<std::path::PathBuf> = [
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        "/System/Library/Fonts",
        "/Library/Fonts",
    ]
    .map(Into::into)
    .into();
    for (var, below) in [
        ("HOME", ".local/share/fonts"),
        ("HOME", ".fonts"),
        ("HOME", "Library/Fonts"),
        ("WINDIR", "Fonts"),
    ] {
        dirs.extend(std::env::var_os(var).map(|root| std::path::Path::new(&root).join(below)));
    }
    let mut found = Vec::new();
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let noto = name.starts_with("Noto")
                && !name.contains("Italic")
                && ["-Regular.ttf", "-Regular.otf", "[wght].ttf"].iter().any(|end| name.ends_with(end));
            let listed = FALLBACK_FONTS.iter().position(|font| name.eq_ignore_ascii_case(font));
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                dirs.push(entry.path());
            } else if listed.is_some() || noto {
                let rank = listed.unwrap_or(FALLBACK_FONTS.len() + usize::from(!name.starts_with("NotoSans")));
                found.push((rank, name.to_lowercase(), entry.path()));
            }
        }
    }
    found.sort();
    found.dedup_by(|a, b| a.1 == b.1);
    found
        .into_iter()
        .filter_map(|(_, name, path)| Some((name, mapped(&path)?)))
        .collect()
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let embedded: [(&str, &'static [u8]); 8] = [
        ("onest-400", include_bytes!("../assets/fonts/onest-latin-400.ttf")),
        ("onest-400-ext", include_bytes!("../assets/fonts/onest-latin-ext-400.ttf")),
        ("onest-500", include_bytes!("../assets/fonts/onest-latin-500.ttf")),
        ("onest-500-ext", include_bytes!("../assets/fonts/onest-latin-ext-500.ttf")),
        ("onest-600", include_bytes!("../assets/fonts/onest-latin-600.ttf")),
        ("onest-600-ext", include_bytes!("../assets/fonts/onest-latin-ext-600.ttf")),
        (
            "shoulders-800",
            include_bytes!("../assets/fonts/big-shoulders-display-latin-800.ttf"),
        ),
        (
            "shoulders-800-ext",
            include_bytes!("../assets/fonts/big-shoulders-display-latin-ext-800.ttf"),
        ),
    ];
    for (name, bytes) in embedded {
        fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    }
    fonts
        .font_data
        .insert("phosphor".to_owned(), Arc::new(egui_phosphor::Variant::Regular.font_data()));
    fonts
        .font_data
        .insert("phosphor-fill".to_owned(), Arc::new(egui_phosphor::Variant::Fill.font_data()));
    let system: &[_] = if cfg!(target_os = "macos") {
        &[
            ("segoe", "SFNS.ttf"),
            ("segoe-bold", "SFNS.ttf"),
            ("yahei", "Hiragino Sans GB.ttc"),
            ("malgun", "AppleSDGothicNeo.ttc"),
            ("thai", "ThonburiUI.ttc"),
            ("indic", "Kohinoor.ttc"),
            ("symbols", "Apple Symbols.ttf"),
        ]
    } else {
        &[
            ("segoe", "segoeui.ttf"),
            ("segoe-bold", "segoeuib.ttf"),
            ("yahei", "msyh.ttc"),
            ("malgun", "malgun.ttf"),
            ("thai", "leelawui.ttf"),
            ("indic", "Nirmala.ttc"),
            ("indic-legacy", "Nirmala.ttf"),
            ("symbols", "seguisym.ttf"),
        ]
    };
    for &(name, file) in system {
        if let Some(bytes) = system_font(file) {
            fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
        }
    }
    let fallback = fallback_fonts();
    for (name, bytes) in &fallback {
        fonts.font_data.insert(name.clone(), Arc::new(FontData::from_static(bytes)));
    }
    let known = fonts.font_data.clone();
    let stack = |own: [&str; 2], system: &str| -> Vec<String> {
        let fallbacks = [
            system,
            "NotoEmoji-Regular",
            "emoji-icon-font",
            "yahei",
            "malgun",
            "thai",
            "indic",
            "indic-legacy",
            "symbols",
            "Ubuntu-Light",
        ];
        own.into_iter()
            .chain(fallbacks)
            .chain(fallback.iter().map(|(name, _)| name.as_str()))
            .filter(|name| known.contains_key(*name))
            .map(str::to_owned)
            .collect()
    };
    fonts
        .families
        .insert(FontFamily::Proportional, stack(["onest-400", "onest-400-ext"], "segoe"));
    fonts
        .families
        .insert(FAMILIES[0].clone(), stack(["onest-500", "onest-500-ext"], "segoe"));
    fonts
        .families
        .insert(FAMILIES[1].clone(), stack(["onest-600", "onest-600-ext"], "segoe-bold"));
    fonts
        .families
        .insert(FAMILIES[2].clone(), stack(["shoulders-800", "shoulders-800-ext"], "segoe-bold"));
    fonts.families.insert(FAMILIES[3].clone(), vec!["phosphor".to_owned()]);
    fonts.families.insert(FAMILIES[4].clone(), vec!["phosphor-fill".to_owned()]);
    ctx.set_fonts(fonts);

    ctx.style_mut(|style| {
        let visuals = &mut style.visuals;
        *visuals = egui::Visuals::dark();
        visuals.panel_fill = BASE;
        visuals.window_fill = FLOATING;
        visuals.extreme_bg_color = PANEL;
        visuals.override_text_color = Some(TEXT);
        visuals.window_stroke = Stroke::new(1.0, veil(0.08));
        visuals.window_corner_radius = 10.into();
        visuals.menu_corner_radius = 13.into();
        visuals.popup_shadow = Shadow {
            offset: [0, 10],
            blur: 28,
            spread: 0,
            color: shade(0.5),
        };
        visuals.selection.bg_fill = ACCENT.gamma_multiply(0.45);
        visuals.selection.stroke = Stroke::new(1.0, TEXT);
        visuals.text_cursor.stroke = Stroke::new(1.5, ACCENT);
        visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
        for (widget, fill) in [
            (&mut visuals.widgets.inactive, Color32::TRANSPARENT),
            (&mut visuals.widgets.hovered, veil(0.08)),
            (&mut visuals.widgets.active, veil(0.12)),
            (&mut visuals.widgets.open, veil(0.08)),
        ] {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.bg_stroke = Stroke::NONE;
            widget.fg_stroke = Stroke::new(1.0, TEXT);
            widget.corner_radius = 6.into();
            widget.expansion = 0.0;
        }
        style.spacing.item_spacing = Vec2::ZERO;
        style.spacing.button_padding = vec2(12.0, 8.0);
        style.spacing.menu_margin = Margin::same(6);
        style.spacing.scroll = ScrollStyle {
            bar_width: 8.0,
            floating_width: 4.0,
            dormant_handle_opacity: 0.0,
            ..ScrollStyle::floating()
        };
        style.interaction.selectable_labels = false;
        style.animation_time = HOVER_TIME;
        style.text_styles = [
            (TextStyle::Body, sans(14.0)),
            (TextStyle::Button, medium(13.5)),
            (TextStyle::Small, sans(12.0)),
            (TextStyle::Heading, bold(20.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
    });
}

pub fn app_icon() -> egui::IconData {
    const SIZE: usize = 128;
    const SAMPLES: usize = 3;
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    for pixel in 0..SIZE * SIZE {
        let (mut covered, mut sum) = (0.0, [0.0; 3]);
        for sample in 0..SAMPLES * SAMPLES {
            let x = (pixel % SIZE) as f32 + ((sample % SAMPLES) as f32 + 0.5) / SAMPLES as f32;
            let y = (pixel / SIZE) as f32 + ((sample / SAMPLES) as f32 + 0.5) / SAMPLES as f32;
            let radius = (x / SIZE as f32 * 2.0 - 1.0).hypot(y / SIZE as f32 * 2.0 - 1.0);
            if radius < 0.96 {
                let color = if radius < 0.17 || (0.53..0.57).contains(&radius) {
                    PANEL
                } else {
                    ACCENT
                };
                covered += 1.0;
                sum = [sum[0] + color.r() as f32, sum[1] + color.g() as f32, sum[2] + color.b() as f32];
            }
        }
        let alpha = covered / (SAMPLES * SAMPLES) as f32 * 255.0;
        rgba.extend(sum.map(|channel| (channel / f32::max(covered, 1.0)) as u8));
        rgba.push(alpha as u8);
    }
    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}

struct Cx<'a> {
    art: &'a Art,
    covers: &'a Covers,
    playing: &'a str,
    paused: bool,
    liked: &'a [Item],
    single: bool,
    out: &'a mut Vec<Action>,
}

impl Cx<'_> {
    fn is_liked(&self, item: &Item) -> bool {
        item.is_song() && self.liked.iter().any(|liked| liked.video_id == item.video_id)
    }

    fn is_playing(&self, item: &Item) -> bool {
        item.is_song() && item.video_id == self.playing
    }

    fn activate(&mut self, items: &[Item], index: usize) {
        let item = &items[index];
        if self.is_playing(item) {
            self.out.push(Action::Toggle);
        } else if item.is_song() {
            let (queue, at) = if self.single {
                (vec![item.clone()], 0)
            } else {
                (items.to_vec(), index)
            };
            self.out.push(Action::Play(queue, at));
        } else if !item.browse_id.is_empty() {
            self.out.push(Action::Go(Route::Browse(item.browse_id.clone())));
        }
    }
}

struct Scrub {
    bar: Rect,
    played: f32,
    buffered: f32,
    hover: f32,
    aim: Option<(f32, u64)>,
}

#[cfg(windows)]
pub struct TitleBar {
    window: isize,
    painted: Option<Color32>,
}

#[cfg(windows)]
impl TitleBar {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let window = match cc.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(window)) => window.hwnd.get(),
            _ => 0,
        };
        Self { window, painted: None }
    }

    fn paint(&mut self, color: Color32) {
        use windows_sys::Win32::Graphics::Dwm::{
            DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
        };
        if self.window == 0 || self.painted == Some(color) {
            return;
        }
        self.painted = Some(color);
        let packed = |color: Color32| color.r() as u32 | (color.g() as u32) << 8 | (color.b() as u32) << 16;
        let attributes = [
            (DWMWA_USE_IMMERSIVE_DARK_MODE, 1),
            (DWMWA_CAPTION_COLOR, packed(color)),
            (DWMWA_TEXT_COLOR, packed(TEXT)),
        ];
        for (attribute, value) in attributes {
            unsafe { DwmSetWindowAttribute(self.window as _, attribute as u32, (&raw const value).cast(), 4) };
        }
    }
}

#[cfg(not(windows))]
pub struct TitleBar;

#[cfg(not(windows))]
impl TitleBar {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self
    }

    fn paint(&mut self, _color: Color32) {}
}

pub fn draw(app: &mut App, ctx: &egui::Context) -> Vec<Action> {
    let mut out = Vec::new();
    let plain = |fill: Color32| Frame::new().fill(fill);
    let scrub = egui::TopBottomPanel::bottom("player")
        .exact_height(BAR_HEIGHT)
        .show_separator_line(false)
        .frame(plain(PANEL))
        .show(ctx, |ui| player_bar(app, ui, &mut out))
        .inner;
    let mut frame = PANEL;
    if app.expanded && app.now_playing().is_some() {
        frame = egui::CentralPanel::default()
            .frame(plain(PANEL))
            .show(ctx, |ui| now_playing(app, ui, &mut out))
            .inner;
    } else {
        egui::SidePanel::left("sidebar")
            .exact_width(SIDEBAR_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(plain(PANEL))
            .show(ctx, |ui| sidebar(app, ui, &mut out));
        let queue_fits = app.saved.queue_open && ctx.content_rect().width() >= QUEUE_MIN_WINDOW;
        egui::SidePanel::right("queue")
            .exact_width(QUEUE_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(plain(PANEL))
            .show_animated(ctx, queue_fits, |ui| queue_panel(app, ui, &mut out));
        egui::CentralPanel::default()
            .frame(plain(BASE))
            .show(ctx, |ui| content(app, ui, &mut out));
    }
    app.title_bar.paint(frame);
    paint_scrubber(ctx, &scrub);
    sign_in_dialog(app, ctx, &mut out);
    notice(app, ctx, &mut out);
    out
}

fn fit(ui: &Ui, text: &str, font: FontId, color: Color32, width: f32, rows: usize) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = TextWrapping {
        max_width: width.max(1.0),
        max_rows: rows,
        break_anywhere: rows == 1,
        overflow_character: Some('…'),
    };
    ui.painter().layout_job(job)
}

fn label(ui: &Ui, at: Pos2, anchor: Align2, text: &str, font: FontId, color: Color32, width: f32) -> Rect {
    let galley = fit(ui, text, font, color, width, 1);
    let rect = anchor.anchor_size(at, galley.size());
    ui.painter().galley(rect.min, galley, color);
    rect
}

fn artist_spans(item: &Item) -> Vec<(Range<usize>, &str)> {
    let subtitle = item.subtitle.as_str();
    if item.artists.is_empty() && !item.artist_id.is_empty() {
        return vec![(0..subtitle.chars().count(), &item.artist_id)];
    }
    let mut searched = 0;
    let mut spans = Vec::new();
    for (name, browse_id) in &item.artists {
        let later = subtitle[searched..].find(name.as_str()).map(|at| searched + at);
        let Some(start) = later.or_else(|| subtitle.find(name.as_str())) else {
            continue;
        };
        searched = start + name.len();
        let first = subtitle[..start].chars().count();
        spans.push((first..first + name.chars().count(), browse_id.as_str()));
    }
    spans
}

fn artist_links(ui: &Ui, out: &mut Vec<Action>, item: &Item, id: Id, origin: Pos2, galley: &Arc<Galley>) {
    if item.is_artist {
        return;
    }
    for (index, (chars, browse_id)) in artist_spans(item).into_iter().enumerate() {
        let edge = |at: usize| galley.pos_from_cursor(CCursor::new(at));
        let rect = Rect::from_min_max(edge(chars.start).min, edge(chars.end).max).translate(origin.to_vec2());
        if rect.width() < 1.0 {
            continue;
        }
        let response = ui.interact(rect, id.with(index), Sense::click());
        let hover = hover_of(ui, &response);
        if hover > 0.0 {
            ui.painter()
                .with_clip_rect(rect)
                .galley_with_override_text_color(origin, galley.clone(), MUTED.lerp_to_gamma(TEXT, hover));
            let line = Stroke::new(1.0, TEXT.gamma_multiply(hover));
            ui.painter().hline(rect.x_range(), rect.bottom() - 1.0, line);
        }
        describe(&response, "Go to artist");
        if response.clicked() {
            out.push(Action::Go(Route::Browse(browse_id.to_owned())));
        }
    }
}

fn byline(ui: &Ui, out: &mut Vec<Action>, item: &Item, id: Id, at: Pos2, font: FontId, width: f32) -> Rect {
    let galley = fit(ui, &item.subtitle, font, MUTED, width, 1);
    let rect = Align2::LEFT_CENTER.anchor_size(at, galley.size());
    ui.painter().galley(rect.min, galley.clone(), MUTED);
    artist_links(ui, out, item, id, rect.min, &galley);
    rect
}

fn paragraph(ui: &mut Ui, text: &str, font: FontId, color: Color32, rows: usize) {
    let galley = fit(ui, text, font, color, ui.available_width(), rows);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), galley.size().y), Sense::hover());
    ui.painter().galley(rect.min, galley, color);
}

fn eyebrow(ui: &mut Ui, text: &str, inset: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let format = TextFormat {
        font_id: bold(11.0),
        color: FAINT,
        extra_letter_spacing: 1.3,
        ..TextFormat::default()
    };
    let mut job = LayoutJob::default();
    job.append(&text.to_uppercase(), 0.0, format);
    let galley = ui.painter().layout_job(job);
    ui.painter()
        .galley(pos2(rect.left() + inset, rect.center().y - galley.size().y / 2.0), galley, FAINT);
}

fn hover_of(ui: &Ui, response: &Response) -> f32 {
    ui.ctx()
        .animate_bool_with_time(response.id, response.hovered() || response.has_focus(), HOVER_TIME)
}

fn describe(response: &Response, name: &str) {
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, name));
    point_at(response);
}

fn point_at(response: &Response) {
    if response.hovered() {
        response.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

fn icon_button(ui: &Ui, rect: Rect, id: Id, symbol: &str, font: FontId, color: Color32, hint: &str) -> Response {
    let response = ui.interact(rect, id, Sense::click());
    let hover = hover_of(ui, &response);
    let pressed = if response.is_pointer_button_down_on() { 1.0 } else { 0.0 };
    ui.painter()
        .circle_filled(rect.center(), rect.width() / 2.0 - pressed, veil(0.09 * hover));
    let tone = if color == MUTED || color == FAINT {
        color.lerp_to_gamma(TEXT, hover)
    } else {
        color
    };
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, symbol, font, tone);
    describe(&response, hint);
    response.on_hover_text(hint)
}

fn text_button(ui: &Ui, right_center: Pos2, id: Id, text: &str) -> Response {
    let galley = fit(ui, text, medium(13.0), MUTED, 120.0, 1);
    let slot = Align2::RIGHT_CENTER
        .anchor_size(right_center, galley.size())
        .expand2(vec2(10.0, 6.0));
    let response = ui.interact(slot, id, Sense::click());
    let hover = hover_of(ui, &response);
    ui.painter().rect_filled(slot, 14.0, veil(0.07 * hover));
    ui.painter()
        .galley_with_override_text_color(slot.shrink2(vec2(10.0, 6.0)).min, galley, MUTED.lerp_to_gamma(TEXT, hover));
    describe(&response, text);
    response
}

fn pill(ui: &Ui, at: Pos2, id: Id, symbol: &str, text: &str, primary: bool) -> Response {
    let galley = fit(ui, text, bold(14.0), if primary { ON_ACCENT } else { TEXT }, 240.0, 1);
    let rect = Rect::from_min_size(at, vec2(galley.size().x + 62.0, 42.0));
    let response = ui.interact(rect, id, Sense::click());
    let hover = hover_of(ui, &response);
    let body = if response.is_pointer_button_down_on() {
        rect.shrink(1.0)
    } else {
        rect
    };
    let fill = if primary {
        ACCENT.lerp_to_gamma(Color32::WHITE, 0.12 * hover)
    } else {
        veil(0.09 + 0.05 * hover)
    };
    let ink = if primary { ON_ACCENT } else { TEXT };
    ui.painter().rect_filled(body, 21.0, fill);
    ui.painter().text(
        pos2(rect.left() + 28.0, rect.center().y),
        Align2::CENTER_CENTER,
        symbol,
        solid(16.0),
        ink,
    );
    ui.painter()
        .galley(pos2(rect.left() + 42.0, rect.center().y - galley.size().y / 2.0), galley, ink);
    describe(&response, text);
    response
}

fn artwork(ui: &Ui, art: &Art, url: &str, rect: Rect, radius: f32) {
    match art.get(ui.ctx(), url, rect.width()) {
        Some(picture) => egui::Image::from_texture((picture.id, rect.size()))
            .corner_radius(radius)
            .paint_at(ui, rect),
        None => drop(ui.painter().rect_filled(rect, radius, RAISED)),
    }
}

fn bars(ui: &Ui, center: Pos2, moving: bool) {
    let time = if moving { ui.input(|input| input.time) } else { 0.9 };
    for bar in 0..3 {
        let wave = (time * (5.2 + bar as f64 * 1.9) + bar as f64 * 2.1).sin() as f32;
        let x = center.x + (bar as f32 - 1.0) * 5.0;
        let top = center.y + 7.0 - (4.0 + 5.0 * (1.0 + wave));
        ui.painter()
            .rect_filled(Rect::from_min_max(pos2(x - 1.5, top), pos2(x + 1.5, center.y + 7.0)), 1.0, ACCENT);
    }
    if moving && ui.input(|input| input.focused) {
        ui.ctx().request_repaint();
    }
}

fn clock(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

fn menu_row(ui: &mut Ui, symbol: &str, font: FontId, tint: Color32, text: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(MENU_WIDTH, 38.0), Sense::click());
    let hover = hover_of(ui, &response);
    ui.painter().rect_filled(rect, 7.0, veil(0.08 * hover));
    let tone = if tint == ACCENT { tint } else { tint.lerp_to_gamma(TEXT, hover) };
    ui.painter()
        .text(pos2(rect.left() + 22.0, rect.center().y), Align2::CENTER_CENTER, symbol, font, tone);
    label(
        ui,
        pos2(rect.left() + 46.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        medium(13.5),
        TEXT,
        rect.width() - 58.0,
    );
    describe(&response, text);
    response
}

fn switch_row(ui: &mut Ui, symbol: &str, text: &str, on: bool) -> bool {
    let response = menu_row(ui, symbol, glyph(17.0), MUTED, text);
    let lit = ui.ctx().animate_bool_with_time(response.id.with("switch"), on, HOVER_TIME);
    let track = Rect::from_center_size(pos2(response.rect.right() - 27.0, response.rect.center().y), vec2(30.0, 18.0));
    ui.painter().rect_filled(track, 9.0, veil(0.16).lerp_to_gamma(ACCENT, lit));
    let knob = pos2(egui::lerp(track.left() + 9.0..=track.right() - 9.0, lit), track.center().y);
    ui.painter().circle_filled(knob, 6.0, TEXT.lerp_to_gamma(ON_ACCENT, lit));
    response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, true, on, text));
    response.clicked()
}

fn menu_divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(MENU_WIDTH, 11.0), Sense::hover());
    ui.painter().hline(
        rect.shrink2(vec2(8.0, 0.0)).x_range(),
        rect.center().y,
        Stroke::new(1.0, veil(0.08)),
    );
}

fn song_actions(ui: &mut Ui, cx: &mut Cx, item: &Item) {
    ui.set_width(MENU_WIDTH);
    let (head, _) = ui.allocate_exact_size(vec2(MENU_WIDTH, 56.0), Sense::hover());
    let art = Rect::from_min_size(pos2(head.left() + 8.0, head.center().y - 20.0), Vec2::splat(40.0));
    artwork(ui, cx.art, cx.covers.of(item), art, 5.0);
    let (left, width) = (art.right() + 12.0, head.right() - art.right() - 20.0);
    label(
        ui,
        pos2(left, head.center().y - 9.0),
        Align2::LEFT_CENTER,
        &item.title,
        medium(13.5),
        TEXT,
        width,
    );
    label(
        ui,
        pos2(left, head.center().y + 9.0),
        Align2::LEFT_CENTER,
        &item.subtitle,
        sans(12.5),
        MUTED,
        width,
    );
    menu_divider(ui);

    let mut choice = None;
    if menu_row(ui, icon::ARROW_BEND_DOWN_RIGHT, glyph(17.0), MUTED, "Play next").clicked() {
        choice = Some(Action::PlayNext(item.clone()));
    }
    if menu_row(ui, icon::LIST_PLUS, glyph(17.0), MUTED, "Add to queue").clicked() {
        choice = Some(Action::Enqueue(item.clone()));
    }
    let (font, tint, text) = if cx.is_liked(item) {
        (solid(17.0), ACCENT, "Remove from Liked songs")
    } else {
        (glyph(17.0), MUTED, "Add to Liked songs")
    };
    if menu_row(ui, icon::HEART, font, tint, text).clicked() {
        choice = Some(Action::Like(item.clone()));
    }
    if !item.artist_id.is_empty() || !item.album_id.is_empty() {
        menu_divider(ui);
    }
    if !item.artist_id.is_empty() && menu_row(ui, icon::MICROPHONE_STAGE, glyph(17.0), MUTED, "Go to artist").clicked() {
        choice = Some(Action::Go(Route::Browse(item.artist_id.clone())));
    }
    if !item.album_id.is_empty() && menu_row(ui, icon::VINYL_RECORD, glyph(17.0), MUTED, "Go to album").clicked() {
        choice = Some(Action::Go(Route::Browse(item.album_id.clone())));
    }
    if let Some(action) = choice {
        cx.out.push(action);
        ui.close();
    }
}

fn song_menu(response: &Response, cx: &mut Cx, item: &Item) {
    if item.is_song() {
        response.context_menu(|ui| song_actions(ui, cx, item));
    }
}

fn track_row(ui: &mut Ui, cx: &mut Cx, items: &[Item], index: usize, number: Option<usize>) {
    let item = &items[index];
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let more_slot = Rect::from_center_size(pos2(rect.right() - 24.0, rect.center().y), Vec2::splat(30.0));
    let more = item
        .is_song()
        .then(|| ui.interact(more_slot, response.id.with("more"), Sense::click()));
    let more_open = more.as_ref().is_some_and(|more| egui::Popup::menu(more).is_open());
    let engaged = response.hovered() || response.context_menu_opened() || more_open;
    let hover = ui.ctx().animate_bool_with_time(response.id, engaged, HOVER_TIME);
    let playing = cx.is_playing(item);
    let mid = rect.center().y;
    ui.painter().rect_filled(rect, ROW_RADIUS, veil(0.06 * hover));
    if response.has_focus() {
        ui.painter()
            .rect_stroke(rect, ROW_RADIUS, Stroke::new(1.0, MUTED), StrokeKind::Inside);
    }

    let mut left = rect.left() + 12.0;
    match number {
        Some(number) => {
            let slot = pos2(left + 12.0, mid);
            if playing {
                bars(ui, slot, !cx.paused);
            } else {
                ui.painter()
                    .text(slot, Align2::CENTER_CENTER, number.to_string(), sans(13.5), FAINT);
            }
            left += 40.0;
        }
        None => {
            let art = Rect::from_min_size(pos2(left, mid - 20.0), Vec2::splat(40.0));
            artwork(ui, cx.art, cx.covers.of(item), art, if item.is_artist { 20.0 } else { 5.0 });
            if playing {
                ui.painter().rect_filled(art, 5.0, shade(0.6));
                bars(ui, art.center(), !cx.paused);
            }
            left += 54.0;
        }
    }

    let mut right = rect.right() - 14.0;
    if let Some(more) = &more {
        if hover > 0.0 {
            let lift = hover_of(ui, more);
            ui.painter().circle_filled(more_slot.center(), 15.0, veil(0.09 * lift));
            ui.painter().text(
                more_slot.center(),
                Align2::CENTER_CENTER,
                icon::DOTS_THREE,
                glyph(19.0),
                MUTED.gamma_multiply(hover).lerp_to_gamma(TEXT, lift),
            );
        }
        describe(more, "More");
        egui::Popup::menu(more).gap(4.0).show(|ui| song_actions(ui, cx, item));
        right -= 36.0;
        label(ui, pos2(right, mid), Align2::RIGHT_CENTER, &item.duration, sans(13.0), MUTED, 60.0);
        right -= 52.0;
        let liked = cx.is_liked(item);
        if liked || hover > 0.0 {
            let slot = Rect::from_center_size(pos2(right - 14.0, mid), Vec2::splat(30.0));
            let (font, color) = if liked {
                (solid(16.0), ACCENT)
            } else {
                (glyph(16.0), MUTED.gamma_multiply(hover))
            };
            let hint = if liked { "Remove from Liked songs" } else { "Add to Liked songs" };
            if icon_button(ui, slot, response.id.with("like"), icon::HEART, font, color, hint).clicked() {
                cx.out.push(Action::Like(item.clone()));
            }
        }
        right -= 40.0;
    } else {
        ui.painter()
            .text(pos2(right - 6.0, mid), Align2::CENTER_CENTER, icon::CARET_RIGHT, glyph(14.0), FAINT);
        right -= 28.0;
    }
    if rect.width() > 700.0 && !item.extra.is_empty() {
        let column = rect.width() * 0.26;
        label(
            ui,
            pos2(right - column, mid),
            Align2::LEFT_CENTER,
            &item.extra,
            sans(13.0),
            MUTED,
            column - 12.0,
        );
        right -= column + 20.0;
    }

    let title = if playing { ACCENT } else { TEXT };
    label(
        ui,
        pos2(left, mid - 10.0),
        Align2::LEFT_CENTER,
        &item.title,
        medium(14.0),
        title,
        right - left,
    );
    let artist = response.id.with("artist");
    byline(ui, cx.out, item, artist, pos2(left, mid + 10.0), sans(13.0), right - left);

    describe(&response, &item.title);
    if response.clicked() {
        cx.activate(items, index);
    }
    song_menu(&response, cx, item);
}

fn card(ui: &Ui, cx: &mut Cx, items: &[Item], index: usize, cell: Rect, id: Id) {
    let item = &items[index];
    let response = ui.interact(cell, id, Sense::click());
    let hover = ui.ctx().animate_bool_with_time(
        id,
        response.hovered() || response.has_focus() || response.context_menu_opened(),
        0.14,
    );
    let playing = cx.is_playing(item);
    let art = Rect::from_min_size(cell.min, Vec2::splat(cell.width()));
    let radius = if item.is_artist { art.width() / 2.0 } else { ART_RADIUS };
    artwork(ui, cx.art, cx.covers.of(item), art, radius);
    if hover > 0.0 || playing {
        let lit = if playing { 1.0 } else { hover };
        ui.painter().rect_filled(art, radius, shade(0.28 * lit));
        if item.is_song() {
            let badge = pos2(art.right() - 32.0, art.bottom() - 26.0 - 6.0 * lit);
            ui.painter().circle_filled(badge, 22.0, ACCENT.gamma_multiply(lit));
            let symbol = if playing && !cx.paused { icon::PAUSE } else { icon::PLAY };
            ui.painter()
                .text(badge, Align2::CENTER_CENTER, symbol, solid(19.0), ON_ACCENT.gamma_multiply(lit));
        }
    }
    if response.has_focus() {
        ui.painter().rect_stroke(art, radius, Stroke::new(1.5, TEXT), StrokeKind::Inside);
    }
    let title = if playing { ACCENT } else { TEXT };
    label(
        ui,
        pos2(cell.left(), art.bottom() + 22.0),
        Align2::LEFT_CENTER,
        &item.title,
        medium(14.0),
        title,
        cell.width(),
    );
    let below = pos2(cell.left(), art.bottom() + 42.0);
    byline(ui, cx.out, item, id.with("artist"), below, sans(13.0), cell.width());
    describe(&response, &item.title);
    if response.clicked() {
        cx.activate(items, index);
    }
    song_menu(&response, cx, item);
}

fn compact(ui: &Ui, cx: &mut Cx, items: &[Item], index: usize, cell: Rect, id: Id) {
    let item = &items[index];
    let response = ui.interact(cell, id, Sense::click());
    let hover = ui.ctx().animate_bool_with_time(
        id,
        response.hovered() || response.has_focus() || response.context_menu_opened(),
        HOVER_TIME,
    );
    let playing = cx.is_playing(item);
    ui.painter().rect_filled(cell, ROW_RADIUS, veil(0.06 * hover));
    let art = Rect::from_min_size(pos2(cell.left() + 8.0, cell.center().y - 24.0), Vec2::splat(48.0));
    artwork(ui, cx.art, cx.covers.of(item), art, if item.is_artist { 24.0 } else { 6.0 });
    if playing {
        ui.painter().rect_filled(art, 6.0, shade(0.6));
        bars(ui, art.center(), !cx.paused);
    }
    let (left, width) = (art.right() + 14.0, cell.right() - art.right() - 26.0);
    let title = if playing { ACCENT } else { TEXT };
    label(
        ui,
        pos2(left, cell.center().y - 10.0),
        Align2::LEFT_CENTER,
        &item.title,
        medium(14.0),
        title,
        width,
    );
    let below = pos2(left, cell.center().y + 10.0);
    byline(ui, cx.out, item, id.with("artist"), below, sans(13.0), width);
    describe(&response, &item.title);
    if response.clicked() {
        cx.activate(items, index);
    }
    song_menu(&response, cx, item);
}

fn columns(width: f32, minimum: f32, gap: f32) -> (usize, f32) {
    let count = (((width + gap) / (minimum + gap)).floor() as usize).max(1);
    (count, ((width - gap * (count - 1) as f32) / count as f32).floor())
}

fn grid(ui: &mut Ui, count: usize, columns: usize, cell: Vec2, gap: Vec2, mut paint: impl FnMut(&Ui, usize, Rect)) {
    let rows = count.div_ceil(columns);
    let height = rows as f32 * cell.y + rows.saturating_sub(1) as f32 * gap.y;
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    for index in 0..count {
        let offset = vec2(
            (index % columns) as f32 * (cell.x + gap.x),
            (index / columns) as f32 * (cell.y + gap.y),
        );
        let rect = Rect::from_min_size(area.min + offset, cell);
        if ui.is_rect_visible(rect) {
            paint(ui, index, rect);
        }
    }
}

fn section_view(ui: &mut Ui, cx: &mut Cx, title: &str, layout: Layout, items: &[Item], id: Id, numbered: bool) {
    let width = ui.available_width();
    let (cards, card_width) = columns(width, CARD_MIN, CARD_GAP);
    let (cells, cell_width) = columns(width, COMPACT_MIN, 12.0);
    let visible = match layout {
        Layout::Cards => cards,
        Layout::Compact => cells * COMPACT_ROWS,
        Layout::Tracks | Layout::Top => usize::MAX,
    };
    let foldable = items.len() > visible;
    let mut open = ui.data(|data| data.get_temp::<bool>(id)).unwrap_or(false);
    if !title.is_empty() || foldable {
        let (row, _) = ui.allocate_exact_size(vec2(width, 32.0), Sense::hover());
        label(ui, row.left_center(), Align2::LEFT_CENTER, title, bold(20.0), TEXT, width - 120.0);
        if foldable {
            let text = if open { "Show less" } else { "Show all" };
            if text_button(ui, row.right_center(), id.with("fold"), text).clicked() {
                open = !open;
                ui.data_mut(|data| data.insert_temp(id, open));
            }
        }
        ui.add_space(12.0);
    }
    let shown = if open { items.len() } else { items.len().min(visible) };
    match layout {
        Layout::Cards => {
            let cell = vec2(card_width, card_width + CARD_TEXT);
            grid(ui, shown, cards, cell, vec2(CARD_GAP, 14.0), |ui, index, rect| {
                card(ui, cx, items, index, rect, id.with(index))
            });
        }
        Layout::Compact => {
            let cell = vec2(cell_width, 64.0);
            grid(ui, shown, cells, cell, vec2(12.0, 2.0), |ui, index, rect| {
                compact(ui, cx, items, index, rect, id.with(index))
            });
        }
        Layout::Tracks => {
            for index in 0..shown {
                track_row(ui, cx, items, index, numbered.then_some(index + 1));
            }
        }
        Layout::Top => {
            top_result(ui, cx, items, id);
            for index in 1..shown {
                track_row(ui, cx, items, index, None);
            }
        }
    }
}

fn top_result(ui: &mut Ui, cx: &mut Cx, items: &[Item], id: Id) {
    let item = &items[0];
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 128.0), Sense::click());
    let hover = hover_of(ui, &response);
    ui.painter().rect_filled(rect, 12.0, veil(0.045 + 0.035 * hover));
    let art = Rect::from_min_size(rect.min + vec2(16.0, 16.0), Vec2::splat(96.0));
    artwork(ui, cx.art, cx.covers.of(item), art, if item.is_artist { 48.0 } else { ART_RADIUS });
    let left = art.right() + 22.0;
    let width = rect.right() - left - 150.0;
    label(
        ui,
        pos2(left, rect.center().y - 16.0),
        Align2::LEFT_CENTER,
        &item.title,
        display(38.0),
        TEXT,
        width,
    );
    let below = pos2(left, rect.center().y + 20.0);
    byline(ui, cx.out, item, id.with("artist"), below, sans(14.0), width);
    let (symbol, text) = if item.is_song() {
        (icon::PLAY, "Play")
    } else {
        (icon::CARET_RIGHT, "Open")
    };
    let button = pill(
        ui,
        pos2(rect.right() - 128.0, rect.center().y - 21.0),
        id.with("top"),
        symbol,
        text,
        item.is_song(),
    );
    describe(&response, &item.title);
    if response.clicked() || button.clicked() {
        cx.activate(items, 0);
    }
    song_menu(&response, cx, item);
    ui.add_space(8.0);
}

fn hero(ui: &mut Ui, cx: &mut Cx, item: &Item, symbol: Option<&str>, playable: bool, pinned: Option<bool>) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HERO_ART), Sense::hover());
    let art = Rect::from_min_size(rect.min, Vec2::splat(HERO_ART));
    match symbol {
        Some(symbol) => {
            ui.painter().rect_filled(art, 12.0, veil(0.06));
            ui.painter().text(art.center(), Align2::CENTER_CENTER, symbol, solid(84.0), ACCENT);
        }
        None => artwork(ui, cx.art, &item.thumb, art, if item.is_artist { HERO_ART / 2.0 } else { 12.0 }),
    }
    let left = art.right() + 30.0;
    let width = (rect.right() - left).max(120.0);
    let mut bottom = rect.bottom();

    let mut at = pos2(left, bottom - 42.0);
    let mut play_clicked = false;
    if playable {
        let play = pill(ui, at, ui.id().with("hero-play"), icon::PLAY, "Play", true);
        at.x = play.rect.right() + 10.0;
        play_clicked = play.clicked();
    }
    if let Some(pinned) = pinned {
        let slot = Rect::from_min_size(at, Vec2::splat(42.0));
        let (font, color, hint) = if pinned {
            (solid(19.0), ACCENT, "Remove from library")
        } else {
            (glyph(19.0), MUTED, "Save to library")
        };
        if icon_button(ui, slot, ui.id().with("hero-pin"), icon::BOOKMARK_SIMPLE, font, color, hint).clicked() {
            cx.out.push(Action::Pin(item.clone()));
        }
    }
    bottom -= 60.0;

    for (text, font, color) in [(&item.subtitle, sans(14.0), MUTED), (&item.extra, medium(15.0), TEXT)] {
        if !text.is_empty() {
            let line = label(ui, pos2(left, bottom), Align2::LEFT_BOTTOM, text, font, color, width);
            bottom = line.top() - 5.0;
        }
    }
    let size = match item.title.chars().count() {
        0..=16 => 68.0,
        17..=34 => 50.0,
        _ => 38.0,
    };
    let title = fit(ui, &item.title, display(size), TEXT, width, 2);
    ui.painter().galley(pos2(left, bottom - title.size().y), title, TEXT);
    play_clicked
}

fn skeleton(ui: &mut Ui) {
    let pulse = veil(0.05 + 0.025 * (ui.input(|input| input.time) * 3.2).sin() as f32);
    ui.ctx().request_repaint_after(Duration::from_millis(60));
    let (count, side) = columns(ui.available_width(), CARD_MIN, CARD_GAP);
    for _ in 0..2 {
        let (heading, _) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::hover());
        ui.painter()
            .rect_filled(Rect::from_min_size(heading.min + vec2(0.0, 5.0), vec2(190.0, 22.0)), 6.0, pulse);
        ui.add_space(12.0);
        grid(
            ui,
            count,
            count,
            vec2(side, side + CARD_TEXT),
            vec2(CARD_GAP, 0.0),
            |ui, _, cell| {
                ui.painter()
                    .rect_filled(Rect::from_min_size(cell.min, Vec2::splat(side)), ART_RADIUS, pulse);
                ui.painter().rect_filled(
                    Rect::from_min_size(cell.min + vec2(0.0, side + 14.0), vec2(side * 0.7, 13.0)),
                    4.0,
                    pulse,
                );
                ui.painter().rect_filled(
                    Rect::from_min_size(cell.min + vec2(0.0, side + 35.0), vec2(side * 0.45, 12.0)),
                    4.0,
                    pulse,
                );
            },
        );
        ui.add_space(32.0);
    }
}

fn message(ui: &mut Ui, symbol: &str, title: &str, hint: &str) -> Rect {
    ui.add_space(72.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 132.0), Sense::hover());
    let center = rect.center_top();
    ui.painter().circle_filled(center + vec2(0.0, 34.0), 34.0, veil(0.06));
    ui.painter()
        .text(center + vec2(0.0, 34.0), Align2::CENTER_CENTER, symbol, glyph(30.0), MUTED);
    label(
        ui,
        center + vec2(0.0, 92.0),
        Align2::CENTER_CENTER,
        title,
        bold(18.0),
        TEXT,
        rect.width(),
    );
    label(
        ui,
        center + vec2(0.0, 118.0),
        Align2::CENTER_CENTER,
        hint,
        sans(14.0),
        MUTED,
        rect.width(),
    );
    rect
}

fn local(ui: &mut Ui, cx: &mut Cx, title: &str, symbol: &str, items: &[Item], empty: [&str; 2], clearable: bool) {
    let count = if items.len() == 1 {
        "1 song".to_owned()
    } else {
        format!("{} songs", items.len())
    };
    let header = Item {
        title: title.to_owned(),
        subtitle: count,
        ..Item::default()
    };
    let play_row_end = pos2(ui.max_rect().right(), ui.cursor().top() + HERO_ART - 21.0);
    if hero(ui, cx, &header, Some(symbol), !items.is_empty(), None) {
        cx.out.push(Action::Play(items.to_vec(), 0));
    }
    if clearable && !items.is_empty() && text_button(ui, play_row_end, ui.id().with("clear"), "Clear history").clicked() {
        cx.out.push(Action::ClearHistory);
    }
    ui.add_space(28.0);
    if items.is_empty() {
        message(ui, symbol, empty[0], empty[1]);
    }
    for index in 0..items.len() {
        track_row(ui, cx, items, index, None);
    }
}

fn page(ui: &mut Ui, cx: &mut Cx, load: Option<&Load>, pinned: &[Item], query: Option<&str>) {
    match load {
        Some(Load::Ready(page)) => {
            if let Some(header) = &page.header {
                let saved = pinned.iter().any(|item| item.browse_id == header.browse_id);
                let playable = page.sections.iter().flat_map(|section| &section.items).any(Item::is_song);
                if hero(ui, cx, header, None, playable, Some(saved)) {
                    cx.out.push(Action::Play(page.songs(), 0));
                }
                ui.add_space(8.0);
            }
            if let (true, Some(query)) = (page.sections.is_empty(), query) {
                message(
                    ui,
                    icon::MAGNIFYING_GLASS,
                    &format!("Nothing found for \"{query}\""),
                    "Check the spelling or try fewer words.",
                );
            }
            for (index, section) in page.sections.iter().enumerate() {
                ui.add_space(if index == 0 && page.header.is_none() { 4.0 } else { 30.0 });
                let numbered = section.layout == Layout::Tracks
                    && page
                        .header
                        .as_ref()
                        .is_some_and(|header| !header.is_artist && section.items.iter().all(|item| item.thumb == header.thumb));
                let id = ui.id().with(index);
                section_view(ui, cx, &section.title, section.layout, &section.items, id, numbered);
            }
            if page.partial {
                ui.add_space(30.0);
                skeleton(ui);
            }
        }
        Some(Load::Failed(error)) => {
            let rect = message(ui, icon::WARNING_CIRCLE, "This page did not load", error);
            ui.add_space(12.0);
            let (row, _) = ui.allocate_exact_size(vec2(rect.width(), 42.0), Sense::hover());
            if pill(
                ui,
                pos2(row.center().x - 70.0, row.top()),
                ui.id().with("retry"),
                icon::ARROW_CLOCKWISE,
                "Try again",
                false,
            )
            .clicked()
            {
                cx.out.push(Action::Reload);
            }
        }
        _ => skeleton(ui),
    }
}

fn mood(ui: &Ui, name: &str, tint: Option<Color32>, floor: Color32, brightness: f32) -> Color32 {
    let target = tint.map_or(floor, |tint| {
        let mut color = Hsva::from(tint);
        color.s = (color.s * 1.5).min(0.8);
        color.v = brightness;
        floor.lerp_to_gamma(Color32::from(color), 0.8)
    });
    let channel = |index: usize, value: u8| ui.ctx().animate_value_with_time(Id::new((name, index)), value as f32, 0.6) as u8;
    Color32::from_rgb(channel(0, target.r()), channel(1, target.g()), channel(2, target.b()))
}

fn luminance(color: Color32) -> f32 {
    let linear = egui::Rgba::from(color);
    0.2126 * linear.r() + 0.7152 * linear.g() + 0.0722 * linear.b()
}

fn dimmed(color: Color32, limit: f32) -> Color32 {
    let scale = limit / luminance(color);
    if scale >= 1.0 {
        return color;
    }
    let linear = egui::Rgba::from(color);
    Color32::from(egui::Rgba::from_rgb(linear.r() * scale, linear.g() * scale, linear.b() * scale))
}

fn wash(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    if top == bottom {
        return;
    }
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(Shape::mesh(mesh));
}

fn entrance(ui: &Ui, scene: Id) -> f32 {
    let now = ui.input(|input| input.time);
    let started = ui.data_mut(|data| {
        let shown = data.get_temp_mut_or_insert_with(Id::new("entrance"), || (scene, f64::NEG_INFINITY));
        if shown.0 != scene {
            *shown = (scene, now);
        }
        shown.1
    });
    let progress = ((now - started) / PAGE_FADE).min(1.0) as f32;
    if progress < 1.0 {
        ui.ctx().request_repaint();
    }
    progress * (2.0 - progress)
}

fn sticky(ui: &Ui, strip: Rect, title: &str, playable: bool) -> bool {
    ui.interact(strip, Id::new("sticky"), Sense::click());
    let mid = strip.bottom() - STICKY_HEIGHT / 2.0;
    let mut left = strip.left() + PAGE_MARGIN as f32;
    let mut clicked = false;
    if playable {
        let slot = Rect::from_center_size(pos2(left + 20.0, mid), Vec2::splat(40.0));
        let play = ui.interact(slot, Id::new("sticky-play"), Sense::click());
        let hover = hover_of(ui, &play);
        let pressed = if play.is_pointer_button_down_on() { 1.0 } else { 0.0 };
        ui.painter().circle_filled(
            slot.center(),
            19.0 + hover - pressed,
            ACCENT.lerp_to_gamma(Color32::WHITE, 0.12 * hover),
        );
        ui.painter()
            .text(slot.center(), Align2::CENTER_CENTER, icon::PLAY, solid(17.0), ON_ACCENT);
        describe(&play, "Play");
        clicked = play.clicked();
        left = slot.right() + 16.0;
    }
    let width = strip.right() - left - PAGE_MARGIN as f32;
    label(ui, pos2(left, mid), Align2::LEFT_CENTER, title, bold(18.0), TEXT, width);
    ui.painter()
        .hline(strip.x_range(), strip.bottom() - 0.5, Stroke::new(1.0, veil(0.07)));
    clicked
}

fn topbar(app: &mut App, ui: &mut Ui, out: &mut Vec<Action>) {
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), TOPBAR_HEIGHT), Sense::hover());
    let mid = bar.center().y;
    let mut left = bar.left() + 24.0;
    let steps = [
        (icon::CARET_LEFT, app.at > 0, Action::Back, "Back"),
        (icon::CARET_RIGHT, app.at + 1 < app.history.len(), Action::Forward, "Forward"),
    ];
    for (symbol, enabled, action, hint) in steps {
        let slot = Rect::from_center_size(pos2(left + 18.0, mid), Vec2::splat(36.0));
        if !enabled {
            ui.painter()
                .text(slot.center(), Align2::CENTER_CENTER, symbol, glyph(17.0), FAINT.gamma_multiply(0.5));
        } else if icon_button(ui, slot, Id::new(hint), symbol, glyph(17.0), MUTED, hint).clicked() {
            out.push(action);
        }
        left += 40.0;
    }
    left += 10.0;

    let field = Rect::from_min_size(
        pos2(left, mid - 21.0),
        vec2((bar.right() - PAGE_MARGIN as f32 - left).min(440.0), 42.0),
    );
    let background = ui.painter().add(Shape::Noop);
    ui.painter().text(
        pos2(field.left() + 22.0, mid),
        Align2::CENTER_CENTER,
        icon::MAGNIFYING_GLASS,
        glyph(17.0),
        MUTED,
    );
    let input = Rect::from_min_max(pos2(field.left() + 44.0, field.top()), pos2(field.right() - 64.0, field.bottom()));
    let layout = egui::Layout::left_to_right(Align::Center);
    let response = ui
        .scope_builder(UiBuilder::new().max_rect(input).layout(layout), |ui| {
            let edit = egui::TextEdit::singleline(&mut app.query)
                .id(Id::new("search"))
                .frame(false)
                .margin(Margin::ZERO)
                .desired_width(input.width())
                .font(sans(14.5))
                .text_color(TEXT)
                .hint_text(RichText::new("Search songs, albums, artists").color(FAINT));
            ui.add(edit)
        })
        .inner;
    if std::mem::take(&mut app.focus_search) {
        response.request_focus();
    }
    if response.changed() {
        app.typed = Some(Instant::now());
    }
    let query = app.query.trim();
    if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) && !query.is_empty() {
        app.typed = None;
        out.push(Action::Go(Route::Search(query.to_owned())));
    }
    let focus = ui.ctx().animate_bool_with_time(Id::new("search-focus"), response.has_focus(), 0.12);
    let stroke = Stroke::new(1.0, veil(0.08 + 0.42 * focus));
    ui.painter().set(
        background,
        RectShape::new(field, 21.0, veil(0.07 + 0.02 * focus), stroke, StrokeKind::Inside),
    );
    let trailing = pos2(field.right() - 24.0, mid);
    if app.query.is_empty() {
        ui.painter()
            .text(pos2(field.right() - 16.0, mid), Align2::RIGHT_CENTER, "Ctrl K", sans(12.0), FAINT);
    } else if icon_button(
        ui,
        Rect::from_center_size(trailing, Vec2::splat(30.0)),
        Id::new("search-clear"),
        icon::X,
        glyph(14.0),
        MUTED,
        "Clear",
    )
    .clicked()
    {
        app.query.clear();
        app.typed = None;
        response.request_focus();
    }
    ui.allocate_rect(bar, Sense::hover());
}

fn content(app: &mut App, ui: &mut Ui, out: &mut Vec<Action>) {
    let area = ui.max_rect();
    let route = app.route().clone();
    let header = match app.pages.get(&route) {
        Some(Load::Ready(page)) => page.header.as_ref().map(|header| (header.thumb.as_str(), HERO_ART)),
        _ => None,
    };
    let playing = app.now_playing().map(|item| (app.saved.covers.of(item), BAR_ART));
    let picture = |source: Option<(&str, f32)>| source.and_then(|(thumb, size)| app.art.get(ui.ctx(), thumb, size));
    let tint = picture(header).or_else(|| picture(playing)).map(|picture| picture.tint);
    let glow = mood(ui, "ambient", tint, BASE, 0.36);
    let haze = Rect::from_min_size(area.min, vec2(area.width(), AMBIENT_HEIGHT.min(area.height())));
    wash(ui.painter(), haze, glow, BASE);
    topbar(app, ui, out);

    let loading = matches!(app.pages.get(&route), Some(Load::Loading));
    let shown = entrance(ui, Id::new((&route, loading)));
    let headline = match (&route, app.pages.get(&route)) {
        (Route::Liked, _) => Some(("Liked songs", !app.saved.liked.is_empty())),
        (Route::Recent, _) => Some(("Recently played", !app.saved.recent.is_empty())),
        (Route::Browse(_), Some(Load::Ready(page))) => page.header.as_ref().map(|header| {
            let playable = page.sections.iter().flat_map(|section| &section.items).any(Item::is_song);
            (header.title.as_str(), playable)
        }),
        _ => None,
    };
    let mut cx = Cx {
        art: &app.art,
        covers: &app.saved.covers,
        playing: app.now_playing().map_or("", |item| item.video_id.as_str()),
        paused: app.player.paused(),
        liked: &app.saved.liked,
        single: matches!(route, Route::Search(_) | Route::Home),
        out: &mut *out,
    };
    let margin = Margin {
        left: PAGE_MARGIN,
        right: PAGE_MARGIN,
        top: 8,
        bottom: 56,
    };
    let scrolled = egui::ScrollArea::vertical().id_salt(&route).auto_shrink(false).show(ui, |ui| {
        Frame::new().inner_margin(margin).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_opacity(shown);
            match &route {
                Route::Liked => {
                    let empty = ["No liked songs yet", "Tap the heart on any track to keep it here."];
                    local(ui, &mut cx, "Liked songs", icon::HEART, &app.saved.liked, empty, false);
                }
                Route::Recent => {
                    let empty = ["Nothing played yet", "Tracks you listen to are remembered here."];
                    local(
                        ui,
                        &mut cx,
                        "Recently played",
                        icon::CLOCK_COUNTER_CLOCKWISE,
                        &app.saved.recent,
                        empty,
                        true,
                    );
                }
                Route::Search(query) => page(ui, &mut cx, app.pages.get(&route), &app.saved.pinned, Some(query)),
                Route::Home | Route::Browse(_) => page(ui, &mut cx, app.pages.get(&route), &app.saved.pinned, None),
            }
        });
    });

    let past_hero = scrolled.state.offset.y > HERO_ART - 36.0;
    let stuck = ui
        .ctx()
        .animate_bool_with_time(Id::new("sticky"), headline.is_some() && past_hero, 0.16);
    let Some((title, playable)) = headline.filter(|_| stuck > 0.0) else {
        return;
    };
    let overlap = ui.visuals().clip_rect_margin;
    let strip = Rect::from_min_size(
        pos2(area.left(), area.top() + TOPBAR_HEIGHT - overlap),
        vec2(area.width(), STICKY_HEIGHT + overlap),
    );
    let mut pane = ui.new_child(UiBuilder::new().id_salt("sticky").max_rect(strip));
    pane.set_clip_rect(strip);
    pane.set_opacity(stuck);
    pane.painter().rect_filled(strip, 0.0, BASE);
    wash(pane.painter(), haze, glow, BASE);
    if sticky(&pane, strip, title, playable) {
        let songs = match (&route, app.pages.get(&route)) {
            (Route::Liked, _) => app.saved.liked.clone(),
            (Route::Recent, _) => app.saved.recent.clone(),
            (_, Some(Load::Ready(page))) => page.songs(),
            _ => Vec::new(),
        };
        out.push(Action::Play(songs, 0));
    }
}

fn nav_item(ui: &mut Ui, symbol: &str, text: &str, active: bool, trailing: &str) -> Response {
    let (slot, response) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
    let rect = slot.shrink2(vec2(12.0, 1.0));
    let hover = hover_of(ui, &response);
    ui.painter()
        .rect_filled(rect, ROW_RADIUS, veil(if active { 0.08 } else { 0.05 * hover }));
    let tone = if active { TEXT } else { MUTED.lerp_to_gamma(TEXT, hover) };
    let (font, color) = if active { (solid(19.0), ACCENT) } else { (glyph(19.0), tone) };
    ui.painter().text(
        pos2(rect.left() + 24.0, rect.center().y),
        Align2::CENTER_CENTER,
        symbol,
        font,
        color,
    );
    label(
        ui,
        pos2(rect.left() + 48.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        medium(14.0),
        tone,
        rect.width() - 96.0,
    );
    label(
        ui,
        pos2(rect.right() - 14.0, rect.center().y),
        Align2::RIGHT_CENTER,
        trailing,
        sans(12.5),
        FAINT,
        40.0,
    );
    describe(&response, text);
    response
}

fn mark(ui: &Ui, center: Pos2, radius: f32) {
    ui.painter().circle_filled(center, radius, ACCENT);
    ui.painter().circle_stroke(center, radius * 0.55, Stroke::new(radius * 0.09, PANEL));
    ui.painter().circle_filled(center, radius * 0.18, PANEL);
}

fn sidebar(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    let area = ui.max_rect();
    ui.painter().vline(area.right() - 0.5, area.y_range(), Stroke::new(1.0, veil(0.06)));
    let (brand, _) = ui.allocate_exact_size(vec2(area.width(), 76.0), Sense::hover());
    let center = pos2(brand.left() + 37.0, brand.center().y + 3.0);
    mark(ui, center, 12.0);
    ui.painter()
        .text(pos2(center.x + 22.0, center.y), Align2::LEFT_CENTER, APP_NAME, display(29.0), TEXT);

    let count = |items: &[Item]| if items.is_empty() { String::new() } else { items.len().to_string() };
    let links = [
        (icon::HOUSE, "Home", Route::Home, String::new()),
        (icon::HEART, "Liked songs", Route::Liked, count(&app.saved.liked)),
        (
            icon::CLOCK_COUNTER_CLOCKWISE,
            "Recently played",
            Route::Recent,
            count(&app.saved.recent),
        ),
    ];
    for (symbol, text, route, trailing) in links {
        if nav_item(ui, symbol, text, *app.route() == route, &trailing).clicked() {
            out.push(Action::Go(route));
        }
    }
    ui.add_space(18.0);
    eyebrow(ui, "Library", 26.0);
    let foot = Rect::from_min_max(pos2(area.left(), area.bottom() - ACCOUNT_HEIGHT), area.max);
    account(app, ui, foot, out);
    let lift = if app.update.is_some() { UPDATE_HEIGHT } else { 0.0 };
    let offer = Rect::from_min_max(pos2(area.left(), foot.top() - lift), foot.right_top());
    update_offer(app, ui, offer, out);
    let pinned = &app.saved.pinned;
    let own = app
        .library
        .iter()
        .filter(|item| !pinned.iter().any(|saved| saved.browse_id == item.browse_id));
    let entries: Vec<&Item> = pinned.iter().chain(own).collect();
    if entries.is_empty() {
        Frame::new().inner_margin(Margin::symmetric(26, 4)).show(ui, |ui| {
            paragraph(ui, "Save an album, playlist or artist and it stays here.", sans(13.0), FAINT, 3);
        });
        return;
    }
    let height = (offer.top() - ui.cursor().top()).max(0.0);
    egui::ScrollArea::vertical().max_height(height).auto_shrink(false).show(ui, |ui| {
        for item in entries {
            let (slot, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
            let rect = slot.shrink2(vec2(12.0, 1.0));
            let active = matches!(app.route(), Route::Browse(id) if *id == item.browse_id);
            let hover = hover_of(ui, &response);
            ui.painter()
                .rect_filled(rect, ROW_RADIUS, veil(if active { 0.08 } else { 0.05 * hover }));
            let art = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 20.0), Vec2::splat(40.0));
            artwork(ui, &app.art, &item.thumb, art, if item.is_artist { 20.0 } else { 5.0 });
            let (left, width) = (art.right() + 12.0, rect.right() - art.right() - 22.0);
            let kind = if item.is_artist { "Artist" } else { item.subtitle.as_str() };
            label(
                ui,
                pos2(left, rect.center().y - 9.0),
                Align2::LEFT_CENTER,
                &item.title,
                medium(13.5),
                TEXT,
                width,
            );
            label(
                ui,
                pos2(left, rect.center().y + 9.0),
                Align2::LEFT_CENTER,
                kind,
                sans(12.5),
                MUTED,
                width,
            );
            describe(&response, &item.title);
            if response.clicked() {
                out.push(Action::Go(Route::Browse(item.browse_id.clone())));
            }
        }
        ui.add_space(12.0);
    });
}

fn update_offer(app: &App, ui: &Ui, slot: Rect, out: &mut Vec<Action>) {
    let Some(version) = &app.update else { return };
    let row = slot.shrink2(vec2(12.0, 4.0));
    let response = ui.interact(row, Id::new("update-offer"), Sense::click());
    let hover = hover_of(ui, &response);
    ui.painter()
        .rect_filled(row, ROW_RADIUS, ACCENT.gamma_multiply(0.14 + 0.08 * hover));
    let mark = pos2(row.left() + 24.0, row.center().y);
    if app.updating {
        egui::Spinner::new()
            .color(ACCENT)
            .paint_at(ui, Rect::from_center_size(mark, Vec2::splat(16.0)));
    } else {
        ui.painter()
            .text(mark, Align2::CENTER_CENTER, icon::ARROW_CIRCLE_UP, solid(19.0), ACCENT);
    }
    let text = if app.updating {
        "Updating".to_owned()
    } else {
        format!("Update to {version}")
    };
    label(
        ui,
        pos2(row.left() + 48.0, row.center().y),
        Align2::LEFT_CENTER,
        &text,
        medium(13.5),
        TEXT,
        row.width() - 60.0,
    );
    describe(&response, &text);
    if response.clicked() {
        out.push(Action::GetUpdate);
    }
}

fn account(app: &App, ui: &Ui, foot: Rect, out: &mut Vec<Action>) {
    ui.painter().hline(foot.x_range(), foot.top(), Stroke::new(1.0, veil(0.06)));
    let mut row = foot.shrink2(vec2(12.0, 10.0));
    let slot = Rect::from_center_size(pos2(row.right() - 20.0, row.center().y), Vec2::splat(34.0));
    let settings = icon_button(ui, slot, Id::new("settings"), icon::GEAR_SIX, glyph(17.0), MUTED, "Settings");
    egui::Popup::menu(&settings)
        .gap(4.0)
        .align(egui::RectAlign::TOP_START)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_width(MENU_WIDTH);
            if switch_row(ui, icon::DISCORD_LOGO, "Show on Discord", app.saved.discord) {
                out.push(Action::ShowOnDiscord(!app.saved.discord));
            }
            if app.saved.discord {
                let status = match app.discord_linked {
                    Some(true) if app.playing() => "Connected to Discord",
                    Some(true) => "Shows while a song is playing",
                    Some(false) => "Discord is not running",
                    None => "",
                };
                let (line, response) = ui.allocate_exact_size(vec2(MENU_WIDTH, 22.0), Sense::hover());
                let at = pos2(line.left() + 46.0, line.center().y - 4.0);
                label(ui, at, Align2::LEFT_CENTER, status, sans(12.0), MUTED, line.width() - 58.0);
                response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, status));
                menu_divider(ui);
                for (headline, text) in [
                    (Headline::App, "Listening to Tubefast"),
                    (Headline::Artist, "Listening to the artist"),
                    (Headline::Song, "Listening to the song"),
                ] {
                    let chosen = app.saved.discord_headline == headline;
                    let symbol = if chosen { icon::CHECK } else { "" };
                    let row = menu_row(ui, symbol, glyph(15.0), ACCENT, text);
                    row.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, true, chosen, text));
                    if row.clicked() {
                        out.push(Action::DiscordHeadline(headline));
                    }
                }
            }
        });
    row.max.x -= 36.0;
    let avatar = Rect::from_center_size(pos2(row.left() + 24.0, row.center().y), Vec2::splat(32.0));
    let (left, mid) = (avatar.right() + 12.0, row.center().y);
    match &app.saved.account {
        Some(account) => {
            artwork(ui, &app.art, &account.photo, avatar, 16.0);
            let width = row.right() - left - 44.0;
            label(
                ui,
                pos2(left, mid - 9.0),
                Align2::LEFT_CENTER,
                &account.name,
                medium(13.5),
                TEXT,
                width,
            );
            label(
                ui,
                pos2(left, mid + 9.0),
                Align2::LEFT_CENTER,
                "Signed in",
                sans(12.0),
                MUTED,
                width,
            );
            let slot = Rect::from_center_size(pos2(row.right() - 20.0, mid), Vec2::splat(34.0));
            if icon_button(ui, slot, Id::new("sign-out"), icon::SIGN_OUT, glyph(17.0), MUTED, "Sign out").clicked() {
                out.push(Action::SignOut);
            }
        }
        None => {
            let response = ui.interact(row, Id::new("sign-in"), Sense::click());
            let hover = hover_of(ui, &response);
            let width = row.right() - left - 8.0;
            ui.painter().rect_filled(row, ROW_RADIUS, veil(0.05 * hover));
            ui.painter().text(
                avatar.center(),
                Align2::CENTER_CENTER,
                icon::USER_CIRCLE,
                glyph(27.0),
                MUTED.lerp_to_gamma(TEXT, hover),
            );
            label(ui, pos2(left, mid - 9.0), Align2::LEFT_CENTER, "Sign in", medium(13.5), TEXT, width);
            label(
                ui,
                pos2(left, mid + 9.0),
                Align2::LEFT_CENTER,
                "Mixes and playlists",
                sans(12.0),
                MUTED,
                width,
            );
            describe(&response, "Sign in");
            if response.clicked() {
                out.push(Action::OpenSignIn);
            }
        }
    }
}

fn sign_in_dialog(app: &mut App, ctx: &egui::Context, out: &mut Vec<Action>) {
    let Some(sign_in) = &mut app.sign_in else { return };
    let frame = Frame::new()
        .fill(rgb(0x15161A))
        .stroke(Stroke::new(1.0, veil(0.08)))
        .corner_radius(18)
        .inner_margin(Margin::same(30));
    let modal = egui::Modal::new(Id::new("sign-in-dialog"))
        .frame(frame)
        .backdrop_color(shade(0.6))
        .show(ctx, |ui| {
            ui.set_width(430.0);
            let width = ui.available_width();
            let (head, _) = ui.allocate_exact_size(vec2(width, 44.0), Sense::hover());
            ui.painter()
                .text(head.left_center(), Align2::LEFT_CENTER, "Sign in", display(38.0), TEXT);
            let slot = Rect::from_center_size(pos2(head.right() - 16.0, head.center().y), Vec2::splat(34.0));
            let closed = icon_button(ui, slot, Id::new("sign-in-close"), icon::X, glyph(16.0), MUTED, "Close").clicked();
            ui.add_space(8.0);
            paragraph(ui, "Your own home feed and playlists from YouTube Music.", sans(14.0), MUTED, 2);
            ui.add_space(24.0);

            let (row, _) = ui.allocate_exact_size(vec2(width, 42.0), Sense::hover());
            if pill(ui, row.min, Id::new("sign-in-browser"), icon::GLOBE, "Continue in browser", true).clicked() && !sign_in.busy {
                out.push(Action::BrowserSignIn);
            }
            ui.add_space(12.0);
            paragraph(
                ui,
                "Opens a separate Edge or Chrome window. Sign in to Google there and it closes by itself.",
                sans(13.0),
                MUTED,
                3,
            );
            ui.add_space(22.0);

            eyebrow(ui, "Or paste your cookie", 0.0);
            let steps = [
                "1.  Open music.youtube.com in your browser and sign in.",
                "2.  Press F12, open Network and click a request to music.youtube.com.",
                "3.  Copy the whole cookie value under Request Headers and paste it here.",
            ];
            for step in steps {
                paragraph(ui, step, sans(13.0), MUTED, 2);
                ui.add_space(5.0);
            }
            ui.add_space(7.0);
            let background = ui.painter().add(Shape::Noop);
            let field = egui::ScrollArea::vertical().max_height(74.0).auto_shrink(false).show(ui, |ui| {
                let edit = egui::TextEdit::multiline(&mut sign_in.cookie)
                    .frame(false)
                    .margin(Margin::same(10))
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .font(FontId::monospace(11.5))
                    .text_color(TEXT)
                    .hint_text(RichText::new("SID=...; SAPISID=...; LOGIN_INFO=...").color(FAINT));
                ui.add(edit);
            });
            ui.painter().set(
                background,
                RectShape::new(field.inner_rect, 10.0, veil(0.06), Stroke::new(1.0, veil(0.08)), StrokeKind::Inside),
            );
            ui.add_space(14.0);
            let (row, _) = ui.allocate_exact_size(vec2(width, 42.0), Sense::hover());
            if pill(
                ui,
                row.min,
                Id::new("sign-in-cookie"),
                icon::CLIPBOARD_TEXT,
                "Use this cookie",
                false,
            )
            .clicked()
                && !sign_in.busy
            {
                out.push(Action::CookieSignIn);
            }

            if !sign_in.status.is_empty() {
                ui.add_space(18.0);
                let indent = if sign_in.busy { 26.0 } else { 0.0 };
                let status = fit(ui, &sign_in.status, medium(13.0), TEXT, width - indent, 3);
                let (row, _) = ui.allocate_exact_size(vec2(width, status.size().y.max(18.0)), Sense::hover());
                if sign_in.busy {
                    egui::Spinner::new()
                        .color(ACCENT)
                        .paint_at(ui, Rect::from_min_size(row.min + vec2(0.0, 1.0), Vec2::splat(16.0)));
                }
                ui.painter().galley(row.min + vec2(indent, 0.0), status, TEXT);
            }
            ui.add_space(20.0);
            paragraph(ui, crate::auth::SESSION_KEPT, sans(12.0), FAINT, 2);
            closed
        });
    if modal.inner || modal.should_close() {
        out.push(Action::CloseSignIn);
    }
}

fn queue_panel(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    let area = ui.max_rect();
    ui.painter().vline(area.left() + 0.5, area.y_range(), Stroke::new(1.0, veil(0.06)));
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        Frame::new().inner_margin(Margin::same(20)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let Some(item) = app.now_playing() else {
                message(
                    ui,
                    icon::VINYL_RECORD,
                    "Nothing playing",
                    "Pick a song and the queue lines up here.",
                );
                return;
            };
            let (art, cover) = ui.allocate_exact_size(Vec2::splat(ui.available_width()), Sense::click());
            artwork(ui, &app.art, app.saved.covers.of(item), art, 12.0);
            let lift = hover_of(ui, &cover);
            ui.painter().rect_filled(art, 12.0, shade(0.4 * lift));
            ui.painter().text(
                art.center(),
                Align2::CENTER_CENTER,
                icon::ARROWS_OUT_SIMPLE,
                glyph(30.0),
                TEXT.gamma_multiply(lift),
            );
            describe(&cover, "Now playing");
            if cover.clicked() {
                out.push(Action::Expand(true));
            }
            ui.add_space(18.0);
            paragraph(ui, &item.title, bold(18.0), TEXT, 2);
            ui.add_space(4.0);
            let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
            let artist = Id::new("queue-artist");
            byline(ui, out, item, artist, line.left_center(), sans(13.5), line.width());
            ui.add_space(22.0);
            queue_rows(app, ui, true, out);
        });
    });
}

fn tab(ui: &Ui, at: Pos2, text: &str, active: bool) -> Response {
    let galley = fit(ui, text, medium(13.5), TEXT, 160.0, 1);
    let rect = Rect::from_min_size(at, vec2(galley.size().x + 28.0, 32.0));
    let response = ui.interact(rect, Id::new(("tab", text)), Sense::click());
    let hover = hover_of(ui, &response);
    ui.painter().rect_filled(rect, 16.0, veil(if active { 0.12 } else { 0.06 * hover }));
    let ink = if active { TEXT } else { MUTED.lerp_to_gamma(TEXT, hover) };
    ui.painter()
        .galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, ink);
    describe(&response, text);
    response
}

fn lyrics_view(app: &App, ui: &mut Ui, out: &mut Vec<Action>) {
    let lyrics = match app.lyrics.as_ref().map(|(_, words)| words) {
        Some(Words::Ready(lyrics)) => lyrics,
        Some(Words::Missing) => {
            message(
                ui,
                icon::SUBTITLES,
                "No lyrics for this song",
                "YouTube Music does not have them yet.",
            );
            return;
        }
        Some(Words::Failed) => {
            message(ui, icon::WARNING_CIRCLE, "Lyrics did not load", "Open this tab again to retry.");
            return;
        }
        Some(Words::Loading) | None => {
            let pulse = veil(0.05 + 0.025 * (ui.input(|input| input.time) * 3.2).sin() as f32);
            ui.ctx().request_repaint_after(Duration::from_millis(60));
            for share in [0.7, 0.9, 0.55, 0.8, 0.65, 0.4] {
                let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
                let bar = Rect::from_min_size(row.min + vec2(0.0, 10.0), vec2(row.width() * share, 20.0));
                ui.painter().rect_filled(bar, 6.0, pulse);
            }
            return;
        }
    };
    let position = app.player.position_ms();
    let sung = lyrics
        .synced
        .then(|| lyrics.lines.iter().rposition(|(start, _)| *start <= position))
        .flatten();
    let follow = Id::new("lyrics-follow");
    let moved = ui.data(|data| data.get_temp::<Option<usize>>(follow)) != Some(sung);
    ui.data_mut(|data| data.insert_temp(follow, sung));
    let width = ui.available_width();
    let sense = if lyrics.synced { Sense::click() } else { Sense::hover() };
    for (index, (start, line)) in lyrics.lines.iter().enumerate() {
        if line.is_empty() {
            ui.add_space(18.0);
            continue;
        }
        let galley = fit(ui, line, bold(20.0), TEXT, width, 4);
        let (rect, response) = ui.allocate_exact_size(vec2(width, galley.size().y + 14.0), sense);
        let current = sung == Some(index);
        if current && moved {
            ui.scroll_to_rect(rect, Some(Align::Center));
        }
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let lit = ui
            .ctx()
            .animate_bool_with_time(response.id.with("lit"), current || !lyrics.synced, 0.25);
        let hover = hover_of(ui, &response);
        let ink = TEXT.gamma_multiply(0.34 + 0.66 * lit.max(0.5 * hover));
        ui.painter().galley_with_override_text_color(rect.min + vec2(0.0, 7.0), galley, ink);
        if lyrics.synced {
            point_at(&response);
        }
        if response.clicked() {
            out.push(Action::Seek(*start));
        }
    }
    ui.add_space(12.0);
    paragraph(ui, &lyrics.source, sans(12.0), FAINT, 2);
    let next = sung.map_or(0, |index| index + 1);
    if let (true, Some((start, _))) = (lyrics.synced && app.playing(), lyrics.lines.get(next)) {
        let wait = start.saturating_sub(position).max(30);
        ui.ctx().request_repaint_after(Duration::from_millis(wait));
    }
}

fn queue_row(ui: &Ui, app: &App, item: &Item, slot: Rect, hover: f32) -> (Pos2, f32) {
    ui.painter()
        .rect_filled(slot.expand2(vec2(8.0, 0.0)), ROW_RADIUS, veil(0.06 * hover));
    let art = Rect::from_min_size(pos2(slot.left(), slot.center().y - 20.0), Vec2::splat(40.0));
    artwork(ui, &app.art, app.saved.covers.of(item), art, 5.0);
    let (left, width) = (art.right() + 12.0, slot.right() - art.right() - 44.0);
    label(
        ui,
        pos2(left, slot.center().y - 9.0),
        Align2::LEFT_CENTER,
        &item.title,
        medium(13.5),
        TEXT,
        width,
    );
    (pos2(left, slot.center().y + 9.0), width)
}

fn queue_rows(app: &App, ui: &mut Ui, heading: bool, out: &mut Vec<Action>) -> bool {
    let queue = &app.saved.queue;
    let upcoming = app.saved.current.map_or(0, |current| current + 1);
    if upcoming >= queue.len() {
        return false;
    }
    if heading {
        eyebrow(ui, "Next up", 0.0);
    }
    let top = ui.cursor().top();
    let pointer = ui.ctx().pointer_interact_pos();
    let grab = Id::new("queue-grab");
    let rows: Vec<(Rect, Response)> = (upcoming..queue.len())
        .map(|_| ui.allocate_exact_size(vec2(ui.available_width(), QUEUE_ROW), Sense::click_and_drag()))
        .collect();
    let carried = rows.iter().position(|(_, row)| row.dragged() || row.drag_stopped());
    if let (Some((slot, _)), Some(pointer)) = (rows.iter().find(|(_, row)| row.drag_started()), pointer) {
        ui.data_mut(|data| data.insert_temp(grab, pointer.y - slot.top()));
    }
    let held = ui.data(|data| data.get_temp::<f32>(grab)).unwrap_or(QUEUE_ROW / 2.0);
    let moving = carried.zip(pointer).map(|(from, pointer)| {
        let lifted = pointer.y - held;
        let over = ((lifted + QUEUE_ROW / 2.0 - top) / QUEUE_ROW).floor().max(0.0) as usize;
        (from, over.min(rows.len() - 1), lifted)
    });
    let glide = if moving.is_some() { 0.12 } else { 0.0 };
    for (at, (slot, response)) in rows.iter().enumerate() {
        let (index, item) = (upcoming + at, &queue[upcoming + at]);
        let aside = match moving {
            Some((from, to, _)) if from < at && at <= to => -QUEUE_ROW,
            Some((from, to, _)) if to <= at && at < from => QUEUE_ROW,
            _ => 0.0,
        };
        let aside = ui.ctx().animate_value_with_time(response.id.with("aside"), aside, glide);
        let slot = slot.translate(vec2(0.0, aside));
        if moving.is_some_and(|(from, ..)| from == at) || !ui.is_rect_visible(slot) {
            continue;
        }
        let hover = hover_of(ui, response);
        let (below, width) = queue_row(ui, app, item, slot, hover);
        byline(ui, out, item, response.id.with("artist"), below, sans(12.5), width);
        describe(response, &item.title);
        if hover > 0.0 {
            let close = Rect::from_center_size(pos2(slot.right() - 12.0, slot.center().y), Vec2::splat(28.0));
            let tone = MUTED.gamma_multiply(hover);
            if icon_button(
                ui,
                close,
                response.id.with("remove"),
                icon::X,
                glyph(13.0),
                tone,
                "Remove from queue",
            )
            .clicked()
            {
                out.push(Action::Remove(index));
            }
        }
        if response.clicked() {
            out.push(Action::Jump(index));
        }
    }

    let Some((from, to, lifted)) = moving else { return true };
    let (slot, response) = &rows[from];
    if response.drag_stopped() {
        let gap = if to > from { to + 1 } else { to };
        out.push(Action::Move(upcoming + from, upcoming + gap));
        return true;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let lifted = slot.translate(vec2(0.0, lifted - slot.top()));
    let plate = lifted.expand2(vec2(8.0, 0.0));
    let layer = egui::LayerId::new(egui::Order::Tooltip, grab);
    let ghost = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(lifted));
    let shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: shade(0.5),
    };
    ghost.painter().add(shadow.as_shape(plate, ROW_RADIUS));
    ghost.painter().rect_filled(plate, ROW_RADIUS, FLOATING);
    let item = &queue[upcoming + from];
    let (below, width) = queue_row(&ghost, app, item, lifted, 0.0);
    label(&ghost, below, Align2::LEFT_CENTER, &item.subtitle, sans(12.5), MUTED, width);
    true
}

fn now_playing(app: &App, ui: &mut Ui, out: &mut Vec<Action>) -> Color32 {
    let area = ui.max_rect();
    let Some(item) = app.now_playing() else { return PANEL };
    let shown = entrance(ui, Id::new("now-playing"));
    ui.set_opacity(shown);
    let cover = app.saved.covers.of(item);
    let column = (area.width() * 0.32).clamp(320.0, 420.0);
    let stage = Rect::from_min_max(area.min, pos2(area.right() - column - 24.0, area.bottom()));
    let side = (stage.width() - 96.0).min(stage.height() - 230.0).clamp(120.0, STAGE_ART);
    let tint = |size: f32| app.art.get(ui.ctx(), cover, size).map(|picture| picture.tint);
    let glow = mood(ui, "stage", tint(side).or_else(|| tint(BAR_ART)), PANEL, 0.46);
    let glow = dimmed(glow, STAGE_LUMINANCE);
    wash(ui.painter(), area, glow, PANEL);

    let close = Rect::from_center_size(area.min + vec2(44.0, 42.0), Vec2::splat(38.0));
    if icon_button(ui, close, Id::new("stage-close"), icon::CARET_DOWN, glyph(19.0), MUTED, "Collapse").clicked() {
        out.push(Action::Expand(false));
    }

    let centered = |text: &str, font: FontId, color: Color32, rows: usize| {
        let mut job = LayoutJob::simple_singleline(text.to_owned(), font, color);
        job.halign = Align::Center;
        job.wrap = TextWrapping {
            max_width: (stage.width() - 64.0).min(side.max(440.0)),
            max_rows: rows,
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        ui.painter().layout_job(job)
    };
    let size = match item.title.chars().count() {
        0..=18 => 54.0,
        19..=40 => 42.0,
        _ => 34.0,
    };
    let title = centered(&item.title, display(size), TEXT, 2);
    let artist = centered(&item.subtitle, sans(16.0), MUTED, 1);
    let block = side + 30.0 + title.size().y + 8.0 + artist.size().y;
    let top = (stage.center().y - block / 2.0).max(stage.top() + 56.0);
    let art = Rect::from_min_size(pos2(stage.center().x - side / 2.0, top), Vec2::splat(side));
    let shadow = Shadow {
        offset: [0, 22],
        blur: 56,
        spread: 0,
        color: shade(0.45),
    };
    ui.painter().add(shadow.as_shape(art, 18.0));
    artwork(ui, &app.art, cover, art, 18.0);
    let below = art.bottom() + 30.0;
    let lower = below + title.size().y + 8.0;
    ui.painter().galley(pos2(stage.center().x, below), title, TEXT);
    let origin = pos2(stage.center().x, lower);
    ui.painter().galley(origin, artist.clone(), MUTED);
    artist_links(ui, out, item, Id::new("stage-artist"), origin, &artist);

    let card = Rect::from_min_max(
        pos2(stage.right(), area.top() + 24.0),
        pos2(area.right() - 24.0, area.bottom() - 12.0),
    );
    ui.painter().rect_filled(card, 18.0, shade(0.24));
    let lyrics = app.saved.lyrics_open;
    let mut at = card.min + vec2(14.0, 14.0);
    for (text, shows_lyrics) in [("Next up", false), ("Lyrics", true)] {
        let tab = tab(ui, at, text, lyrics == shows_lyrics);
        if tab.clicked() {
            out.push(Action::ShowLyrics(shows_lyrics));
        }
        at.x = tab.rect.right() + 6.0;
    }
    let body = Rect::from_min_max(pos2(card.left(), card.top() + 56.0), card.max);
    let margin = Margin {
        left: 20,
        right: 20,
        top: 6,
        bottom: 14,
    };
    let mut list = ui.new_child(UiBuilder::new().id_salt("stage-card").max_rect(body));
    egui::ScrollArea::vertical()
        .id_salt(lyrics)
        .auto_shrink(false)
        .show(&mut list, |ui| {
            Frame::new().inner_margin(margin).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if lyrics {
                    lyrics_view(app, ui, out);
                } else if !queue_rows(app, ui, false, out) {
                    message(ui, icon::QUEUE, "Nothing queued", "Songs you add line up here.");
                }
            });
        });
    PANEL.lerp_to_gamma(glow, shown)
}

fn player_bar(app: &App, ui: &mut Ui, out: &mut Vec<Action>) -> Scrub {
    let bar = ui.max_rect();
    let mid = bar.center().y + 3.0;
    let duration = app.player.duration_ms();
    let item = app.now_playing();
    let backdrop = ui.interact(bar, Id::new("bar-backdrop"), Sense::click());
    if backdrop.clicked() && item.is_some() {
        out.push(Action::Expand(!app.expanded));
    }

    let zone = Rect::from_min_size(bar.min, vec2(bar.width(), 12.0));
    let sense = if duration > 0 { Sense::click_and_drag() } else { Sense::hover() };
    let scrubber = ui.interact(zone, Id::new("scrubber"), sense);
    if duration > 0 {
        point_at(&scrubber);
    }
    let fraction_at = |pointer: Pos2| ((pointer.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
    let pointed = scrubber.interact_pointer_pos().map(fraction_at);
    let aimed = pointed.or(scrubber.hover_pos().map(fraction_at)).filter(|_| duration > 0);
    let mut position = app.player.position_ms().min(duration);
    if let (true, Some(fraction)) = (scrubber.dragged() || scrubber.is_pointer_button_down_on(), pointed) {
        position = (fraction * duration as f32) as u64;
    }
    if let (true, Some(fraction)) = (scrubber.clicked() || scrubber.drag_stopped(), pointed) {
        out.push(Action::Seek((fraction * duration as f32) as u64));
    }
    let scrub = Scrub {
        bar,
        played: if duration > 0 { position as f32 / duration as f32 } else { 0.0 },
        buffered: if item.is_some() { app.player.buffered() } else { 0.0 },
        hover: ui
            .ctx()
            .animate_bool_with_time(scrubber.id, scrubber.hovered() || scrubber.dragged(), HOVER_TIME),
        aim: aimed.map(|fraction| (fraction, (fraction * duration as f32) as u64)),
    };

    match item {
        Some(item) => {
            let art = Rect::from_min_size(pos2(bar.left() + 18.0, mid - BAR_ART / 2.0), Vec2::splat(BAR_ART));
            artwork(ui, &app.art, app.saved.covers.of(item), art, 6.0);
            let left = art.right() + 14.0;
            let width = (bar.center().x - 170.0 - left - 44.0).max(60.0);
            let title = label(
                ui,
                pos2(left, mid - 10.0),
                Align2::LEFT_CENTER,
                &item.title,
                medium(14.0),
                TEXT,
                width,
            );
            let reach = left + fit(ui, &item.subtitle, sans(12.5), MUTED, width, 1).size().x;
            let zone = Rect::from_min_max(art.min, pos2(title.right().max(reach), art.bottom()));
            let open = ui.interact(zone, Id::new("bar-now-playing"), Sense::click());
            let lift = hover_of(ui, &open);
            let symbol = if app.expanded { icon::CARET_DOWN } else { icon::CARET_UP };
            ui.painter().rect_filled(art, 6.0, shade(0.55 * lift));
            ui.painter()
                .text(art.center(), Align2::CENTER_CENTER, symbol, glyph(20.0), TEXT.gamma_multiply(lift));
            describe(&open, "Now playing");
            if open.clicked() {
                out.push(Action::Expand(!app.expanded));
            }
            let below = pos2(left, mid + 10.0);
            let artist = byline(ui, out, item, Id::new("bar-artist"), below, sans(12.5), width);
            let liked = app.saved.liked.iter().any(|liked| liked.video_id == item.video_id);
            let slot = Rect::from_center_size(pos2(title.right().max(artist.right()) + 26.0, mid), Vec2::splat(34.0));
            let (font, color, hint) = if liked {
                (solid(17.0), ACCENT, "Remove from Liked songs")
            } else {
                (glyph(17.0), MUTED, "Add to Liked songs")
            };
            if icon_button(ui, slot, Id::new("bar-like"), icon::HEART, font, color, hint).clicked() {
                out.push(Action::Like(item.clone()));
            }
        }
        None => {
            let art = Rect::from_min_size(pos2(bar.left() + 18.0, mid - BAR_ART / 2.0), Vec2::splat(BAR_ART));
            ui.painter().rect_filled(art, 6.0, veil(0.05));
            ui.painter()
                .text(art.center(), Align2::CENTER_CENTER, icon::VINYL_RECORD, glyph(22.0), FAINT);
            label(
                ui,
                pos2(art.right() + 14.0, mid),
                Align2::LEFT_CENTER,
                "Nothing playing",
                sans(13.5),
                FAINT,
                200.0,
            );
        }
    }

    let center = pos2(bar.center().x, mid);
    let active = item.is_some();
    let play = Rect::from_center_size(center, Vec2::splat(42.0));
    let toggle = ui.interact(play, Id::new("bar-play"), Sense::click());
    let lift = hover_of(ui, &toggle);
    let pressed = if toggle.is_pointer_button_down_on() { 1.0 } else { 0.0 };
    ui.painter()
        .circle_filled(center, 20.0 + lift - pressed, if active { TEXT } else { veil(0.14) });
    if app.player.loading() {
        egui::Spinner::new()
            .color(PANEL)
            .paint_at(ui, Rect::from_center_size(center, Vec2::splat(18.0)));
    } else {
        let symbol = if app.playing() { icon::PAUSE } else { icon::PLAY };
        ui.painter().text(
            center,
            Align2::CENTER_CENTER,
            symbol,
            solid(19.0),
            if active { PANEL } else { FAINT },
        );
    }
    let name = if app.playing() { "Pause" } else { "Play" };
    describe(&toggle, name);
    if toggle.on_hover_text(name).clicked() {
        out.push(Action::Toggle);
    }
    let around = |offset: f32, size: f32| Rect::from_center_size(center + vec2(offset, 0.0), Vec2::splat(size));
    let tone = if active { TEXT } else { FAINT };
    if icon_button(
        ui,
        around(-56.0, 36.0),
        Id::new("bar-previous"),
        icon::SKIP_BACK,
        solid(18.0),
        tone,
        "Previous",
    )
    .clicked()
    {
        out.push(Action::Previous);
    }
    if icon_button(
        ui,
        around(56.0, 36.0),
        Id::new("bar-next"),
        icon::SKIP_FORWARD,
        solid(18.0),
        tone,
        "Next",
    )
    .clicked()
    {
        out.push(Action::Next);
    }
    let shuffle = if app.saved.shuffle { ACCENT } else { MUTED };
    if icon_button(
        ui,
        around(-104.0, 34.0),
        Id::new("bar-shuffle"),
        icon::SHUFFLE,
        glyph(17.0),
        shuffle,
        "Shuffle",
    )
    .clicked()
    {
        out.push(Action::ToggleShuffle);
    }
    let (symbol, repeat, hint) = match app.saved.repeat {
        Repeat::Off => (icon::REPEAT, MUTED, "Repeat"),
        Repeat::All => (icon::REPEAT, ACCENT, "Repeat one"),
        Repeat::One => (icon::REPEAT_ONCE, ACCENT, "Stop repeating"),
    };
    if icon_button(ui, around(104.0, 34.0), Id::new("bar-repeat"), symbol, glyph(17.0), repeat, hint).clicked() {
        out.push(Action::CycleRepeat);
    }

    let mut right = bar.right() - 22.0;
    let volume = app.saved.volume;
    let track = Rect::from_min_max(pos2(right - 96.0, mid - 8.0), pos2(right, mid + 8.0));
    let slider = ui.interact(track, Id::new("bar-volume"), Sense::click_and_drag());
    point_at(&slider);
    let grip = ui
        .ctx()
        .animate_bool_with_time(slider.id, slider.hovered() || slider.dragged(), HOVER_TIME);
    if let (true, Some(pointer)) = (
        slider.dragged() || slider.is_pointer_button_down_on(),
        slider.interact_pointer_pos(),
    ) {
        out.push(Action::Volume((pointer.x - track.left()) / track.width()));
    }
    let wheel = ui.input(|input| input.smooth_scroll_delta.y);
    if slider.hovered() && wheel != 0.0 {
        out.push(Action::Volume(volume + wheel * 0.002));
    }
    let line = Rect::from_center_size(track.center(), vec2(track.width(), 4.0));
    let level = Rect::from_min_size(line.min, vec2(line.width() * volume, 4.0));
    ui.painter().rect_filled(line, 2.0, veil(0.14));
    ui.painter().rect_filled(level, 2.0, TEXT.lerp_to_gamma(ACCENT, grip));
    ui.painter().circle_filled(pos2(level.right(), mid), 6.0 * grip, TEXT);
    slider.widget_info(|| WidgetInfo::slider(true, volume as f64, "Volume"));
    right -= 96.0 + 22.0;

    let symbol = if volume == 0.0 {
        icon::SPEAKER_X
    } else if volume < 0.5 {
        icon::SPEAKER_LOW
    } else {
        icon::SPEAKER_HIGH
    };
    let memory = Id::new("volume-before-mute");
    if icon_button(
        ui,
        Rect::from_center_size(pos2(right, mid), Vec2::splat(34.0)),
        Id::new("bar-mute"),
        symbol,
        glyph(18.0),
        MUTED,
        "Mute",
    )
    .clicked()
    {
        let restored = ui.data(|data| data.get_temp::<f32>(memory)).unwrap_or(0.8);
        ui.data_mut(|data| data.insert_temp(memory, if volume > 0.0 { volume } else { 0.8 }));
        out.push(Action::Volume(if volume > 0.0 { 0.0 } else { restored }));
    }
    right -= 38.0;
    let queue = if app.saved.queue_open { ACCENT } else { MUTED };
    if icon_button(
        ui,
        Rect::from_center_size(pos2(right, mid), Vec2::splat(34.0)),
        Id::new("bar-queue"),
        icon::QUEUE,
        glyph(18.0),
        queue,
        "Queue",
    )
    .clicked()
    {
        out.push(Action::ToggleQueue);
    }
    right -= 38.0;
    let singing = app.expanded && app.saved.lyrics_open;
    if icon_button(
        ui,
        Rect::from_center_size(pos2(right, mid), Vec2::splat(34.0)),
        Id::new("bar-lyrics"),
        icon::SUBTITLES,
        glyph(18.0),
        if singing { ACCENT } else { MUTED },
        "Lyrics",
    )
    .clicked()
    {
        out.push(Action::ShowLyrics(true));
        out.push(Action::Expand(!singing));
    }
    right -= 34.0;
    if duration > 0 {
        let time = format!("{}  /  {}", clock(position), clock(duration));
        label(ui, pos2(right, mid), Align2::RIGHT_CENTER, &time, sans(12.5), MUTED, 140.0);
    }
    scrub
}

fn paint_scrubber(ctx: &egui::Context, scrub: &Scrub) {
    let painter = ctx.layer_painter(egui::LayerId::background());
    let height = 2.0 + 2.0 * scrub.hover;
    let span = |fraction: f32| Rect::from_min_size(scrub.bar.min - vec2(0.0, height / 2.0), vec2(scrub.bar.width() * fraction, height));
    painter.rect_filled(span(1.0), 0.0, rgb(0x26272C));
    painter.rect_filled(span(scrub.buffered), 0.0, rgb(0x45474E));
    painter.rect_filled(span(scrub.played), 0.0, ACCENT);
    painter.circle_filled(pos2(span(scrub.played).right(), scrub.bar.top()), 6.0 * scrub.hover, TEXT);

    let Some((fraction, ms)) = scrub.aim else { return };
    let mut painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, Id::new("scrub-time")));
    painter.set_opacity(scrub.hover);
    let text = painter.layout_no_wrap(clock(ms), medium(12.5), TEXT);
    let size = text.size() + vec2(20.0, 12.0);
    let reach = (scrub.bar.width() - size.x) / 2.0 - 8.0;
    let x = scrub.bar.center().x + (scrub.bar.width() * (fraction - 0.5)).clamp(-reach, reach);
    let bubble = Rect::from_center_size(pos2(x, scrub.bar.top() - 14.0 - size.y / 2.0), size);
    painter.rect(bubble, 9.0, FLOATING, Stroke::new(1.0, veil(0.1)), StrokeKind::Inside);
    painter.galley(bubble.center() - text.size() / 2.0, text, TEXT);
}

fn notice(app: &App, ctx: &egui::Context, out: &mut Vec<Action>) {
    let Some(notice) = &app.notice else { return };
    egui::Area::new(Id::new("notice"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -(BAR_HEIGHT + 22.0)))
        .interactable(notice.action.is_some())
        .show(ctx, |ui| {
            let text = fit(ui, &notice.text, medium(13.5), PANEL, 420.0, 2);
            let button = notice.action.as_ref().map(|(name, _)| fit(ui, name, bold(13.0), TEXT, 160.0, 1));
            let button_width = button.as_ref().map_or(0.0, |name| name.size().x + 28.0 + 14.0);
            let height = (text.size().y + 24.0).max(44.0);
            let (toast, _) = ui.allocate_exact_size(vec2(20.0 + text.size().x + button_width + 20.0, height), Sense::hover());
            ui.painter().rect_filled(toast, 22.0, TEXT);
            ui.painter()
                .galley(pos2(toast.left() + 20.0, toast.center().y - text.size().y / 2.0), text, PANEL);
            let (Some(name), Some((hint, action))) = (button, &notice.action) else {
                return;
            };
            let slot = Rect::from_min_max(
                pos2(toast.right() - 8.0 - name.size().x - 28.0, toast.center().y - 15.0),
                pos2(toast.right() - 8.0, toast.center().y + 15.0),
            );
            let response = ui.interact(slot, Id::new("notice-action"), Sense::click());
            let hover = hover_of(ui, &response);
            ui.painter().rect_filled(slot, 15.0, PANEL.lerp_to_gamma(ACCENT, hover));
            let ink = TEXT.lerp_to_gamma(ON_ACCENT, hover);
            ui.painter()
                .galley_with_override_text_color(slot.center() - name.size() / 2.0, name, ink);
            describe(&response, hint);
            if response.clicked() {
                out.push(action.clone());
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_each_artist_name_to_its_own_page() {
        let credit = |name: &str, id: &str| (name.to_owned(), id.to_owned());
        let mut song = Item {
            subtitle: "Çağrı Sinci, Şehinşah & Çağrı Sinci Jr • 4.9M plays".to_owned(),
            artists: vec![
                credit("Çağrı Sinci", "UC1"),
                credit("Şehinşah", "UC2"),
                credit("Çağrı Sinci Jr", "UC3"),
            ],
            ..Item::default()
        };
        assert_eq!(artist_spans(&song), [(0..11, "UC1"), (13..21, "UC2"), (24..38, "UC3")]);

        song.artists = vec![credit("Çağrı Sinci Jr", "UC3"), credit("Şehinşah", "UC2"), credit("Nobody", "UC9")];
        assert_eq!(artist_spans(&song), [(24..38, "UC3"), (13..21, "UC2")]);

        song.artists.clear();
        assert!(artist_spans(&song).is_empty());
        song.artist_id = "UC1".to_owned();
        assert_eq!(artist_spans(&song), [(0..51, "UC1")]);
    }

    #[test]
    fn keeps_the_now_playing_tint_dark_enough_for_white_text() {
        let contrast = |ink: Color32, paper: Color32| (luminance(ink) + 0.05) / (luminance(paper) + 0.05);
        let pale = rgb(0x86917F);
        assert!(contrast(TEXT, pale) < 3.0);
        let tint = dimmed(pale, STAGE_LUMINANCE);
        assert!(contrast(TEXT, tint) > 4.5);
        assert!(tint.g() > tint.r() && tint.r() > tint.b());
        assert_eq!(dimmed(PANEL, STAGE_LUMINANCE), PANEL);
    }

    #[test]
    fn draws_text_with_every_system_font_it_finds() {
        for (name, bytes) in fallback_fonts() {
            let mut fonts = FontDefinitions::empty();
            fonts.font_data.insert(name.clone(), Arc::new(FontData::from_static(bytes)));
            fonts.families.insert(FontFamily::Proportional, vec![name.clone()]);
            fonts.families.insert(FontFamily::Monospace, vec![name.clone()]);
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts);
            let _ = ctx.run(Default::default(), |_| {});
            let text: String = "A1中한กकبאঅத"
                .chars()
                .filter(|c| ctx.fonts_mut(|fonts| fonts.has_glyph(&sans(16.0), *c)))
                .collect();
            let galley = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(text.clone(), sans(16.0), TEXT));
            let blank = galley
                .rows
                .iter()
                .flat_map(|row| &row.glyphs)
                .any(|glyph| glyph.uv_rect.size == Vec2::ZERO);
            assert!(!blank, "{name} cannot draw {text:?}");
        }
    }
}
