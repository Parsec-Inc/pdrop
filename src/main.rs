#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod clip;
mod firewall;
mod net;
mod secure;
mod ui;

use net::{Grants, Msg, Peer, Peers, Policy, State, Transfer, Who};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, Sender},
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
    pub uid: u128,
    pub policy: Arc<Policy>,
    pub hist: Vec<Rec>,
    pub hist_sel: Option<usize>,
    pub clips: VecDeque<(String, clip::Clip)>,
    pub clip_sel: Option<usize>,
    pub marked: std::collections::HashSet<u64>,
    pub view: u8,
    pub cols: usize,
}

#[derive(Clone)]
pub struct Rec {
    pub ts: u64,
    pub up: bool,
    pub kind: char,
    pub peer: String,
    pub label: String,
    pub size: u64,
    pub ok: bool,
    pub detail: String,
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

    fn mark(&mut self, peers: &[Peer], all: bool) {
        if all {
            if peers.iter().all(|p| self.marked.contains(&p.id)) {
                self.marked.clear();
            } else {
                self.marked = peers.iter().map(|p| p.id).collect();
            }
        } else if let Some(id) = self.sel {
            if !self.marked.remove(&id) {
                self.marked.insert(id);
            }
        }
    }

    pub fn trusted(&self, uid: u128) -> bool {
        self.policy.trust.lock().unwrap().contains_key(&uid)
    }

    pub fn vis(&self) -> u8 {
        self.policy.vis.load(Ordering::Relaxed)
    }

    fn toggle_trust(&mut self, peers: &[Peer]) {
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

    fn cycle_vis(&mut self) {
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

    fn cycle_view(&mut self) {
        self.view = (self.view + 1) % VIEWS.len() as u8;
        save_setting("visao", &self.view.to_string());
    }

    fn toggle_hist(&mut self) {
        self.hist_sel = if self.hist_sel.is_some() {
            None
        } else {
            Some(0)
        };
        self.clip_sel = None;
    }

    fn toggle_clips(&mut self) {
        self.clip_sel = if self.clip_sel.is_some() {
            None
        } else {
            Some(0)
        };
        self.hist_sel = None;
    }

    fn clip_step(&mut self, d: isize) {
        if let Some(i) = self.clip_sel {
            self.clip_sel = Some(
                (i as isize + d).clamp(0, self.clips.len().saturating_sub(1) as isize) as usize,
            );
        }
    }

    fn clip_paste(&mut self) {
        let Some((_, c)) = self.clip_sel.and_then(|i| self.clips.get(i)).cloned() else {
            return;
        };
        self.clip_sel = None;
        match clip::set(&c) {
            Ok(()) => self.flash(format!("no clipboard: {}", net::preview(&c))),
            Err(e) => self.flash(format!("não deu para colocar no clipboard: {e}")),
        }
    }

    fn hist_step(&mut self, d: isize) {
        if let Some(i) = self.hist_sel {
            self.hist_sel = Some(
                (i as isize + d).clamp(0, self.hist.len().saturating_sub(1) as isize) as usize,
            );
        }
    }

    fn hist_rec(&self) -> Option<&Rec> {
        self.hist.iter().rev().nth(self.hist_sel?)
    }

    fn hist_reveal(&mut self) {
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

    fn hist_resend(&mut self, peers: &[Peer]) {
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

    fn stop_bridge(&mut self) {
        if let Some((_, _, stop)) = self.bridge.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }

    fn clip_once(&mut self, peers: &[Peer]) {
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

    fn toggle_bridge(&mut self, peers: &[Peer]) {
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

    fn answer(&mut self, yes: bool) {
        if let Some((_, r)) = self.pending.pop_front() {
            r.send(yes).ok();
        }
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
                notify(format!(
                    "{} quer enviar {} ({})",
                    t.peer,
                    t.label,
                    ui::human(t.size)
                ));
                self.pending.push_back((t, r));
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
                notify(format!("{} enviou uma mensagem", t.peer));
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

    fn send_input(&mut self, peers: &[Peer]) {
        let text = self.input.take().unwrap_or_default();
        if !text.trim().is_empty() {
            for p in self.targets(peers) {
                net::send(p, self.who(), vec![text.clone()], self.tx.clone());
            }
        }
    }

    fn send_files(&mut self, files: Vec<PathBuf>, peers: &[Peer]) {
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

    fn set_color(&mut self, i: u8) {
        (self.color, self.pick) = (i, None);
        self.sync();
        save_setting("cor", &i.to_string());
    }

    fn set_icon(&mut self, i: u8) {
        (self.icon, self.pick_icon) = (i, None);
        self.sync();
        save_setting("icone", &i.to_string());
    }

    fn set_name(&mut self) {
        let n = net::clean(self.rename.take().unwrap_or_default().trim());
        if !n.is_empty() {
            *self.shared_name.lock().unwrap() = n.clone();
            save_setting("nome", &n);
            self.me = n;
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
            self.flash("ponte encerrada: o aparelho saiu da rede");
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

#[cfg(windows)]
const WIN_TOAST: &str = "[void][Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime];\
$x=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent('ToastText02');\
$t=$x.GetElementsByTagName('text');$t[0].AppendChild($x.CreateTextNode('pdrop'))|Out-Null;$t[1].AppendChild($x.CreateTextNode($env:PDROP_BODY))|Out-Null;\
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))";

fn notify(body: String) {
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                "on run {t, b}",
                "-e",
                "display notification b with title t sound name \"Ping\"",
                "-e",
                "end run",
                "pdrop",
                &body,
            ])
            .status();
        #[cfg(windows)]
        let _ = clip::quiet(
            std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", WIN_TOAST])
                .env("PDROP_BODY", &body),
        )
        .status();
        #[cfg(not(any(target_os = "macos", windows)))]
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "pdrop", "-u", "critical", "pdrop"])
            .arg(format!(
                "{}",
                body.replace('&', "&amp;").replace('<', "&lt;")
            ))
            .status();
    });
}

const NAME_MAX: usize = 32;
const NO_CLIP: &str = "sem clipboard: instale wl-clipboard (Wayland) ou xclip (X11)";

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into())
}

pub const VIEWS: [&str; 3] = ["sonar", "seleção", "constelação"];

fn setting_path(k: &str) -> PathBuf {
    home().join(".config/pdrop").join(k)
}

fn load_setting(k: &str) -> Option<String> {
    std::fs::read_to_string(setting_path(k))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn saved_peers() -> Vec<std::net::IpAddr> {
    load_setting("peers")
        .map(|s| {
            s.lines()
                .filter_map(|l| l.split('#').next()?.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn save_setting(k: &str, v: &str) {
    let p = setting_path(k);
    let _ = std::fs::create_dir_all(p.parent().unwrap()).and_then(|_| std::fs::write(p, v));
}

const HIST_MAX: usize = 200;

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn unesc(s: &str) -> String {
    let (mut out, mut it) = (String::new(), s.chars());
    while let Some(c) = it.next() {
        match (c, c == '\\') {
            (_, true) => out.push(match it.next() {
                Some('n') => '\n',
                Some('t') => '\t',
                Some(o) => o,
                None => '\\',
            }),
            _ => out.push(c),
        }
    }
    out
}

fn load_hist() -> Vec<Rec> {
    let raw = std::fs::read_to_string(setting_path("historico")).unwrap_or_default();
    let mut v: Vec<Rec> = raw
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let &[ts, up, kind, peer, label, size, ok, detail] = f.as_slice() else {
                return None;
            };
            Some(Rec {
                ts: ts.parse().ok()?,
                up: up == "1",
                kind: kind.chars().next()?,
                peer: peer.into(),
                label: label.into(),
                size: size.parse().ok()?,
                ok: ok == "1",
                detail: unesc(detail),
            })
        })
        .collect();
    v.drain(..v.len().saturating_sub(HIST_MAX));
    v
}

fn append_hist(r: &Rec) {
    use std::io::Write;
    let p = setting_path("historico");
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let _ = writeln!(
            f,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.ts,
            r.up as u8,
            r.kind,
            r.peer,
            r.label,
            r.size,
            r.ok as u8,
            esc(&r.detail)
        );
    }
}

fn load_routes() -> Vec<(String, PathBuf)> {
    let raw = load_setting("destinos").unwrap_or_default();
    let real_home = std::env::var_os("SNAP_REAL_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(home);
    raw.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(r, p)| {
            let p = p.trim();
            (
                r.trim().to_string(),
                p.strip_prefix("~/")
                    .map_or_else(|| PathBuf::from(p), |rest| real_home.join(rest)),
            )
        })
        .collect()
}

fn load_trust() -> std::collections::HashMap<u128, String> {
    let raw = load_setting("confiaveis").unwrap_or_default();
    raw.lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(u, n)| Some((u128::from_str_radix(u, 16).ok()?, n.to_string())))
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    let b: Vec<u8> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect::<Option<_>>()?;
    b.try_into().ok()
}

fn load_identity() -> ([u8; 32], u128) {
    let saved = load_setting("chave").and_then(|s| {
        let (k, p) = s.split_once('\n')?;
        Some((unhex(k.trim())?, unhex(p.trim())?))
    });
    let (k, p) = saved.unwrap_or_else(|| {
        let (k, p) = secure::generate();
        save_setting("chave", &format!("{}\n{}", hex(&k), hex(&p)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                setting_path("chave"),
                std::fs::Permissions::from_mode(0o600),
            );
        }
        (k, p)
    });
    (k, secure::id_of(&p))
}

fn reveal(p: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", p.display()))
        .spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = std::process::Command::new("xdg-open")
        .arg(if p.is_dir() {
            p
        } else {
            p.parent().unwrap_or(p)
        })
        .spawn();
}

fn load_color() -> u8 {
    let c = load_setting("cor").and_then(|s| s.parse::<u8>().ok());
    c.filter(|&c| (c as usize) < ui::COLORS.len()).unwrap_or(0)
}

fn load_icon() -> u8 {
    let i = load_setting("icone").and_then(|s| s.parse::<u8>().ok());
    i.filter(|&i| (i as usize) < ui::ICONS.len()).unwrap_or(0)
}

fn hostname() -> String {
    let h = std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        })
        .unwrap_or_default();
    let h = net::clean(h.trim().split('.').next().unwrap_or(""));
    if h.is_empty() { "pdrop".into() } else { h }
}

fn window_icon() -> eframe::egui::IconData {
    let im = image::load_from_memory(include_bytes!("../assets/pdrop.png"))
        .expect("ícone embutido inválido")
        .to_rgba8();
    eframe::egui::IconData {
        width: im.width(),
        height: im.height(),
        rgba: im.into_raw(),
    }
}

const USAGE: &str = "uso: pdrop [--name NOME] [--peer IP]... [--dir PASTA] [--calm]
     pdrop send [--to NOME] [--peer IP]... [--as NOME] CAMINHO|TEXTO|- ...
recebidos vão para ~/Downloads (ou --dir); regras de destino em ~/.config/pdrop/destinos;
IPs fixos (VPN/Tailscale) em ~/.config/pdrop/peers, um por linha";

fn proc_id() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    nanos ^ ((std::process::id() as u64) << 32)
}

fn cli_send(args: Vec<String>) -> i32 {
    let (mut to, mut name_as, mut direct, mut inputs) = (None, None, saved_peers(), vec![]);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--to" | "-t" => to = it.next(),
            "--as" => name_as = it.next(),
            "--peer" | "-p" => match it.next().and_then(|ip| ip.parse::<std::net::IpAddr>().ok()) {
                Some(ip) => direct.push(ip),
                None => return usage(),
            },
            "-" => inputs.push(a),
            f if f.starts_with("--") => return usage(),
            _ => inputs.push(a),
        }
    }
    if inputs.is_empty() {
        return usage();
    }
    let tmp = std::env::temp_dir().join(format!("pdrop-{}", std::process::id()));
    for i in inputs.iter_mut().filter(|i| *i == "-") {
        let file =
            tmp.join(net::clean(name_as.as_deref().unwrap_or("stdin.txt")).replace('/', "_"));
        let ok = std::fs::create_dir_all(&tmp).and_then(|_| {
            std::io::copy(
                &mut std::io::stdin().lock(),
                &mut std::fs::File::create(&file)?,
            )
        });
        if let Err(e) = ok {
            eprintln!("não consegui ler o stdin: {e}");
            return 1;
        }
        *i = file.display().to_string();
    }
    let code = cli_run(to, direct, inputs);
    let _ = std::fs::remove_dir_all(tmp);
    code
}

fn usage() -> i32 {
    eprintln!("{USAGE}");
    2
}

fn cli_run(to: Option<String>, direct: Vec<std::net::IpAddr>, inputs: Vec<String>) -> i32 {
    let (key, uid) = load_identity();
    let me = load_setting("nome")
        .map(|n| net::clean(&n))
        .unwrap_or_else(hostname);
    let policy = Arc::new(Policy {
        key,
        vis: AtomicU8::new(net::VIS_HIDDEN),
        ..Default::default()
    });
    let peers = Peers::default();
    if let Err(e) = net::discover(
        proc_id(),
        uid,
        Arc::new(Mutex::new(me.clone())),
        0,
        Arc::new(AtomicU8::new(0)),
        policy,
        peers.clone(),
        direct,
    ) {
        eprintln!("sem rede: {e}");
        return 1;
    }
    let pick = |v: &[Peer]| -> Option<Peer> {
        let t = to.as_ref()?.to_lowercase();
        v.iter()
            .find(|p| p.name.to_lowercase() == t)
            .or_else(|| v.iter().find(|p| p.name.to_lowercase().starts_with(&t)))
            .cloned()
    };
    let deadline = Instant::now() + Duration::from_secs(6);
    let target = loop {
        std::thread::sleep(Duration::from_millis(200));
        let v: Vec<Peer> = peers.lock().unwrap().values().cloned().collect();
        match (&to, v.as_slice()) {
            (Some(_), _) if pick(&v).is_some() => break pick(&v),
            (None, _) if Instant::now() >= deadline => match v.as_slice() {
                [p] => break Some(p.clone()),
                [] => break None,
                _ => {
                    eprintln!(
                        "mais de um aparelho por perto, use --to: {}",
                        v.iter()
                            .map(|p| p.name.clone())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    return 2;
                }
            },
            _ if Instant::now() >= deadline => break None,
            _ => {}
        }
    };
    let Some(peer) = target else {
        eprintln!(
            "{}",
            to.map_or("ninguém por perto".to_string(), |t| format!(
                "não achei \"{t}\" na rede"
            ))
        );
        return 2;
    };
    let (tx, rx) = mpsc::channel();
    net::send(peer.clone(), Who { name: me, key }, inputs, tx);
    while let Ok(m) = rx.recv() {
        match m {
            Msg::Start(t) if matches!(t.state, State::Failed(_)) => {
                eprintln!(
                    "{}: {}",
                    t.label,
                    if let State::Failed(e) = t.state {
                        e
                    } else {
                        String::new()
                    }
                );
                return 1;
            }
            Msg::Start(t) => eprintln!(
                "enviando {} ({}) para {}, aguardando aceitar…",
                t.label,
                ui::human(t.size),
                peer.name
            ),
            Msg::Progress(_, _) => {}
            Msg::End(_, Ok(_)) => {
                eprintln!("enviado");
                return 0;
            }
            Msg::End(_, Err(e)) => {
                eprintln!("falhou: {e}");
                return 1;
            }
            _ => {}
        }
    }
    1
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("send") {
        std::process::exit(cli_send(std::env::args().skip(2).collect()));
    }
    let mut args = std::env::args().skip(1);
    let mut me = load_setting("nome")
        .map(|n| net::clean(&n))
        .unwrap_or_else(hostname);
    let mut direct = saved_peers();
    let mut calm = false;
    let mut out_dir = None;
    while let Some(a) = args.next() {
        if a == "--calm" {
            calm = true;
            continue;
        }
        match (a.as_str(), args.next()) {
            ("--name" | "-n", Some(n)) => me = net::clean(&n),
            ("--peer" | "-p", Some(ip)) if ip.parse::<std::net::IpAddr>().is_ok() => {
                direct.push(ip.parse().unwrap())
            }
            ("--dir" | "-d", Some(d)) => out_dir = Some(PathBuf::from(d)),
            _ => {
                eprintln!("{USAGE}");
                return Ok(());
            }
        }
    }
    let id = proc_id();
    let dir = out_dir.unwrap_or_else(|| {
        std::env::var_os("SNAP_REAL_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(home)
            .join("Downloads")
    });
    let (tx, rx) = mpsc::channel();
    let peers = Peers::default();
    let grants = Grants::default();
    let (key, uid) = load_identity();
    let policy = Arc::new(Policy {
        key,
        routes: load_routes(),
        trust: Mutex::new(load_trust()),
        vis: AtomicU8::new(
            load_setting("visibilidade")
                .and_then(|s| s.parse().ok())
                .filter(|v| *v <= net::VIS_HIDDEN)
                .unwrap_or(net::VIS_ALL),
        ),
    });
    let port = net::listen(dir, tx.clone(), grants.clone(), policy.clone())?;
    let color = load_color();
    let icon = load_icon();
    let shared_color = Arc::new(AtomicU8::new(color | icon << 3));
    let shared_name = Arc::new(Mutex::new(me.clone()));
    net::discover(
        id,
        uid,
        shared_name.clone(),
        port,
        shared_color.clone(),
        policy.clone(),
        peers.clone(),
        direct,
    )?;

    let firewall = firewall::watch();

    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_app_id("pdrop")
            .with_icon(window_icon())
            .with_title("pdrop")
            .with_inner_size([1000.0, 640.0])
            .with_min_inner_size([640.0, 420.0]),
        #[cfg(target_os = "linux")]
        event_loop_builder: std::env::var_os("DISPLAY").map(|_| -> eframe::EventLoopBuilderHook {
            Box::new(|b| {
                winit::platform::x11::EventLoopBuilderExtX11::with_x11(b);
            })
        }),
        ..Default::default()
    };
    eframe::run_native(
        "pdrop",
        opts,
        Box::new(move |cc| {
            ui::setup(&cc.egui_ctx);
            let sp = ui::Sprites::new(&cc.egui_ctx);
            Ok(Box::new(App {
                me,
                peers,
                sel: None,
                input: None,
                xfers: vec![],
                pending: VecDeque::new(),
                anim: 0.0,
                last: Instant::now(),
                calm,
                color,
                pick: None,
                icon,
                pick_icon: None,
                rename: None,
                shared_name,
                shared_color,
                bridge: None,
                grants,
                flash: None,
                rx,
                tx,
                sp,
                focus: false,
                uid,
                policy,
                hist: load_hist(),
                hist_sel: None,
                clips: VecDeque::new(),
                clip_sel: None,
                marked: Default::default(),
                firewall,
                view: load_setting("visao")
                    .and_then(|s| s.parse().ok())
                    .filter(|v| (*v as usize) < VIEWS.len())
                    .unwrap_or(0),
                cols: 1,
            }))
        }),
    )?;
    Ok(())
}
