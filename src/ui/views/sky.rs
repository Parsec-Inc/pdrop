use super::{Raster, angle, gone, hash, labeled, pop, unit};
use crate::{
    App,
    net::{Peer, State},
    ui::theme::*,
};
use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Rect, Sense, TextureOptions, pos2, vec2,
};
use std::f64::consts::TAU;

// constelação: céu noturno, cada peer é uma estrela ligada a você
pub(in crate::ui) fn sky(ui: &mut egui::Ui, app: &mut App, peers: &[Peer]) {
    let (acc, t) = (app.acc(), app.anim);
    let (resp, p) = ui.allocate_painter(ui.available_size(), Sense::click());
    let outer = Rect::from_min_max(snap(resp.rect.min), snap(resp.rect.max)).shrink(6.0 * S);
    let rect = outer.shrink(4.0 * S);
    let (w, h) = ((rect.width() / S) as usize, (rect.height() / S) as usize);
    if w < 16 || h < 16 {
        return;
    }
    p.add(app.sp.ui.nine(7, outer, 4.0, Color32::WHITE));
    let (cx, cy) = (w as i32 / 2, h as i32 / 2);
    let (rx, ry) = (
        (w as f64 / 2.0 - 30.0).max(20.0),
        (h as f64 / 2.0 - 24.0).max(20.0),
    );
    let place = |q: &Peer| {
        let (a, k) = (angle(q.id), 0.45 + 0.5 * unit(q.id));
        (
            cx + (rx * k * a.cos()).round() as i32,
            cy - (ry * k * a.sin()).round() as i32,
        )
    };
    let scr = |(x, y): (i32, i32)| rect.min + vec2((x as f32 + 0.5) * S, (y as f32 + 0.5) * S);
    if resp.clicked()
        && let Some(m) = resp.interact_pointer_pos()
        && let Some((q, _)) = peers
            .iter()
            .map(|q| (q, scr(place(q)).distance(m)))
            .filter(|(_, d)| *d < 34.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
    {
        app.sel = Some(q.id);
    }
    let granted: Vec<u128> = app.grants.lock().unwrap().keys().copied().collect();
    let mut img = Raster {
        w,
        h,
        px: vec![Color32::TRANSPARENT; w * h],
    };
    for i in 0..(w * h / 90) as u64 {
        let (x, y) = (
            (hash(i * 2) % w as u64) as i32,
            (hash(i * 2 + 1) % h as u64) as i32,
        );
        let lit = ((t * 0.8 + unit(i + 900) * TAU).sin() > 0.85) as usize;
        img.set(x, y, c([GRID, DIM][lit]));
    }
    let mut labels = vec![];
    for q in peers {
        let (pc, fade) = (COLORS[q.color as usize % COLORS.len()].1, gone(q));
        let at = place(q);
        let sel = app.sel == Some(q.id);
        let out = app.bridge.as_ref().is_some_and(|(id, ..)| *id == q.id)
            || app.xfers.iter().any(|x| {
                x.peer == q.name && x.up && matches!(x.state, State::Waiting | State::Active)
            });
        let inn = granted.contains(&q.uid)
            || app.xfers.iter().any(|x| {
                x.peer == q.name && !x.up && matches!(x.state, State::Waiting | State::Active)
            });
        let (dx, dy) = ((at.0 - cx) as f64, (at.1 - cy) as f64);
        let steps = dx.hypot(dy) as i32 / 4;
        for i in 1..steps {
            let f = i as f64 / steps as f64;
            img.set(
                cx + (dx * f).round() as i32,
                cy + (dy * f).round() as i32,
                c(if sel { lerp(DIM, pc, 0.5) } else { GRID }),
            );
        }
        if out || inn {
            for i in 0..6 {
                let f = (t * 0.7 + i as f64 / 6.0) % 1.0;
                let f = if inn && !out { 1.0 - f } else { f };
                img.set(
                    cx + (dx * f).round() as i32,
                    cy + (dy * f).round() as i32,
                    c(acc),
                );
            }
        }
        let col = c(lerp(if sel { pc } else { lerp(MUTED, pc, 0.7) }, DIM, fade));
        let big = sel || (t * 1.7 + angle(q.id) * 5.0).sin() > 0.92;
        img.blip(at.0, at.1, 1, col);
        for k in 2..=if big { 4 } else { 2 } {
            for (a, b) in [(k, 0), (-k, 0), (0, k), (0, -k)] {
                img.set(at.0 + a, at.1 + b, col);
            }
        }
        if sel {
            img.ring(
                at.0,
                at.1,
                7 + ((t * 4.0).sin() > 0.0) as i32,
                c(lerp(acc, DIM, fade)),
            );
        }
        labels.push((q, scr(at), pc, sel, fade));
    }
    img.blip(cx, cy, 2, c(FG));
    let tex = ColorImage::new([w, h], img.px);
    match &mut app.sp.radar {
        Some(t) => t.set(tex, TextureOptions::NEAREST),
        None => app.sp.radar = Some(ui.ctx().load_texture("radar", tex, TextureOptions::NEAREST)),
    }
    let id = app.sp.radar.as_ref().unwrap().id();
    p.image(
        id,
        Rect::from_min_size(rect.min, vec2(w as f32 * S, h as f32 * S)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    let font = FontId::proportional(TXT);
    for (q, pos, pc, sel, fade) in labels {
        pop(app, &p, q, pos, pc);
        let lab = lerp(if sel { pc } else { MUTED }, DIM, fade);
        p.text(
            snap(pos + vec2(0.0, 6.0 * S)),
            Align2::CENTER_TOP,
            labeled(app, q),
            font.clone(),
            c(lab),
        );
    }
    let ctr = scr((cx, cy));
    p.text(
        snap(ctr + vec2(0.0, 5.0 * S)),
        Align2::CENTER_TOP,
        if peers.is_empty() {
            format!("procurando{}", ".".repeat((t * 2.0) as usize % 4))
        } else {
            "você".into()
        },
        font,
        c(MUTED),
    );
    let mine = app.pick_icon.unwrap_or(app.icon) as usize;
    if mine > 0 {
        app.sp
            .icons
            .draw(&p, mine, ctr + vec2(0.0, -15.0 * S), S, c(acc));
    }
}
