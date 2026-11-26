use super::{App, VIEWS};
use crate::{
    clip,
    config::save_setting,
    desktop::reveal,
    history::Rec,
    net::{self, Peer},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const NO_CLIP: &str = "sem clipboard: instale wl-clipboard (Wayland) ou xclip (X11)";

impl App {
    pub(crate) fn mark(&mut self, peers: &[Peer], all: bool) {
        if all {
            if peers.iter().all(|p| self.marked.contains(&p.id)) {
                self.marked.clear();
            } else {
                self.marked = peers.iter().map(|p| p.id).collect();
            }
        } else if let Some(id) = self.sel
            && !self.marked.remove(&id)
        {
            self.marked.insert(id);
        }
    }

    pub(crate) fn toggle_trust(&mut self, peers: &[Peer]) {
        let Some(p) = peers.iter().find(|p| Some(p.id) == self.sel) else {
            return;
        };
        let mut g = self.policy.trust.lock().unwrap();
        let msg = if g.remove(&p.uid).is_some() {
            format!("{} não é mais confiável", p.name)
        } else {
            g.insert(p.uid, p.name.clone());
            format!(
                "{} agora é confiável: as mensagens dele chegam sem perguntar",
                p.name
            )
        };
        save_setting(
            "confiaveis",
            &g.iter()
                .map(|(u, n)| format!("{u:x}\t{n}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        drop(g);
        self.flash(msg);
    }

    pub(crate) fn cycle_vis(&mut self) {
        let v = (self.vis() + 1) % 3;
        self.policy.vis.store(v, Ordering::Relaxed);
        save_setting("visibilidade", &v.to_string());
        self.flash(
            [
                "visível para todos",
                "visível só para confiáveis",
                "invisível: ninguém te vê, você ainda envia",
            ][v as usize],
        );
    }

    pub(crate) fn cycle_view(&mut self) {
        self.view = (self.view + 1) % VIEWS.len() as u8;
        save_setting("visao", &self.view.to_string());
    }

    pub(crate) fn toggle_hist(&mut self) {
        self.hist_sel = if self.hist_sel.is_some() {
            None
        } else {
            Some(0)
        };
        self.clip_sel = None;
    }

    pub(crate) fn toggle_clips(&mut self) {
        self.clip_sel = if self.clip_sel.is_some() {
            None
        } else {
            Some(0)
        };
        self.hist_sel = None;
    }

    pub(crate) fn clip_step(&mut self, d: isize) {
        if let Some(i) = self.clip_sel {
            self.clip_sel = Some(
                (i as isize + d).clamp(0, self.clips.len().saturating_sub(1) as isize) as usize,
            );
        }
    }

    pub(crate) fn clip_paste(&mut self) {
        let Some((_, c)) = self.clip_sel.and_then(|i| self.clips.get(i)).cloned() else {
            return;
        };
        self.clip_sel = None;
        match clip::set(&c) {
            Ok(()) => self.flash(format!("no clipboard: {}", net::preview(&c))),
            Err(e) => self.flash(format!("não deu para colocar no clipboard: {e}")),
        }
    }

    pub(crate) fn hist_step(&mut self, d: isize) {
        if let Some(i) = self.hist_sel {
            self.hist_sel = Some(
                (i as isize + d).clamp(0, self.hist.len().saturating_sub(1) as isize) as usize,
            );
        }
    }

    pub(crate) fn hist_rec(&self) -> Option<&Rec> {
        self.hist.iter().rev().nth(self.hist_sel?)
    }

    pub(crate) fn hist_reveal(&mut self) {
        let Some(r) = self.hist_rec().cloned() else {
            return;
        };
        match r
            .detail
            .lines()
            .next()
            .map(PathBuf::from)
            .filter(|p| r.kind == 'f' && p.exists())
        {
            Some(p) => reveal(&p),
            None => self.flash("isso não é um arquivo, ou ele não está mais lá"),
        }
    }

    pub(crate) fn hist_resend(&mut self, peers: &[Peer]) {
        let Some(r) = self.hist_rec().cloned() else {
            return;
        };
        let Some(p) = peers.iter().find(|p| Some(p.id) == self.sel).cloned() else {
            return self.flash("escolha alguém no radar para reenviar");
        };
        if !matches!(r.kind, 'f' | 't') || r.detail.is_empty() {
            return self.flash("só arquivos e mensagens podem ser reenviados");
        }
        let inputs = if r.kind == 't' {
            vec![r.detail]
        } else {
            r.detail.lines().map(String::from).collect()
        };
        net::send(p.clone(), self.who(), inputs, self.tx.clone());
        self.flash(format!("reenviando para {}", p.name));
    }

    // Responde só o pedido `id`; se já foi resolvido (ex.: pela notificação), ignora.
    pub(crate) fn answer(&mut self, id: u64, yes: bool) {
        if let Some(i) = self.pending.iter().position(|(t, _)| t.id == id) {
            if let Some((_, r)) = self.pending.remove(i) {
                r.send(yes).ok();
            }
            if let Some(c) = self.prompts.remove(&id) {
                c.store(true, Ordering::Relaxed);
            }
        }
    }

    // seleciona o `step`-ésimo resultado da busca a partir do atual (0: mantém, se ainda casar)
    pub(crate) fn search_pick(&mut self, peers: &[Peer], step: isize) {
        let hits = self.search_hits(peers);
        if !hits.is_empty() {
            let i = hits.iter().position(|&id| Some(id) == self.sel);
            let i = i.map_or(0, |i| i as isize + step);
            self.sel = Some(hits[i.rem_euclid(hits.len() as isize) as usize]);
        }
    }

    pub(crate) fn search_hits(&self, peers: &[Peer]) -> Vec<u64> {
        let q = self.search.as_deref().unwrap_or("").trim().to_lowercase();
        peers
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&q))
            .map(|p| p.id)
            .collect()
    }

    pub(crate) fn send_input(&mut self, peers: &[Peer]) {
        let text = self.input.take().unwrap_or_default();
        if !text.trim().is_empty() {
            for p in self.targets(peers) {
                net::send(p, self.who(), vec![text.clone()], self.tx.clone());
            }
        }
    }

    pub(crate) fn send_files(&mut self, files: Vec<PathBuf>, peers: &[Peer]) {
        if files.is_empty() {
            return;
        }
        let to = self.targets(peers);
        if to.is_empty() {
            return self.flash("escolha alguém no radar antes de soltar o arquivo");
        }
        for p in to {
            net::send(
                p,
                self.who(),
                files
                    .iter()
                    .map(|f| f.to_string_lossy().into_owned())
                    .collect(),
                self.tx.clone(),
            );
        }
    }

    pub(crate) fn clip_once(&mut self, peers: &[Peer]) {
        let to = self.targets(peers);
        if to.is_empty() {
            return;
        }
        if !clip::available() {
            return self.flash(NO_CLIP);
        }
        match clip::get().filter(|c| !c.data.is_empty()) {
            None => self.flash(
                "clipboard vazio, grande demais (64 MB) ou num formato que não dá para enviar",
            ),
            Some(text) => {
                for p in to {
                    let (me, tx, text) = (self.who(), self.tx.clone(), text.clone());
                    std::thread::spawn(move || {
                        let _ = net::push_clip(&p, &me, &text, 'c', &tx);
                    });
                }
            }
        }
    }

    pub(crate) fn toggle_bridge(&mut self, peers: &[Peer]) {
        if self.bridge.is_some() {
            self.stop_bridge();
            return self.flash("ponte desligada");
        }
        let Some(p) = peers.iter().find(|p| Some(p.id) == self.sel).cloned() else {
            return;
        };
        if !clip::available() {
            return self.flash(NO_CLIP);
        }
        let stop = Arc::new(AtomicBool::new(false));
        net::bridge(p.clone(), self.who(), self.tx.clone(), stop.clone());
        self.flash(format!("ponte ligada: tudo que você copiar vai para {}. Senhas costumam ficar de fora, mas confira.", p.name));
        self.bridge = Some((p.id, p.name, stop));
    }

    pub(crate) fn stop_bridge(&mut self) {
        if let Some((_, _, stop)) = self.bridge.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn set_color(&mut self, i: u8) {
        (self.color, self.pick) = (i, None);
        self.sync();
        save_setting("cor", &i.to_string());
    }

    pub(crate) fn set_icon(&mut self, i: u8) {
        (self.icon, self.pick_icon) = (i, None);
        self.sync();
        save_setting("icone", &i.to_string());
    }

    pub(crate) fn set_name(&mut self) {
        let n = net::clean(self.rename.take().unwrap_or_default().trim());
        if !n.is_empty() {
            *self.shared_name.lock().unwrap() = n.clone();
            save_setting("nome", &n);
            self.me = n;
        }
    }
}
