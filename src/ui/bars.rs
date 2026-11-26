use super::{theme::*, widgets::pixel_frame};
use crate::{App, net::Peer, relay::Link};
use eframe::egui::{self, Align, Color32, Layout, Sense, vec2};

pub(super) fn header(ui: &mut egui::Ui, app: &App, peers: &[Peer]) -> bool {
    let mut open_relays = false;
    let acc = app.acc();
    let n = peers.len();
    let near = if peers.iter().any(Peer::via_relay) {
        "no radar"
    } else {
        "por perto"
    };
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(64.0 * S, 16.0 * S), Sense::hover());
        app.sp
            .logo
            .draw(ui.painter(), 0, snap(r.center()), S, Color32::WHITE);
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
                n => format!("{n} {near}"),
            };
            ui.label(txt(r, MUTED));
            if let Some(link) = app.relay.link() {
                let (text, col) = match link {
                    Link::Connecting => ("relay conectando…", MUTED),
                    Link::Up => ("relay conectado", MUTED),
                    Link::Down(_) => ("relay sem conexão", WARN),
                    Link::Off(_) => ("relay desligado", WARN),
                };
                let label = egui::Label::new(txt(text, col)).sense(Sense::click());
                open_relays = ui.add(label).on_hover_text("configurar relays").clicked();
            }
            if let Some((_, name, _)) = &app.bridge {
                ui.label(txt(format!("ponte para {name}"), acc));
            }
            let from: Vec<String> = app.grants.lock().unwrap().values().cloned().collect();
            if !from.is_empty() {
                ui.label(txt(format!("ponte de {}", from.join(", ")), acc));
            }
        });
    });
    open_relays
}

pub(super) fn firewall_bar(ui: &mut egui::Ui, app: &mut App) {
    let Some(w) = app.firewall.lock().unwrap().clone() else {
        return;
    };
    egui::Panel::top("firewall")
        .frame(pixel_frame(4.0 * S))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(txt(format!("{}. Para liberar:", w.text), WARN));
                let fix = ui
                    .add(egui::Label::new(txt(&w.fix, FG).monospace()).sense(Sense::click()))
                    .on_hover_text("clique para copiar");
                if fix.clicked() {
                    ui.ctx().copy_text(w.fix.clone());
                    app.flash("copiado");
                }
            });
        });
}

pub(super) fn relay_bar(ui: &mut egui::Ui, app: &App) {
    let Some(link) = app.relay.link() else {
        return;
    };
    let text = match link {
        Link::Down(why) => format!("relay: {why}. Tentando de novo; a rede local segue normal."),
        Link::Off(why) => format!("relay desligado: {why}"),
        Link::Connecting | Link::Up => return,
    };
    egui::Panel::top("relay")
        .frame(pixel_frame(4.0 * S))
        .show_separator_line(false)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| ui.label(txt(text, WARN)));
        });
}
