use crate::{
    config::{config_dir, load_identity, load_setting, saved_peers},
    desktop::hostname,
    fmt::human,
    net::{self, Msg, Peer, Peers, Policy, State, Who},
    proc_id, relay, secure,
};
use std::{
    sync::{Arc, Mutex, atomic::AtomicU8, mpsc},
    time::{Duration, Instant},
};

pub const USAGE: &str = "uso: pdrop [--name NOME] [--peer IP]... [--dir PASTA] [--calm] [--relay HOST:PORTA] [--room SEGREDO]
     pdrop send [--to NOME] [--peer IP]... [--as NOME] [--relay HOST:PORTA] [--room SEGREDO] CAMINHO|TEXTO|- ...
relay (aparelhos em outra rede): configure na janela (botão Relay), que guarda vários em
~/.config/pdrop/relays; --relay e --room também são salvos por ela. Valem ainda PDROP_RELAY e
PDROP_ROOM. Pela janela ou pelo ambiente a senha não fica no histórico do shell
recebidos vão para ~/Downloads (ou --dir); regras de destino em ~/.config/pdrop/destinos;
IPs fixos (VPN/Tailscale) em ~/.config/pdrop/peers, um por linha";

fn usage() -> i32 {
    eprintln!("{USAGE}");
    2
}

pub fn send(args: Vec<String>) -> i32 {
    let (mut to, mut name_as, mut direct, mut inputs) = (None, None, saved_peers(), vec![]);
    let mut flags = relay::saved::Flags::default();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--to" | "-t" => to = it.next(),
            "--as" => name_as = it.next(),
            "--relay" => match it.next() {
                Some(a) => flags.addr = Some(a),
                None => return usage(),
            },
            "--room" => match it.next() {
                Some(s) => flags.room = Some(s),
                None => return usage(),
            },
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
    let saved = relay::saved::load(&config_dir());
    let relay = match relay::saved::resolve(&flags, &relay::saved::Env::read(), &saved) {
        Ok((r, _)) => r,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
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
    let code = cli_run(to, direct, relay, inputs);
    let _ = std::fs::remove_dir_all(tmp);
    code
}

fn cli_run(
    to: Option<String>,
    direct: Vec<std::net::IpAddr>,
    relay: Option<relay::Config>,
    inputs: Vec<String>,
) -> i32 {
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
    let id = proc_id();
    if let Err(e) = net::discover(
        id,
        uid,
        Arc::new(Mutex::new(me.clone())),
        0,
        Arc::new(AtomicU8::new(0)),
        policy.clone(),
        peers.clone(),
        direct,
    ) {
        eprintln!("sem rede: {e}");
        return 1;
    }
    let with_relay = relay.is_some();
    let wait = Duration::from_secs(if relay.is_some() { 8 } else { 6 });
    if let Some(cfg) = relay {
        relay::watch(
            cfg,
            relay::Me {
                id,
                uid: secure::id_of(&secure::generate().1),
                name: Arc::new(Mutex::new(me.clone())),
                color: Arc::new(AtomicU8::new(0)),
                policy,
            },
            peers.clone(),
            Arc::new(|_| {}),
        );
    }
    let pick = |v: &[Peer]| -> Option<Peer> {
        let t = to.as_ref()?.to_lowercase();
        v.iter()
            .find(|p| p.name.to_lowercase() == t)
            .or_else(|| v.iter().find(|p| p.name.to_lowercase().starts_with(&t)))
            .cloned()
    };
    let start = Instant::now();
    let deadline = start + wait;
    let target = loop {
        std::thread::sleep(Duration::from_millis(200));
        let v: Vec<Peer> = peers
            .lock()
            .unwrap()
            .values()
            .filter(|p| !(matches!(p.route, net::Route::Relay(_)) && p.uid == uid))
            .cloned()
            .collect();
        let v = one_per_device(v);
        let found = pick(&v).filter(|p| {
            start.elapsed() >= LAN_GRACE
                || (!p.via_relay() && (!with_relay || p.fallback.is_some()))
        });
        match (&to, v.as_slice()) {
            (Some(_), _) if found.is_some() => break found,
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
                human(t.size),
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

fn one_per_device(mut v: Vec<Peer>) -> Vec<Peer> {
    v.sort_by_key(|p| std::cmp::Reverse(p.seen));
    let mut seen = std::collections::HashSet::new();
    v.retain(|p| seen.insert(p.uid));
    v
}

const LAN_GRACE: Duration = Duration::from_secs(3);

#[cfg(test)]
mod tests {

    #[test]
    fn send_keeps_the_latest_session_of_each_device() {
        use crate::net::{Peer, Route};
        let now = std::time::Instant::now();
        let peer = |id, uid, age| Peer {
            id,
            name: "a".into(),
            route: Route::Lan(([127, 0, 0, 1], 1).into()),
            fallback: None,
            seen: now - std::time::Duration::from_secs(age),
            born: now,
            rtt: None,
            color: 0,
            uid,
        };
        let v = super::one_per_device(vec![peer(1, 7, 3), peer(2, 7, 0), peer(3, 8, 1)]);
        let ids: Vec<u64> = v.iter().map(|p| p.id).collect();
        assert_eq!(ids, [2, 3]);
    }
}
