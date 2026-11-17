use crate::{
    App,
    net::{Peer, State},
};
use eframe::egui::{self, Align, Align2, Color32, ColorImage, FontId, Key, Layout, Mesh, Pos2, Rect, RichText, Sense, Shape, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use std::{f64::consts::TAU, time::Duration};

pub type Rgb = [u8; 3];
const BG: Rgb = [17, 16, 26];
const FG: Rgb = [236, 230, 222];
const MUTED: Rgb = [132, 127, 160];
const DIM: Rgb = [70, 66, 100];
pub const COLORS: [(&str, Rgb); 8] = [
    ("coral", [255, 112, 102]),
    ("âmbar", [227, 154, 7]),
    ("lima", [148, 187, 59]),
    ("verde", [35, 201, 135]),
    ("turquesa", [12, 195, 195]),
    ("azul", [84, 179, 253]),
    ("violeta", [178, 150, 255]),
    ("magenta", [240, 126, 194]),
];
pub const ICONS: [&str; 11] = ["ponto", "gato", "robô", "laptop", "fantasma", "peixe", "cobra", "coruja", "coração", "estrela", "alien"];
const OK: Rgb = [130, 214, 170];
const WARN: Rgb = [255, 190, 90];

fn c(a: Rgb) -> Color32 {
    Color32::from_rgb(a[0], a[1], a[2])
}
fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let m = |i: usize| (a[i] as f64 * (1.0 - t) + b[i] as f64 * t) as u8;
    [m(0), m(1), m(2)]
}
fn mix(a: Rgb, b: Rgb, t: f64) -> Color32 {
    c(lerp(a, b, t))
}
fn txt(s: impl Into<String>, col: Rgb) -> RichText {
    RichText::new(s).color(c(col))
}

pub fn human(n: u64) -> String {
    let (mut v, mut i) = (n as f64, 0);
    while v >= 1024.0 && i < 4 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", ["", "KB", "MB", "GB", "TB"][i]) }
}

fn angle(id: u64) -> f64 {
    (id.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f64 / (1u64 << 24) as f64 * TAU
}

fn out_expo(x: f64) -> f64 {
    1.0 - (-6.0 * x.clamp(0.0, 1.0)).exp()
}

const S: f32 = 2.0;
const TXT: f32 = 22.0;
const DISC: Rgb = [12, 11, 19];
const GRID: Rgb = [30, 28, 46];
const INK: Rgb = [8, 7, 15];

fn snap(p: Pos2) -> Pos2 {
    pos2((p.x / S).round() * S, (p.y / S).round() * S)
}

pub struct Sheet {
    tex: TextureHandle,
    n: usize,
}

impl Sheet {
    fn load(ctx: &egui::Context, name: &str, png: &[u8], n: usize) -> Self {
        Self::load_with(ctx, name, png, n, TextureOptions::NEAREST)
    }

    fn load_with(ctx: &egui::Context, name: &str, png: &[u8], n: usize, opt: TextureOptions) -> Self {
        let im = image::load_from_memory(png).expect("sprite embutido inválido").to_rgba8();
        let size = [im.width() as usize, im.height() as usize];
        let tex = ctx.load_texture(name, ColorImage::from_rgba_unmultiplied(size, im.as_raw()), opt);
        Self { tex, n }
    }

    fn nine(&self, i: usize, r: Rect, m: f32, tint: Color32) -> Shape {
        let (fw, fh) = (self.tex.size()[0] as f32 / self.n as f32, self.tex.size()[1] as f32);
        let (u0, du) = (i as f32 / self.n as f32, 1.0 / self.n as f32);
        let r = Rect::from_min_max(snap(r.min), snap(r.max));
        let xs = [r.left(), r.left() + m * S, r.right() - m * S, r.right()];
        let ys = [r.top(), r.top() + m * S, r.bottom() - m * S, r.bottom()];
        let us = [u0, u0 + du * m / fw, u0 + du * (fw - m) / fw, u0 + du];
        let vs = [0.0, m / fh, (fh - m) / fh, 1.0];
        let mut mesh = Mesh::with_texture(self.tex.id());
        for y in 0..3 {
            for x in 0..3 {
                let rr = Rect::from_min_max(pos2(xs[x], ys[y]), pos2(xs[x + 1], ys[y + 1]));
                let uv = Rect::from_min_max(pos2(us[x], vs[y]), pos2(us[x + 1], vs[y + 1]));
                mesh.add_rect_with_uv(rr, uv, tint);
            }
        }
        Shape::mesh(mesh)
    }

    fn draw(&self, p: &egui::Painter, i: usize, at: Pos2, scale: f32, tint: Color32) {
        let n = self.n as f32;
        let i = (i % self.n) as f32;
        let size = self.tex.size_vec2() * vec2(1.0 / n, 1.0) * scale;
        let r = egui::Rect::from_center_size(pos2(at.x.round(), at.y.round()), size);
        let uv = egui::Rect::from_min_max(pos2(i / n, 0.0), pos2((i + 1.0) / n, 1.0));
        p.image(self.tex.id(), r, uv, tint);
    }
}

pub struct Sprites {
    icons: Sheet,
    ping: Sheet,
    transfer: Sheet,
    drop: Sheet,
    logo: Sheet,
    ui: Sheet,
    actions: Sheet,
    bg: Sheet,
    radar: Option<TextureHandle>,
}

impl Sprites {
    pub fn new(ctx: &egui::Context) -> Self {
        Self {
            icons: Sheet::load(ctx, "icons", include_bytes!("../assets/icons.png"), ICONS.len()),
            ping: Sheet::load(ctx, "ping", include_bytes!("../assets/ping.png"), 6),
            transfer: Sheet::load(ctx, "transfer", include_bytes!("../assets/transfer.png"), 4),
            drop: Sheet::load(ctx, "drop", include_bytes!("../assets/drop.png"), 5),
            logo: Sheet::load(ctx, "logo", include_bytes!("../assets/logo.png"), 1),
            ui: Sheet::load(ctx, "ui", include_bytes!("../assets/ui.png"), 9),
            actions: Sheet::load(ctx, "actions", include_bytes!("../assets/actions.png"), 7),
            bg: Sheet::load_with(ctx, "bg", include_bytes!("../assets/bg.png"), 1, TextureOptions::NEAREST_REPEAT),
            radar: None,
        }
    }
}

pub fn setup(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let tweak = egui::FontTweak { hinting: Some(false), subpixel_binning: Some(false), ..Default::default() };
    fonts.font_data.insert("pdrop".into(), std::sync::Arc::new(egui::FontData::from_static(include_bytes!("../assets/pdrop.ttf")).tweak(tweak)));
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(fam).or_default().insert(0, "pdrop".into());
    }
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|s| {
        for f in s.text_styles.values_mut() {
            f.size = TXT;
        }
        s.spacing.item_spacing = vec2(8.0, 6.0);
    });

    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT;
    v.window_fill = c(DISC);
    v.window_stroke = Stroke::new(S, c(DIM));
    v.window_corner_radius = egui::CornerRadius::ZERO;
    v.menu_corner_radius = egui::CornerRadius::ZERO;
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = egui::CornerRadius::ZERO;
    }
    v.extreme_bg_color = c(INK);
    v.override_text_color = Some(c(FG));
    v.selection.bg_fill = mix(BG, COLORS[0].1, 0.45);
    v.text_cursor.stroke = Stroke::new(S, c(FG));
    ctx.set_visuals(v);
}

fn pixel_scale(ctx: &egui::Context) {
    let want = ctx.native_pixels_per_point().unwrap_or(1.0).round().max(1.0);
    if ctx.pixels_per_point() != want {
        ctx.set_pixels_per_point(want);
    }
}

#[derive(Clone, Copy)]
enum Btn {
    Send,
    Clip,
    Bridge,
    Color,
    Icon,
    Name,
    Cut,
}

fn button(ui: &mut egui::Ui, sp: &Sprites, icon: Option<Btn>, label: &str, key: &str, on: bool, hot: Option<Rgb>) -> egui::Response {
    let font = FontId::proportional(TXT);
    let lab = ui.painter().layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE);
    let k = ui.painter().layout_no_wrap(key.to_string(), font, Color32::WHITE);
    let pad = 5.0 * S;
    let w = pad + icon.map_or(0.0, |_| 8.0 * S + 4.0 * S) + lab.size().x + 4.0 * S + k.size().x + 8.0 * S + pad;
    let (r, resp) = ui.allocate_exact_size(vec2((w / S).ceil() * S, 20.0 * S), if on { Sense::click() } else { Sense::hover() });
    if !ui.is_rect_visible(r) {
        return resp;
    }
    let down = on && resp.is_pointer_button_down_on();
    let hov = on && (resp.hovered() || resp.has_focus());
    let (frame, tint) = match (on, hot, down) {
        (false, ..) => (3, Color32::WHITE),
        (_, Some(h), d) => (if d { 5 } else { 4 }, if hov { mix(h, FG, 0.25) } else { c(h) }),
        (_, None, true) => (2, Color32::WHITE),
        (_, None, false) => (if hov { 1 } else { 0 }, Color32::WHITE),
    };
    let p = ui.painter();
    p.add(sp.ui.nine(frame, r, 4.0, tint));
    let dy = if down { S } else { 0.0 };
    let cy = r.top() + (r.height() - 2.0 * S) / 2.0 + dy;
    let (ink, sub) = match (on, hot) {
        (false, _) => (c(DIM), c(DIM)),
        (_, Some(h)) => (c(BG), mix(h, INK, 0.6)),
        _ => (c(FG), c(MUTED)),
    };
    let mut x = r.left() + pad;
    if let Some(b) = icon {
        sp.actions.draw(p, b as usize, pos2(x + 4.0 * S, cy), S, ink);
        x += 12.0 * S;
    }
    let ty = cy - lab.size().y / 2.0;
    p.galley(snap(pos2(x, ty)), lab.clone(), ink);
    x += lab.size().x + 4.0 * S;
    let kr = Rect::from_min_size(snap(pos2(x, cy - 7.0 * S)), vec2(k.size().x + 5.0 * S, 14.0 * S));
    let edge = match hot.filter(|_| on) {
        Some(h) => mix(h, INK, 0.6),
        None => Color32::WHITE,
    };
    p.add(sp.ui.nine(8, kr, 3.0, edge));
    p.galley(snap(pos2(kr.left() + 3.0 * S, kr.top() + S)), k, sub);
    resp
}

fn panel(ui: &egui::Ui, sp: &Sprites) {
    let p = ui.ctx().layer_painter(ui.layer_id());
    p.add(sp.ui.nine(6, ui.max_rect().expand(5.0 * S), 4.0, Color32::WHITE));
}

fn pixel_frame(m: f32) -> egui::Frame {
    egui::Frame::NONE.inner_margin(m).outer_margin(0.0)
}

enum Act {
    None,
    Answer(bool),
    Step(isize),
    OpenInput,
    Send,
    Clip,
    Bridge,
    PickColor,
    PickIcon,
    SetColor(u8),
    SetIcon(u8),
    OpenRename,
    SetName,
    CutBridges,
    Trust,
    Vis,
    Hist,
    Mark(bool),
    Clips,
    ClipStep(isize),
    ClipPaste,
    HistStep(isize),
    HistReveal,
    HistResend,
    Cancel,
    Browse(isize),
    Quit,
}

fn apply(app: &mut App, a: Act, peers: &[Peer]) {
    match a {
        Act::None => {}
        Act::Answer(y) => app.answer(y),
        Act::Step(d) => {
            let i = peers.iter().position(|p| Some(p.id) == app.sel).unwrap_or(0) as isize;
            if !peers.is_empty() {
                app.sel = Some(peers[(i + d).rem_euclid(peers.len() as isize) as usize].id);
            }
        }
        Act::OpenInput if app.sel.is_some() => (app.input, app.focus) = (Some(String::new()), true),
        Act::OpenInput => {}
        Act::Send => app.send_input(peers),
        Act::Clip => app.clip_once(peers),
        Act::Bridge => app.toggle_bridge(peers),
        Act::PickColor => (app.pick, app.pick_icon) = (Some(app.color), None),
        Act::PickIcon => (app.pick_icon, app.pick) = (Some(app.icon), None),
        Act::SetColor(i) => app.set_color(i),
        Act::SetIcon(i) => app.set_icon(i),
        Act::OpenRename => (app.rename, app.focus) = (Some(app.me.clone()), true),
        Act::SetName => app.set_name(),
        Act::CutBridges => {
            app.grants.lock().unwrap().clear();
            app.flash("pontes recebidas cortadas");
        }
        Act::Trust => app.toggle_trust(peers),
        Act::Vis => app.cycle_vis(),
        Act::Hist => app.toggle_hist(),
        Act::Mark(all) => app.mark(peers, all),
        Act::Clips => app.toggle_clips(),
        Act::ClipStep(d) => app.clip_step(d),
        Act::ClipPaste => app.clip_paste(),
        Act::HistStep(d) => app.hist_step(d),
        Act::HistReveal => app.hist_reveal(),
        Act::HistResend => app.hist_resend(peers),
        Act::Cancel if app.input.is_none() && app.rename.is_none() && app.pick.is_none() && app.pick_icon.is_none() && app.hist_sel.is_none() && app.clip_sel.is_none() => app.marked.clear(),
        Act::Cancel => (app.input, app.rename, app.pick, app.pick_icon, app.hist_sel, app.clip_sel) = (None, None, None, None, None, None),
        Act::Browse(d) => {
            let step = |v: u8, n: usize| (v as isize + d).rem_euclid(n as isize) as u8;
            app.pick = app.pick.map(|v| step(v, COLORS.len()));
            app.pick_icon = app.pick_icon.map(|v| step(v, ICONS.len()));
        }
        Act::Quit => {}
    }
}

fn keys(ctx: &egui::Context, app: &App) -> Act {
    let typing = ctx.text_edit_focused();
    if !typing
        && let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }
    ctx.input(|i| {
        if i.key_pressed(Key::Escape) {
            return Act::Cancel;
        }
        if typing {
            return Act::None;
        }
        let k = |k| i.key_pressed(k);
        let prev = k(Key::ArrowLeft) || k(Key::H) || (k(Key::Tab) && i.modifiers.shift);
        let next = k(Key::ArrowRight) || k(Key::L) || (k(Key::Tab) && !i.modifiers.shift);
        let ok = k(Key::Enter) || k(Key::Space);
        if k(Key::Q) || i.events.iter().any(|e| matches!(e, egui::Event::Copy)) {
            return Act::Quit;
        }
        if !app.pending.is_empty() && (k(Key::Y) || k(Key::N)) {
            return Act::Answer(k(Key::Y));
        }
        if let Some(v) = app.pick.or(app.pick_icon) {
            return match () {
                _ if prev => Act::Browse(-1),
                _ if next => Act::Browse(1),
                _ if ok && app.pick.is_some() => Act::SetColor(v),
                _ if ok => Act::SetIcon(v),
                _ => Act::None,
            };
        }
        if app.clip_sel.is_some() {
            match () {
                _ if k(Key::ArrowUp) => return Act::ClipStep(-1),
                _ if k(Key::ArrowDown) => return Act::ClipStep(1),
                _ if ok => return Act::ClipPaste,
                _ => {}
            }
        }
        if app.hist_sel.is_some() {
            match () {
                _ if k(Key::ArrowUp) => return Act::HistStep(-1),
                _ if k(Key::ArrowDown) => return Act::HistStep(1),
                _ if ok => return Act::HistReveal,
                _ if k(Key::E) => return Act::HistResend,
                _ => {}
            }
        }
        match () {
            _ if prev => Act::Step(-1),
            _ if next => Act::Step(1),
            _ if ok || k(Key::S) => Act::OpenInput,
            _ if k(Key::C) => Act::PickColor,
            _ if k(Key::I) => Act::PickIcon,
            _ if k(Key::V) => Act::Clip,
            _ if k(Key::B) => Act::Bridge,
            _ if k(Key::X) => Act::CutBridges,
            _ if k(Key::R) => Act::OpenRename,
            _ if k(Key::T) => Act::Trust,
            _ if k(Key::M) => Act::Vis,
            _ if k(Key::G) => Act::Hist,
            _ if k(Key::A) => Act::Mark(i.modifiers.shift),
            _ if k(Key::P) => Act::Clips,
            _ => Act::None,
        }
    })
}

pub fn draw(ui: &mut egui::Ui, app: &mut App, peers: &[Peer]) {
    let ctx = ui.ctx().clone();
    let dropped: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
    app.send_files(dropped, peers);
    let a = keys(&ctx, app);
    if matches!(a, Act::Quit) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    apply(app, a, peers);

    pixel_scale(&ctx);
    let all = ctx.content_rect();
    let tile = 32.0 * S;
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(all.width() / tile, all.height() / tile));
    ctx.layer_painter(egui::LayerId::background()).image(app.sp.bg.tex.id(), all, uv, Color32::WHITE);

    egui::Panel::top("top").frame(pixel_frame(6.0 * S)).show_separator_line(false).show(ui, |ui| header(ui, app, peers.len()));
    firewall_bar(ui, app);
    egui::Panel::bottom("bottom").frame(pixel_frame(10.0 * S)).show_separator_line(false).show(ui, |ui| {
        panel(ui, &app.sp);
        footer(ui, app, peers)
    });
    egui::Panel::right("xfers").frame(pixel_frame(10.0 * S)).show_separator_line(false).default_size(340.0).size_range(260.0..=540.0).show(ui, |ui| {
        panel(ui, &app.sp);
        if app.hist_sel.is_some() {
            history(ui, app)
        } else if app.clip_sel.is_some() {
            clips(ui, app)
        } else {
            transfers(ui, app)
        }
    });
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| radar(ui, app, peers));
}

fn header(ui: &mut egui::Ui, app: &App, n: usize) {
    let acc = app.acc();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(64.0 * S, 16.0 * S), Sense::hover());
        app.sp.logo.draw(ui.painter(), 0, snap(r.center()), S, Color32::WHITE);
        ui.label(txt(
            match app.vis() {
                crate::net::VIS_ALL => format!("visível como {}", app.me),
                crate::net::VIS_TRUSTED => format!("só confiáveis veem {}", app.me),
                _ => format!("invisível · você é {}", app.me),
            },
            MUTED,
        ));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = match n {
                0 => format!("procurando{:<3}", ".".repeat((app.anim * 2.0) as usize % 4)),
                1 => "1 por perto".to_string(),
                n => format!("{n} por perto"),
            };
            ui.label(txt(r, MUTED));
            if let Some((_, name, _)) = &app.bridge {
                ui.label(txt(format!("ponte para {name}"), acc));
            }
            let from: Vec<String> = app.grants.lock().unwrap().values().cloned().collect();
            if !from.is_empty() {
                ui.label(txt(format!("ponte de {}", from.join(", ")), acc));
            }
        });
    });
}

fn firewall_bar(ui: &mut egui::Ui, app: &mut App) {
    let Some(w) = app.firewall.lock().unwrap().clone() else { return };
    egui::Panel::top("firewall").frame(pixel_frame(4.0 * S)).show_separator_line(false).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(txt(format!("{}. Para liberar:", w.text), WARN));
            let fix = ui.add(egui::Label::new(txt(&w.fix, FG).monospace()).sense(Sense::click())).on_hover_text("clique para copiar");
            if fix.clicked() {
                ui.ctx().copy_text(w.fix.clone());
                app.flash("copiado");
            }
        });
    });
}

struct Raster {
    w: usize,
    h: usize,
    px: Vec<Color32>,
}

impl Raster {
    fn set(&mut self, x: i32, y: i32, col: Color32) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = col;
        }
    }
    fn blip(&mut self, x: i32, y: i32, r: i32, col: Color32) {
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() + dy.abs() <= r {
                    self.set(x + dx, y + dy, col);
                }
            }
        }
    }
    fn ring(&mut self, cx: i32, cy: i32, r: i32, col: Color32) {
        let (mut x, mut y, mut e) = (r, 0, 1 - r);
        while x >= y {
            for (a, b) in [(x, y), (y, x), (-y, x), (-x, y), (-x, -y), (-y, -x), (y, -x), (x, -y)] {
                self.set(cx + a, cy + b, col);
            }
            y += 1;
            if e < 0 {
                e += 2 * y + 1;
            } else {
                x -= 1;
                e += 2 * (y - x) + 1;
            }
        }
    }
}

fn radar(ui: &mut egui::Ui, app: &mut App, peers: &[Peer]) {
    let acc = app.acc();
    let t = app.anim;
    let (resp, p) = ui.allocate_painter(ui.available_size(), Sense::click());
    let rect = Rect::from_min_max(snap(resp.rect.min), snap(resp.rect.max));
    let (w, h) = ((rect.width() / S) as usize, (rect.height() / S) as usize);
    if w < 8 || h < 8 {
        return;
    }
    let (cx, cy) = (w as i32 / 2, h as i32 / 2);
    let r = ((w.min(h) as f64 / 2.0) - 20.0).max(30.0);
    let sweep = -(t * 1.1);
    let at = |ang: f64, k: f64| (cx + (r * k * ang.cos()).round() as i32, cy - (r * k * ang.sin()).round() as i32);
    let scr = |(x, y): (i32, i32)| rect.min + vec2((x as f32 + 0.5) * S, (y as f32 + 0.5) * S);
    let place = |q: &Peer| at(angle(q.id), if q.rtt.is_some_and(|ms| ms > 15.0) { 0.83 } else { 0.5 });
    let granted: Vec<u128> = app.grants.lock().unwrap().keys().copied().collect();
    let busy: Vec<_> = app.xfers.iter().filter(|x| matches!(x.state, State::Waiting | State::Active)).collect();

    if resp.clicked()
        && let Some(m) = resp.interact_pointer_pos() {
            let hit = peers.iter().map(|q| (q, scr(place(q)).distance(m))).filter(|(_, d)| *d < 34.0).min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((q, _)) = hit {
                app.sel = Some(q.id);
            }
        }

    let trail = [0.8, 0.34, 0.18, 0.08].map(|k| mix(DISC, acc, k));
    let mut img = Raster { w, h, px: vec![Color32::TRANSPARENT; w * h] };
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let (dx, dy) = ((x - cx) as f64, (cy - y) as f64);
            let d = dx.hypot(dy);
            if d > r + 0.5 {
                continue;
            }
            let mut col = c(DISC);
            if (dx == 0.0 || dy == 0.0) && (x + y) % 2 == 0 {
                col = c(GRID);
            }
            let back = (dy.atan2(dx) - sweep).rem_euclid(TAU);
            if d > r * 0.12 && back < 0.5 {
                col = trail[match back {
                    b if b < 0.035 => 0,
                    b if b < 0.14 => 1,
                    b if b < 0.3 => 2,
                    _ => 3,
                }];
            }
            img.px[y as usize * w + x as usize] = col;
        }
    }
    for k in [1.0, 2.0 / 3.0, 1.0 / 3.0] {
        img.ring(cx, cy, (r * k).round() as i32, c(DIM));
    }
    let rip = (t * 0.35) % 1.0;
    if rip > 0.05 {
        img.ring(cx, cy, (r * rip).round() as i32, mix(acc, DISC, (rip * 4.0).floor() / 4.0 * 0.8 + 0.1));
    }

    let mut labels = vec![];
    for q in peers {
        let ang = angle(q.id);
        let pc = COLORS[q.color as usize % COLORS.len()].1;
        let at_q = place(q);
        let pos = scr(at_q);
        let sel = app.sel == Some(q.id);
        let since = (ang - sweep).rem_euclid(TAU);
        let ping = (-since * 1.4).exp();
        let frame = |e: f64| ((e * 6.0) as usize).min(5);
        let age = q.born.elapsed().as_secs_f64();
        let gone = ((q.seen.elapsed().as_secs_f64() - 4.0) / 2.0).clamp(0.0, 1.0);
        let base = if sel { pc } else { lerp(MUTED, pc, 0.55 + 0.45 * ping) };
        let out = app.bridge.as_ref().is_some_and(|(id, ..)| *id == q.id) || busy.iter().any(|x| x.peer == q.name && x.up);
        let inn = granted.contains(&q.uid) || busy.iter().any(|x| x.peer == q.name && !x.up);
        if out || inn {
            for i in 0..6 {
                let fr = (t * 0.7 + i as f64 / 6.0) % 1.0;
                let fr = if inn && !out { 1.0 - fr } else { fr };
                let (x, y) = (cx + ((at_q.0 - cx) as f64 * fr).round() as i32, cy + ((at_q.1 - cy) as f64 * fr).round() as i32);
                img.set(x, y, c(acc));
            }
        }
        img.blip(at_q.0, at_q.1, if age < 0.3 { 0 } else { 1 }, c(lerp(base, DIM, gone)));
        if sel {
            let pulse = ((t * 4.0).sin() > 0.0) as i32;
            img.ring(at_q.0, at_q.1, 6 + pulse, c(lerp(acc, DIM, gone)));
        }
        labels.push((q, pos, pc, sel, ping, gone, since, age, ang, frame));
    }
    img.blip(cx, cy, 1, c(FG));

    let tex = ColorImage::new([w, h], img.px);
    match &mut app.sp.radar {
        Some(t) => t.set(tex, TextureOptions::NEAREST),
        None => app.sp.radar = Some(ui.ctx().load_texture("radar", tex, TextureOptions::NEAREST)),
    }
    let id = app.sp.radar.as_ref().unwrap().id();
    p.image(id, Rect::from_min_size(rect.min, vec2(w as f32 * S, h as f32 * S)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

    let font = FontId::proportional(TXT);
    for (q, pos, pc, sel, ping, gone, since, age, ang, frame) in labels {
        if since < 1.6 {
            let burst = out_expo(since / 1.6);
            app.sp.ping.draw(&p, frame(burst), pos, S, c(pc).gamma_multiply(((1.0 - burst) * 0.8) as f32));
        }
        if age < 0.9 {
            let e = out_expo(age / 0.9);
            app.sp.ping.draw(&p, frame(e), pos, S, c(pc).gamma_multiply((1.0 - e) as f32));
        }
        let icon = (q.color / 8) as usize % ICONS.len();
        if icon > 0 {
            let ph = t * 1.3 + ang * 3.0;
            let off = vec2((3.0 * ph.cos()).round() as f32 * S, (-15.0 + (2.0 * ph.sin()).round()) as f32 * S);
            let ic = lerp(if sel { pc } else { lerp(MUTED, pc, 0.7 + 0.3 * ping) }, DIM, gone);
            app.sp.icons.draw(&p, icon, pos + off, S, c(ic));
        }
        let lab = lerp(if sel { pc } else { lerp(MUTED, pc, 0.5 + 0.5 * ping) }, DIM, gone);
        let tags = format!("{}{}", if app.trusted(q.uid) { "*" } else { "" }, if app.marked.contains(&q.id) { "+" } else { "" });
        let name = if tags.is_empty() { q.name.clone() } else { format!("{tags} {}", q.name) };
        p.text(snap(pos + vec2(0.0, 5.0 * S)), Align2::CENTER_TOP, name, font.clone(), c(lab));
    }

    let ctr = scr((cx, cy));
    p.text(snap(ctr + vec2(0.0, 4.0 * S)), Align2::CENTER_TOP, "você", font, c(MUTED));
    let mine = app.pick_icon.unwrap_or(app.icon) as usize;
    if mine > 0 {
        let ph = t * 1.3;
        let off = vec2((3.0 * ph.cos()).round() as f32 * S, (-14.0 + (2.0 * ph.sin()).round()) as f32 * S);
        app.sp.icons.draw(&p, mine, ctr + off, S, c(acc));
    }
}

fn transfers(ui: &mut egui::Ui, app: &App) {
    let acc = app.acc();
    ui.label(txt("TRANSFERÊNCIAS", MUTED));
    ui.add_space(4.0 * S);
    if app.xfers.is_empty() {
        ui.label(txt("Nada por aqui ainda.", FG));
        ui.label(txt("Escolha alguém no radar e aperte enter, ou arraste um arquivo para a janela.", MUTED));
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for t in app.xfers.iter().rev() {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0 * S, 18.0 * S), Sense::hover());
                if matches!(t.state, State::Active) && t.kind == 'f' {
                    app.sp.drop.draw(ui.painter(), (app.anim * 9.0) as usize, r.center(), S, c(acc));
                } else {
                    let f = if t.kind == 'c' || t.kind == 'b' { 1 } else if t.up { 2 } else { 3 };
                    app.sp.transfer.draw(ui.painter(), f, r.center(), S, c(acc));
                }
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&t.label).strong());
                        ui.label(txt(format!("{} {}", if t.up { "para" } else { "de" }, t.peer), MUTED));
                    });
                    match &t.state {
                        State::Waiting => {
                            ui.label(txt("aguardando aceitar…", MUTED));
                        }
                        State::Active => {
                            let fr = if t.size == 0 { 1.0 } else { t.done as f32 / t.size as f32 };
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(260.0), 4.0 * S), Sense::hover());
                            let n = ((r.width() / S + 1.0) / 5.0) as usize;
                            let lit = (fr.clamp(0.0, 1.0) * n as f32).round() as usize;
                            let o = snap(r.min);
                            for i in 0..n {
                                let b = Rect::from_min_size(o + vec2(i as f32 * 5.0 * S, 0.0), vec2(4.0 * S, 4.0 * S));
                                ui.painter().rect_filled(b, 0.0, if i < lit { c(acc) } else { c(GRID) });
                            }
                            ui.label(txt(format!("{:>3}%  {}", (fr * 100.0) as u32, human(t.size)), MUTED));
                        }
                        State::Done(m) => {
                            let pre = if !t.up && t.kind == 'f' { "salvo em " } else { "" };
                            ui.label(txt(format!("ok · {pre}{m}"), OK));
                        }
                        State::Failed(m) => {
                            ui.label(txt(m.clone(), WARN));
                        }
                    }
                });
            });
            ui.add_space(5.0 * S);
        }
    });
}

fn ago(ts: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    match now.saturating_sub(ts) {
        s if s < 60 => "agora".into(),
        s if s < 3600 => format!("há {} min", s / 60),
        s if s < 86400 => format!("há {} h", s / 3600),
        s => format!("há {} d", s / 86400),
    }
}

fn history(ui: &mut egui::Ui, app: &App) {
    let acc = app.acc();
    ui.label(txt("HISTÓRICO", MUTED));
    ui.add_space(4.0 * S);
    if app.hist.is_empty() {
        ui.label(txt("Nada por aqui ainda.", FG));
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, r) in app.hist.iter().rev().enumerate() {
            let sel = app.hist_sel == Some(i);
            let row = ui.horizontal(|ui| {
                let (ic, _) = ui.allocate_exact_size(vec2(18.0 * S, 18.0 * S), Sense::hover());
                let f = if r.kind == 'c' { 1 } else if r.up { 2 } else { 3 };
                app.sp.transfer.draw(ui.painter(), f, ic.center(), S, c(if sel { acc } else { MUTED }));
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&r.label).strong());
                        ui.label(txt(format!("{} {}", if r.up { "para" } else { "de" }, r.peer), MUTED));
                    });
                    let (m, col) = if r.ok { (format!("{} · {}", human(r.size), ago(r.ts)), MUTED) } else { (format!("falhou · {}", ago(r.ts)), WARN) };
                    ui.label(txt(m, col));
                });
            });
            if sel {
                ui.painter().rect_stroke(row.response.rect.expand(2.0 * S), 0.0, Stroke::new(S, c(acc)), egui::StrokeKind::Outside);
                row.response.scroll_to_me(None);
            }
            ui.add_space(5.0 * S);
        }
    });
}

fn clips(ui: &mut egui::Ui, app: &App) {
    let acc = app.acc();
    ui.label(txt("CLIPBOARDS RECEBIDOS", MUTED));
    ui.add_space(4.0 * S);
    if app.clips.is_empty() {
        ui.label(txt("Nada recebido nesta sessão.", FG));
        ui.label(txt("Ficam só na memória e somem ao fechar o pdrop.", MUTED));
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for (i, (from, c)) in app.clips.iter().enumerate() {
            let sel = app.clip_sel == Some(i);
            let row = ui.vertical(|ui| {
                ui.label(txt(format!("de {from}"), if sel { acc } else { MUTED }));
                ui.label(RichText::new(crate::net::preview(c)));
            });
            if sel {
                ui.painter().rect_stroke(row.response.rect.expand(2.0 * S), 0.0, Stroke::new(S, self::c(acc)), egui::StrokeKind::Outside);
                row.response.scroll_to_me(None);
            }
            ui.add_space(5.0 * S);
        }
    });
}

fn well(ui: &mut egui::Ui, sp: &Sprites, edit: egui::TextEdit) -> egui::Response {
    let slot = ui.painter().add(Shape::Noop);
    let r = ui.add(edit.frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 6))).text_color(c(FG)));
    ui.painter().set(slot, sp.ui.nine(7, r.rect, 3.0, Color32::WHITE));
    r
}

fn footer(ui: &mut egui::Ui, app: &mut App, peers: &[Peer]) {
    let acc = app.acc();
    let to = peers.iter().find(|p| Some(p.id) == app.sel).map(|p| p.name.clone());
    let mut act = Act::None;
    ui.horizontal(|ui| {
        ui.set_min_height(20.0 * S);
        if let Some((t, _)) = app.pending.front().map(|(t, _)| (t.clone(), ())) {
            ui.label(txt(&t.peer, FG));
            ui.label(txt(format!("[{}]", crate::secure::code(app.uid, t.uid)), DIM));
            match t.kind {
                'c' => ui.label(txt(format!("quer colar no seu clipboard: {}", t.note), MUTED)),
                'b' => ui.label(txt("quer ligar uma ponte de clipboard: tudo que copiar vai para o seu", MUTED)),
                _ => ui.label(txt(format!("quer enviar {} ({})", t.label, human(t.size)), MUTED)),
            };
            if app.pending.len() > 1 {
                ui.label(txt(format!("+{} na fila", app.pending.len() - 1), MUTED));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, &app.sp, None, "Recusar", "N", true, None).clicked() {
                    act = Act::Answer(false);
                }
                if button(ui, &app.sp, None, "Aceitar", "Y", true, Some(WARN)).clicked() {
                    act = Act::Answer(true);
                }
            });
        } else if app.pick.is_some() {
            ui.label(txt("cor", MUTED));
            for (i, (name, rgb)) in COLORS.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(14.0 * S, 14.0 * S), Sense::click());
                let on = i as u8 == app.pick.unwrap();
                let sq = Rect::from_center_size(snap(r.center()) - vec2(0.0, if on { S } else { 0.0 }), vec2(10.0 * S, 10.0 * S));
                ui.painter().rect_filled(sq.expand(S), 0.0, if on { c(FG) } else { c(INK) });
                ui.painter().rect_filled(sq, 0.0, c(*rgb));
                ui.painter().rect_filled(Rect::from_min_size(sq.min, vec2(sq.width(), S)), 0.0, mix(*rgb, FG, 0.35));
                if resp.hovered() {
                    app.pick = Some(i as u8);
                }
                if resp.on_hover_text(*name).clicked() {
                    act = Act::SetColor(i as u8);
                }
            }
            ui.label(txt(COLORS[app.pick.unwrap() as usize].0, COLORS[app.pick.unwrap() as usize].1));
            ui.label(txt("←→ escolhe · enter confirma · esc cancela", DIM));
        } else if app.pick_icon.is_some() {
            ui.label(txt("ícone", MUTED));
            for (i, name) in ICONS.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(20.0 * S, 20.0 * S), Sense::click());
                let on = Some(i as u8) == app.pick_icon;
                if on || resp.hovered() {
                    ui.painter().add(app.sp.ui.nine(1, r, 4.0, Color32::WHITE));
                }
                app.sp.icons.draw(ui.painter(), i, r.center() - vec2(0.0, S), S, if on || resp.hovered() { c(acc) } else { c(MUTED) });
                if resp.hovered() {
                    app.pick_icon = Some(i as u8);
                }
                if resp.on_hover_text(*name).clicked() {
                    act = Act::SetIcon(i as u8);
                }
            }
            ui.label(txt(format!("{} · ←→ enter esc", ICONS[app.pick_icon.unwrap() as usize]), DIM));
        } else if let Some(buf) = app.rename.as_mut() {
            ui.label(txt("seu nome ›", acc));
            let focus = std::mem::take(&mut app.focus);
            let r = well(ui, &app.sp, egui::TextEdit::singleline(buf).char_limit(crate::NAME_MAX).desired_width(220.0));
            if focus {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if enter | button(ui, &app.sp, Some(Btn::Name), "Salvar", "Enter", true, Some(acc)).clicked() {
                act = Act::SetName;
            }
        } else if app.input.is_some() {
            let n = app.targets(peers).len();
            ui.label(txt(if n > 1 { format!("para {n} aparelhos ›") } else { format!("para {} ›", to.as_deref().unwrap_or("?")) }, acc));
            let focus = std::mem::take(&mut app.focus);
            #[allow(clippy::unnecessary_unwrap)]
            let buf = app.input.as_mut().unwrap();
            let r = well(ui, &app.sp, egui::TextEdit::singleline(buf).hint_text(txt("caminho de arquivo ou pasta (ou arraste) · ou texto livre", DIM)).desired_width(f32::INFINITY));
            if focus {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                act = Act::Send;
            }
        } else if let Some((m, _)) = app.flash.as_ref().filter(|(_, at)| at.elapsed() < Duration::from_secs(6)) {
            ui.label(txt(m.clone(), FG));
        } else if let Some(n) = &to {
            ui.label(txt("selecionado", MUTED));
            if app.marked.len() > 1 {
                ui.label(txt(format!("(envio para {} marcados)", app.marked.len()), acc));
            }
            ui.label(txt(n, acc));
            if let Some(p) = peers.iter().find(|p| Some(p.id) == app.sel) {
                ui.label(txt(format!("· código {}{}", crate::secure::code(app.uid, p.uid), if app.trusted(p.uid) { " · confiável" } else { "" }), MUTED));
            }
        } else {
            ui.label(txt("Abra o pdrop em outro aparelho da mesma rede.", MUTED));
        }
    });
    ui.add_space(4.0 * S);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0 * S, 3.0 * S);
        let has = to.is_some();
        let sp = &app.sp;
        if button(ui, sp, Some(Btn::Send), "Enviar", "Enter", has, Some(acc)).clicked() {
            act = Act::OpenInput;
        }
        let on = app.sel.is_some_and(|id| app.marked.contains(&id));
        if button(ui, sp, None, if on { "Desmarcar" } else { "Marcar" }, "A", has, on.then_some(acc)).clicked() {
            act = Act::Mark(false);
        }
        if button(ui, sp, Some(Btn::Clip), "Clipboard", "V", has, None).clicked() {
            act = Act::Clip;
        }
        let ponte = if app.bridge.is_some() { "Parar ponte" } else { "Ponte" };
        if button(ui, sp, Some(Btn::Bridge), ponte, "B", has || app.bridge.is_some(), app.bridge.is_some().then_some(acc)).clicked() {
            act = Act::Bridge;
        }
        if button(ui, sp, Some(Btn::Color), "Cor", "C", true, None).clicked() {
            act = Act::PickColor;
        }
        if button(ui, sp, Some(Btn::Icon), "Ícone", "I", true, None).clicked() {
            act = Act::PickIcon;
        }
        if button(ui, sp, Some(Btn::Name), "Nome", "R", true, None).clicked() {
            act = Act::OpenRename;
        }
        let trusted = peers.iter().any(|p| Some(p.id) == app.sel && app.trusted(p.uid));
        if button(ui, sp, None, if trusted { "Não confiar" } else { "Confiar" }, "T", has, trusted.then_some(acc)).clicked() {
            act = Act::Trust;
        }
        let vis = ["Visível: todos", "Visível: confiáveis", "Invisível"][app.vis() as usize];
        if button(ui, sp, None, vis, "M", true, (app.vis() != 0).then_some(acc)).clicked() {
            act = Act::Vis;
        }
        if button(ui, sp, None, "Histórico", "G", true, app.hist_sel.is_some().then_some(acc)).clicked() {
            act = Act::Hist;
        }
        if button(ui, sp, None, "Clipboards", "P", true, app.clip_sel.is_some().then_some(acc)).clicked() {
            act = Act::Clips;
        }
        if app.clip_sel.is_some() && button(ui, sp, None, "Colar", "Enter", !app.clips.is_empty(), Some(acc)).clicked() {
            act = Act::ClipPaste;
        }
        if app.hist_sel.is_some() {
            if button(ui, sp, None, "Mostrar na pasta", "Enter", !app.hist.is_empty(), None).clicked() {
                act = Act::HistReveal;
            }
            if button(ui, sp, None, "Reenviar", "E", has && !app.hist.is_empty(), Some(acc)).clicked() {
                act = Act::HistResend;
            }
        }
        if !app.grants.lock().unwrap().is_empty() && button(ui, sp, Some(Btn::Cut), "Cortar ponte recebida", "X", true, Some(WARN)).clicked() {
            act = Act::CutBridges;
        }
    });
    apply(app, act, peers);
}
