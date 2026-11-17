#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod clip;
mod net;
mod ui;

use net::{Grants, Msg, Peer, Peers, State, Transfer};
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
    /// relógio da animação (segundos); anda mais devagar quando há algo importante na tela
    pub anim: f64,
    pub last: Instant,
    pub calm: bool,
    /// cor confirmada (índice em ui::COLORS), e a que está em pré-visualização enquanto se escolhe
    pub color: u8,
    pub pick: Option<u8>,
    /// ícone confirmado (índice em ui::ICONS; 0 = só o ponto) e o em pré-visualização
    pub icon: u8,
    pub pick_icon: Option<u8>,
    /// edição do próprio nome (texto em andamento)
    pub rename: Option<String>,
    pub shared_name: Arc<Mutex<String>>,
    /// ponte de clipboard que eu liguei: (id do peer, nome, flag de parada)
    pub bridge: Option<(u64, String, Arc<AtomicBool>)>,
    /// pontes que outros ligaram para mim e eu permiti
    pub grants: Grants,
    pub flash: Option<(String, Instant)>,
    pub shared_color: Arc<AtomicU8>,
    pub rx: Receiver<Msg>,
    pub tx: Sender<Msg>,
    pub sp: ui::Sprites,
    /// pede foco para o campo de texto que acabou de abrir
    pub focus: bool,
}

impl App {
    /// cor de acento: a que está em pré-visualização ou, na falta, a confirmada
    pub fn acc(&self) -> ui::Rgb {
        ui::COLORS[self.pick.unwrap_or(self.color) as usize].1
    }

    /// cor e ícone viajam juntos no campo de cor do beacon: cor + 8 * ícone.
    /// Builds antigas fazem `% 8` na cor, então ainda mostram a cor certa.
    fn sync(&self) {
        self.shared_color.store(self.color | self.icon << 3, Ordering::Relaxed);
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
        let Some(p) = peers.iter().find(|p| Some(p.id) == self.sel).cloned() else { return };
        if !clip::available() {
            return self.flash(NO_CLIP);
        }
        match clip::get().filter(|c| !c.data.is_empty()) {
            None => self.flash("clipboard vazio, grande demais (64 MB) ou num formato que não dá para enviar"),
            Some(text) => {
                let (me, tx) = (self.me.clone(), self.tx.clone());
                std::thread::spawn(move || {
                    let _ = net::push_clip(&p, &me, &text, 'c', &tx);
                });
            }
        }
    }

    fn toggle_bridge(&mut self, peers: &[Peer]) {
        if self.bridge.is_some() {
            self.stop_bridge();
            return self.flash("ponte desligada");
        }
        let Some(p) = peers.iter().find(|p| Some(p.id) == self.sel).cloned() else { return };
        if !clip::available() {
            return self.flash(NO_CLIP);
        }
        let stop = Arc::new(AtomicBool::new(false));
        net::bridge(p.clone(), self.me.clone(), self.tx.clone(), stop.clone());
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
                notify(&t);
                self.pending.push_back((t, r));
            }
            Msg::Start(t) => {
                self.xfers.push(t);
                if self.xfers.len() > 40 {
                    self.xfers.remove(0);
                }
            }
            Msg::Progress(id, d) => {
                if let Some(t) = self.xfers.iter_mut().find(|t| t.id == id) {
                    (t.done, t.state) = (d, State::Active);
                }
            }
            Msg::End(id, r) => {
                if let Some(t) = self.xfers.iter_mut().find(|t| t.id == id) {
                    t.state = match r {
                        Ok(m) => (t.done = t.size, State::Done(m)).1,
                        Err(e) => State::Failed(e),
                    };
                }
            }
        }
    }

    fn send_input(&mut self, peers: &[Peer]) {
        let text = self.input.take().unwrap_or_default();
        let peer = peers.iter().find(|p| Some(p.id) == self.sel);
        if let (Some(p), false) = (peer, text.trim().is_empty()) {
            net::send(p.clone(), self.me.clone(), text, self.tx.clone());
        }
    }

    /// arquivos soltos na janela vão direto para o peer selecionado
    fn send_files(&mut self, files: Vec<PathBuf>, peers: &[Peer]) {
        if files.is_empty() {
            return;
        }
        let Some(p) = peers.iter().find(|p| Some(p.id) == self.sel) else {
            return self.flash("escolha alguém no radar antes de soltar o arquivo");
        };
        for f in files {
            net::send(p.clone(), self.me.clone(), f.to_string_lossy().into_owned(), self.tx.clone());
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

    /// um passo do relógio: mensagens da rede, animação, peers vivos, seleção válida
    fn tick(&mut self) -> Vec<Peer> {
        while let Ok(m) = self.rx.try_recv() {
            self.on(m);
        }
        let now = Instant::now();
        let busy = self.input.is_some() || self.rename.is_some() || !self.pending.is_empty();
        let speed = if busy { 0.3 } else if self.calm { 0.5 } else { 1.0 };
        self.anim += now.duration_since(self.last).as_secs_f64().min(0.1) * speed;
        self.last = now;
        let peers = self.live();
        if self.bridge.as_ref().is_some_and(|(id, ..)| !peers.iter().any(|p| p.id == *id)) {
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

/// Avisa fora da janela (o pdrop pode estar em segundo plano) com notificação do sistema.
/// Remetente e nome do arquivo vêm da rede, então vão como argumentos, nunca montados em comando.
fn notify(t: &Transfer) {
    let body = format!("{} quer enviar {} ({})", t.peer, t.label, ui::human(t.size));
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("osascript")
            .args(["-e", "on run {t, b}", "-e", "display notification b with title t sound name \"Ping\"", "-e", "end run", "pdrop", &body])
            .status();
        #[cfg(not(target_os = "macos"))]
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "pdrop", "-u", "critical", "pdrop"])
            .arg(format!("{}", body.replace('&', "&amp;").replace('<', "&lt;")))
            .status();
    });
}

const NAME_MAX: usize = 32;
const NO_CLIP: &str = "sem clipboard: instale wl-clipboard (Wayland) ou xclip (X11)";

/// Pasta do usuário: HOME (Linux/macOS) ou USERPROFILE (Windows).
pub fn home() -> PathBuf {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(|| ".".into())
}

fn setting_path(k: &str) -> PathBuf {
    home().join(".config/pdrop").join(k)
}

fn load_setting(k: &str) -> Option<String> {
    std::fs::read_to_string(setting_path(k)).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Falhar ao gravar não pode derrubar o app: o valor vale na sessão e só não persiste.
fn save_setting(k: &str, v: &str) {
    let p = setting_path(k);
    let _ = std::fs::create_dir_all(p.parent().unwrap()).and_then(|_| std::fs::write(p, v));
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
        .or_else(|| std::process::Command::new("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()))
        .unwrap_or_default();
    let h = net::clean(h.trim().split('.').next().unwrap_or(""));
    if h.is_empty() { "pdrop".into() } else { h }
}

/// Ícone da janela (barra de tarefas no Windows/X11); no Wayland quem manda é o .desktop.
fn window_icon() -> eframe::egui::IconData {
    let im = image::load_from_memory(include_bytes!("../assets/pdrop.png")).expect("ícone embutido inválido").to_rgba8();
    eframe::egui::IconData { width: im.width(), height: im.height(), rgba: im.into_raw() }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut me = load_setting("nome").map(|n| net::clean(&n)).unwrap_or_else(hostname); // --name vale só na sessão
    let mut direct: Vec<std::net::IpAddr> = vec![];
    let mut calm = false;
    while let Some(a) = args.next() {
        if a == "--calm" {
            calm = true;
            continue;
        }
        match (a.as_str(), args.next()) {
            ("--name" | "-n", Some(n)) => me = net::clean(&n),
            ("--peer" | "-p", Some(ip)) if ip.parse::<std::net::IpAddr>().is_ok() => direct.push(ip.parse().unwrap()),
            _ => {
                eprintln!("uso: pdrop [--name NOME] [--peer IP]... [--calm]\nrecebidos vão para ~/Downloads");
                return Ok(());
            }
        }
    }
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
    let id = nanos ^ ((std::process::id() as u64) << 32);
    // no Snap o HOME é a pasta isolada do app; os recebidos vão para o Downloads de verdade
    let dir = std::env::var_os("SNAP_REAL_HOME").map(PathBuf::from).unwrap_or_else(home).join("Downloads");
    let (tx, rx) = mpsc::channel();
    let peers = Peers::default();
    let grants = Grants::default();
    let port = net::listen(dir, tx.clone(), grants.clone())?;
    let color = load_color();
    let icon = load_icon();
    let shared_color = Arc::new(AtomicU8::new(color | icon << 3));
    let shared_name = Arc::new(Mutex::new(me.clone()));
    net::discover(id, shared_name.clone(), port, shared_color.clone(), peers.clone(), direct)?;

    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_app_id("pdrop").with_icon(window_icon()).with_title("pdrop").with_inner_size([1000.0, 640.0]).with_min_inner_size([640.0, 420.0]),
        // arrastar e soltar só funciona no backend X11 do winit; com XWayland disponível, vai por ele
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
            Ok(Box::new(App { me, peers, sel: None, input: None, xfers: vec![], pending: VecDeque::new(), anim: 0.0, last: Instant::now(), calm, color, pick: None, icon, pick_icon: None, rename: None, shared_name, shared_color, bridge: None, grants, flash: None, rx, tx, sp, focus: false }))
        }),
    )?;
    Ok(())
}
