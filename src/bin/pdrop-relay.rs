use std::{
    collections::HashMap,
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

const MAX_CONNS: u32 = 2048;
const MAX_NEW_PER_IP: u32 = 32;
const MAX_CONTROL_PER_IP: u32 = 32;
const MAX_TUNNEL_PER_IP: u32 = 32;
const MAX_ROOM: usize = 32;
const MAX_ROOMS: usize = 1024;
const MAX_ROOMS_PER_IP: usize = 8;
const MAX_TRACKED: usize = 10_000;
const QUEUE: usize = 256;
const MAX_RATE: u32 = 30;
const MAX_TUNNEL_RATE: u32 = 120;
const WINDOW: Duration = Duration::from_secs(60);
const MAX_LINE: usize = 512;
const WAIT: Duration = Duration::from_secs(30);
const FIRST_LINE: Duration = Duration::from_secs(10);
const WRITE: Duration = Duration::from_secs(5);
const PAIR: Duration = Duration::from_secs(10);
const IDLE: u64 = 120;
const POLL: Duration = Duration::from_secs(5);

struct Member {
    tx: mpsc::SyncSender<String>,
    sock: Arc<TcpStream>,
    key: u128,
    ip: IpAddr,
    epoch: u64,
}

struct Pending {
    room: String,
    uid: u128,
    tx: mpsc::Sender<(TcpStream, Slot)>,
}

#[derive(Default)]
pub struct State {
    rooms: Mutex<HashMap<String, HashMap<u128, Member>>>,
    pending: Mutex<HashMap<u128, Pending>>,
    conns: Mutex<HashMap<(IpAddr, Kind), u32>>,
    rate: Mutex<HashMap<(IpAddr, bool), (Instant, u32)>>,
    epoch: AtomicU64,
}

impl State {
    fn allow(&self, ip: IpAddr, tunnel: bool) -> bool {
        let mut r = self.rate.lock().unwrap();
        let key = (ip, tunnel);
        if r.len() >= MAX_TRACKED && !r.contains_key(&key) {
            r.retain(|_, (t, _)| t.elapsed() < WINDOW);
            if r.len() >= MAX_TRACKED {
                return false;
            }
        }
        let e = r.entry(key).or_insert((Instant::now(), 0));
        if e.0.elapsed() >= WINDOW {
            *e = (Instant::now(), 0);
        }
        e.1 += 1;
        e.1 <= if tunnel { MAX_TUNNEL_RATE } else { MAX_RATE }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    New,
    Control,
    Tunnel,
}

impl Kind {
    fn max_per_ip(self) -> u32 {
        match self {
            Kind::New => MAX_NEW_PER_IP,
            Kind::Control => MAX_CONTROL_PER_IP,
            Kind::Tunnel => MAX_TUNNEL_PER_IP,
        }
    }
}

struct Slot {
    st: Arc<State>,
    ip: IpAddr,
    kind: Kind,
}

impl Slot {
    fn open(st: &Arc<State>, ip: IpAddr) -> Option<Slot> {
        let mut c = st.conns.lock().unwrap();
        if c.values().sum::<u32>() >= MAX_CONNS {
            return None;
        }
        let n = c.entry((ip, Kind::New)).or_insert(0);
        if *n >= Kind::New.max_per_ip() {
            return None;
        }
        *n += 1;
        Some(Slot {
            st: st.clone(),
            ip,
            kind: Kind::New,
        })
    }

    fn become_kind(&mut self, kind: Kind) -> bool {
        let mut c = self.st.conns.lock().unwrap();
        let n = c.entry((self.ip, kind)).or_insert(0);
        if *n >= kind.max_per_ip() {
            return false;
        }
        *n += 1;
        release(&mut c, (self.ip, self.kind));
        self.kind = kind;
        true
    }
}

fn release(c: &mut HashMap<(IpAddr, Kind), u32>, key: (IpAddr, Kind)) {
    if let Some(n) = c.get_mut(&key) {
        *n -= 1;
        if *n == 0 {
            c.remove(&key);
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        release(&mut self.st.conns.lock().unwrap(), (self.ip, self.kind));
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

struct Until<'a> {
    s: &'a mut TcpStream,
    end: Instant,
}

impl Read for Until<'_> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let left = self.end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        self.s.set_read_timeout(Some(left))?;
        self.s.read(b)
    }
}

fn read_line_within(s: &mut TcpStream, limit: Duration) -> io::Result<String> {
    read_line(&mut Until {
        s,
        end: Instant::now() + limit,
    })
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
    random(&mut b)?;
    Ok(u128::from_ne_bytes(b))
}

#[cfg(unix)]
fn random(b: &mut [u8]) -> io::Result<()> {
    std::fs::File::open("/dev/urandom")?.read_exact(b)
}

#[cfg(windows)]
fn random(b: &mut [u8]) -> io::Result<()> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(
            algorithm: *mut std::ffi::c_void,
            buffer: *mut u8,
            len: u32,
            flags: u32,
        ) -> i32;
    }
    const USE_SYSTEM_PREFERRED_RNG: u32 = 2;
    let status = unsafe {
        BCryptGenRandom(
            std::ptr::null_mut(),
            b.as_mut_ptr(),
            b.len() as u32,
            USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::other("sem fonte de aleatoriedade"))
    }
}

fn broadcast(st: &State, room: &str, uid: u128, epoch: u64, msg: &str) -> bool {
    let rooms = st.rooms.lock().unwrap();
    let Some(r) = rooms.get(room) else {
        return false;
    };
    if r.get(&uid).is_none_or(|m| m.epoch != epoch) {
        return false;
    }
    for (_, m) in r.iter().filter(|(u, _)| **u != uid) {
        let _ = m.tx.try_send(msg.into());
    }
    true
}

fn writer(sock: Arc<TcpStream>) -> io::Result<mpsc::SyncSender<String>> {
    let (tx, rx) = mpsc::sync_channel::<String>(QUEUE);
    thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            for msg in rx {
                if (&*sock).write_all(msg.as_bytes()).is_err() {
                    let _ = sock.shutdown(Shutdown::Both);
                    break;
                }
            }
        })?;
    Ok(tx)
}

fn too_many_rooms(rooms: &HashMap<String, HashMap<u128, Member>>, ip: IpAddr, room: &str) -> bool {
    let mine = |r: &HashMap<u128, Member>| r.values().any(|m| m.ip == ip);
    !rooms.get(room).is_some_and(mine)
        && rooms.values().filter(|r| mine(r)).count() >= MAX_ROOMS_PER_IP
}

fn hello(
    st: &State,
    mut s: TcpStream,
    ip: IpAddr,
    room: String,
    uid: u128,
    key: u128,
) -> io::Result<()> {
    let sock = Arc::new(s.try_clone()?);
    let tx = writer(sock.clone())?;
    let epoch = st.epoch.fetch_add(1, Ordering::Relaxed);
    {
        let mut rooms = st.rooms.lock().unwrap();
        if !rooms.contains_key(&room) && rooms.len() >= MAX_ROOMS {
            return Err(bad("relay cheio"));
        }
        if too_many_rooms(&rooms, ip, &room) {
            return Err(bad("salas demais para este IP"));
        }
        if let Some(r) = rooms.get(&room) {
            match r.get(&uid) {
                Some(old) if old.key != key => return Err(bad("uid em uso")),
                None if r.len() >= MAX_ROOM => return Err(bad("sala cheia")),
                _ => {}
            }
        }
        tx.try_send("OK\n".into())
            .map_err(|_| bad("conexão fechada"))?;
        let member = Member {
            tx,
            sock,
            key,
            ip,
            epoch,
        };
        let pending = st.pending.lock().unwrap();
        let invites = pending
            .iter()
            .filter(|(_, p)| p.room == room && p.uid == uid);
        for (tok, _) in invites {
            let _ = member.tx.try_send(format!("INCOMING {tok:x}\n"));
        }
        drop(pending);
        if let Some(old) = rooms.entry(room.clone()).or_default().insert(uid, member) {
            let _ = old.sock.shutdown(Shutdown::Both);
        }
    }
    let res: io::Result<()> = loop {
        match read_line_within(&mut s, WAIT) {
            Ok(l) => {
                if let Some(t) = l.strip_prefix("BEACON ")
                    && !broadcast(st, &room, uid, epoch, &format!("BEACON {uid:x} {t}\n"))
                {
                    break Err(bad("substituído"));
                }
            }
            Err(e) => break Err(e),
        }
    };
    let mut rooms = st.rooms.lock().unwrap();
    if let Some(r) = rooms.get_mut(&room) {
        if r.get(&uid).is_some_and(|m| m.epoch == epoch) {
            r.remove(&uid);
            for m in r.values() {
                let _ = m.tx.try_send(format!("GONE {uid:x}\n"));
            }
        }
        if r.is_empty() {
            rooms.remove(&room);
        }
    }
    drop(rooms);
    res
}

fn dial(st: &State, mut s: TcpStream, room: &str, target: u128) -> io::Result<()> {
    let tok = token()?;
    let (tx, rx) = mpsc::channel();
    let sent = {
        let rooms = st.rooms.lock().unwrap();
        rooms.get(room).and_then(|r| r.get(&target)).map(|m| {
            st.pending.lock().unwrap().insert(
                tok,
                Pending {
                    room: room.into(),
                    uid: target,
                    tx,
                },
            );
            m.tx.try_send(format!("INCOMING {tok:x}\n")).is_ok()
        })
    };
    let other = sent
        .filter(|&ok| ok)
        .and_then(|_| rx.recv_timeout(PAIR).ok());
    st.pending.lock().unwrap().remove(&tok);
    let Some((mut b, _slot)) = other else {
        return s.write_all(b"ERR\n");
    };
    b.write_all(b"OK\n")?;
    s.write_all(b"OK\n")?;
    splice(s, b)
}

fn accept(
    st: &State,
    s: TcpStream,
    slot: Slot,
    room: &str,
    uid: u128,
    tok: u128,
) -> io::Result<()> {
    let p = {
        let mut pending = st.pending.lock().unwrap();
        match pending.get(&tok) {
            Some(p) if p.room == room && p.uid == uid => pending.remove(&tok),
            _ => None,
        }
    };
    let p = p.ok_or_else(|| bad("token"))?;
    let _ = p.tx.send((s, slot));
    Ok(())
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

fn handle(st: &State, mut s: TcpStream, mut slot: Slot) -> io::Result<()> {
    s.set_write_timeout(Some(WRITE))?;
    s.set_nodelay(true)?;
    let line = read_line_within(&mut s, FIRST_LINE)?;
    let tunnel = line.starts_with("DIAL ") || line.starts_with("ACCEPT ");
    let ip = slot.ip;
    if !st.allow(ip, tunnel) {
        return Err(bad("rate"));
    }
    let kind = if tunnel { Kind::Tunnel } else { Kind::Control };
    if !slot.become_kind(kind) {
        return Err(bad("conexões demais"));
    }
    match line.split(' ').collect::<Vec<_>>()[..] {
        ["HELLO", room, uid, key] => hello(st, s, ip, room_id(room)?, hex(uid)?, hex(key)?),
        ["DIAL", room, uid] => dial(st, s, &room_id(room)?, hex(uid)?),
        ["ACCEPT", room, uid, tok] => accept(st, s, slot, &room_id(room)?, hex(uid)?, hex(tok)?),
        _ => Err(bad("comando")),
    }
}

pub fn serve(l: TcpListener, st: Arc<State>) {
    for s in l.incoming().flatten() {
        let Ok(ip) = s.peer_addr().map(|a| a.ip()) else {
            continue;
        };
        let Some(slot) = Slot::open(&st, ip) else {
            continue;
        };
        let st = st.clone();
        let _ = thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                let _ = handle(&st, s, slot);
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
        let (s, ip) = l.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        s.shutdown(Shutdown::Write).unwrap();
        let st = State::default();
        assert!(hello(&st, s, ip.ip(), "a".repeat(64), 1, 1).is_err());
        assert!(
            st.rooms.lock().unwrap().is_empty(),
            "sobrou membro fantasma"
        );
    }

    fn start() -> (std::net::SocketAddr, Arc<State>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let st = Arc::new(State::default());
        let st2 = st.clone();
        thread::spawn(move || serve(l, st2));
        (addr, st)
    }

    fn refused(mut s: TcpStream) -> bool {
        read_line(&mut s).is_err()
    }

    #[test]
    fn a_uid_is_only_replaced_with_its_key() {
        let (addr, _) = start();
        let room = "a".repeat(64);
        let join = |uid, key| conn(addr, &format!("HELLO {room} {uid} {key}"));
        let mut watcher = join(9, 9);
        assert_eq!(read_line(&mut watcher).unwrap(), "OK");
        let mut old = join(1, 111);
        assert_eq!(read_line(&mut old).unwrap(), "OK");

        assert!(refused(join(1, 222)), "outra chave não toma o uid");
        writeln!(old, "BEACON sigo aqui").unwrap();
        assert_eq!(read_line(&mut watcher).unwrap(), "BEACON 1 sigo aqui");

        let mut new = join(1, 111);
        assert_eq!(read_line(&mut new).unwrap(), "OK");
        assert!(refused(old), "a sessão substituída é fechada");
        writeln!(new, "BEACON voltei").unwrap();
        assert_eq!(read_line(&mut watcher).unwrap(), "BEACON 1 voltei");
    }

    #[test]
    fn a_member_holds_its_slot_while_connected() {
        let (addr, st) = start();
        let mut a = conn(addr, &format!("HELLO {} 1 1", "a".repeat(64)));
        assert_eq!(read_line(&mut a).unwrap(), "OK");
        let held = |kind| {
            let c = st.conns.lock().unwrap();
            c.get(&(addr.ip(), kind)).copied().unwrap_or(0)
        };
        thread::sleep(Duration::from_millis(200));
        assert_eq!((held(Kind::Control), held(Kind::New)), (1, 0));
        drop(a);
        let end = Instant::now() + Duration::from_secs(3);
        while held(Kind::Control) != 0 {
            assert!(Instant::now() < end, "a vaga não voltou");
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_dripped_line_has_a_deadline() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (mut s, _) = l.accept().unwrap();
        let drip = thread::spawn(move || {
            for _ in 0..40 {
                if client.write_all(b"a").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        });
        let t0 = Instant::now();
        assert!(read_line_within(&mut s, Duration::from_millis(400)).is_err());
        assert!(t0.elapsed() < Duration::from_millis(1500));
        drop(s);
        drip.join().unwrap();
    }

    #[test]
    fn a_reconnecting_target_gets_the_pending_invite() {
        let (addr, _) = start();
        let room = "a".repeat(64);
        let mut old = conn(addr, &format!("HELLO {room} 2 7"));
        assert_eq!(read_line(&mut old).unwrap(), "OK");
        let mut d = conn(addr, &format!("DIAL {room} 2"));
        let invite = read_line(&mut old).unwrap();
        let mut new = conn(addr, &format!("HELLO {room} 2 7"));
        assert_eq!(read_line(&mut new).unwrap(), "OK");
        assert_eq!(read_line(&mut new).unwrap(), invite);
        let tok = invite.strip_prefix("INCOMING ").unwrap();
        let mut e = conn(addr, &format!("ACCEPT {room} 2 {tok}"));
        assert_eq!(read_line(&mut d).unwrap(), "OK");
        assert_eq!(read_line(&mut e).unwrap(), "OK");
    }

    #[test]
    fn a_wrong_accept_keeps_the_token() {
        let (addr, _) = start();
        let room = "a".repeat(64);
        let mut b = conn(addr, &format!("HELLO {room} 2 2"));
        assert_eq!(read_line(&mut b).unwrap(), "OK");
        let mut d = conn(addr, &format!("DIAL {room} 2"));
        let line = read_line(&mut b).unwrap();
        let tok = line.strip_prefix("INCOMING ").unwrap();
        assert!(refused(conn(addr, &format!("ACCEPT {room} 3 {tok}"))));
        let mut e = conn(addr, &format!("ACCEPT {room} 2 {tok}"));
        assert_eq!(read_line(&mut d).unwrap(), "OK");
        assert_eq!(read_line(&mut e).unwrap(), "OK");
    }

    #[test]
    fn connection_slots_are_limited() {
        let st = Arc::new(State::default());
        let ip = |n: u32| IpAddr::from(n.to_be_bytes());
        let mut held = vec![];
        for kind in [Kind::Control, Kind::Tunnel] {
            for _ in 0..kind.max_per_ip() {
                let mut s = Slot::open(&st, ip(1)).unwrap();
                assert!(s.become_kind(kind));
                held.push(s);
            }
            assert!(!Slot::open(&st, ip(1)).unwrap().become_kind(kind));
        }
        let mut n = 2;
        while let Some(s) = Slot::open(&st, ip(n)) {
            held.push(s);
            n += 1;
        }
        assert_eq!(held.len() as u32, MAX_CONNS);
        held.clear();
        assert!(st.conns.lock().unwrap().is_empty());
    }

    #[test]
    fn one_ip_cannot_take_every_room() {
        let (addr, _) = start();
        let join = |n: usize, uid| conn(addr, &format!("HELLO {n:064x} {uid} 1"));
        let mut held = vec![];
        for n in 0..MAX_ROOMS_PER_IP {
            let mut s = join(n, 1);
            assert_eq!(read_line(&mut s).unwrap(), "OK");
            held.push(s);
        }
        assert!(refused(join(MAX_ROOMS_PER_IP, 1)));
        assert_eq!(read_line(&mut join(0, 2)).unwrap(), "OK");
    }

    #[test]
    fn rate_table_has_a_ceiling() {
        let st = State::default();
        for n in 0..MAX_TRACKED as u32 {
            assert!(st.allow(IpAddr::from(n.to_be_bytes()), false));
        }
        assert!(!st.allow(IpAddr::from([255, 0, 0, 1]), false));
        assert!(
            st.allow(IpAddr::from([0, 0, 0, 0]), false),
            "quem já está segue"
        );
        assert_eq!(st.rate.lock().unwrap().len(), MAX_TRACKED);
    }

    #[test]
    fn rooms_isolate_and_tunnel() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        thread::spawn(move || serve(l, Arc::new(State::default())));
        let (ra, rb) = ("a".repeat(64), "b".repeat(64));

        let mut a = conn(addr, &format!("HELLO {ra} 1 1"));
        let mut b = conn(addr, &format!("HELLO {ra} 2 2"));
        let mut c = conn(addr, &format!("HELLO {rb} 3 3"));
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
