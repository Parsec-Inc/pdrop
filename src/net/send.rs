use super::{
    HashRead, MAX_FILES, MAX_TEXT, Msg, Peer, Route, State, TRANSFER_PORT, Transfer, Who, bad,
    next_id, preview, pump,
};
use crate::{
    clip, relay,
    secure::{self, Secure},
};
use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, Cursor, Read, Write},
    net::TcpStream,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread::{self, sleep},
    time::{Duration, UNIX_EPOCH},
};

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
                    t.src = t.src.trim().to_string();
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
    if let ([input], [path]) = (inputs, paths.as_slice())
        && !path.exists()
    {
        let b = input.trim().as_bytes().to_vec();
        if b.len() as u64 > MAX_TEXT {
            return Err(io::Error::other("texto grande demais"));
        }
        return Ok(Job::Text(b));
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

fn connect(to: &Peer, key: &[u8; 32]) -> io::Result<Secure> {
    let verify = |tcp: TcpStream| {
        tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
        tcp.set_write_timeout(Some(Duration::from_secs(30)))?;
        let s = secure::handshake(tcp, key, true)?;
        if s.remote != to.uid {
            return Err(io::Error::other(
                "a identidade desse aparelho mudou ou não confere: recusei por segurança",
            ));
        }
        s.tcp().set_read_timeout(Some(Duration::from_secs(60)))?;
        Ok(s)
    };
    let via_relay = |cfg: &relay::Config| {
        relay::dial(cfg, to.uid)
            .map_err(|e| io::Error::new(e.kind(), format!("não conectou pelo relay ({e})")))
            .and_then(verify)
    };
    let addr = match &to.route {
        Route::Lan(addr) => addr,
        Route::Relay(cfg) => return via_relay(cfg),
    };
    TcpStream::connect_timeout(addr, Duration::from_secs(5))
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "não conectou ({e}); confira o firewall dos dois lados (TCP {TRANSFER_PORT})"
                ),
            )
        })
        .and_then(verify)
        .or_else(|lan| match &to.fallback {
            Some(cfg) => {
                via_relay(cfg).map_err(|r| io::Error::new(r.kind(), format!("{lan}; {r}")))
            }
            None => Err(lan),
        })
}

fn open(to: &Peer, me: &Who, kind: char, size: u64, name: &str, extra: &str) -> io::Result<Secure> {
    let mut s = connect(to, &me.key)?;
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
        Some(rest) => crate::config::home().join(rest),
        None => s.into(),
    }
}
