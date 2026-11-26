mod grid;
mod radar;
mod sky;

pub(super) use grid::grid;
pub(super) use radar::radar;
pub(super) use sky::sky;

use super::theme::*;
use crate::{
    App,
    net::{Peer, State},
};
use eframe::egui::{self, Color32, Pos2};
use std::{collections::HashMap, f32::consts::PI, f64::consts::TAU};

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
            for (a, b) in [
                (x, y),
                (y, x),
                (-y, x),
                (-x, y),
                (-x, -y),
                (-y, -x),
                (y, -x),
                (x, -y),
            ] {
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

fn angle(id: u64) -> f64 {
    (id.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f64 / (1u64 << 24) as f64 * TAU
}

const ABOVE: f32 = 46.0;
const BELOW: f32 = 36.0;
const ICON_HALF: f32 = 22.0;

fn half_width(p: &egui::Painter, q: &Peer) -> f32 {
    let relay = if q.via_relay() { " · relay" } else { "" };
    text_half(p, format!("*+ {}{relay}", q.name))
}

fn text_half(p: &egui::Painter, s: String) -> f32 {
    let font = egui::FontId::proportional(TXT);
    let w = p.layout_no_wrap(s, font, Color32::WHITE).size().x;
    (w / 2.0).max(ICON_HALF) + 2.0
}

fn view_bounds((w, h): (usize, usize)) -> egui::Rect {
    let half = egui::vec2((w / 2) as f32 * S, (h / 2) as f32 * S);
    egui::Rect::from_min_max((-half).to_pos2(), half.to_pos2())
}

fn clipped<'a>(spots: impl Iterator<Item = ((i32, i32), f32)> + 'a, size: (usize, usize)) -> usize {
    let (mid, bounds) = (
        ((size.0 / 2) as i32, (size.1 / 2) as i32),
        view_bounds(size),
    );
    spots
        .filter(|&((x, y), half)| !bounds.contains_rect(footprint((x - mid.0, y - mid.1), half)))
        .count()
}

fn footprint((x, y): (i32, i32), half: f32) -> egui::Rect {
    let at = egui::pos2(x as f32 * S, y as f32 * S);
    egui::Rect::from_min_max(at - egui::vec2(half, ABOVE), at + egui::vec2(half, BELOW))
}

fn spread(
    wants: &[(f64, f64, f32)],
    (rx, ry): (f64, f64),
    bounds: egui::Rect,
    center_half: f32,
) -> Vec<((i32, i32), bool)> {
    const TURNS: i32 = 90;
    let mut taken = vec![footprint((0, 0), center_half)];
    wants
        .iter()
        .map(|&(ang, want, half)| {
            let at = |k: f64, turn: f64| {
                (
                    (rx * k * (ang + turn).cos()).round() as i32,
                    -(ry * k * (ang + turn).sin()).round() as i32,
                )
            };
            let free = |at| {
                let me = footprint(at, half);
                !taken.iter().any(|o| o.intersect(me).is_positive())
            };
            let mut best: Option<(f64, (i32, i32))> = None;
            for k in (5..=20).map(|k| k as f64 * 0.05) {
                for i in 0..TURNS {
                    let turn = i as f64 / TURNS as f64 * TAU;
                    let cost = turn.min(TAU - turn) + 4.0 * (k - want).abs();
                    if best.is_some_and(|(b, _)| cost >= b) {
                        continue;
                    }
                    let p = at(k, turn);
                    if bounds.contains_rect(footprint(p, half)) && free(p) {
                        best = Some((cost, p));
                    }
                }
            }
            let spot = match best {
                Some((_, p)) => (p, true),
                None => {
                    let turns = (0..TURNS).map(|i| {
                        let side = if i % 2 == 0 { -1.0 } else { 1.0 };
                        side * ((i + 1) / 2) as f64 / TURNS as f64 * TAU
                    });
                    let p = (5..)
                        .find_map(|k| {
                            turns
                                .clone()
                                .map(|t| at(k as f64 * 0.05, t))
                                .find(|&p| free(p))
                        })
                        .unwrap();
                    (p, bounds.contains_rect(footprint(p, half)))
                }
            };
            taken.push(footprint(spot.0, half));
            spot
        })
        .collect()
}

#[derive(Clone, Copy)]
struct Spot {
    at: (i32, i32),
    fits: bool,
    half: f32,
}

type Cached = (
    (Vec<(f64, f64, f32)>, (f64, f64), egui::Rect, f32),
    Vec<((i32, i32), bool)>,
);

fn layout(
    p: &egui::Painter,
    peers: &[Peer],
    (w, h): (usize, usize),
    radii: (f64, f64),
    dist: impl Fn(&Peer) -> f64,
) -> HashMap<u64, Spot> {
    let mut order: Vec<&Peer> = peers.iter().collect();
    order.sort_by_key(|q| (q.born, q.id));
    let wants: Vec<_> = order
        .iter()
        .map(|q| (angle(q.id), dist(q), half_width(p, q)))
        .collect();
    let bounds = view_bounds((w, h));
    let halves: Vec<f32> = wants.iter().map(|w| w.2).collect();
    let key = (wants, radii, bounds, text_half(p, "você".into()));
    let id = egui::Id::unique("spots");
    let cached = p.ctx().data(|d| d.get_temp::<Cached>(id));
    let at = match cached.filter(|(k, _)| *k == key) {
        Some((_, at)) => at,
        None => {
            let at = spread(&key.0, key.1, key.2, key.3);
            p.ctx().data_mut(|d| d.insert_temp(id, (key, at.clone())));
            at
        }
    };
    order
        .iter()
        .zip(at)
        .zip(halves)
        .map(|((q, (at, fits)), half)| (q.id, Spot { at, fits, half }))
        .collect()
}

fn glide(from: egui::Vec2, to: egui::Vec2, t: f32) -> egui::Vec2 {
    if (to - from).length() < 0.5 {
        return to;
    }
    let turn = (to.angle() - from.angle() + PI).rem_euclid(2.0 * PI) - PI;
    egui::Vec2::angled(from.angle() + turn * t) * egui::lerp(from.length()..=to.length(), t)
}

type Scene = ((i32, i32), HashMap<u64, (i32, i32)>, usize);

fn camera(
    app: &mut App,
    resp: &egui::Response,
    spots: &HashMap<u64, Spot>,
    (w, h): (usize, usize),
) -> Scene {
    let sel = app.sel.and_then(|id| Some((id, *spots.get(&id)?)));
    let key = sel.map(|(id, s)| (app.view, id, s.at, (w, h)));
    if app.cam_key != key {
        app.cam_key = key;
        app.pan = match sel {
            Some((_, s)) if !s.fits => egui::vec2(s.at.0 as f32, s.at.1 as f32),
            _ => egui::Vec2::ZERO,
        };
    }
    if resp.dragged() {
        let far =
            |f: fn(&Spot) -> i32| spots.values().map(|s| f(s).abs()).max().unwrap_or(0) as f32;
        let (mx, my) = (far(|s| s.at.0), far(|s| s.at.1));
        app.pan -= resp.drag_delta() / S;
        app.pan = app.pan.clamp(egui::vec2(-mx, -my), egui::vec2(mx, my));
    }
    let t = if app.calm {
        1.0
    } else {
        1.0 - (-15.0 * resp.ctx.input(|i| i.stable_dt).min(0.1)).exp()
    };
    app.pan_now = if resp.dragged() || (app.pan - app.pan_now).length() < 0.5 {
        app.pan
    } else {
        app.pan_now + (app.pan - app.pan_now) * t
    };
    let center = (
        (w / 2) as i32 - app.pan_now.x.round() as i32,
        (h / 2) as i32 - app.pan_now.y.round() as i32,
    );
    app.drawn.retain(|id, _| spots.contains_key(id));
    let placed = spots
        .iter()
        .map(|(&id, s)| {
            let to = egui::vec2(s.at.0 as f32, s.at.1 as f32);
            let now = app.drawn.entry(id).or_insert(to);
            *now = glide(*now, to, t);
            let (x, y) = (now.x.round() as i32, now.y.round() as i32);
            (id, (center.0 + x, center.1 + y))
        })
        .collect::<HashMap<_, _>>();
    let hidden = clipped(placed.iter().map(|(id, &at)| (at, spots[id].half)), (w, h));
    (center, placed, hidden)
}

fn offscreen(p: &egui::Painter, rect: egui::Rect, n: usize) {
    if n > 0 {
        p.text(
            snap(rect.left_top() + egui::vec2(4.0 * S, 2.0 * S)),
            egui::Align2::LEFT_TOP,
            format!("+{n} fora da tela · arraste ou busque (F)"),
            egui::FontId::proportional(TXT),
            c(MUTED),
        );
    }
}

fn out_expo(x: f64) -> f64 {
    1.0 - (-6.0 * x.clamp(0.0, 1.0)).exp()
}

fn hash(n: u64) -> u64 {
    let mut z = n.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn unit(n: u64) -> f64 {
    (hash(n) >> 11) as f64 / (1u64 << 53) as f64
}

fn tagged(app: &App, q: &Peer) -> String {
    let tags = format!(
        "{}{}",
        if app.trusted(q.uid) { "*" } else { "" },
        if app.marked.contains(&q.id) { "+" } else { "" }
    );
    if tags.is_empty() {
        q.name.clone()
    } else {
        format!("{tags} {}", q.name)
    }
}

fn labeled(app: &App, q: &Peer) -> String {
    let name = tagged(app, q);
    if q.via_relay() {
        format!("{name} · relay")
    } else {
        name
    }
}

fn gone(q: &Peer) -> f64 {
    ((q.seen.elapsed().as_secs_f64() - 4.0) / 2.0).clamp(0.0, 1.0)
}

fn busy_with(app: &App, q: &Peer) -> bool {
    app.xfers
        .iter()
        .any(|x| x.peer == q.name && matches!(x.state, State::Waiting | State::Active))
}

fn pop(app: &App, p: &egui::Painter, q: &Peer, pos: Pos2, col: Rgb) {
    let age = q.born.elapsed().as_secs_f64();
    if age < 0.9 {
        let e = out_expo(age / 0.9);
        app.sp.ping.draw(
            p,
            ((e * 6.0) as usize).min(5),
            pos,
            S,
            c(col).gamma_multiply((1.0 - e) as f32),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(wants: &[(f64, f64, f32)], bounds: egui::Rect) -> Vec<((i32, i32), bool)> {
        let spots = spread(wants, (60.0, 45.0), bounds, 30.0);
        let mut boxes = vec![footprint((0, 0), 30.0)];
        for (&(at, fits), w) in spots.iter().zip(wants) {
            let me = footprint(at, w.2);
            assert_eq!(fits, bounds.contains_rect(me), "{me:?}");
            boxes.push(me);
        }
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                assert!(!a.intersect(*b).is_positive(), "{a:?} {b:?}");
            }
        }
        spots
    }

    #[test]
    fn peers_never_overlap() {
        let wants = vec![(1.0, 0.5, 60.0); 40];
        let rect = |x: f32, y: f32| egui::Rect::from_min_max(egui::pos2(-x, -y), egui::pos2(x, y));
        let wide = check(&wants[..4], rect(640.0, 480.0));
        assert!(wide.iter().all(|s| s.1), "cabia na tela");
        let tight = check(&wants, rect(200.0, 140.0));
        assert!(tight.iter().any(|s| !s.1));
        assert_eq!(check(&wants[..5], rect(200.0, 140.0)), tight[..5]);
    }

    #[test]
    fn cut_labels_count_as_off_screen() {
        let spots = [((100, 100), 60.0), ((190, 100), 60.0), ((400, 100), 60.0)];
        assert_eq!(clipped(spots.into_iter(), (200, 200)), 2);
    }

    #[test]
    fn glide_orbits_and_arrives() {
        let (mut at, to) = (egui::vec2(100.0, 0.0), egui::vec2(-100.0, 1.0));
        for _ in 0..60 {
            at = glide(at, to, 0.2);
            assert!(at.length() > 99.0, "cortou pelo centro: {at:?}");
        }
        assert_eq!(at, to);
    }
}
