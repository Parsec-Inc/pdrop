use super::theme::*;
use crate::{App, fmt::human, net::State};
use eframe::egui::{self, Rect, RichText, Sense, Stroke, vec2};

pub(super) fn transfers(ui: &mut egui::Ui, app: &App) {
    let acc = app.acc();
    ui.label(txt("TRANSFERÊNCIAS", MUTED));
    ui.add_space(4.0 * S);
    if app.xfers.is_empty() {
        ui.label(txt("Nada por aqui ainda.", FG));
        ui.label(txt(
            "Escolha alguém no radar e aperte enter, ou arraste um arquivo para a janela.",
            MUTED,
        ));
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for t in app.xfers.iter().rev() {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0 * S, 18.0 * S), Sense::hover());
                    if matches!(t.state, State::Active) && t.kind == 'f' {
                        app.sp.drop.draw(
                            ui.painter(),
                            (app.anim * 9.0) as usize,
                            r.center(),
                            S,
                            c(acc),
                        );
                    } else {
                        let f = if t.kind == 'c' || t.kind == 'b' {
                            1
                        } else if t.up {
                            2
                        } else {
                            3
                        };
                        app.sp.transfer.draw(ui.painter(), f, r.center(), S, c(acc));
                    }
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&t.label).strong());
                            ui.label(txt(
                                format!("{} {}", if t.up { "para" } else { "de" }, t.peer),
                                MUTED,
                            ));
                        });
                        match &t.state {
                            State::Waiting => {
                                ui.label(txt("aguardando aceitar…", MUTED));
                            }
                            State::Active => {
                                let fr = if t.size == 0 {
                                    1.0
                                } else {
                                    t.done as f32 / t.size as f32
                                };
                                let (r, _) = ui.allocate_exact_size(
                                    vec2(ui.available_width().min(260.0), 4.0 * S),
                                    Sense::hover(),
                                );
                                let n = ((r.width() / S + 1.0) / 5.0) as usize;
                                let lit = (fr.clamp(0.0, 1.0) * n as f32).round() as usize;
                                let o = snap(r.min);
                                for i in 0..n {
                                    let b = Rect::from_min_size(
                                        o + vec2(i as f32 * 5.0 * S, 0.0),
                                        vec2(4.0 * S, 4.0 * S),
                                    );
                                    ui.painter().rect_filled(
                                        b,
                                        0.0,
                                        if i < lit { c(acc) } else { c(GRID) },
                                    );
                                }
                                let rate = t.t0.map_or(0.0, |(t0, d0)| {
                                    t.done.saturating_sub(d0) as f64
                                        / t0.elapsed().as_secs_f64().max(0.001)
                                });
                                let eta = if rate > 1.0 {
                                    format!(
                                        "  ·  {}/s  ·  {}",
                                        human(rate as u64),
                                        eta((t.size.saturating_sub(t.done) as f64 / rate) as u64)
                                    )
                                } else {
                                    String::new()
                                };
                                ui.label(txt(
                                    format!("{:>3}%  {}{eta}", (fr * 100.0) as u32, human(t.size)),
                                    MUTED,
                                ));
                            }
                            State::Done(_) if t.up && t.kind == 't' => {
                                ui.label(txt("ok · enviado", OK));
                                message(ui, &t.src, false);
                            }
                            State::Done(m) => {
                                let pre = if !t.up && t.kind == 'f' {
                                    "salvo em "
                                } else {
                                    ""
                                };
                                ui.label(txt(format!("ok · {pre}{m}"), OK));
                            }
                            State::Failed(m) => {
                                ui.label(txt(m.clone(), WARN));
                            }
                        }
                    });
                });
                ui.add_space(5.0 * S);
            }
        });
}

fn eta(s: u64) -> String {
    if s >= 3600 {
        format!("{}h{:02}m", s / 3600, s % 3600 / 60)
    } else if s >= 60 {
        format!("{}m{:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

fn message(ui: &mut egui::Ui, text: &str, open: bool) {
    let max = if open { usize::MAX } else { 80 };
    let mut s: String = text.chars().take(max).collect();
    if !open {
        s = s.replace(['\n', '\r', '\t'], " ");
    }
    if s.len() < text.len() {
        s.push('…');
    }
    ui.add(egui::Label::new(s).wrap().selectable(open));
}

pub(super) fn history(ui: &mut egui::Ui, app: &mut App) {
    let acc = app.acc();
    ui.label(txt("HISTÓRICO", MUTED));
    ui.add_space(4.0 * S);
    if app.hist.is_empty() {
        ui.label(txt("Nada por aqui ainda.", FG));
    }
    let mut pick = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, r) in app.hist.iter().rev().enumerate() {
                let sel = app.hist_sel == Some(i);
                let row = ui.horizontal(|ui| {
                    let (ic, _) = ui.allocate_exact_size(vec2(18.0 * S, 18.0 * S), Sense::hover());
                    let f = if r.kind == 'c' {
                        1
                    } else if r.up {
                        2
                    } else {
                        3
                    };
                    app.sp.transfer.draw(
                        ui.painter(),
                        f,
                        ic.center(),
                        S,
                        c(if sel { acc } else { MUTED }),
                    );
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&r.label).strong());
                            ui.label(txt(
                                format!("{} {}", if r.up { "para" } else { "de" }, r.peer),
                                MUTED,
                            ));
                        });
                        let (m, col) = if r.ok {
                            (format!("{} · {}", human(r.size), ago(r.ts)), MUTED)
                        } else {
                            (format!("falhou · {}", ago(r.ts)), WARN)
                        };
                        ui.label(txt(m, col));
                        if r.kind == 't' && !r.detail.is_empty() {
                            message(ui, &r.detail, sel);
                        }
                    });
                });
                if row.response.interact(Sense::click()).clicked() {
                    pick = Some(i);
                }
                if sel {
                    ui.painter().rect_stroke(
                        row.response.rect.expand(2.0 * S),
                        0.0,
                        Stroke::new(S, c(acc)),
                        egui::StrokeKind::Outside,
                    );
                    let seen = ui.make_persistent_id("histsel");
                    if ui.data_mut(|d| d.get_temp::<usize>(seen)) != Some(i) {
                        ui.data_mut(|d| d.insert_temp(seen, i));
                        row.response.scroll_to_me(None);
                    }
                }
                ui.add_space(5.0 * S);
            }
        });
    if pick.is_some() {
        app.hist_sel = pick;
    }
}

fn ago(ts: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    match now.saturating_sub(ts) {
        s if s < 60 => "agora".into(),
        s if s < 3600 => format!("há {} min", s / 60),
        s if s < 86400 => format!("há {} h", s / 3600),
        s => format!("há {} d", s / 86400),
    }
}

pub(super) fn clips(ui: &mut egui::Ui, app: &App) {
    let acc = app.acc();
    ui.label(txt("CLIPBOARDS RECEBIDOS", MUTED));
    ui.add_space(4.0 * S);
    if app.clips.is_empty() {
        ui.label(txt("Nada recebido nesta sessão.", FG));
        ui.label(txt("Ficam só na memória e somem ao fechar o pdrop.", MUTED));
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (i, (from, c)) in app.clips.iter().enumerate() {
                let sel = app.clip_sel == Some(i);
                let row = ui.vertical(|ui| {
                    ui.label(txt(format!("de {from}"), if sel { acc } else { MUTED }));
                    ui.label(RichText::new(crate::net::preview(c)));
                });
                if sel {
                    ui.painter().rect_stroke(
                        row.response.rect.expand(2.0 * S),
                        0.0,
                        Stroke::new(S, self::c(acc)),
                        egui::StrokeKind::Outside,
                    );
                    row.response.scroll_to_me(None);
                }
                ui.add_space(5.0 * S);
            }
        });
}
