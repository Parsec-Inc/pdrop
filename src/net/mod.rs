mod discovery;
mod recv;
mod send;

pub use discovery::{discover, shown, upsert};
pub use recv::{listen, recv};
pub use send::{bridge, push_clip, send};

use crate::{clip, relay};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicU64, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
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
pub enum Route {
    Lan(SocketAddr),
    Relay(Arc<relay::Config>),
}

impl Route {
    pub fn lan_ip(&self) -> Option<IpAddr> {
        match self {
            Route::Lan(a) => Some(a.ip()),
            Route::Relay(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct Peer {
    pub id: u64,
    pub name: String,
    pub route: Route,
    pub fallback: Option<Arc<relay::Config>>,
    pub seen: Instant,
    pub born: Instant,
    pub rtt: Option<f64>,
    pub color: u8,
    pub uid: u128,
}

impl Peer {
    pub fn via_relay(&self) -> bool {
        matches!(self.route, Route::Relay(_))
    }
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
    Reply(u64, bool),
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

pub fn preview(c: &clip::Clip) -> String {
    if c.is_text() {
        String::from_utf8_lossy(&c.data[..c.data.len().min(512)])
            .chars()
            .map(|ch| if ch.is_control() { ' ' } else { ch })
            .take(60)
            .collect()
    } else {
        format!("{} ({})", c.mime, crate::fmt::human(c.data.len() as u64))
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

#[cfg(test)]
mod tests {
    use super::{
        recv::{part_name, rel_components},
        *,
    };
    use crate::secure;
    use std::{fs, sync::mpsc::channel, time::UNIX_EPOCH};

    fn peer(port: u16, uid: u128) -> Peer {
        Peer {
            id: 1,
            name: "t".into(),
            route: Route::Lan(([127, 0, 0, 1], port).into()),
            fallback: None,
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
}
