#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod cli;
mod clip;
mod config;
mod contacts;
mod desktop;
mod firewall;
mod fmt;
mod history;
mod net;
mod relay;
mod secure;
mod ui;

use app::{App, VIEWS};
use config::{
    config_dir, home, load_color, load_icon, load_identity, load_routes, load_setting, load_trust,
    saved_peers,
};
use desktop::hostname;
use history::load_hist;
use net::{Grants, Peers, Policy};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicU8, mpsc},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("send") {
        std::process::exit(cli::send(std::env::args().skip(2).collect()));
    }
    let mut args = std::env::args().skip(1);
    let mut me = load_setting("nome")
        .map(|n| net::clean(&n))
        .unwrap_or_else(hostname);
    let mut direct = saved_peers();
    let mut calm = false;
    let mut out_dir = None;
    let mut flags = relay::saved::Flags::default();
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
            ("--relay", Some(a)) => flags.addr = Some(a),
            ("--room", Some(s)) => flags.room = Some(s),
            _ => {
                eprintln!("{}", cli::USAGE);
                return Ok(());
            }
        }
    }
    let mut saved = relay::saved::load(&config_dir());
    let relay_start = match relay::saved::resolve(&flags, &relay::saved::Env::read(), &saved) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = relay::saved::remember(&config_dir(), &flags, &mut saved) {
        eprintln!("{e}");
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
    let port = net::listen(dir.clone(), tx.clone(), grants.clone(), policy.clone())?;
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

    let incoming: relay::Incoming = {
        let (tx, grants, rules) = (tx.clone(), grants.clone(), policy.clone());
        Arc::new(move |s| {
            let _ = net::recv(s, &tx, &dir, &grants, &rules);
        })
    };
    let relay = relay::saved::Control::new(
        config_dir(),
        saved,
        relay::Me {
            id,
            uid,
            name: shared_name.clone(),
            color: shared_color.clone(),
            policy: policy.clone(),
            offline: Some(net::inbox(tx.clone(), policy.clone())),
        },
        peers.clone(),
        incoming,
        relay_start,
    );

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
                prompts: Default::default(),
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
                board: false,
                known: Default::default(),
                marked: Default::default(),
                firewall,
                relay,
                relay_ui: None,
                view: load_setting("visao")
                    .and_then(|s| s.parse().ok())
                    .filter(|v| (*v as usize) < VIEWS.len())
                    .unwrap_or(0),
                cols: 1,
                search: None,
                pan: Default::default(),
                cam_key: None,
                pan_now: Default::default(),
                drawn: Default::default(),
                more: false,
            }))
        }),
    )?;
    Ok(())
}

fn proc_id() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    nanos ^ ((std::process::id() as u64) << 32)
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
