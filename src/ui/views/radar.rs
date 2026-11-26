use super::{Raster, labeled, layout, out_expo};
use crate::{
    App,
    net::{Peer, State},
    ui::theme::*,
};
use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Rect, Sense, TextureOptions, pos2, vec2,
};
use std::f64::consts::TAU;

pub(in crate::ui) fn radar(ui: &mut egui::Ui, app: &mut App, peers: &[Peer]) {
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
    let scr = |(x, y): (i32, i32)| rect.min + vec2((x as f32 + 0.5) * S, (y as f32 + 0.5) * S);
    let spots = layout(&p, peers, (cx, cy), (r, r), |q| {
        if q.via_relay() || q.rtt.is_some_and(|ms| ms > 15.0) {
            0.85
        } else {
            0.5
        }
    });
    let place = |q: &Peer| spots[&q.id];
    let granted: Vec<u128> = app.grants.lock().unwrap().keys().copied().collect();
    let busy: Vec<_> = app
        .xfers
        .iter()
        .filter(|x| matches!(x.state, State::Waiting | State::Active))
        .collect();

    if resp.clicked()
        && let Some(m) = resp.interact_pointer_pos()
    {
        let hit = peers
            .iter()
            .map(|q| (q, scr(place(q)).distance(m)))
            .filter(|(_, d)| *d < 34.0)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((q, _)) = hit {
            app.sel = Some(q.id);
        }
    }

    let trail = [0.8, 0.34, 0.18, 0.08].map(|k| mix(DISC, acc, k));
    let mut img = Raster {
        w,
        h,
        px: vec![Color32::TRANSPARENT; w * h],
    };
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
        img.ring(
            cx,
            cy,
            (r * rip).round() as i32,
            mix(acc, DISC, (rip * 4.0).floor() / 4.0 * 0.8 + 0.1),
        );
    }

    let mut labels = vec![];
    for q in peers {
        let pc = COLORS[q.color as usize % COLORS.len()].1;
        let at_q = place(q);
        let ang = ((cy - at_q.1) as f64)
            .atan2((at_q.0 - cx) as f64)
            .rem_euclid(TAU);
        let pos = scr(at_q);
        let sel = app.sel == Some(q.id);
        let since = (ang - sweep).rem_euclid(TAU);
        let ping = (-since * 1.4).exp();
        let frame = |e: f64| ((e * 6.0) as usize).min(5);
        let age = q.born.elapsed().as_secs_f64();
        let gone = ((q.seen.elapsed().as_secs_f64() - 4.0) / 2.0).clamp(0.0, 1.0);
        let base = if sel {
            pc
        } else {
            lerp(MUTED, pc, 0.55 + 0.45 * ping)
        };
        let out = app.bridge.as_ref().is_some_and(|(id, ..)| *id == q.id)
            || busy.iter().any(|x| x.peer == q.name && x.up);
        let inn = granted.contains(&q.uid) || busy.iter().any(|x| x.peer == q.name && !x.up);
        if out || inn {
            for i in 0..6 {
                let fr = (t * 0.7 + i as f64 / 6.0) % 1.0;
                let fr = if inn && !out { 1.0 - fr } else { fr };
                let (x, y) = (
                    cx + ((at_q.0 - cx) as f64 * fr).round() as i32,
                    cy + ((at_q.1 - cy) as f64 * fr).round() as i32,
                );
                img.set(x, y, c(acc));
            }
        }
        img.blip(
            at_q.0,
            at_q.1,
            if age < 0.3 { 0 } else { 1 },
            c(lerp(base, DIM, gone)),
        );
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
    p.image(
        id,
        Rect::from_min_size(rect.min, vec2(w as f32 * S, h as f32 * S)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE,
    );

    let font = FontId::proportional(TXT);
    for (q, pos, pc, sel, ping, gone, since, age, ang, frame) in labels {
        if since < 1.6 {
            let burst = out_expo(since / 1.6);
            app.sp.ping.draw(
                &p,
                frame(burst),
                pos,
                S,
                c(pc).gamma_multiply(((1.0 - burst) * 0.8) as f32),
            );
        }
        if age < 0.9 {
            let e = out_expo(age / 0.9);
            app.sp
                .ping
                .draw(&p, frame(e), pos, S, c(pc).gamma_multiply((1.0 - e) as f32));
        }
        let icon = (q.color / 8) as usize % ICONS.len();
        if icon > 0 {
            let ph = t * 1.3 + ang * 3.0;
            let off = vec2(
                (3.0 * ph.cos()).round() as f32 * S,
                (-15.0 + (2.0 * ph.sin()).round()) as f32 * S,
            );
            let ic = lerp(
                if sel {
                    pc
                } else {
                    lerp(MUTED, pc, 0.7 + 0.3 * ping)
                },
                DIM,
                gone,
            );
            app.sp.icons.draw(&p, icon, pos + off, S, c(ic));
        }
        let lab = lerp(
            if sel {
                pc
            } else {
                lerp(MUTED, pc, 0.5 + 0.5 * ping)
            },
            DIM,
            gone,
        );
        p.text(
            snap(pos + vec2(0.0, 5.0 * S)),
            Align2::CENTER_TOP,
            labeled(app, q),
            font.clone(),
            c(lab),
        );
    }

    let ctr = scr((cx, cy));
    p.text(
        snap(ctr + vec2(0.0, 4.0 * S)),
        Align2::CENTER_TOP,
        "você",
        font,
        c(MUTED),
    );
    let mine = app.pick_icon.unwrap_or(app.icon) as usize;
    if mine > 0 {
        let ph = t * 1.3;
        let off = vec2(
            (3.0 * ph.cos()).round() as f32 * S,
            (-14.0 + (2.0 * ph.sin()).round()) as f32 * S,
        );
        app.sp.icons.draw(&p, mine, ctr + off, S, c(acc));
    }
}
