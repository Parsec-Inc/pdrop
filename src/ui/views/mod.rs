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
use std::{collections::HashMap, f64::consts::TAU};

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

// quanto cada peer ocupa em volta do ponto, em pontos de tela: ícone acima, nome abaixo
const ABOVE: f32 = 46.0;
const BELOW: f32 = 36.0;
const ICON_HALF: f32 = 22.0;

// meia largura do espaço de um peer. Reserva as marcas `*+` para ninguém pular ao ser marcado
fn half_width(p: &egui::Painter, q: &Peer) -> f32 {
    let relay = if q.via_relay() { " · relay" } else { "" };
    text_half(p, format!("*+ {}{relay}", q.name))
}

fn text_half(p: &egui::Painter, s: String) -> f32 {
    let font = egui::FontId::proportional(TXT);
    let w = p.layout_no_wrap(s, font, Color32::WHITE).size().x;
    (w / 2.0).max(ICON_HALF) + 2.0
}

fn footprint((x, y): (i32, i32), half: f32) -> egui::Rect {
    let at = egui::pos2(x as f32 * S, y as f32 * S);
    egui::Rect::from_min_max(at - egui::vec2(half, ABOVE), at + egui::vec2(half, BELOW))
}

// Posição de cada peer, em pixels do raster a partir do centro. `wants` é (ângulo, distância
// preferida de 0 a 1, meia largura), em ordem de chegada: quem já estava não sai do lugar quando
// entra alguém. Entre as vagas livres, fica a mais perto da preferida, mudando antes o ângulo que
// a distância. Sem vaga livre (janela pequena demais), fica a que menos sobrepõe.
fn spread(wants: &[(f64, f64, f32)], (rx, ry): (f64, f64), center_half: f32) -> Vec<(i32, i32)> {
    const TURNS: i32 = 90;
    let mut taken = vec![footprint((0, 0), center_half)];
    wants
        .iter()
        .map(|&(ang, want, half)| {
            let (_, _, at) = (5..=20)
                .flat_map(|k| (0..TURNS).map(move |i| (k as f64 * 0.05, i)))
                .map(|(k, i)| {
                    let turn = i as f64 / TURNS as f64 * TAU;
                    let a = ang + turn;
                    let at = (
                        (rx * k * a.cos()).round() as i32,
                        -(ry * k * a.sin()).round() as i32,
                    );
                    let me = footprint(at, half);
                    let over: f32 = taken
                        .iter()
                        .filter(|o| o.intersects(me))
                        .map(|o| o.intersect(me).area())
                        .sum();
                    (over, turn.min(TAU - turn) + 4.0 * (k - want).abs(), at)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
                .unwrap();
            taken.push(footprint(at, half));
            at
        })
        .collect()
}

// `spread` para os peers da tela, já em coordenadas do raster
fn layout(
    p: &egui::Painter,
    peers: &[Peer],
    (cx, cy): (i32, i32),
    radii: (f64, f64),
    dist: impl Fn(&Peer) -> f64,
) -> HashMap<u64, (i32, i32)> {
    let mut order: Vec<&Peer> = peers.iter().collect();
    order.sort_by_key(|q| (q.born, q.id));
    let wants: Vec<_> = order
        .iter()
        .map(|q| (angle(q.id), dist(q), half_width(p, q)))
        .collect();
    let at = spread(&wants, radii, text_half(p, "você".into()));
    order
        .iter()
        .zip(at)
        .map(|(q, (x, y))| (q.id, (cx + x, cy + y)))
        .collect()
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

    #[test]
    fn peers_never_overlap() {
        // pior caso: todo mundo querendo o mesmo ponto
        let wants = vec![(1.0, 0.5, 60.0); 12];
        let at = spread(&wants, (300.0, 220.0), 30.0);
        let mut boxes = vec![footprint((0, 0), 30.0)];
        boxes.extend(at.iter().map(|&a| footprint(a, 60.0)));
        for (i, a) in boxes.iter().enumerate() {
            for b in &boxes[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} {b:?}");
            }
        }
        // quem chega depois não move quem já estava
        assert_eq!(spread(&wants[..5], (300.0, 220.0), 30.0), at[..5]);
    }
}
