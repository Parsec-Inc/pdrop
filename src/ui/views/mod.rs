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
use std::f64::consts::TAU;

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
