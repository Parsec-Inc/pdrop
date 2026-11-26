use super::{
    theme::*,
    widgets::{button, panel, pixel_frame, well},
};
use crate::{
    App,
    relay::saved::{Env, NAME_MAX, Profile},
};
use eframe::egui::{self, Align, FontId, Key, Layout, Sense, vec2};

#[derive(Default)]
pub struct RelayPanel {
    form: Option<Form>,
    deleting: Option<usize>,
}

impl RelayPanel {
    pub fn back(&mut self) -> bool {
        self.form.take().is_some() | self.deleting.take().is_some()
    }
}

#[derive(Default)]
struct Form {
    at: Option<usize>,
    name: String,
    addr: String,
    secret: String,
    show: bool,
    error: Option<String>,
    focus: bool,
}

fn new_secret() -> String {
    let (key, _) = crate::secure::generate();
    blake3::hash(&key).to_hex()[..32].to_string()
}

enum Todo {
    Close,
    New,
    Edit(usize),
    Use(Option<usize>),
    AskDelete(usize),
    Delete(usize),
    Save,
    Back,
}

pub(super) fn draw(ui: &mut egui::Ui, app: &mut App) {
    let Some(mut panel_state) = app.relay_ui.take() else {
        return;
    };
    let mut todo = None;
    pixel_frame(10.0 * S).show(ui, |ui| {
        panel(ui, &app.sp);
        ui.set_min_size(ui.available_size());
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match &mut panel_state.form {
                Some(f) => form(ui, app, f, &mut todo),
                None => list(ui, app, panel_state.deleting, &mut todo),
            });
    });

    let done = match todo {
        None => Ok(()),
        Some(Todo::Close) => return,
        Some(Todo::New) => {
            panel_state.form = Some(Form {
                focus: true,
                ..Default::default()
            });
            Ok(())
        }
        Some(Todo::Edit(i)) => {
            panel_state.form = app.relay.book().list.get(i).map(|p| Form {
                at: Some(i),
                name: p.name.clone(),
                addr: p.addr.clone(),
                secret: p.secret.clone(),
                focus: true,
                ..Default::default()
            });
            Ok(())
        }
        Some(Todo::Back) => {
            panel_state.back();
            Ok(())
        }
        Some(Todo::AskDelete(i)) => {
            panel_state.deleting = Some(i);
            Ok(())
        }
        Some(Todo::Use(i)) => {
            panel_state.deleting = None;
            app.relay.activate(i)
        }
        Some(Todo::Delete(i)) => {
            panel_state.deleting = None;
            app.relay.remove(i)
        }
        Some(Todo::Save) => {
            if let Some(f) = &mut panel_state.form {
                let saved =
                    Profile::new(&f.name, &f.addr, &f.secret).and_then(|p| app.relay.save(f.at, p));
                match saved {
                    Ok(()) => panel_state.form = None,
                    Err(e) => f.error = Some(e),
                }
            }
            Ok(())
        }
    };
    if let Err(e) = done {
        app.flash(e);
    }
    app.relay_ui = Some(panel_state);
}

fn list(ui: &mut egui::Ui, app: &App, deleting: Option<usize>, todo: &mut Option<Todo>) {
    let (acc, sp) = (app.acc(), &app.sp);
    ui.horizontal(|ui| {
        ui.label(txt("Relays", acc));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if button(ui, sp, None, "Fechar", "Esc", true, None).clicked() {
                *todo = Some(Todo::Close);
            }
            if button(ui, sp, None, "Adicionar", "", true, Some(acc)).clicked() {
                *todo = Some(Todo::New);
            }
        });
    });
    ui.label(txt(
        "Alcançam aparelhos em outras redes. A rede local continua tendo prioridade.",
        MUTED,
    ));
    if app.relay.external() {
        ui.label(txt(
            "A conexão atual veio de uma variável de ambiente.",
            WARN,
        ));
    }
    if Env::read().any() {
        ui.label(txt(
            "PDROP_RELAY ou PDROP_ROOM está definida: na próxima abertura ela vale no lugar do relay em uso.",
            WARN,
        ));
    }
    if app.relay.external() && button(ui, sp, None, "Desconectar", "", true, None).clicked() {
        *todo = Some(Todo::Use(None));
    }
    ui.add_space(6.0 * S);

    let book = app.relay.book();
    if book.list.is_empty() {
        ui.label(txt("Nenhum relay salvo.", FG));
    }
    let in_use = app.relay.in_use();
    for (i, p) in book.list.iter().enumerate() {
        let on = in_use == Some(i);
        ui.horizontal_wrapped(|ui| {
            ui.label(txt(&p.name, if on { acc } else { FG }));
            ui.label(txt(&p.addr, MUTED));
            if on {
                ui.label(txt("· em uso", acc));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(3.0 * S, 3.0 * S);
            let (label, next) = if on {
                ("Desativar", None)
            } else {
                ("Usar", Some(i))
            };
            if button(ui, sp, None, label, "", true, None).clicked() {
                *todo = Some(Todo::Use(next));
            }
            if button(ui, sp, None, "Editar", "", true, None).clicked() {
                *todo = Some(Todo::Edit(i));
            }
            let sure = deleting == Some(i);
            let label = if sure {
                "Confirmar exclusão"
            } else {
                "Excluir"
            };
            if button(ui, sp, None, label, "", true, sure.then_some(WARN)).clicked() {
                *todo = Some(if sure {
                    Todo::Delete(i)
                } else {
                    Todo::AskDelete(i)
                });
            }
        });
        ui.add_space(6.0 * S);
    }
}

fn form(ui: &mut egui::Ui, app: &App, f: &mut Form, todo: &mut Option<Todo>) {
    let (acc, sp) = (app.acc(), &app.sp);
    ui.label(txt(
        if f.at.is_some() {
            "Editar relay"
        } else {
            "Novo relay"
        },
        acc,
    ));
    ui.add_space(4.0 * S);

    let focus = std::mem::take(&mut f.focus);
    let mut enter = false;
    let mut field = |ui: &mut egui::Ui, label: &str, edit: egui::TextEdit| {
        let r = ui
            .horizontal(|ui| {
                let (slot, _) = ui.allocate_exact_size(vec2(55.0 * S, 16.0 * S), Sense::hover());
                ui.painter().text(
                    slot.left_center(),
                    egui::Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(TXT),
                    c(MUTED),
                );
                well(ui, sp, edit.desired_width(f32::INFINITY))
            })
            .inner;
        enter |= r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        r
    };
    let name = field(
        ui,
        "Nome",
        egui::TextEdit::singleline(&mut f.name)
            .char_limit(NAME_MAX)
            .hint_text(txt("opcional", DIM)),
    );
    if focus {
        name.request_focus();
    }
    field(
        ui,
        "Servidor",
        egui::TextEdit::singleline(&mut f.addr).hint_text(txt("host:porta", DIM)),
    );
    field(
        ui,
        "Senha",
        egui::TextEdit::singleline(&mut f.secret)
            .password(!f.show)
            .hint_text(txt("mínimo de 16 caracteres", DIM)),
    );
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0 * S, 3.0 * S);
        let label = if f.show { "Ocultar" } else { "Mostrar" };
        if button(ui, sp, None, label, "", true, None).clicked() {
            f.show = !f.show;
        }
        let empty = f.secret.is_empty();
        if button(ui, sp, None, "Gerar", "", empty, None).clicked() && empty {
            f.secret = new_secret();
        }
        if button(ui, sp, None, "Copiar", "", !empty, None).clicked() && !empty {
            ui.ctx().copy_text(f.secret.trim().to_string());
        }
    });
    ui.label(txt(
        "Use o mesmo servidor e a mesma senha em todos os aparelhos. Prefira uma senha gerada.",
        MUTED,
    ));
    if let Some(e) = &f.error {
        ui.label(txt(e, WARN));
    }
    ui.add_space(6.0 * S);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0 * S, 3.0 * S);
        let label = if f.at.is_some() {
            "Salvar"
        } else {
            "Salvar e usar"
        };
        if button(ui, sp, None, label, "Enter", true, Some(acc)).clicked() {
            *todo = Some(Todo::Save);
        }
        if button(ui, sp, None, "Cancelar", "Esc", true, None).clicked() {
            *todo = Some(Todo::Back);
        }
    });
    if enter {
        *todo = Some(Todo::Save);
    }
}
