use super::{busy_with, gone, pop, tagged};
use crate::{App, net::Peer, ui::theme::*};
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, pos2, vec2};

pub(in crate::ui) fn grid(ui: &mut egui::Ui, app: &mut App, peers: &[Peer]) {
    const TW: f32 = 56.0;
    const TH: f32 = 64.0;
    const GAP: f32 = 6.0;
    let (acc, t) = (app.acc(), app.anim);
    let area = ui.available_size();
    let cols = (((area.x / S + GAP) / (TW + GAP)) as usize).clamp(1, 12);
    app.cols = cols;
    let font = FontId::proportional(TXT);
    let mut pick = None;
    let gid = ui.make_persistent_id("gridsel");
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            let rows = peers.len().max(3).div_ceil(cols).max(1) as f32;
            let n = peers.len().max(1).min(cols) as f32;
            let (gw, gh) = (n * (TW + GAP) - GAP, rows * (TH + GAP) - GAP);
            let (rect, _) = ui.allocate_exact_size(
                vec2(area.x, (gh * S + 16.0 * S).max(area.y)),
                Sense::hover(),
            );
            let p = ui.painter().clone();
            let x0 = rect.left() + ((rect.width() - gw * S) / 2.0 / S).floor() * S;
            let y0 = rect.top() + (((area.y - gh * S) / 2.0 / S).floor() * S).max(8.0 * S);
            let slot = |i: usize| {
                Rect::from_min_size(
                    pos2(
                        x0 + (i % cols) as f32 * (TW + GAP) * S,
                        y0 + (i / cols) as f32 * (TH + GAP) * S,
                    ),
                    vec2(TW * S, TH * S),
                )
            };
            if peers.is_empty() {
                for i in 0..3 {
                    p.add(app.sp.ui.nine(3, slot(i), 4.0, Color32::WHITE));
                }
                let dots = ".".repeat((t * 2.0) as usize % 4);
                p.text(
                    snap(pos2(x0 + gw * S / 2.0, y0 + (TH + 10.0) * S)),
                    Align2::CENTER_TOP,
                    format!("procurando{dots}"),
                    font,
                    c(MUTED),
                );
                return;
            }
            for (i, q) in peers.iter().enumerate() {
                let r = slot(i);
                let resp = ui.interact(r, ui.make_persistent_id(("tile", q.id)), Sense::click());
                if resp.clicked() {
                    pick = Some(q.id);
                }
                let (sel, fade) = (app.sel == Some(q.id), gone(q));
                let pc = COLORS[q.color as usize % COLORS.len()].1;
                if sel && ui.data_mut(|d| d.get_temp::<u64>(gid) != Some(q.id)) {
                    ui.data_mut(|d| d.insert_temp(gid, q.id));
                    ui.scroll_to_rect(r.expand(8.0 * S), None);
                }
                p.add(app.sp.ui.nine(
                    if resp.hovered() && !sel { 1 } else { 7 },
                    r,
                    4.0,
                    Color32::WHITE,
                ));
                if sel {
                    p.add(app.sp.ui.nine(8, r, 3.0, c(lerp(pc, DIM, fade))));
                }
                let ic = c(lerp(
                    if sel || resp.hovered() {
                        pc
                    } else {
                        lerp(MUTED, pc, 0.75)
                    },
                    DIM,
                    fade,
                ));
                let bob = if sel {
                    ((t * 4.0).sin() > 0.0) as i32 as f32 * -S
                } else {
                    0.0
                };
                let at = snap(pos2(r.center().x, r.top() + 24.0 * S + bob));
                app.sp
                    .icons
                    .draw(&p, (q.color / 8) as usize % ICONS.len(), at, 2.0 * S, ic);
                pop(app, &p, q, at, pc);
                let mut name = tagged(app, q);
                while name.len() > 1
                    && p.layout_no_wrap(name.clone(), font.clone(), Color32::WHITE)
                        .size()
                        .x
                        > (TW - 6.0) * S
                {
                    name.pop();
                }
                p.text(
                    snap(pos2(r.center().x, r.top() + 42.0 * S)),
                    Align2::CENTER_TOP,
                    name,
                    font.clone(),
                    c(lerp(if sel { pc } else { MUTED }, DIM, fade)),
                );
                let lvl = q.rtt.map_or(0, |ms| {
                    if ms < 3.0 {
                        4
                    } else if ms < 15.0 {
                        3
                    } else if ms < 50.0 {
                        2
                    } else {
                        1
                    }
                });
                if q.via_relay() {
                    p.text(
                        snap(pos2(r.center().x, r.bottom() - 15.0 * S)),
                        Align2::CENTER_TOP,
                        "relay",
                        font.clone(),
                        c(lerp(MUTED, DIM, fade)),
                    );
                }
                let bars = if q.via_relay() { 0 } else { 4 };
                for b in 0..bars {
                    let h = 2.0 * (b + 1) as f32;
                    let br = Rect::from_min_size(
                        pos2(
                            r.left() + (TW / 2.0 - 8.0 + 4.0 * b as f32) * S,
                            r.bottom() - (5.0 + h) * S,
                        ),
                        vec2(3.0 * S, h * S),
                    );
                    p.rect_filled(
                        br,
                        0.0,
                        if b < lvl {
                            c(lerp(pc, DIM, fade))
                        } else {
                            c(GRID)
                        },
                    );
                }
                if busy_with(app, q) {
                    let lit = (t * 6.0) as usize % 3;
                    for d in 0..3 {
                        p.rect_filled(
                            Rect::from_min_size(
                                pos2(r.right() - (14.0 - 4.0 * d as f32) * S, r.top() + 4.0 * S),
                                vec2(2.0 * S, 2.0 * S),
                            ),
                            0.0,
                            if d == lit { c(acc) } else { c(DIM) },
                        );
                    }
                }
            }
        });
    if pick.is_some() {
        app.sel = pick;
    }
}
