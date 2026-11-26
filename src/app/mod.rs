mod actions;

use crate::{
    clip,
    desktop::notify,
    firewall,
    fmt::human,
    history::{HIST_MAX, Rec, append_hist},
    net::{Grants, Msg, Peer, Peers, Policy, State, Transfer, Who},
    relay, secure, ui,
};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{Receiver, Sender},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub struct App {
    pub me: String,
    pub peers: Peers,
    pub sel: Option<u64>,
    pub input: Option<String>,
    pub xfers: Vec<Transfer>,
    pub pending: VecDeque<(Transfer, Sender<bool>)>,
    pub(crate) prompts: std::collections::HashMap<u64, Arc<AtomicBool>>,
    pub anim: f64,
    pub last: Instant,
    pub calm: bool,
    pub color: u8,
    pub pick: Option<u8>,
    pub icon: u8,
    pub pick_icon: Option<u8>,
    pub rename: Option<String>,
    pub shared_name: Arc<Mutex<String>>,
    pub bridge: Option<(u64, String, Arc<AtomicBool>)>,
    pub grants: Grants,
    pub flash: Option<(String, Instant)>,
    pub shared_color: Arc<AtomicU8>,
    pub rx: Receiver<Msg>,
    pub tx: Sender<Msg>,
    pub sp: ui::Sprites,
    pub focus: bool,
    pub firewall: firewall::Shared,
    pub relay: relay::saved::Control,
    pub relay_ui: Option<ui::RelayPanel>,
    pub uid: u128,
    pub policy: Arc<Policy>,
    pub hist: Vec<Rec>,
    pub hist_sel: Option<usize>,
    pub clips: VecDeque<(String, clip::Clip)>,
    pub clip_sel: Option<usize>,
    pub marked: std::collections::HashSet<u64>,
    pub view: u8,
    pub cols: usize,
    pub more: bool,
}

impl App {
    pub fn who(&self) -> Who {
        Who {
            name: self.me.clone(),
            key: self.policy.key,
        }
    }

    pub fn targets(&self, peers: &[Peer]) -> Vec<Peer> {
        let t: Vec<Peer> = peers
            .iter()
            .filter(|p| self.marked.contains(&p.id))
            .cloned()
            .collect();
        if t.is_empty() {
            peers
                .iter()
                .filter(|p| Some(p.id) == self.sel)
                .cloned()
                .collect()
        } else {
            t
        }
    }

    pub fn trusted(&self, uid: u128) -> bool {
        self.policy.trust.lock().unwrap().contains_key(&uid)
    }

    pub fn vis(&self) -> u8 {
        self.policy.vis.load(Ordering::Relaxed)
    }

    pub fn acc(&self) -> ui::Rgb {
        ui::COLORS[self.pick.unwrap_or(self.color) as usize].1
    }

    fn sync(&self) {
        self.shared_color
            .store(self.color | self.icon << 3, Ordering::Relaxed);
    }

    pub fn flash(&mut self, s: impl Into<String>) {
        self.flash = Some((s.into(), Instant::now()));
    }

    fn live(&self) -> Vec<Peer> {
        let mut g = self.peers.lock().unwrap();
        g.retain(|_, p| p.seen.elapsed() < Duration::from_secs(6));
        let mut v: Vec<Peer> = g.values().cloned().collect();
        v.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
        v
    }

    fn on(&mut self, m: Msg) {
        match m {
            Msg::BridgeOff => {
                self.stop_bridge();
                self.flash("ponte encerrada");
            }
            Msg::Ask(t, r) => {
                let what = match t.kind {
                    'c' => format!("quer colar no seu clipboard: {}", t.note),
                    'b' => "quer ligar uma ponte de clipboard: tudo que copiar vai para o seu, até você desligar".into(),
                    _ => format!("quer enviar {} ({})", t.label, human(t.size)),
                };
                let c = notify(
                    format!("{} [{}] {what}", t.peer, secure::code(self.uid, t.uid)),
                    Some((t.id, self.tx.clone())),
                );
                self.prompts.insert(t.id, c);
                self.pending.push_back((t, r));
            }
            Msg::Reply(id, yes) => {
                self.answer(id, yes);
            }
            Msg::Clip(from, c) => {
                if self.clips.front().is_none_or(|(_, f)| f.data != c.data) {
                    self.clips.push_front((from, c));
                }
                self.clips.truncate(20);
                while self.clips.len() > 1
                    && self
                        .clips
                        .iter()
                        .map(|(_, c)| c.data.len() as u64)
                        .sum::<u64>()
                        > clip::MAX
                {
                    self.clips.pop_back();
                }
            }
            Msg::Auto(t) => {
                notify(format!("{} enviou uma mensagem", t.peer), None);
                self.flash(format!("mensagem de {} (confiável)", t.peer));
            }
            Msg::Start(t) => {
                self.xfers.push(t);
                if self.xfers.len() > 40 {
                    self.xfers.remove(0);
                }
            }
            Msg::Progress(id, d) => {
                if let Some(t) = self.xfers.iter_mut().find(|t| t.id == id) {
                    t.t0.get_or_insert((Instant::now(), d));
                    (t.done, t.state) = (d, State::Active);
                }
            }
            Msg::End(id, r) => {
                if let Some(t) = self.xfers.iter_mut().find(|t| t.id == id) {
                    let rec = Rec {
                        ts: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs()),
                        up: t.up,
                        kind: t.kind,
                        peer: t.peer.clone(),
                        label: t.label.clone(),
                        size: t.size,
                        ok: r.is_ok(),
                        detail: if t.up {
                            t.src.clone()
                        } else {
                            r.clone().unwrap_or_default()
                        },
                    };
                    t.state = match r {
                        Ok(m) if !t.up && t.kind == 'f' && m.lines().count() > 1 => {
                            let dir = std::path::Path::new(m.lines().next().unwrap_or(""))
                                .parent()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default();
                            (
                                t.done = t.size,
                                State::Done(format!("{} itens em {dir}", m.lines().count())),
                            )
                                .1
                        }
                        Ok(m) => (t.done = t.size, State::Done(m)).1,
                        Err(e) => State::Failed(e),
                    };
                    if t.kind != 'b' {
                        append_hist(&rec);
                        self.hist.push(rec);
                        self.hist.drain(..self.hist.len().saturating_sub(HIST_MAX));
                    }
                }
            }
        }
    }

    fn tick(&mut self) -> Vec<Peer> {
        while let Ok(m) = self.rx.try_recv() {
            self.on(m);
        }
        let now = Instant::now();
        let busy = self.input.is_some() || self.rename.is_some() || !self.pending.is_empty();
        let speed = if busy {
            0.3
        } else if self.calm {
            0.5
        } else {
            1.0
        };
        self.anim += now.duration_since(self.last).as_secs_f64().min(0.1) * speed;
        self.last = now;
        let peers = self.live();
        self.marked.retain(|id| peers.iter().any(|p| p.id == *id));
        if self
            .bridge
            .as_ref()
            .is_some_and(|(id, ..)| !peers.iter().any(|p| p.id == *id))
        {
            self.stop_bridge();
            self.flash("ponte encerrada: o aparelho saiu do radar");
        }
        if !peers.iter().any(|p| Some(p.id) == self.sel) {
            self.sel = peers.first().map(|p| p.id);
        }
        peers
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        let peers = self.tick();
        ui::draw(ui, self, &peers);
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

pub const NAME_MAX: usize = 32;

pub const VIEWS: [&str; 3] = ["sonar", "seleção", "constelação"];
