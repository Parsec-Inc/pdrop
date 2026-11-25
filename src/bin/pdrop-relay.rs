use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Write},
    net::{IpAddr, Shutdown, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const MAX_PER_IP: u32 = 32;
const MAX_ROOM: usize = 32;
const MAX_ROOMS: usize = 1024;
const MAX_RATE: u32 = 30;
/// túneis têm cota própria: cada transferência gasta um DIAL e um ACCEPT
const MAX_TUNNEL_RATE: u32 = 120;
const WINDOW: Duration = Duration::from_secs(60);
const MAX_LINE: usize = 512;
const WAIT: Duration = Duration::from_secs(30);
const WRITE: Duration = Duration::from_secs(5);
const PAIR: Duration = Duration::from_secs(10);
const IDLE: u64 = 120;
const POLL: Duration = Duration::from_secs(5);

type Out = Arc<Mutex<TcpStream>>;

struct Member {
    out: Out,
    epoch: u64,
}

struct Pending {
    room: String,
    uid: u128,
    tx: mpsc::Sender<TcpStream>,
}

#[derive(Default)]
pub struct State {
    rooms: Mutex<HashMap<String, HashMap<u128, Member>>>,
    pending: Mutex<HashMap<u128, Pending>>,
    conns: Mutex<HashMap<IpAddr, u32>>,
    rate: Mutex<HashMap<(IpAddr, bool), (Instant, u32)>>,
    epoch: AtomicU64,
}

impl State {
    fn allow(&self, ip: IpAddr, tunnel: bool) -> bool {
        let mut r = self.rate.lock().unwrap();
        if r.len() > 10_000 {
            r.retain(|_, (t, _)| t.elapsed() < WINDOW);
        }
        let e = r.entry((ip, tunnel)).or_insert((Instant::now(), 0));
        if e.0.elapsed() >= WINDOW {
            *e = (Instant::now(), 0);
        }
        e.1 += 1;
        e.1 <= if tunnel { MAX_TUNNEL_RATE } else { MAX_RATE }
    }
}

struct Slot(Arc<State>, IpAddr);
impl Drop for Slot {
    fn drop(&mut self) {
        let mut c = self.0.conns.lock().unwrap();
        if let Some(n) = c.get_mut(&self.1) {
            *n -= 1;
            if *n == 0 {
                c.remove(&self.1);
            }
        }
    }
}

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

fn read_line(s: &mut impl Read) -> io::Result<String> {
    let (mut v, mut b) = (Vec::new(), [0u8; 1]);
    loop {
        if s.read(&mut b)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if b[0] == b'\n' {
            break;
        }
        if v.len() >= MAX_LINE {
            return Err(bad("linha longa"));
        }
        v.push(b[0]);
    }
    let l = String::from_utf8(v).map_err(|_| bad("utf8"))?;
    if l.chars().any(char::is_control) {
        return Err(bad("controle"));
    }
    Ok(l)
}

fn room_id(s: &str) -> io::Result<String> {
    if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(s.to_ascii_lowercase())
    } else {
        Err(bad("sala"))
    }
}

fn hex(s: &str) -> io::Result<u128> {
    u128::from_str_radix(s, 16).map_err(|_| bad("hex"))
}

fn token() -> io::Result<u128> {
    let mut b = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(u128::from_ne_bytes(b))
}

fn broadcast(st: &State, room: &str, except: u128, msg: &str) {
    let outs: Vec<Out> = st
        .rooms
        .lock()
        .unwrap()
        .get(room)
        .map(|r| {
            r.iter()
                .filter(|(u, _)| **u != except)
                .map(|(_, m)| m.out.clone())
                .collect()
        })
        .unwrap_or_default();
    for o in outs {
        let _ = o.lock().unwrap().write_all(msg.as_bytes());
    }
}

fn hello(st: &State, mut s: TcpStream, room: String, uid: u128) -> io::Result<()> {
    let out: Out = Arc::new(Mutex::new(s.try_clone()?));
    let epoch = st.epoch.fetch_add(1, Ordering::Relaxed);
    let welcome = {
        let mut guard = out.lock().unwrap();
        let mut rooms = st.rooms.lock().unwrap();
        if !rooms.contains_key(&room) && rooms.len() >= MAX_ROOMS {
            return Err(bad("relay cheio"));
        }
        let r = rooms.entry(room.clone()).or_default();
        if r.len() >= MAX_ROOM && !r.contains_key(&uid) {
            return Err(bad("sala cheia"));
        }
        r.insert(
            uid,
            Member {
                out: out.clone(),
                epoch,
            },
        );
        drop(rooms);
        guard.write_all(b"OK\n")
    };
    // mesmo se o OK falhar, a saída passa pela limpeza abaixo
    let res = welcome.and_then(|()| {
        loop {
            let l = read_line(&mut s)?;
            if let Some(t) = l.strip_prefix("BEACON ") {
                broadcast(st, &room, uid, &format!("BEACON {uid:x} {t}\n"));
            }
        }
    });
    let gone = {
        let mut rooms = st.rooms.lock().unwrap();
        let r = rooms.get_mut(&room);
        let mine = r
            .as_ref()
            .is_some_and(|r| r.get(&uid).is_some_and(|m| m.epoch == epoch));
        if let Some(r) = r {
            if mine {
                r.remove(&uid);
            }
            if r.is_empty() {
                rooms.remove(&room);
            }
        }
        mine
    };
    if gone {
        broadcast(st, &room, uid, &format!("GONE {uid:x}\n"));
    }
    res
}

fn dial(st: &State, mut s: TcpStream, room: &str, target: u128) -> io::Result<()> {
    let out = st
        .rooms
        .lock()
        .unwrap()
        .get(room)
        .and_then(|r| r.get(&target))
        .map(|m| m.out.clone());
    let Some(out) = out else {
        return s.write_all(b"ERR\n");
    };
    let tok = token()?;
    let (tx, rx) = mpsc::channel();
    st.pending.lock().unwrap().insert(
        tok,
        Pending {
            room: room.into(),
            uid: target,
            tx,
        },
    );
    let sent = out
        .lock()
        .unwrap()
        .write_all(format!("INCOMING {tok:x}\n").as_bytes());
    let other = sent.ok().and_then(|_| rx.recv_timeout(PAIR).ok());
    st.pending.lock().unwrap().remove(&tok);
    let Some(mut b) = other else {
        return s.write_all(b"ERR\n");
    };
    b.write_all(b"OK\n")?;
    s.write_all(b"OK\n")?;
    splice(s, b)
}

fn accept(st: &State, s: TcpStream, room: &str, uid: u128, tok: u128) -> io::Result<()> {
    let p = st.pending.lock().unwrap().remove(&tok);
    match p {
        Some(p) if p.room == room && p.uid == uid => {
            let _ = p.tx.send(s);
            Ok(())
        }
        _ => Err(bad("token")),
    }
}

fn splice(a: TcpStream, b: TcpStream) -> io::Result<()> {
    for s in [&a, &b] {
        s.set_read_timeout(Some(POLL))?;
        s.set_write_timeout(Some(Duration::from_secs(30)))?;
    }
    let (a2, b2) = (a.try_clone()?, b.try_clone()?);
    let t0 = Instant::now();
    let last = Arc::new(AtomicU64::new(0));
    let l2 = last.clone();
    let h = thread::spawn(move || pump(a2, b2, t0, &l2));
    pump(b, a, t0, &last);
    let _ = h.join();
    Ok(())
}

fn pump(mut from: TcpStream, mut to: TcpStream, t0: Instant, last: &AtomicU64) {
    let mut buf = vec![0u8; 65536];
    loop {
        match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if to.write_all(&buf[..n]).is_err() {
                    break;
                }
                last.store(t0.elapsed().as_secs(), Ordering::Relaxed);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if t0
                    .elapsed()
                    .as_secs()
                    .saturating_sub(last.load(Ordering::Relaxed))
                    > IDLE
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = to.shutdown(Shutdown::Write);
}

fn handle(st: &State, mut s: TcpStream, ip: IpAddr) -> io::Result<()> {
    s.set_read_timeout(Some(WAIT))?;
    s.set_write_timeout(Some(WRITE))?;
    s.set_nodelay(true)?;
    let line = read_line(&mut s)?;
    // linhas inválidas caem na cota menor
    let tunnel = line.starts_with("DIAL ") || line.starts_with("ACCEPT ");
    if !st.allow(ip, tunnel) {
        return Err(bad("rate"));
    }
    match line.split(' ').collect::<Vec<_>>()[..] {
        ["HELLO", room, uid] => hello(st, s, room_id(room)?, hex(uid)?),
        ["DIAL", room, uid] => dial(st, s, &room_id(room)?, hex(uid)?),
        ["ACCEPT", room, uid, tok] => accept(st, s, &room_id(room)?, hex(uid)?, hex(tok)?),
        _ => Err(bad("comando")),
    }
}

pub fn serve(l: TcpListener, st: Arc<State>) {
    for s in l.incoming().flatten() {
        let Ok(ip) = s.peer_addr().map(|a| a.ip()) else {
            continue;
        };
        {
            let mut c = st.conns.lock().unwrap();
            let n = c.entry(ip).or_insert(0);
            if *n >= MAX_PER_IP {
                continue;
            }
            *n += 1;
        }
        let (slot, st) = (Slot(st.clone(), ip), st.clone());
        let _ = thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let _slot = slot;
                let _ = handle(&st, s, ip);
            });
    }
}

fn main() -> io::Result<()> {
    let mut addr = "0.0.0.0:443".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match (a.as_str(), args.next()) {
            ("--listen", Some(v)) => addr = v,
            _ => {
                eprintln!("uso: pdrop-relay [--listen ENDERECO:PORTA]   (padrão {addr})");
                std::process::exit(2);
            }
        }
    }
    let l = TcpListener::bind(&addr)?;
    eprintln!("pdrop-relay ouvindo em {addr}");
    serve(l, Arc::new(State::default()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn(addr: std::net::SocketAddr, first: &str) -> TcpStream {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        writeln!(s, "{first}").unwrap();
        s
    }

    #[test]
    fn tunnels_have_their_own_rate_limit() {
        let st = State::default();
        let ip = IpAddr::from([127, 0, 0, 1]);
        for (tunnel, max) in [(false, MAX_RATE), (true, MAX_TUNNEL_RATE)] {
            assert!((0..max).all(|_| st.allow(ip, tunnel)));
            assert!(!st.allow(ip, tunnel));
        }
    }

    #[test]
    fn failed_welcome_leaves_no_ghost_member() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let _client = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (s, _) = l.accept().unwrap();
        s.shutdown(Shutdown::Write).unwrap(); // o envio do OK vai falhar
        let st = State::default();
        assert!(hello(&st, s, "a".repeat(64), 1).is_err());
        assert!(
            st.rooms.lock().unwrap().is_empty(),
            "sobrou membro fantasma"
        );
    }

    #[test]
    fn rooms_isolate_and_tunnel() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        thread::spawn(move || serve(l, Arc::new(State::default())));
        let (ra, rb) = ("a".repeat(64), "b".repeat(64));

        let mut a = conn(addr, &format!("HELLO {ra} 1"));
        let mut b = conn(addr, &format!("HELLO {ra} 2"));
        let mut c = conn(addr, &format!("HELLO {rb} 3"));
        for s in [&mut a, &mut b, &mut c] {
            assert_eq!(read_line(s).unwrap(), "OK");
        }

        writeln!(a, "BEACON oi").unwrap();
        assert_eq!(read_line(&mut b).unwrap(), "BEACON 1 oi");
        c.set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        assert!(read_line(&mut c).is_err(), "outra sala não pode ver");

        let mut x = conn(addr, &format!("DIAL {ra} 3"));
        assert_eq!(read_line(&mut x).unwrap(), "ERR");

        let mut d = conn(addr, &format!("DIAL {ra} 2"));
        let tok = read_line(&mut b)
            .unwrap()
            .strip_prefix("INCOMING ")
            .unwrap()
            .to_string();
        let mut e = conn(addr, &format!("ACCEPT {ra} 2 {tok}"));
        assert_eq!(read_line(&mut d).unwrap(), "OK");
        assert_eq!(read_line(&mut e).unwrap(), "OK");
        d.write_all(b"hi").unwrap();
        let mut buf = [0u8; 2];
        e.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"hi");
        e.write_all(b"yo").unwrap();
        d.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"yo");

        drop(a);
        assert_eq!(read_line(&mut b).unwrap(), "GONE 1");
    }
}
