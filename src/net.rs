use crate::{
    clip,
    secure::{self, Secure},
};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Cursor, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{Sender, channel},
    },
    thread::{self, sleep},
    time::{Duration, Instant, UNIX_EPOCH},
};

pub const DISCOVERY_PORT: u16 = 47474;
pub const TRANSFER_PORT: u16 = 47475;
const MAX_TEXT: u64 = 1 << 20;
const MAX_FILES: usize = 100_000;
const PARTIAL: &str = ".pdrop-partial";

pub const VIS_ALL: u8 = 0;
pub const VIS_TRUSTED: u8 = 1;
pub const VIS_HIDDEN: u8 = 2;

#[derive(Default)]
pub struct Policy {
    pub trust: Mutex<HashMap<u128, String>>,
    pub vis: AtomicU8,
    pub key: [u8; 32],
    pub routes: Vec<(String, PathBuf)>,
}

#[derive(Clone)]
pub struct Who {
    pub name: String,
    pub key: [u8; 32],
}

#[derive(Clone)]
pub struct Peer {
    pub id: u64,
    pub name: String,
    pub addr: SocketAddr,
    pub seen: Instant,
    pub born: Instant,
    pub rtt: Option<f64>,
    pub color: u8,
    pub uid: u128,
}
pub type Peers = Arc<Mutex<HashMap<u64, Peer>>>;

#[derive(Clone)]
pub enum State {
    Waiting,
    Active,
    Done(String),
    Failed(String),
}

#[derive(Clone)]
pub struct Transfer {
    pub id: u64,
    pub peer: String,
    pub label: String,
    pub size: u64,
    pub done: u64,
    pub up: bool,
    pub state: State,
    pub kind: char,
    pub note: String,
    pub src: String,
    pub uid: u128,
    pub t0: Option<(Instant, u64)>,
}

pub type Grants = Arc<Mutex<HashMap<u128, String>>>;

pub enum Msg {
    BridgeOff,
    Auto(Transfer),
    Clip(String, clip::Clip),
    Ask(Transfer, Sender<bool>),
    Start(Transfer),
    Progress(u64, u64),
    End(u64, Result<String, String>),
}

fn next_id() -> u64 {
    static N: AtomicU64 = AtomicU64::new(1);
    N.fetch_add(1, Ordering::Relaxed)
}

pub fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(32).collect()
}

#[allow(clippy::too_many_arguments)]
pub fn discover(
    id: u64,
    uid: u128,
    name: Arc<Mutex<String>>,
    port: u16,
    color: Arc<AtomicU8>,
    policy: Arc<Policy>,
    peers: Peers,
    direct: Vec<IpAddr>,
) -> io::Result<()> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?;
    sock.set_broadcast(true)?;
    sock.bind(&SocketAddr::from(([0, 0, 0, 0], DISCOVERY_PORT)).into())?;
    let rx: UdpSocket = sock.into();
    let tx = rx.try_clone()?;
    let reply = rx.try_clone()?;
    let start = Instant::now();
    let pings = peers.clone();
    let beacon = {
        let (color, name, policy) = (color.clone(), name.clone(), policy.clone());
        move || {
            format!(
                "PDROP3|{id:x}|{port}|{}|{}|{uid:x}|{}",
                color.load(Ordering::Relaxed),
                policy.vis.load(Ordering::Relaxed),
                name.lock().unwrap()
            )
        }
    };
    let b2 = beacon.clone();
    let sweep_beacon = beacon.clone();
    let tx_policy = policy.clone();
    let sweep_policy = policy.clone();
    let sweep = UdpSocket::bind(("0.0.0.0", 0))?;
    thread::spawn(move || {
        loop {
            if sweep_policy.vis.load(Ordering::Relaxed) == VIS_HIDDEN {
                sleep(Duration::from_millis(500));
                continue;
            }
            let beacon = sweep_beacon();
            for ip in subnet_hosts() {
                let _ = sweep.send_to(beacon.as_bytes(), (ip, DISCOVERY_PORT));
                sleep(Duration::from_millis(2));
            }
            sleep(Duration::from_millis(1500));
        }
    });
    thread::spawn(move || {
        loop {
            if tx_policy.vis.load(Ordering::Relaxed) == VIS_HIDDEN {
                sleep(Duration::from_millis(500));
                continue;
            }
            let send = |ip: IpAddr, msg: &str| {
                let _ = tx.send_to(msg.as_bytes(), (ip, DISCOVERY_PORT));
            };
            let beacon = beacon();
            send(Ipv4Addr::BROADCAST.into(), &beacon);
            direct.iter().for_each(|&ip| send(ip, &beacon));
            let ping = format!("PING|{id:x}|{}", start.elapsed().as_micros());
            let known: Vec<IpAddr> = pings
                .lock()
                .unwrap()
                .values()
                .map(|p| p.addr.ip())
                .collect();
            for ip in known {
                send(ip, &beacon);
                send(ip, &ping);
            }
            sleep(Duration::from_millis(1500));
        }
    });
    thread::spawn(move || {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = rx.recv_from(&mut buf) {
            let Ok(s) = std::str::from_utf8(&buf[..n]) else {
                continue;
            };
            let hidden = policy.vis.load(Ordering::Relaxed) == VIS_HIDDEN;
            let f: Vec<&str> = s.splitn(7, '|').collect();
            match f.as_slice() {
                ["PING", _, us, ..] if !hidden => {
                    let _ = reply.send_to(
                        format!("PONG|{id:x}|{us}").as_bytes(),
                        (from.ip(), DISCOVERY_PORT),
                    );
                }
                ["PONG", rid, us, ..] => {
                    let (Ok(rid), Ok(us)) = (u64::from_str_radix(rid, 16), us.parse::<u128>())
                    else {
                        continue;
                    };
                    let ms = start.elapsed().as_micros().saturating_sub(us) as f64 / 1000.0;
                    if let Some(p) = peers.lock().unwrap().get_mut(&rid) {
                        p.rtt = Some(p.rtt.map_or(ms, |old| old * 0.7 + ms * 0.3));
                    }
                }
                ["PDROP3", pid, port, col, vis, uid, name] => {
                    let (Ok(pid), Ok(port), Ok(col), Ok(vis), Ok(puid)) = (
                        u64::from_str_radix(pid, 16),
                        port.parse::<u16>(),
                        col.parse::<u8>(),
                        vis.parse::<u8>(),
                        u128::from_str_radix(uid, 16),
                    ) else {
                        continue;
                    };
                    if pid == id {
                        continue;
                    }
                    if !shown(
                        vis,
                        policy.vis.load(Ordering::Relaxed),
                        policy.trust.lock().unwrap().contains_key(&puid),
                    ) {
                        continue;
                    }
                    let (now, addr) = (Instant::now(), SocketAddr::new(from.ip(), port));
                    let is_new = {
                        let mut g = peers.lock().unwrap();
                        match g.get_mut(&pid) {
                            Some(p) => {
                                (p.name, p.addr, p.seen, p.color, p.uid) =
                                    (clean(name), addr, now, col, puid);
                                false
                            }
                            None => {
                                g.insert(
                                    pid,
                                    Peer {
                                        id: pid,
                                        name: clean(name),
                                        addr,
                                        seen: now,
                                        born: now,
                                        rtt: None,
                                        color: col,
                                        uid: puid,
                                    },
                                );
                                true
                            }
                        }
                    };
                    if is_new && !hidden {
                        let _ = reply.send_to(b2().as_bytes(), (from.ip(), DISCOVERY_PORT));
                    }
                }
                _ => {}
            }
        }
    });
    Ok(())
}

fn shown(theirs: u8, mine: u8, trusted: bool) -> bool {
    trusted || (theirs != VIS_TRUSTED && mine != VIS_TRUSTED)
}

fn subnet_hosts() -> Vec<Ipv4Addr> {
    let mut v = vec![];
    for i in if_addrs::get_if_addrs().unwrap_or_default() {
        let loopback = i.is_loopback();
        let if_addrs::IfAddr::V4(a) = i.addr else {
            continue;
        };
        let (ip, mask) = (u32::from(a.ip), u32::from(a.netmask));
        if loopback || mask.leading_zeros() > 0 || mask.count_zeros() > 10 || mask == 0 {
            continue;
        }
        let net = ip & mask;
        v.extend(
            (1..!mask)
                .map(|h| Ipv4Addr::from(net | h))
                .filter(|&h| h != a.ip),
        );
    }
    v
}

pub fn listen(
    dir: PathBuf,
    tx: Sender<Msg>,
    grants: Grants,
    policy: Arc<Policy>,
) -> io::Result<u16> {
    let l = TcpListener::bind(("0.0.0.0", TRANSFER_PORT))
        .or_else(|_| TcpListener::bind(("0.0.0.0", 0)))?;
    let port = l.local_addr()?.port();
    sweep_partials(&dir);
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (tx, dir, grants, policy) =
                (tx.clone(), dir.clone(), grants.clone(), policy.clone());
            thread::spawn(move || {
                let _ = recv(s, &tx, &dir, &grants, &policy);
            });
        }
    });
    Ok(port)
}

fn sweep_partials(dir: &Path) {
    let old = Duration::from_secs(7 * 24 * 3600);
    for e in fs::read_dir(dir.join(PARTIAL))
        .into_iter()
        .flatten()
        .flatten()
    {
        if e.metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|a| a > old))
        {
            let _ = fs::remove_file(e.path());
        }
    }
}

fn bad() -> io::Error {
    io::ErrorKind::InvalidData.into()
}

fn eof(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, why.to_string())
}

struct HashRead<'a, R> {
    r: R,
    h: &'a mut blake3::Hasher,
}

impl<R: Read> Read for HashRead<'_, R> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        let n = self.r.read(b)?;
        self.h.update(&b[..n]);
        Ok(n)
    }
}

fn recv(
    s: TcpStream,
    tx: &Sender<Msg>,
    dir: &Path,
    grants: &Grants,
    policy: &Policy,
) -> io::Result<()> {
    let id = next_id();
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let sec = secure::handshake(s, &policy.key, false)?;
    let uid = sec.remote;
    let mut r = BufReader::new(sec);
    let mut line = String::new();
    (&mut r).take(4096).read_line(&mut line)?;
    let f: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    let &["PDROP3", kind, size, from, raw, extra] = f.as_slice() else {
        return Err(bad());
    };
    let kind = kind
        .chars()
        .next()
        .filter(|k| "ftcb".contains(*k))
        .ok_or_else(bad)?;
    let clip = matches!(kind, 'c' | 'b');
    let size = size.parse::<u64>().map_err(|_| bad())?;
    if (kind == 't' && size > MAX_TEXT) || (clip && size > clip::MAX) {
        return Err(bad());
    }
    let count = if kind == 'f' {
        extra
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=MAX_FILES).contains(n))
            .ok_or_else(bad)?
    } else {
        0
    };
    let mime = if extra.is_empty() || extra.len() >= 128 {
        clip::TEXT
    } else {
        extra
    }
    .to_string();
    if clip && !clip::available() {
        r.get_mut().write_all(b"N")?;
        return Ok(());
    }
    let name: String = match kind {
        'f' => raw.chars().filter(|c| !c.is_control()).take(80).collect(),
        't' => "mensagem".to_string(),
        _ => "clipboard".to_string(),
    };
    let name = if name.is_empty() {
        "arquivo".to_string()
    } else {
        name
    };
    let note = if clip {
        raw.chars().filter(|c| !c.is_control()).take(60).collect()
    } else {
        String::new()
    };
    let t_peer = clean(from);
    let mut t = Transfer {
        id,
        peer: t_peer.clone(),
        label: name,
        size,
        done: 0,
        up: false,
        state: State::Waiting,
        kind,
        note,
        src: String::new(),
        uid,
        t0: None,
    };
    let granted = kind == 'b' && grants.lock().unwrap().contains_key(&uid);
    let auto = kind == 't' && policy.trust.lock().unwrap().contains_key(&uid);
    let ok = granted || auto || {
        let (rtx, rrx) = channel();
        tx.send(Msg::Ask(t.clone(), rtx)).ok();
        rrx.recv().unwrap_or(false)
    };
    if ok && kind == 'b' && !granted {
        grants.lock().unwrap().insert(uid, t.peer.clone());
    }
    r.get_mut().write_all(if ok { b"Y" } else { b"N" })?;
    if !ok {
        return Ok(());
    }
    if auto {
        tx.send(Msg::Auto(t.clone())).ok();
    }
    r.get_ref()
        .tcp()
        .set_read_timeout(Some(Duration::from_secs(30)))?;
    t.state = State::Active;
    tx.send(Msg::Start(t)).ok();
    let res = (|| -> io::Result<String> {
        if kind == 'f' {
            return recv_files(
                &mut r,
                dir,
                &policy.routes,
                &t_peer,
                uid,
                count,
                size,
                id,
                tx,
            );
        }
        let mut buf = Vec::with_capacity(size as usize);
        if pump(&mut (&mut r).take(size), &mut buf, id, 0, size, tx)? != size {
            return Err(eof("conexão interrompida"));
        }
        if clip {
            let c = clip::Clip { mime, data: buf };
            tx.send(Msg::Clip(t_peer.clone(), c.clone())).ok();
            clip::set(&c)?;
            return Ok(format!("no clipboard · {}", preview(&c)));
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    })();
    tx.send(Msg::End(id, res.map_err(|e| e.to_string()))).ok();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn recv_files(
    r: &mut BufReader<Secure>,
    dir: &Path,
    routes: &[(String, PathBuf)],
    from: &str,
    uid: u128,
    n: usize,
    total: u64,
    id: u64,
    tx: &Sender<Msg>,
) -> io::Result<String> {
    let parts = dir.join(PARTIAL);
    fs::create_dir_all(&parts)?;
    let (mut roots, mut outs, mut done): (HashMap<String, PathBuf>, Vec<PathBuf>, u64) =
        Default::default();
    for _ in 0..n {
        let mut line = String::new();
        if (&mut *r).take(4096).read_line(&mut line)? == 0 {
            return Err(eof("conexão interrompida"));
        }
        let f: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
        let &[rel, size, mtime] = f.as_slice() else {
            return Err(bad());
        };
        // sufixo `z` no mtime = remetente oferece zstd (remetentes antigos nunca oferecem; receptores antigos tratam como texto opaco)
        let (mtime, zip) = mtime
            .strip_suffix('z')
            .map_or((mtime, false), |m| (m, true));
        let size = size.parse::<u64>().map_err(|_| bad())?;
        let comps = rel_components(rel).ok_or_else(bad)?;
        if done.saturating_add(size) > total {
            return Err(bad());
        }
        let part = parts.join(part_name(uid, rel, size, mtime));
        let _lock = PartLock::take(&part)?;
        let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let off = if have < size { have } else { 0 };
        writeln!(r.get_mut(), "{off}{}", if zip { "\tz" } else { "" })?;
        let mut h = blake3::Hasher::new();
        let mut file = if off > 0 {
            let mut f = OpenOptions::new().read(true).append(true).open(&part)?;
            io::copy(&mut (&mut f).take(off), &mut h)?;
            f
        } else {
            File::create(&part)?
        };
        let want = size - off;
        let got = if zip {
            // +1 limita bomba de descompressão e força ler o fim do quadro (EOF) antes do hash
            let dec = zstd::Decoder::with_buffer(&mut *r)?.single_frame();
            pump(
                &mut HashRead {
                    r: dec.take(want + 1),
                    h: &mut h,
                },
                &mut file,
                id,
                done + off,
                total,
                tx,
            )?
        } else {
            pump(
                &mut HashRead {
                    r: (&mut *r).take(want),
                    h: &mut h,
                },
                &mut file,
                id,
                done + off,
                total,
                tx,
            )?
        };
        if got != want {
            return Err(eof("conexão interrompida"));
        }
        let mut theirs = String::new();
        (&mut *r).take(128).read_line(&mut theirs)?;
        if theirs.trim() != h.finalize().to_hex().as_str() {
            let _ = fs::remove_file(&part);
            r.get_mut().write_all(b"E\n")?;
            return Err(io::Error::other(format!(
                "{rel}: arquivo corrompido na transferência"
            )));
        }
        drop(file);
        let dest = place(dir, routes, from, &mut roots, &mut outs, &comps)?;
        move_file(&part, &dest)?;
        r.get_mut().write_all(b"K\n")?;
        done += size;
    }
    Ok(outs
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n"))
}

static BUSY: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

struct PartLock(PathBuf);

impl PartLock {
    fn take(p: &Path) -> io::Result<Self> {
        let mut g = BUSY.lock().unwrap();
        if g.iter().any(|b| b == p) {
            return Err(io::Error::other(
                "esse arquivo já está sendo recebido em outra transferência",
            ));
        }
        g.push(p.to_path_buf());
        Ok(Self(p.to_path_buf()))
    }
}

impl Drop for PartLock {
    fn drop(&mut self) {
        BUSY.lock().unwrap().retain(|b| *b != self.0);
    }
}

fn part_name(uid: u128, rel: &str, size: u64, mtime: &str) -> String {
    format!(
        "{}.part",
        &blake3::hash(format!("{uid:x}\0{rel}\0{size}\0{mtime}").as_bytes()).to_hex()[..32]
    )
}

fn rel_components(rel: &str) -> Option<Vec<String>> {
    let v: Vec<String> = rel
        .split('/')
        .map(|c| {
            c.chars()
                .filter(|c| !c.is_control() && *c != '\\')
                .collect()
        })
        .collect();
    (!v.is_empty() && v.len() <= 32 && v.iter().all(|c| !matches!(c.as_str(), "" | "." | "..")))
        .then_some(v)
}

fn place(
    dir: &Path,
    routes: &[(String, PathBuf)],
    from: &str,
    roots: &mut HashMap<String, PathBuf>,
    outs: &mut Vec<PathBuf>,
    comps: &[String],
) -> io::Result<PathBuf> {
    if let [only] = comps {
        let (p, _) = create_unique(&route(routes, dir, from, Some(only)), only)?;
        outs.push(p.clone());
        return Ok(p);
    }
    if !roots.contains_key(&comps[0]) {
        let root = create_unique_dir(&route(routes, dir, from, None), &comps[0])?;
        outs.push(root.clone());
        roots.insert(comps[0].clone(), root);
    }
    let mut p = roots[&comps[0]].clone();
    p.extend(&comps[1..]);
    fs::create_dir_all(p.parent().unwrap())?;
    Ok(p)
}

fn route(routes: &[(String, PathBuf)], dir: &Path, from: &str, file: Option<&str>) -> PathBuf {
    let ext = file
        .and_then(|f| Path::new(f).extension())
        .map(|e| e.to_string_lossy().to_lowercase());
    let types: &[(&str, &[&str])] = &[
        (
            "imagem",
            &[
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "heic", "avif", "tiff",
            ],
        ),
        ("video", &["mp4", "mkv", "mov", "webm", "avi", "m4v"]),
        (
            "audio",
            &["mp3", "flac", "wav", "ogg", "m4a", "opus", "aac"],
        ),
        (
            "documento",
            &[
                "pdf", "doc", "docx", "odt", "txt", "md", "xls", "xlsx", "ods", "ppt", "pptx",
                "odp", "csv", "epub",
            ],
        ),
        (
            "compactado",
            &["zip", "tar", "gz", "xz", "bz2", "7z", "rar", "zst"],
        ),
    ];
    for (rule, to) in routes {
        let hit = match rule.split_once(':') {
            Some(("de", n)) => n.trim().eq_ignore_ascii_case(from),
            Some(("ext", e)) => {
                ext.as_deref() == Some(e.trim().trim_start_matches('.').to_lowercase().as_str())
            }
            Some(("tipo", t)) => types.iter().any(|(n, es)| {
                *n == t.trim().to_lowercase() && ext.as_deref().is_some_and(|e| es.contains(&e))
            }),
            _ => false,
        };
        if hit {
            return to.clone();
        }
    }
    dir.to_path_buf()
}

fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to).or_else(|_| fs::copy(from, to).and_then(|_| fs::remove_file(from)))
}

fn unique_name(name: &str, i: u32, dir_like: bool) -> String {
    if i == 0 {
        return name.to_string();
    }
    let p = Path::new(name);
    let (stem, ext) = if dir_like {
        (name.to_string(), String::new())
    } else {
        (
            p.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            p.extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default(),
        )
    };
    format!("{stem} ({i}){ext}")
}

fn create_unique(dir: &Path, name: &str) -> io::Result<(PathBuf, File)> {
    fs::create_dir_all(dir)?;
    for i in 0.. {
        let path = dir.join(unique_name(name, i, false));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn create_unique_dir(dir: &Path, name: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    for i in 0.. {
        let path = dir.join(unique_name(name, i, true));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn pump<R: Read + ?Sized, W: Write + ?Sized>(
    r: &mut R,
    w: &mut W,
    id: u64,
    base: u64,
    total: u64,
    tx: &Sender<Msg>,
) -> io::Result<u64> {
    let mut buf = vec![0u8; 1 << 16];
    let (mut done, mut last) = (0u64, Instant::now());
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        done += n as u64;
        if last.elapsed() > Duration::from_millis(50) {
            tx.send(Msg::Progress(id, (base + done).min(total))).ok();
            last = Instant::now();
        }
    }
    tx.send(Msg::Progress(id, (base + done).min(total))).ok();
    Ok(done)
}

struct Entry {
    rel: String,
    path: PathBuf,
    size: u64,
    mtime: u64,
}

enum Job {
    Text(Vec<u8>),
    Files(String, Vec<Entry>),
}

pub fn send(to: Peer, me: Who, inputs: Vec<String>, tx: Sender<Msg>) {
    let id = next_id();
    thread::spawn(move || {
        let mut t = Transfer {
            id,
            peer: to.name.clone(),
            label: String::new(),
            size: 0,
            done: 0,
            up: true,
            state: State::Waiting,
            kind: 'f',
            note: String::new(),
            src: inputs.join("\n"),
            uid: to.uid,
            t0: None,
        };
        match prepare(&inputs) {
            Err(e) => {
                t.label = inputs.join(" ").trim().chars().take(40).collect();
                t.state = State::Failed(e.to_string());
                tx.send(Msg::Start(t)).ok();
            }
            Ok(job) => {
                let (kind, size) = match &job {
                    Job::Text(b) => ('t', b.len() as u64),
                    Job::Files(label, es) => {
                        t.label = label.clone();
                        ('f', es.iter().map(|e| e.size).sum())
                    }
                };
                if kind == 't' {
                    t.label = "mensagem".into();
                }
                (t.size, t.kind) = (size, kind);
                tx.send(Msg::Start(t.clone())).ok();
                let r = match job {
                    Job::Text(b) => open(&to, &me, 't', size, "mensagem", "").and_then(|mut s| {
                        pump(&mut Cursor::new(b), &mut s, id, 0, size, &tx).map(|_| ())
                    }),
                    Job::Files(label, es) => {
                        open(&to, &me, 'f', size, &label, &es.len().to_string())
                            .and_then(|s| send_files(s, &es, id, size, &tx))
                    }
                };
                tx.send(Msg::End(
                    id,
                    r.map(|_| "enviado".to_string()).map_err(|e| e.to_string()),
                ))
                .ok();
            }
        }
    });
}

pub fn preview(c: &clip::Clip) -> String {
    if c.is_text() {
        String::from_utf8_lossy(&c.data[..c.data.len().min(512)])
            .chars()
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .take(60)
            .collect()
    } else {
        format!("{} ({})", c.mime, crate::ui::human(c.data.len() as u64))
    }
}

pub fn push_clip(
    to: &Peer,
    me: &Who,
    c: &clip::Clip,
    kind: char,
    tx: &Sender<Msg>,
) -> io::Result<()> {
    let id = next_id();
    let note = preview(c);
    let size = c.data.len() as u64;
    let t = Transfer {
        id,
        peer: to.name.clone(),
        label: "clipboard".into(),
        size,
        done: 0,
        up: true,
        state: State::Waiting,
        kind,
        note: note.clone(),
        src: note.clone(),
        uid: to.uid,
        t0: None,
    };
    tx.send(Msg::Start(t)).ok();
    let r = if size > clip::MAX {
        Err(io::Error::other("clipboard grande demais"))
    } else {
        open(to, me, kind, size, &note, &c.mime)
            .and_then(|mut s| pump(&mut Cursor::new(&c.data), &mut s, id, 0, size, tx).map(|_| ()))
    };
    tx.send(Msg::End(
        id,
        r.as_ref().map(|_| note.clone()).map_err(|e| e.to_string()),
    ))
    .ok();
    r
}

pub fn bridge(to: Peer, me: Who, tx: Sender<Msg>, stop: Arc<AtomicBool>) {
    thread::spawn(move || {
        clip::mark_current();
        while !stop.load(Ordering::Relaxed) {
            sleep(Duration::from_millis(400));
            let Some(cur) = clip::get() else { continue };
            if cur.data.is_empty() || !clip::changed(&cur) {
                continue;
            }
            if push_clip(&to, &me, &cur, 'b', &tx).is_err() {
                tx.send(Msg::BridgeOff).ok();
                break;
            }
        }
    });
}

fn prepare(inputs: &[String]) -> io::Result<Job> {
    let paths: Vec<PathBuf> = inputs.iter().map(|i| clean_path(i)).collect();
    if let ([input], [path]) = (inputs, paths.as_slice()) {
        if !path.exists() {
            let b = input.trim().as_bytes().to_vec();
            if b.len() as u64 > MAX_TEXT {
                return Err(io::Error::other("texto grande demais"));
            }
            return Ok(Job::Text(b));
        }
    }
    let mut es = vec![];
    for p in &paths {
        let p = fs::canonicalize(p)
            .map_err(|_| io::Error::other(format!("não achei {}", p.display())))?;
        let name = p
            .file_name()
            .map(|n| {
                n.to_string_lossy()
                    .replace(|c: char| c.is_control() || c == '/', "_")
            })
            .ok_or_else(|| io::Error::other("caminho sem nome"))?;
        walk(&p, name, &mut es)?;
    }
    if es.is_empty() {
        return Err(io::Error::other("nada para enviar (pasta vazia?)"));
    }
    let label = match (paths.as_slice(), es.as_slice()) {
        ([p], [e]) if p.is_file() => e.rel.clone(),
        ([p], es) if p.is_dir() => format!(
            "{}/ · {} arquivos",
            es[0].rel.split('/').next().unwrap_or(""),
            es.len()
        ),
        _ => format!("{} arquivos", es.len()),
    };
    Ok(Job::Files(label, es))
}

fn walk(p: &Path, rel: String, out: &mut Vec<Entry>) -> io::Result<()> {
    let m = fs::symlink_metadata(p)?;
    if m.is_dir() {
        for e in fs::read_dir(p)? {
            let e = e?;
            let n = e
                .file_name()
                .to_string_lossy()
                .replace(|c: char| c.is_control() || c == '/', "_");
            walk(&e.path(), format!("{rel}/{n}"), out)?;
        }
    } else if m.is_file() {
        if out.len() >= MAX_FILES {
            return Err(io::Error::other(
                "arquivos demais (limite de 100 mil por envio)",
            ));
        }
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        out.push(Entry {
            rel,
            path: p.to_path_buf(),
            size: m.len(),
            mtime,
        });
    }
    Ok(())
}

fn open(to: &Peer, me: &Who, kind: char, size: u64, name: &str, extra: &str) -> io::Result<Secure> {
    let tcp = TcpStream::connect_timeout(&to.addr, Duration::from_secs(5)).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("não conectou ({e}); confira o firewall dos dois lados (TCP {TRANSFER_PORT})"),
        )
    })?;
    tcp.set_read_timeout(Some(Duration::from_secs(60)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut s = secure::handshake(tcp, &me.key, true)?;
    if s.remote != to.uid {
        return Err(io::Error::other(
            "a identidade desse aparelho mudou ou não confere: recusei por segurança",
        ));
    }
    let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
    writeln!(
        s,
        "PDROP3\t{kind}\t{size}\t{}\t{}\t{}",
        clean(&me.name),
        clean(name),
        clean(extra)
    )?;
    let mut ans = [0u8; 1];
    s.read_exact(&mut ans)?;
    if ans[0] != b'Y' {
        return Err(io::Error::other("recusado"));
    }
    Ok(s)
}

/// só vale comprimir o que ainda não está comprimido
fn compressible(rel: &str) -> bool {
    const SKIP: &[&str] = &[
        "jpg", "jpeg", "png", "gif", "webp", "heic", "avif", "mp4", "mkv", "mov", "webm", "avi",
        "m4v", "mp3", "flac", "ogg", "m4a", "opus", "aac", "zip", "gz", "tgz", "bz2", "xz", "zst",
        "7z", "rar", "apk", "jar", "docx", "xlsx", "pptx",
    ];
    Path::new(rel)
        .extension()
        .is_none_or(|e| !SKIP.contains(&e.to_string_lossy().to_lowercase().as_str()))
}

fn send_files(s: Secure, es: &[Entry], id: u64, total: u64, tx: &Sender<Msg>) -> io::Result<()> {
    let mut rd = BufReader::new(s);
    let mut line = String::new();
    let mut done = 0u64;
    for e in es {
        let offer = if compressible(&e.rel) { "z" } else { "" };
        writeln!(rd.get_mut(), "{}\t{}\t{}{offer}", e.rel, e.size, e.mtime)?;
        line.clear();
        rd.read_line(&mut line)?;
        let (off, zip) = line
            .trim()
            .split_once('\t')
            .map_or((line.trim(), false), |(o, z)| {
                (o, z == "z" && !offer.is_empty())
            });
        let off = off
            .parse::<u64>()
            .ok()
            .filter(|&o| o <= e.size)
            .ok_or_else(bad)?;
        let mut f = File::open(&e.path)?;
        let mut h = blake3::Hasher::new();
        io::copy(&mut (&mut f).take(off), &mut h)?;
        let want = e.size - off;
        let src = &mut HashRead {
            r: (&mut f).take(want),
            h: &mut h,
        };
        let sent = if zip {
            let mut enc = zstd::Encoder::new(rd.get_mut(), 3)?;
            let n = pump(src, &mut enc, id, done + off, total, tx)?;
            enc.finish()?;
            n
        } else {
            pump(src, rd.get_mut(), id, done + off, total, tx)?
        };
        if sent != want {
            return Err(io::Error::other(format!("{} mudou durante o envio", e.rel)));
        }
        writeln!(rd.get_mut(), "{}", h.finalize().to_hex())?;
        line.clear();
        rd.read_line(&mut line)?;
        if line.trim() != "K" {
            return Err(io::Error::other(format!(
                "{}: o destinatário recebeu corrompido",
                e.rel
            )));
        }
        done += e.size;
    }
    Ok(())
}

fn clean_path(s: &str) -> PathBuf {
    let s = s.trim().trim_matches(|c| c == '\'' || c == '"');
    let s = s
        .strip_prefix("file://")
        .unwrap_or(s)
        .replace("%20", " ")
        .replace("\\ ", " ");
    match s.strip_prefix("~/") {
        Some(rest) => crate::home().join(rest),
        None => s.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(port: u16, uid: u128) -> Peer {
        Peer {
            id: 1,
            name: "t".into(),
            addr: ([127, 0, 0, 1], port).into(),
            seen: Instant::now(),
            born: Instant::now(),
            rtt: None,
            color: 0,
            uid,
        }
    }

    fn drain(rx: &std::sync::mpsc::Receiver<Msg>, n: usize) -> Vec<Result<String, String>> {
        let mut out = vec![];
        while out.len() < 2 * n {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Msg::Ask(_, r) => r.send(true).unwrap(),
                Msg::End(_, r) => out.push(r),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn loopback_file_text_folder_resume() {
        let dir = std::env::temp_dir().join(format!("pdrop-test-{}", std::process::id()));
        let tree = dir.join("tree");
        fs::create_dir_all(tree.join("sub")).unwrap();
        fs::write(dir.join("src.txt"), "olá").unwrap();
        fs::write(tree.join("a.txt"), "aa").unwrap();
        fs::write(tree.join("sub/b.txt"), "bb").unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(dir.join("big.bin"), &big).unwrap();
        let (tx, rx) = channel();
        let ((rk, rp), (sk, sp)) = (secure::generate(), secure::generate());
        let policy = Arc::new(Policy {
            key: rk,
            routes: vec![("ext:bin".into(), dir.join("bins"))],
            ..Default::default()
        });
        let port = listen(dir.clone(), tx.clone(), Grants::default(), policy.clone()).unwrap();
        let me = Who {
            name: "eu".into(),
            key: sk,
        };
        let (ruid, suid) = (secure::id_of(&rp), secure::id_of(&sp));
        let s = |v: &[&str]| {
            send(
                peer(port, ruid),
                me.clone(),
                v.iter().map(|s| s.to_string()).collect(),
                tx.clone(),
            )
        };

        s(&[&dir.join("src.txt").display().to_string()]);
        s(&["oi mundo"]);
        s(&[&tree.display().to_string()]);
        let mut res = drain(&rx, 3);
        s(&[
            &dir.join("src.txt").display().to_string(),
            &dir.join("big.bin").display().to_string(),
        ]);
        res.extend(drain(&rx, 1));
        assert!(res.iter().all(|r| r.is_ok()), "{res:?}");
        assert!(res.iter().any(|r| r.as_deref() == Ok("oi mundo")));
        assert_eq!(fs::read_to_string(dir.join("src (1).txt")).unwrap(), "olá");
        assert_eq!(fs::read_to_string(dir.join("src (2).txt")).unwrap(), "olá");
        assert_eq!(
            fs::read_to_string(dir.join("tree (1)/sub/b.txt")).unwrap(),
            "bb"
        );
        assert_eq!(fs::read(dir.join("bins/big.bin")).unwrap(), big);
        assert!(
            fs::read_dir(dir.join(PARTIAL)).unwrap().next().is_none(),
            "sobrou pedaço"
        );

        let m = fs::metadata(dir.join("big.bin")).unwrap();
        let mtime = m
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let part = dir
            .join(PARTIAL)
            .join(part_name(suid, "big.bin", m.len(), &mtime));
        fs::write(&part, vec![0u8; 50_000]).unwrap();
        s(&[&dir.join("big.bin").display().to_string()]);
        let res = drain(&rx, 1);
        assert!(
            res.iter().all(|r| r.is_err())
                && res
                    .iter()
                    .any(|r| r.as_ref().unwrap_err().contains("corromp")),
            "{res:?}"
        );
        assert!(!part.exists());
        fs::write(&part, &big[..50_000]).unwrap();
        s(&[&dir.join("big.bin").display().to_string()]);
        assert!(drain(&rx, 1).iter().all(|r| r.is_ok()));
        assert_eq!(fs::read(dir.join("bins/big (1).bin")).unwrap(), big);

        send(
            peer(port, ruid ^ 1),
            me.clone(),
            vec!["oi".into()],
            tx.clone(),
        );
        loop {
            if let Msg::End(_, r) = rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                assert!(r.unwrap_err().contains("identidade"));
                break;
            }
        }

        assert_eq!(rel_components("../x"), None);
        assert_eq!(rel_components("a//b"), None);
        assert_eq!(rel_components("a/b").unwrap(), ["a", "b"]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn routing() {
        let r = vec![
            ("de:Ana".to_string(), PathBuf::from("/ana")),
            ("tipo:imagem".into(), "/pics".into()),
            ("ext:PDF".into(), "/docs".into()),
        ];
        let d = Path::new("/dl");
        assert_eq!(route(&r, d, "ana", Some("a.png")), Path::new("/ana"));
        assert_eq!(route(&r, d, "bob", Some("A.JPG")), Path::new("/pics"));
        assert_eq!(route(&r, d, "bob", Some("x.pdf")), Path::new("/docs"));
        assert_eq!(route(&r, d, "bob", Some("x.rs")), d);
        assert_eq!(route(&r, d, "bob", None), d);
    }

    #[test]
    fn visibility_rule() {
        assert!(shown(VIS_ALL, VIS_ALL, false));
        assert!(!shown(VIS_TRUSTED, VIS_ALL, false));
        assert!(shown(VIS_TRUSTED, VIS_ALL, true));
        assert!(!shown(VIS_ALL, VIS_TRUSTED, false));
        assert!(shown(VIS_ALL, VIS_TRUSTED, true));
    }
}
