mod bars;
mod footer;
mod input;
mod relay_panel;
mod sidebar;
mod sprites;
mod theme;
mod views;
mod widgets;

pub use relay_panel::RelayPanel;
pub use sprites::Sprites;
pub use theme::{COLORS, ICONS, Rgb, setup};

use crate::{App, net::Peer};
use bars::{firewall_bar, header, relay_bar};
use eframe::egui::{self, Color32, Rect, pos2};
use footer::footer;
use input::{Act, apply, keys};
use sidebar::{board, clips, history, transfers};
use theme::S;
use views::{grid, radar, sky};
use widgets::{panel, pixel_frame};

pub fn draw(ui: &mut egui::Ui, app: &mut App, peers: &[Peer]) {
    let ctx = ui.ctx().clone();
    let dropped: Vec<_> = ctx.input(|i| {
        i.raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .collect()
    });
    app.send_files(dropped, peers);
    let a = keys(&ctx, app);
    if matches!(a, Act::Quit) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
    apply(app, a, peers);

    pixel_scale(&ctx);
    let all = ctx.content_rect();
    let tile = 32.0 * S;
    let uv = Rect::from_min_max(
        pos2(0.0, 0.0),
        pos2(all.width() / tile, all.height() / tile),
    );
    ctx.layer_painter(egui::LayerId::background()).image(
        app.sp.bg.tex.id(),
        all,
        uv,
        Color32::WHITE,
    );

    egui::Panel::top("top")
        .frame(pixel_frame(6.0 * S))
        .show_separator_line(false)
        .show(ui, |ui| {
            if header(ui, app, peers) {
                apply(app, Act::Relays, peers);
            }
        });
    firewall_bar(ui, app);
    relay_bar(ui, app);
    egui::Panel::bottom("bottom")
        .frame(pixel_frame(10.0 * S))
        .show_separator_line(false)
        .show(ui, |ui| {
            panel(ui, &app.sp);
            footer(ui, app, peers)
        });
    egui::Panel::right("xfers")
        .frame(pixel_frame(10.0 * S))
        .show_separator_line(false)
        .default_size(340.0)
        .size_range(260.0..=540.0)
        .show(ui, |ui| {
            panel(ui, &app.sp);
            if app.board {
                board(ui, app)
            } else if app.hist_sel.is_some() {
                history(ui, app)
            } else if app.clip_sel.is_some() {
                clips(ui, app)
            } else {
                transfers(ui, app)
            }
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| match app.view {
            _ if app.relay_ui.is_some() => relay_panel::draw(ui, app),
            1 => grid(ui, app, peers),
            2 => sky(ui, app, peers),
            _ => radar(ui, app, peers),
        });
}

fn pixel_scale(ctx: &egui::Context) {
    let want = ctx
        .native_pixels_per_point()
        .unwrap_or(1.0)
        .round()
        .max(1.0);
    if ctx.pixels_per_point() != want {
        ctx.set_pixels_per_point(want);
    }
}
