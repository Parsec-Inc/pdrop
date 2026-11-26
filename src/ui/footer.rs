use super::{
    input::{Act, apply},
    theme::*,
    widgets::{Btn, button, well},
};
use crate::{App, fmt::human, net::Peer};
use eframe::egui::{self, Align, Color32, Key, Layout, Rect, Sense, vec2};
use std::time::Duration;

pub(super) fn footer(ui: &mut egui::Ui, app: &mut App, peers: &[Peer]) {
    let acc = app.acc();
    let to = peers
        .iter()
        .find(|p| Some(p.id) == app.sel)
        .map(|p| p.name.clone());
    let mut act = Act::None;
    ui.horizontal(|ui| {
        ui.set_min_height(20.0 * S);
        if let Some((t, _)) = app.pending.front().map(|(t, _)| (t.clone(), ())) {
            ui.label(txt(&t.peer, FG));
            ui.label(txt(
                format!("[{}]", crate::secure::code(app.uid, t.uid)),
                DIM,
            ));
            match t.kind {
                'c' => ui.label(txt(
                    format!("quer colar no seu clipboard: {}", t.note),
                    MUTED,
                )),
                'b' => ui.label(txt(
                    "quer ligar uma ponte de clipboard: tudo que copiar vai para o seu",
                    MUTED,
                )),
                _ => ui.label(txt(
                    format!("quer enviar {} ({})", t.label, human(t.size)),
                    MUTED,
                )),
            };
            if app.pending.len() > 1 {
                ui.label(txt(format!("+{} na fila", app.pending.len() - 1), MUTED));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if button(ui, &app.sp, None, "Recusar", "N", true, None).clicked() {
                    act = Act::Answer(t.id, false);
                }
                if button(ui, &app.sp, None, "Aceitar", "Y", true, Some(WARN)).clicked() {
                    act = Act::Answer(t.id, true);
                }
            });
        } else if app.pick.is_some() {
            ui.label(txt("cor", MUTED));
            for (i, (name, rgb)) in COLORS.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(14.0 * S, 14.0 * S), Sense::click());
                let on = i as u8 == app.pick.unwrap();
                let sq = Rect::from_center_size(
                    snap(r.center()) - vec2(0.0, if on { S } else { 0.0 }),
                    vec2(10.0 * S, 10.0 * S),
                );
                ui.painter()
                    .rect_filled(sq.expand(S), 0.0, if on { c(FG) } else { c(INK) });
                ui.painter().rect_filled(sq, 0.0, c(*rgb));
                ui.painter().rect_filled(
                    Rect::from_min_size(sq.min, vec2(sq.width(), S)),
                    0.0,
                    mix(*rgb, FG, 0.35),
                );
                if resp.hovered() {
                    app.pick = Some(i as u8);
                }
                if resp.on_hover_text(*name).clicked() {
                    act = Act::SetColor(i as u8);
                }
            }
            ui.label(txt(
                COLORS[app.pick.unwrap() as usize].0,
                COLORS[app.pick.unwrap() as usize].1,
            ));
            ui.label(txt("←→ escolhe · enter confirma · esc cancela", DIM));
        } else if app.pick_icon.is_some() {
            ui.label(txt("ícone", MUTED));
            for (i, name) in ICONS.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(20.0 * S, 20.0 * S), Sense::click());
                let on = Some(i as u8) == app.pick_icon;
                if on || resp.hovered() {
                    ui.painter().add(app.sp.ui.nine(1, r, 4.0, Color32::WHITE));
                }
                app.sp.icons.draw(
                    ui.painter(),
                    i,
                    r.center() - vec2(0.0, S),
                    S,
                    if on || resp.hovered() {
                        c(acc)
                    } else {
                        c(MUTED)
                    },
                );
                if resp.hovered() {
                    app.pick_icon = Some(i as u8);
                }
                if resp.on_hover_text(*name).clicked() {
                    act = Act::SetIcon(i as u8);
                }
            }
            ui.label(txt(
                format!("{} · ←→ enter esc", ICONS[app.pick_icon.unwrap() as usize]),
                DIM,
            ));
        } else if let Some(buf) = app.rename.as_mut() {
            ui.label(txt("seu nome ›", acc));
            let focus = std::mem::take(&mut app.focus);
            let r = well(
                ui,
                &app.sp,
                egui::TextEdit::singleline(buf)
                    .char_limit(crate::app::NAME_MAX)
                    .desired_width(220.0),
            );
            if focus {
                r.request_focus();
            }
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if enter
                | button(
                    ui,
                    &app.sp,
                    Some(Btn::Name),
                    "Salvar",
                    "Enter",
                    true,
                    Some(acc),
                )
                .clicked()
            {
                act = Act::SetName;
            }
        } else if app.input.is_some() {
            let n = app.targets(peers).len();
            ui.label(txt(
                if n > 1 {
                    format!("para {n} aparelhos ›")
                } else {
                    format!("para {} ›", to.as_deref().unwrap_or("?"))
                },
                acc,
            ));
            let focus = std::mem::take(&mut app.focus);
            #[allow(clippy::unnecessary_unwrap)]
            let buf = app.input.as_mut().unwrap();
            let r = well(
                ui,
                &app.sp,
                egui::TextEdit::singleline(buf)
                    .hint_text(txt(
                        "caminho de arquivo ou pasta (ou arraste) · ou texto livre",
                        DIM,
                    ))
                    .desired_width(f32::INFINITY),
            );
            if focus {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                act = Act::Send;
            }
        } else if let Some((m, _)) = app
            .flash
            .as_ref()
            .filter(|(_, at)| at.elapsed() < Duration::from_secs(6))
        {
            ui.label(txt(m.clone(), FG));
        } else if let Some(n) = &to {
            ui.label(txt(n, acc));
            if let Some(p) = peers.iter().find(|p| Some(p.id) == app.sel) {
                ui.label(txt(
                    format!(
                        "· {}{}{}",
                        crate::secure::code(app.uid, p.uid),
                        if app.trusted(p.uid) {
                            " · confiável"
                        } else {
                            ""
                        },
                        if app.marked.len() > 1 {
                            format!(" · {} marcados", app.marked.len())
                        } else {
                            String::new()
                        }
                    ),
                    MUTED,
                ));
            }
        } else {
            ui.label(txt("Abra o pdrop em outro aparelho da mesma rede.", MUTED));
        }
    });
    ui.add_space(4.0 * S);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0 * S, 3.0 * S);
        let has = to.is_some();
        let sp = &app.sp;
        if button(ui, sp, Some(Btn::Send), "Enviar", "Enter", has, Some(acc)).clicked() {
            act = Act::OpenInput;
        }
        let on = app.sel.is_some_and(|id| app.marked.contains(&id));
        if button(
            ui,
            sp,
            None,
            if on { "Desmarcar" } else { "Marcar" },
            "A",
            has,
            on.then_some(acc),
        )
        .clicked()
        {
            act = Act::Mark(false);
        }
        if button(ui, sp, Some(Btn::Clip), "Clipboard", "V", has, None).clicked() {
            act = Act::Clip;
        }
        let ponte = if app.bridge.is_some() {
            "Parar ponte"
        } else {
            "Ponte"
        };
        if button(
            ui,
            sp,
            Some(Btn::Bridge),
            ponte,
            "B",
            has || app.bridge.is_some(),
            app.bridge.is_some().then_some(acc),
        )
        .clicked()
        {
            act = Act::Bridge;
        }
        let trusted = peers
            .iter()
            .any(|p| Some(p.id) == app.sel && app.trusted(p.uid));
        if button(
            ui,
            sp,
            None,
            if trusted { "Não confiar" } else { "Confiar" },
            "T",
            has,
            trusted.then_some(acc),
        )
        .clicked()
        {
            act = Act::Trust;
        }
        if app.clip_sel.is_some()
            && button(
                ui,
                sp,
                None,
                "Colar",
                "Enter",
                !app.clips.is_empty(),
                Some(acc),
            )
            .clicked()
        {
            act = Act::ClipPaste;
        }
        if app.hist_sel.is_some() {
            if button(
                ui,
                sp,
                None,
                "Mostrar na pasta",
                "Enter",
                !app.hist.is_empty(),
                None,
            )
            .clicked()
            {
                act = Act::HistReveal;
            }
            if button(
                ui,
                sp,
                None,
                "Reenviar",
                "E",
                has && !app.hist.is_empty(),
                Some(acc),
            )
            .clicked()
            {
                act = Act::HistResend;
            }
        }
        if !app.grants.lock().unwrap().is_empty()
            && button(
                ui,
                sp,
                Some(Btn::Cut),
                "Cortar ponte recebida",
                "X",
                true,
                Some(WARN),
            )
            .clicked()
        {
            act = Act::CutBridges;
        }
        let relays_open = app.relay_ui.is_some().then_some(acc);
        if button(ui, sp, None, "Relay", "W", true, relays_open).clicked() {
            act = Act::Relays;
        }
        if button(ui, sp, None, "Opções", "O", true, app.more.then_some(acc)).clicked() {
            act = Act::More;
        }
    });
    if app.more || app.hist_sel.is_some() || app.clip_sel.is_some() {
        ui.add_space(3.0 * S);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0 * S, 3.0 * S);
            let sp = &app.sp;
            if button(ui, sp, Some(Btn::Color), "Cor", "C", true, None).clicked() {
                act = Act::PickColor;
            }
            if button(ui, sp, Some(Btn::Icon), "Ícone", "I", true, None).clicked() {
                act = Act::PickIcon;
            }
            if button(ui, sp, Some(Btn::Name), "Nome", "R", true, None).clicked() {
                act = Act::OpenRename;
            }
            let vis = ["Visível: todos", "Visível: confiáveis", "Invisível"][app.vis() as usize];
            if button(
                ui,
                sp,
                None,
                vis,
                "M",
                true,
                (app.vis() != 0).then_some(acc),
            )
            .clicked()
            {
                act = Act::Vis;
            }
            if button(
                ui,
                sp,
                None,
                &format!("Visão: {}", crate::VIEWS[app.view as usize]),
                "D",
                true,
                None,
            )
            .clicked()
            {
                act = Act::View;
            }
            if button(
                ui,
                sp,
                None,
                "Histórico",
                "G",
                true,
                app.hist_sel.is_some().then_some(acc),
            )
            .clicked()
            {
                act = Act::Hist;
            }
            if button(
                ui,
                sp,
                None,
                "Clipboards",
                "P",
                true,
                app.clip_sel.is_some().then_some(acc),
            )
            .clicked()
            {
                act = Act::Clips;
            }
        });
    }
    apply(app, act, peers);
}
