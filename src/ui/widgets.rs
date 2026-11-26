use super::{sprites::Sprites, theme::*};
use eframe::egui::{self, Color32, FontId, Rect, Sense, Shape, pos2, vec2};

#[derive(Clone, Copy)]
pub(super) enum Btn {
    Send,
    Clip,
    Bridge,
    Color,
    Icon,
    Name,
    Cut,
}

pub(super) fn button(
    ui: &mut egui::Ui,
    sp: &Sprites,
    icon: Option<Btn>,
    label: &str,
    key: &str,
    on: bool,
    hot: Option<Rgb>,
) -> egui::Response {
    let font = FontId::proportional(TXT);
    let keyed = !key.is_empty();
    let lab = ui
        .painter()
        .layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE);
    let k = ui
        .painter()
        .layout_no_wrap(key.to_string(), font, Color32::WHITE);
    let pad = 5.0 * S;
    let w = pad
        + icon.map_or(0.0, |_| 8.0 * S + 4.0 * S)
        + lab.size().x
        + if keyed {
            4.0 * S + k.size().x + 8.0 * S
        } else {
            0.0
        }
        + pad;
    let (r, resp) = ui.allocate_exact_size(
        vec2((w / S).ceil() * S, 20.0 * S),
        if on { Sense::click() } else { Sense::hover() },
    );
    if !ui.is_rect_visible(r) {
        return resp;
    }
    let down = on && resp.is_pointer_button_down_on();
    let hov = on && (resp.hovered() || resp.has_focus());
    let (frame, tint) = match (on, hot, down) {
        (false, ..) => (3, Color32::WHITE),
        (_, Some(h), d) => (
            if d { 5 } else { 4 },
            if hov { mix(h, FG, 0.25) } else { c(h) },
        ),
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
        sp.actions
            .draw(p, b as usize, pos2(x + 4.0 * S, cy), S, ink);
        x += 12.0 * S;
    }
    let ty = cy - lab.size().y / 2.0;
    p.galley(snap(pos2(x, ty)), lab.clone(), ink);
    x += lab.size().x + 4.0 * S;
    if !keyed {
        return resp;
    }
    let kr = Rect::from_min_size(
        snap(pos2(x, cy - 7.0 * S)),
        vec2(k.size().x + 5.0 * S, 14.0 * S),
    );
    let edge = match hot.filter(|_| on) {
        Some(h) => mix(h, INK, 0.6),
        None => Color32::WHITE,
    };
    p.add(sp.ui.nine(8, kr, 3.0, edge));
    p.galley(snap(pos2(kr.left() + 3.0 * S, kr.top() + S)), k, sub);
    resp
}

pub(super) fn panel(ui: &egui::Ui, sp: &Sprites) {
    let p = ui.ctx().layer_painter(ui.layer_id());
    p.add(
        sp.ui
            .nine(6, ui.max_rect().expand(5.0 * S), 4.0, Color32::WHITE),
    );
}

pub(super) fn pixel_frame(m: f32) -> egui::Frame {
    egui::Frame::NONE.inner_margin(m).outer_margin(0.0)
}

pub(super) fn well(ui: &mut egui::Ui, sp: &Sprites, edit: egui::TextEdit) -> egui::Response {
    let slot = ui.painter().add(Shape::Noop);
    let r = ui.add(
        edit.frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(8, 6)))
            .text_color(c(FG)),
    );
    ui.painter()
        .set(slot, sp.ui.nine(7, r.rect, 3.0, Color32::WHITE));
    r
}
