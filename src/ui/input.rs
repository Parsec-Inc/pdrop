use super::{
    relay_panel::RelayPanel,
    theme::{COLORS, ICONS},
};
use crate::{App, net::Peer};
use eframe::egui::{self, Key};

pub(super) enum Act {
    None,
    Answer(u64, bool),
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
    View,
    Hist,
    More,
    Mark(bool),
    Clips,
    ClipStep(isize),
    ClipPaste,
    HistStep(isize),
    HistReveal,
    HistResend,
    Cancel,
    Relays,
    Browse(isize),
    Quit,
}

pub(super) fn apply(app: &mut App, a: Act, peers: &[Peer]) {
    match a {
        Act::None => {}
        Act::Answer(id, y) => app.answer(id, y),
        Act::Step(d) => {
            let i = peers
                .iter()
                .position(|p| Some(p.id) == app.sel)
                .unwrap_or(0) as isize;
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
        Act::View => app.cycle_view(),
        Act::More => app.more = !app.more,
        Act::Hist => app.toggle_hist(),
        Act::Mark(all) => app.mark(peers, all),
        Act::Clips => app.toggle_clips(),
        Act::ClipStep(d) => app.clip_step(d),
        Act::ClipPaste => app.clip_paste(),
        Act::HistStep(d) => app.hist_step(d),
        Act::HistReveal => app.hist_reveal(),
        Act::HistResend => app.hist_resend(peers),
        Act::Cancel if app.relay_ui.is_some() => {
            if app.relay_ui.as_mut().is_some_and(|p| !p.back()) {
                app.relay_ui = None;
            }
        }
        Act::Cancel
            if app.input.is_none()
                && app.rename.is_none()
                && app.pick.is_none()
                && app.pick_icon.is_none()
                && app.hist_sel.is_none()
                && app.clip_sel.is_none() =>
        {
            app.marked.clear()
        }
        Act::Cancel => {
            (
                app.input,
                app.rename,
                app.pick,
                app.pick_icon,
                app.hist_sel,
                app.clip_sel,
            ) = (None, None, None, None, None, None)
        }
        Act::Browse(d) => {
            let step = |v: u8, n: usize| (v as isize + d).rem_euclid(n as isize) as u8;
            app.pick = app.pick.map(|v| step(v, COLORS.len()));
            app.pick_icon = app.pick_icon.map(|v| step(v, ICONS.len()));
        }
        Act::Relays => {
            app.relay_ui = match app.relay_ui {
                Some(_) => None,
                None => Some(RelayPanel::default()),
            }
        }
        Act::Quit => {}
    }
}

pub(super) fn keys(ctx: &egui::Context, app: &App) -> Act {
    let typing = ctx.text_edit_focused();
    if !typing && let Some(id) = ctx.memory(|m| m.focused()) {
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
        if let Some((t, _)) = app.pending.front().filter(|_| k(Key::Y) || k(Key::N)) {
            return Act::Answer(t.id, k(Key::Y));
        }
        if k(Key::W) {
            return Act::Relays;
        }
        if app.relay_ui.is_some() {
            return Act::None;
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
            _ if app.view == 1 && k(Key::ArrowUp) => Act::Step(-(app.cols as isize)),
            _ if app.view == 1 && k(Key::ArrowDown) => Act::Step(app.cols as isize),
            _ if ok || k(Key::S) => Act::OpenInput,
            _ if k(Key::C) => Act::PickColor,
            _ if k(Key::I) => Act::PickIcon,
            _ if k(Key::V) => Act::Clip,
            _ if k(Key::B) => Act::Bridge,
            _ if k(Key::X) => Act::CutBridges,
            _ if k(Key::R) => Act::OpenRename,
            _ if k(Key::T) => Act::Trust,
            _ if k(Key::M) => Act::Vis,
            _ if k(Key::D) => Act::View,
            _ if k(Key::G) => Act::Hist,
            _ if k(Key::O) => Act::More,
            _ if k(Key::A) => Act::Mark(i.modifiers.shift),
            _ if k(Key::P) => Act::Clips,
            _ => Act::None,
        }
    })
}
