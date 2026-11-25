//! cliente do `pdrop-relay`: a conexão de controle, que põe no radar quem está em outra rede,
//! e os túneis por onde passam as transferências

use crate::net::{self, Peers, Policy, Route};
use socket2::{SockRef, TcpKeepalive};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpStream, ToSocketAddrs},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread::{self, sleep},
    time::{Duration, Instant},
};

const EVERY: Duration = Duration::from_millis(1500);
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// o relay espera até 10 s o alvo aceitar um túnel: a resposta pode levar mais que isso
const TUNNEL_TIMEOUT: Duration = Duration::from_secs(15);
/// o relay limita o que recebe a 512 bytes e acrescenta o uid ao repassar
const MAX_LINE: u64 = 1024;

pub struct Config {
    pub addr: String,
    pub room: String,
}

const MIN_SECRET: usize = 16;

/// monta a configuração; sem endereço e sem segredo, o relay fica desligado
pub fn config(addr: Option<&str>, secret: Option<&str>) -> Result<Option<Config>, String> {
    let (addr, secret) = match (addr.map(str::trim), secret.map(str::trim)) {
        (None, None) => return Ok(None),
        (Some(a), Some(s)) => (a, s),
        (Some(_), None) => return Err("relay sem o segredo da sala".into()),
        (None, Some(_)) => return Err("segredo da sala sem o endereço do relay".into()),
    };
    let valid = addr
        .rsplit_once(':')
        .is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p > 0))
        && !addr.chars().any(|c| c.is_whitespace() || c == '/');
    if !valid {
        return Err("endereço do relay inválido, use host:porta".into());
    }
    if secret.chars().count() < MIN_SECRET {
        return Err(format!(
            "segredo da sala curto demais (mínimo de {MIN_SECRET} caracteres); gere um com: openssl rand -hex 16"
        ));
    }
    Ok(Some(Config {
        addr: addr.into(),
        room: room_of(secret),
    }))
}

#[derive(Clone)]
pub struct Me {
    pub id: u64,
    pub uid: u128,
    pub name: Arc<Mutex<String>>,
    pub color: Arc<AtomicU8>,
    pub policy: Arc<Policy>,
}

/// recebe a transferência de um túnel que outro aparelho abriu até nós
pub type Incoming = Arc<dyn Fn(TcpStream) + Send + Sync>;

/// o relay só conhece o hash: o segredo não sai daqui
pub fn room_of(secret: &str) -> String {
    blake3::hash(secret.trim().as_bytes()).to_hex().to_string()
}

pub fn watch(cfg: Config, me: Me, peers: Peers, incoming: Incoming) {
    let cfg = Arc::new(cfg);
    thread::spawn(move || {
        let mut wait = Duration::from_secs(1);
        let mut last_error = String::new();
        loop {
            let start = Instant::now();
            let error = match connect(&cfg, me.uid) {
                Ok(s) => {
                    eprintln!("relay: conectado a {}", cfg.addr);
                    last_error.clear();
                    let e = session(s, &cfg, &me, &peers, &incoming);
                    drop_relay_peers(&peers, None);
                    e
                }
                Err(e) => e,
            }
            .to_string();
            if error != last_error {
                eprintln!("relay: {error}; tentando de novo");
                last_error = error;
            }
            // só zera a espera se a conexão durou, para não insistir num relay que fecha logo
            if start.elapsed() > MAX_BACKOFF {
                wait = Duration::from_secs(1);
            }
            sleep(wait);
            wait = (wait * 2).min(MAX_BACKOFF);
        }
    });
}

fn dial_tcp(addr: &str) -> io::Result<TcpStream> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "endereço do relay sem IP");
    for a in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&a, Duration::from_secs(5)) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn connect(cfg: &Config, uid: u128) -> io::Result<BufReader<TcpStream>> {
    let mut s = dial_tcp(&cfg.addr)?;
    s.set_read_timeout(Some(TIMEOUT))?;
    s.set_write_timeout(Some(TIMEOUT))?;
    // o relay não manda heartbeat: é o TCP que precisa perceber a conexão morta
    let sock = SockRef::from(&s);
    sock.set_tcp_keepalive(
        &TcpKeepalive::new()
            .with_time(Duration::from_secs(20))
            .with_interval(Duration::from_secs(5)),
    )?;
    // com beacons sem confirmação o keepalive não dispara; no Linux isto cobre o caso
    #[cfg(target_os = "linux")]
    sock.set_tcp_user_timeout(Some(Duration::from_secs(30)))?;
    s.write_all(format!("HELLO {} {uid:x}\n", cfg.room).as_bytes())?;
    let mut r = BufReader::new(s);
    if read_line(&mut r)? != "OK" {
        return Err(io::Error::other("o relay recusou a entrada na sala"));
    }
    r.get_ref().set_read_timeout(None)?;
    Ok(r)
}

/// abre um túnel até outro membro da sala
pub fn dial(cfg: &Config, target: u128) -> io::Result<TcpStream> {
    tunnel(&cfg.addr, &format!("DIAL {} {target:x}", cfg.room))
}

fn accept(cfg: &Config, uid: u128, token: u128) -> io::Result<TcpStream> {
    tunnel(&cfg.addr, &format!("ACCEPT {} {uid:x} {token:x}", cfg.room))
}

fn tunnel(addr: &str, request: &str) -> io::Result<TcpStream> {
    let mut s = dial_tcp(addr)?;
    s.set_read_timeout(Some(TUNNEL_TIMEOUT))?;
    s.set_write_timeout(Some(TIMEOUT))?;
    s.set_nodelay(true)?;
    s.write_all(format!("{request}\n").as_bytes())?;
    // um byte por vez: o que vem depois do OK já é do handshake e não pode ficar num buffer
    let mut answer = Vec::new();
    let mut byte = [0u8; 1];
    while answer.len() < 8 {
        if s.read(&mut byte)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "o relay fechou a conexão; pode ser o limite de uso",
            ));
        }
        if byte[0] == b'\n' {
            break;
        }
        answer.push(byte[0]);
    }
    if answer != b"OK" {
        return Err(io::Error::other("o relay não alcançou o aparelho"));
    }
    s.set_write_timeout(Some(Duration::from_secs(30)))?;
    Ok(s)
}

fn read_line(r: &mut impl BufRead) -> io::Result<String> {
    let mut l = String::new();
    r.take(MAX_LINE).read_line(&mut l)?;
    if l.pop() != Some('\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "o relay fechou a conexão",
        ));
    }
    Ok(l)
}

/// anuncia e escuta até a conexão cair
fn session(
    mut r: BufReader<TcpStream>,
    cfg: &Arc<Config>,
    me: &Me,
    peers: &Peers,
    incoming: &Incoming,
) -> io::Error {
    let mut w = match r.get_ref().try_clone() {
        Ok(w) => w,
        Err(e) => return e,
    };
    let stop = Arc::new(AtomicBool::new(false));
    let (me2, stop2) = (me.clone(), stop.clone());
    let writer = thread::spawn(move || {
        while !stop2.load(Ordering::Relaxed) {
            if w.write_all(announce(&me2).as_bytes()).is_err() {
                let _ = w.shutdown(Shutdown::Both);
                break;
            }
            thread::park_timeout(EVERY);
        }
    });
    let error = loop {
        match read_line(&mut r) {
            Ok(l) => match l.strip_prefix("INCOMING ") {
                Some(token) => answer(token, cfg, me.uid, incoming),
                None => handle(&l, cfg, me, peers),
            },
            Err(e) => break e,
        }
    };
    stop.store(true, Ordering::Relaxed);
    let _ = r.get_ref().shutdown(Shutdown::Both);
    writer.thread().unpark();
    let _ = writer.join();
    error
}

/// atende um INCOMING numa thread própria, sem parar a conexão de controle
fn answer(token: &str, cfg: &Arc<Config>, uid: u128, incoming: &Incoming) {
    let Ok(token) = u128::from_str_radix(token, 16) else {
        return;
    };
    let (cfg, incoming) = (cfg.clone(), incoming.clone());
    thread::spawn(move || match accept(&cfg, uid, token) {
        Ok(s) => incoming(s),
        Err(e) => eprintln!("relay: não consegui atender uma conexão: {e}"),
    });
}

fn announce(me: &Me) -> String {
    let vis = me.policy.vis.load(Ordering::Relaxed);
    if vis == net::VIS_HIDDEN {
        // o relay ignora a linha, mas ela evita ser derrubado por 30 s de silêncio
        return "PING\n".into();
    }
    format!(
        "BEACON {:x}|{}|{vis}|{}\n",
        me.id,
        me.color.load(Ordering::Relaxed),
        net::clean(&me.name.lock().unwrap())
    )
}

fn handle(line: &str, cfg: &Arc<Config>, me: &Me, peers: &Peers) {
    if let Some(uid) = line.strip_prefix("GONE ") {
        if let Ok(uid) = u128::from_str_radix(uid, 16) {
            drop_relay_peers(peers, Some(uid));
        }
        return;
    }
    let Some((uid, beacon)) = line.strip_prefix("BEACON ").and_then(|l| l.split_once(' ')) else {
        return;
    };
    let f: Vec<&str> = beacon.splitn(4, '|').collect();
    let &[id, color, vis, name] = f.as_slice() else {
        return;
    };
    let (Ok(uid), Ok(id), Ok(color), Ok(vis)) = (
        u128::from_str_radix(uid, 16),
        u64::from_str_radix(id, 16),
        color.parse::<u8>(),
        vis.parse::<u8>(),
    ) else {
        return;
    };
    let trusted = me.policy.trust.lock().unwrap().contains_key(&uid);
    if id == me.id || !net::shown(vis, me.policy.vis.load(Ordering::Relaxed), trusted) {
        return;
    }
    net::upsert(
        peers,
        id,
        net::clean(name),
        Route::Relay(cfg.clone()),
        color,
        uid,
        Instant::now(),
    );
}

/// tira do radar quem só era alcançado pelo relay (um uid, ou todos)
fn drop_relay_peers(peers: &Peers, uid: Option<u128>) {
    peers
        .lock()
        .unwrap()
        .retain(|_, p| !matches!(p.route, Route::Relay(_)) || uid.is_some_and(|u| u != p.uid));
}

#[cfg(test)]
#[path = "bin/pdrop-relay.rs"]
#[allow(dead_code)]
mod server;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        net::{Grants, Msg, Who},
        secure,
    };
    use std::{net::TcpListener, sync::mpsc::channel};

    fn start_server() -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        thread::spawn(move || server::serve(l, Arc::new(server::State::default())));
        addr
    }

    fn config(addr: &str) -> Config {
        Config {
            addr: addr.into(),
            room: room_of("segredo"),
        }
    }

    fn me(id: u64, vis: u8) -> Me {
        Me {
            id,
            uid: id as u128 + 100,
            name: Arc::new(Mutex::new(format!("aparelho {id}"))),
            color: Arc::new(AtomicU8::new(3)),
            policy: Arc::new(Policy {
                vis: AtomicU8::new(vis),
                ..Default::default()
            }),
        }
    }

    fn until(what: &str, f: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !f() {
            assert!(Instant::now() < end, "{what}");
            sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn peers_show_up_through_a_real_relay() {
        let addr = start_server();
        let join = |id, vis| {
            let peers = Peers::default();
            let incoming: Incoming = Arc::new(|_| {});
            watch(config(&addr), me(id, vis), peers.clone(), incoming);
            peers
        };
        let a = join(1, net::VIS_ALL);
        let b = join(2, net::VIS_ALL);
        let hidden = join(3, net::VIS_HIDDEN);

        until("a e b se veem", || {
            a.lock().unwrap().contains_key(&2) && b.lock().unwrap().contains_key(&1)
        });
        until("o invisível vê os outros", || {
            hidden.lock().unwrap().len() == 2
        });
        let g = a.lock().unwrap();
        assert!(!g.contains_key(&3), "o invisível não aparece");
        let p = &g[&2];
        assert!(matches!(p.route, Route::Relay(_)));
        assert_eq!((p.name.as_str(), p.color, p.uid), ("aparelho 2", 3, 102));
    }

    #[test]
    fn configuration_is_checked() {
        let secret = "0123456789abcdef";
        assert!(super::config(None, None).unwrap().is_none());
        let c = super::config(Some(" ex.com:443 "), Some(secret))
            .unwrap()
            .unwrap();
        assert_eq!((c.addr.as_str(), c.room), ("ex.com:443", room_of(secret)));
        assert!(super::config(Some("[::1]:443"), Some(secret)).is_ok());
        for addr in [
            "ex.com",
            ":443",
            "ex.com:0",
            "ex.com:x",
            "https://ex.com:443",
        ] {
            assert!(super::config(Some(addr), Some(secret)).is_err(), "{addr}");
        }
        assert!(super::config(Some("ex.com:443"), Some(&secret[1..])).is_err());
        assert!(super::config(Some("ex.com:443"), None).is_err());
        assert!(super::config(None, Some(secret)).is_err());
    }

    #[test]
    fn transfers_go_through_the_relay() {
        let addr = start_server();
        let dir = std::env::temp_dir().join(format!("pdrop-relay-test-{}", std::process::id()));
        let (out, src) = (dir.join("recebidos"), dir.join("origem"));
        std::fs::create_dir_all(&src).unwrap();
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(src.join("big.bin"), &big).unwrap();
        std::fs::write(src.join("foto.jpg"), &big).unwrap();

        let ((sk, sp), (rk, rp)) = (secure::generate(), secure::generate());
        let (suid, ruid) = (secure::id_of(&sp), secure::id_of(&rp));
        let device = |id, uid| Me {
            uid,
            ..me(id, net::VIS_ALL)
        };

        // quem recebe aceita tudo
        let (rtx, rrx) = channel();
        let policy = Arc::new(Policy {
            key: rk,
            ..Default::default()
        });
        let (out2, grants) = (out.clone(), Grants::default());
        let incoming: Incoming = Arc::new(move |s| {
            net::recv(s, &rtx, &out2, &grants, &policy).unwrap();
        });
        watch(config(&addr), device(2, ruid), Peers::default(), incoming);
        let (got_tx, got) = channel();
        thread::spawn(move || {
            for m in rrx {
                match m {
                    Msg::Ask(_, answer) => answer.send(true).unwrap(),
                    Msg::End(_, r) => got_tx.send(r).unwrap(),
                    _ => {}
                }
            }
        });

        let peers = Peers::default();
        watch(
            config(&addr),
            device(1, suid),
            peers.clone(),
            Arc::new(|_| {}),
        );
        until("o remetente vê o destinatário", || {
            peers.lock().unwrap().contains_key(&2)
        });
        let to = peers.lock().unwrap()[&2].clone();
        let who = Who {
            name: "eu".into(),
            key: sk,
        };
        let (tx, rx) = channel();
        let sent = || loop {
            if let Msg::End(_, r) = rx.recv_timeout(Duration::from_secs(10)).unwrap() {
                break r;
            }
        };
        let received = || got.recv_timeout(Duration::from_secs(10)).unwrap();

        net::send(
            to.clone(),
            who.clone(),
            vec!["oi de longe".into()],
            tx.clone(),
        );
        assert_eq!(sent(), Ok("enviado".into()));
        assert_eq!(received(), Ok("oi de longe".into()));

        net::send(
            to.clone(),
            who.clone(),
            vec![src.display().to_string()],
            tx.clone(),
        );
        assert_eq!(sent(), Ok("enviado".into()));
        received().unwrap();
        for f in ["big.bin", "foto.jpg"] {
            assert_eq!(std::fs::read(out.join("origem").join(f)).unwrap(), big);
        }

        // ninguém com esse uid na sala
        let ghost = net::Peer {
            uid: ruid ^ 1,
            ..to
        };
        net::send(ghost, who, vec!["oi".into()], tx.clone());
        assert!(sent().unwrap_err().contains("não alcançou"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn gone_only_removes_relay_routes() {
        let (peers, me) = (Peers::default(), me(1, net::VIS_ALL));
        let cfg = Arc::new(config("sem-uso:1"));
        let handle = |line, me, peers| super::handle(line, &cfg, me, peers);
        handle("BEACON a 2|0|0|longe", &me, &peers);
        handle("BEACON b 3|0|0|com|barra", &me, &peers);
        handle("BEACON c 1|0|0|eu mesmo", &me, &peers);
        handle("BEACON d lixo", &me, &peers);
        let lan = Route::Lan(([127, 0, 0, 1], 1).into());
        net::upsert(&peers, 4, "perto".into(), lan, 0, 0xa, Instant::now());
        assert_eq!(peers.lock().unwrap().len(), 3);
        assert_eq!(peers.lock().unwrap()[&3].name, "com|barra");

        handle("GONE a", &me, &peers);
        let mut left: Vec<u64> = peers.lock().unwrap().keys().copied().collect();
        left.sort();
        assert_eq!(left, [3, 4]);
        drop_relay_peers(&peers, None);
        assert!(peers.lock().unwrap().keys().eq([&4]));
    }
}
