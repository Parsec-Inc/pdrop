//! Descoberta (UDP broadcast) e transferência (TCP) do pdrop.
//! Protocolo: o remetente manda "PDROP1\t<f|t>\t<bytes>\t<remetente>\t<nome>\n",
//! o destinatário responde 'Y' ou 'N' (como o aceitar/recusar do AirDrop), depois vêm os bytes.
use crate::clip;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Cursor, Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        mpsc::{Sender, channel},
        Arc, Mutex,
    },
    thread::{self, sleep},
    time::{Duration, Instant},
};

const DISCOVERY_PORT: u16 = 47474;
const TRANSFER_PORT: u16 = 47475;
const MAX_TEXT: u64 = 1 << 20;

#[derive(Clone)]
pub struct Peer {
    pub id: u64,
    pub name: String,
    pub addr: SocketAddr,
    pub seen: Instant,
    pub born: Instant,
    /// RTT suavizado em ms, medido por PING/PONG; None até o primeiro PONG.
    pub rtt: Option<f64>,
    /// índice na paleta (ui::COLORS) que o peer escolheu
    pub color: u8,
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
    /// f arquivo, t texto, c clipboard (uma vez), b ponte de clipboard
    pub kind: char,
    /// prévia do texto copiado (só clipboard)
    pub note: String,
}

/// IPs que permitiram a ponte de clipboard nesta sessão (valor: nome para mostrar).
pub type Grants = Arc<Mutex<HashMap<IpAddr, String>>>;

pub enum Msg {
    BridgeOff,
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

// ---------- descoberta ----------

// ponytail: só broadcast 255.255.255.255 (interface da rota padrão); mDNS/multi-interface se precisar.
// `direct`: IPs para onde o beacon também vai em unicast (redes que filtram broadcast).
pub fn discover(id: u64, name: Arc<Mutex<String>>, port: u16, color: Arc<AtomicU8>, peers: Peers, direct: Vec<IpAddr>) -> io::Result<()> {
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    #[cfg(unix)]
    sock.set_reuse_port(true)?; // permite várias instâncias na mesma máquina
    sock.set_broadcast(true)?;
    sock.bind(&SocketAddr::from(([0, 0, 0, 0], DISCOVERY_PORT)).into())?;
    let rx: UdpSocket = sock.into();
    let tx = rx.try_clone()?;
    let reply = rx.try_clone()?;
    let start = Instant::now();
    let pings = peers.clone();
    // o beacon é remontado a cada uso para refletir a cor atual
    let beacon = {
        let (color, name) = (color.clone(), name.clone());
        move || format!("PDROP1|{id:x}|{port}|{}|{}", color.load(Ordering::Relaxed), name.lock().unwrap())
    };
    let b2 = beacon.clone();
    thread::spawn(move || loop {
        let send = |ip: IpAddr, msg: &str| {
            let _ = tx.send_to(msg.as_bytes(), (ip, DISCOVERY_PORT));
        };
        let beacon = beacon();
        send(Ipv4Addr::BROADCAST.into(), &beacon);
        direct.iter().for_each(|&ip| send(ip, &beacon));
        for ip in subnet_hosts() {
            send(ip.into(), &beacon);
            sleep(Duration::from_millis(2)); // não despeja 500 pacotes de uma vez
        }
        // latência até quem já conhecemos: vira a distância no radar
        let ping = format!("PING|{id:x}|{}", start.elapsed().as_micros());
        let known: Vec<IpAddr> = pings.lock().unwrap().values().map(|p| p.addr.ip()).collect();
        known.into_iter().for_each(|ip| send(ip, &ping));
        sleep(Duration::from_millis(1500));
    });
    thread::spawn(move || {
        let mut buf = [0u8; 512];
        while let Ok((n, from)) = rx.recv_from(&mut buf) {
            let Ok(s) = std::str::from_utf8(&buf[..n]) else { continue };
            let mut p = s.splitn(5, '|');
            match (p.next(), p.next(), p.next(), p.next(), p.next()) {
                (Some("PING"), Some(_), Some(us), _, _) => {
                    let _ = reply.send_to(format!("PONG|{id:x}|{us}").as_bytes(), (from.ip(), DISCOVERY_PORT));
                }
                (Some("PONG"), Some(rid), Some(us), _, _) => {
                    let (Ok(rid), Ok(us)) = (u64::from_str_radix(rid, 16), us.parse::<u128>()) else { continue };
                    let ms = start.elapsed().as_micros().saturating_sub(us) as f64 / 1000.0;
                    if let Some(p) = peers.lock().unwrap().get_mut(&rid) {
                        p.rtt = Some(p.rtt.map_or(ms, |old| old * 0.7 + ms * 0.3));
                    }
                }
                (Some("PDROP1"), Some(pid), Some(port), Some(col), Some(name)) => {
                    let (Ok(pid), Ok(port), Ok(col)) = (u64::from_str_radix(pid, 16), port.parse::<u16>(), col.parse::<u8>()) else { continue };
                    if pid == id {
                        continue;
                    }
                    let (now, addr) = (Instant::now(), SocketAddr::new(from.ip(), port));
                    let mut g = peers.lock().unwrap();
                    match g.get_mut(&pid) {
                        Some(p) => (p.name, p.addr, p.seen, p.color) = (clean(name), addr, now, col),
                        None => {
                            g.insert(pid, Peer { id: pid, name: clean(name), addr, seen: now, born: now, rtt: None, color: col });
                            // peer novo: responde em unicast, assim basta um lado conhecer o IP do outro
                            let _ = reply.send_to(b2().as_bytes(), (from.ip(), DISCOVERY_PORT));
                        }
                    }
                }
                _ => {}
            }
        }
    });
    Ok(())
}

/// Todos os IPs das sub-redes locais. Redes de Wi-Fi costumam filtrar broadcast mas deixam passar
/// unicast, então varrer a sub-rede é o que faz a descoberta funcionar sozinha.
// ponytail: só sub-redes até /22 (1022 hosts); redes maiores contam com o broadcast.
fn subnet_hosts() -> Vec<Ipv4Addr> {
    let mut v = vec![];
    for i in if_addrs::get_if_addrs().unwrap_or_default() {
        let loopback = i.is_loopback();
        let if_addrs::IfAddr::V4(a) = i.addr else { continue };
        let (ip, mask) = (u32::from(a.ip), u32::from(a.netmask));
        if loopback || mask.leading_zeros() > 0 || mask.count_zeros() > 10 || mask == 0 {
            continue;
        }
        let net = ip & mask;
        v.extend((1..!mask).map(|h| Ipv4Addr::from(net | h)).filter(|&h| h != a.ip));
    }
    v
}

// ---------- receber ----------

pub fn listen(dir: PathBuf, tx: Sender<Msg>, grants: Grants) -> io::Result<u16> {
    let l = TcpListener::bind(("0.0.0.0", TRANSFER_PORT)).or_else(|_| TcpListener::bind(("0.0.0.0", 0)))?;
    let port = l.local_addr()?.port();
    thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (tx, dir, grants) = (tx.clone(), dir.clone(), grants.clone());
            thread::spawn(move || {
                let _ = recv(s, &tx, &dir, &grants);
            });
        }
    });
    Ok(port)
}

fn bad() -> io::Error {
    io::ErrorKind::InvalidData.into()
}

fn recv(s: TcpStream, tx: &Sender<Msg>, dir: &Path, grants: &Grants) -> io::Result<()> {
    let id = next_id();
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    (&mut r).take(4096).read_line(&mut line)?;
    let f: Vec<&str> = line.trim_end().split('\t').collect();
    // 6º campo (opcional): tipo MIME do clipboard; sem ele, é texto
    let (kind, size, from, raw, mime) = match f.as_slice() {
        &["PDROP1", k, s, f, r] => (k, s, f, r, clip::TEXT),
        &["PDROP1", k, s, f, r, m] if !m.is_empty() && m.len() < 128 => (k, s, f, r, m),
        _ => return Err(bad()),
    };
    let kind = kind.chars().next().filter(|k| "ftcb".contains(*k)).ok_or_else(bad)?;
    let (text, clip) = (kind != 'f', matches!(kind, 'c' | 'b'));
    let size = size.parse::<u64>().map_err(|_| bad())?;
    if (kind == 't' && size > MAX_TEXT) || (clip && size > clip::MAX) {
        return Err(bad());
    }
    let mime = mime.to_string();
    if clip && !clip::available() {
        r.get_mut().write_all(b"N")?;
        return Ok(());
    }
    let name = match kind {
        'f' => safe_name(raw),
        't' => "mensagem".to_string(),
        _ => "clipboard".to_string(),
    };
    let note = if clip { raw.chars().filter(|c| !c.is_control()).take(60).collect() } else { String::new() };
    let ip = r.get_ref().peer_addr()?.ip();
    let mut t = Transfer { id, peer: clean(from), label: name.clone(), size, done: 0, up: false, state: State::Waiting, kind, note };
    // ponte já permitida nesta sessão: as cópias seguintes entram sem perguntar
    let granted = kind == 'b' && grants.lock().unwrap().contains_key(&ip);
    let ok = granted || {
        let (rtx, rrx) = channel();
        tx.send(Msg::Ask(t.clone(), rtx)).ok();
        rrx.recv().unwrap_or(false)
    };
    if ok && kind == 'b' && !granted {
        grants.lock().unwrap().insert(ip, t.peer.clone());
    }
    r.get_mut().write_all(if ok { b"Y" } else { b"N" })?;
    if !ok {
        return Ok(());
    }
    r.get_ref().set_read_timeout(Some(Duration::from_secs(30)))?;
    t.state = State::Active;
    tx.send(Msg::Start(t)).ok();
    let res = (|| -> io::Result<String> {
        let mut src = (&mut r).take(size);
        if text {
            let mut buf = Vec::with_capacity(size as usize);
            pump(&mut src, &mut buf, id, size, tx)?;
            if clip {
                let c = clip::Clip { mime, data: buf };
                clip::set(&c)?;
                return Ok(format!("no clipboard · {}", preview(&c)));
            }
            return Ok(String::from_utf8_lossy(&buf).into_owned());
        }
        let (path, mut f) = create_unique(dir, &name)?;
        match pump(&mut src, &mut f, id, size, tx) {
            Ok(()) => Ok(path.display().to_string()),
            Err(e) => {
                let _ = fs::remove_file(&path);
                Err(e)
            }
        }
    })();
    tx.send(Msg::End(id, res.map_err(|e| e.to_string()))).ok();
    Ok(())
}

/// Nome vindo da rede nunca é confiável: só o último componente, sem controles.
fn safe_name(s: &str) -> String {
    let n = Path::new(s).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let n: String = n.chars().filter(|c| !c.is_control()).collect();
    if n.is_empty() { "arquivo".into() } else { n }
}

fn create_unique(dir: &Path, name: &str) -> io::Result<(PathBuf, File)> {
    fs::create_dir_all(dir)?;
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    for i in 0.. {
        let path = dir.join(if i == 0 { name.to_string() } else { format!("{stem} ({i}){ext}") });
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn pump<R: Read + ?Sized, W: Write + ?Sized>(r: &mut R, w: &mut W, id: u64, size: u64, tx: &Sender<Msg>) -> io::Result<()> {
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
            tx.send(Msg::Progress(id, done)).ok();
            last = Instant::now();
        }
    }
    if done != size {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "conexão interrompida"));
    }
    tx.send(Msg::Progress(id, done)).ok();
    Ok(())
}

// ---------- enviar ----------

/// `input` é um caminho de arquivo (aceita ~, aspas, file://) ou, se não existir, texto livre.
pub fn send(to: Peer, me: String, input: String, tx: Sender<Msg>) {
    let id = next_id();
    thread::spawn(move || {
        let mut t = Transfer { id, peer: to.name.clone(), label: String::new(), size: 0, done: 0, up: true, state: State::Waiting, kind: 'f', note: String::new() };
        match prepare(&input) {
            Err(e) => {
                t.label = input.trim().chars().take(40).collect();
                t.state = State::Failed(e.to_string());
                tx.send(Msg::Start(t)).ok();
            }
            Ok((kind, name, size, mut src)) => {
                (t.label, t.size, t.kind) = (name.clone(), size, kind);
                tx.send(Msg::Start(t)).ok();
                let r = go(id, &to, &me, kind, &name, None, size, &mut *src, &tx);
                tx.send(Msg::End(id, r.map_err(|e| e.to_string()))).ok();
            }
        }
    });
}

/// Resumo de uma linha do que está no clipboard: começo do texto, ou tipo e tamanho.
pub fn preview(c: &clip::Clip) -> String {
    if c.is_text() {
        String::from_utf8_lossy(&c.data[..c.data.len().min(512)]).chars().map(|ch| if ch.is_control() { ' ' } else { ch }).take(60).collect()
    } else {
        format!("{} ({})", c.mime, crate::ui::human(c.data.len() as u64))
    }
}

/// Envia o conteúdo inteiro do clipboard, com o tipo, ao clipboard do peer (kind 'c' uma vez, 'b' ponte). Bloqueia; emite Start/End.
pub fn push_clip(to: &Peer, me: &str, c: &clip::Clip, kind: char, tx: &Sender<Msg>) -> io::Result<()> {
    let id = next_id();
    let note = preview(c);
    let size = c.data.len() as u64;
    let t = Transfer { id, peer: to.name.clone(), label: "clipboard".into(), size, done: 0, up: true, state: State::Waiting, kind, note: note.clone() };
    tx.send(Msg::Start(t)).ok();
    let r = if size > clip::MAX {
        Err(io::Error::other("clipboard grande demais"))
    } else {
        go(id, to, me, kind, &note, Some(&c.mime), size, &mut Cursor::new(&c.data), tx)
    };
    tx.send(Msg::End(id, r.as_ref().map(|_| note.clone()).map_err(|e| e.to_string()))).ok();
    r.map(|_| ())
}

/// Ponte contínua: a cada cópia nova, empurra para o peer. Para sozinha se o peer recusar ou cair.
// ponytail: confere o clipboard a cada 400ms (processo por leitura); eventos nativos se pesar.
pub fn bridge(to: Peer, me: String, tx: Sender<Msg>, stop: Arc<AtomicBool>) {
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

type Prepared = (char, String, u64, Box<dyn Read + Send>);

fn prepare(input: &str) -> io::Result<Prepared> {
    let path = clean_path(input);
    if path.is_file() {
        let f = File::open(&path)?;
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        Ok(('f', name, f.metadata()?.len(), Box::new(f)))
    } else if path.is_dir() {
        Err(io::Error::other("pastas ainda não são suportadas"))
    } else {
        let b = input.trim().as_bytes().to_vec();
        if b.len() as u64 > MAX_TEXT {
            return Err(io::Error::other("texto grande demais"));
        }
        Ok(('t', "mensagem".into(), b.len() as u64, Box::new(Cursor::new(b))))
    }
}

#[allow(clippy::too_many_arguments)]
fn go(id: u64, to: &Peer, me: &str, kind: char, name: &str, mime: Option<&str>, size: u64, src: &mut dyn Read, tx: &Sender<Msg>) -> io::Result<String> {
    let mut s = TcpStream::connect_timeout(&to.addr, Duration::from_secs(5))?;
    s.set_read_timeout(Some(Duration::from_secs(60)))?;
    s.set_write_timeout(Some(Duration::from_secs(30)))?;
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    match mime.filter(|m| !m.contains(['\t', '\n'])) {
        Some(m) => writeln!(s, "PDROP1\t{kind}\t{size}\t{me}\t{name}\t{m}")?,
        None => writeln!(s, "PDROP1\t{kind}\t{size}\t{me}\t{name}")?,
    }
    let mut ans = [0u8; 1];
    s.read_exact(&mut ans)?;
    if ans[0] != b'Y' {
        return Err(io::Error::other("recusado"));
    }
    pump(src, &mut s, id, size, tx)?;
    Ok("enviado".into())
}

fn clean_path(s: &str) -> PathBuf {
    let s = s.trim().trim_matches(|c| c == '\'' || c == '"');
    let s = s.strip_prefix("file://").unwrap_or(s).replace("%20", " ").replace("\\ ", " ");
    match s.strip_prefix("~/") {
        Some(rest) => crate::home().join(rest),
        None => s.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_file_and_text() {
        let dir = std::env::temp_dir().join(format!("pdrop-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.txt");
        fs::write(&src, "olá").unwrap();
        let (tx, rx) = channel();
        let port = listen(dir.clone(), tx.clone(), Grants::default()).unwrap();
        let peer = Peer { id: 1, name: "t".into(), addr: ([127, 0, 0, 1], port).into(), seen: Instant::now(), born: Instant::now(), rtt: None, color: 0 };
        send(peer.clone(), "eu".into(), src.display().to_string(), tx.clone());
        send(peer, "eu".into(), "oi mundo".into(), tx);
        let (mut ends, mut text) = (0, false);
        while ends < 4 {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                Msg::Ask(_, r) => r.send(true).unwrap(),
                Msg::End(_, r) => {
                    ends += 1;
                    text |= r.unwrap() == "oi mundo";
                }
                _ => {}
            }
        }
        assert!(text);
        // o destino é a mesma pasta da origem: prova também o "(1)" anti-sobrescrita
        assert_eq!(fs::read_to_string(dir.join("src (1).txt")).unwrap(), "olá");
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        fs::remove_dir_all(dir).ok();
    }
}
