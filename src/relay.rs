//! cliente do `pdrop-relay`: a conexão de controle, que põe no radar quem está em outra rede

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
/// o relay limita o que recebe a 512 bytes e acrescenta o uid ao repassar
const MAX_LINE: u64 = 1024;

pub struct Config {
    pub addr: String,
    pub room: String,
}

#[derive(Clone)]
pub struct Me {
    pub id: u64,
    pub uid: u128,
    pub name: Arc<Mutex<String>>,
    pub color: Arc<AtomicU8>,
    pub policy: Arc<Policy>,
}

/// o relay só conhece o hash: o segredo não sai daqui
pub fn room_of(secret: &str) -> String {
    blake3::hash(secret.trim().as_bytes()).to_hex().to_string()
}

pub fn watch(cfg: Config, me: Me, peers: Peers) {
    thread::spawn(move || {
        let mut wait = Duration::from_secs(1);
        let mut last_error = String::new();
        loop {
            let start = Instant::now();
            let error = match connect(&cfg, me.uid) {
                Ok(s) => {
                    eprintln!("relay: conectado a {}", cfg.addr);
                    last_error.clear();
                    let e = session(s, &me, &peers);
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
fn session(mut r: BufReader<TcpStream>, me: &Me, peers: &Peers) -> io::Error {
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
            Ok(l) => handle(&l, me, peers),
            Err(e) => break e,
        }
    };
    stop.store(true, Ordering::Relaxed);
    let _ = r.get_ref().shutdown(Shutdown::Both);
    writer.thread().unpark();
    let _ = writer.join();
    error
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

fn handle(line: &str, me: &Me, peers: &Peers) {
    if let Some(uid) = line.strip_prefix("GONE ") {
        if let Ok(uid) = u128::from_str_radix(uid, 16) {
            drop_relay_peers(peers, Some(uid));
        }
        return;
    }
    // INCOMING ainda é ignorado: o recebimento pelo relay não está implementado
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
        Route::Relay,
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
        .retain(|_, p| !matches!(p.route, Route::Relay) || uid.is_some_and(|u| u != p.uid));
}

#[cfg(test)]
#[path = "bin/pdrop-relay.rs"]
#[allow(dead_code)]
mod server;

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

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
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        thread::spawn(move || server::serve(l, Arc::new(server::State::default())));
        let join = |id, vis| {
            let peers = Peers::default();
            let cfg = Config {
                addr: addr.clone(),
                room: room_of("segredo"),
            };
            watch(cfg, me(id, vis), peers.clone());
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
        assert!(matches!(p.route, Route::Relay));
        assert_eq!((p.name.as_str(), p.color, p.uid), ("aparelho 2", 3, 102));
    }

    #[test]
    fn gone_only_removes_relay_routes() {
        let (peers, me) = (Peers::default(), me(1, net::VIS_ALL));
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
