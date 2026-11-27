use super::{
    Grants, HashRead, MAX_FILES, MAX_TEXT, Msg, PARTIAL, Policy, State, TRANSFER_PORT, Transfer,
    bad, clean, eof, next_id, preview, pump,
};
use crate::{
    clip, contacts,
    secure::{self, Secure},
};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{Sender, channel},
    },
    thread,
    time::Duration,
};

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

pub fn recv(
    s: TcpStream,
    tx: &Sender<Msg>,
    dir: &Path,
    grants: &Grants,
    policy: &Policy,
) -> io::Result<()> {
    let id = next_id();
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    let sec = secure::handshake(s, &policy.key, false)?;
    let (uid, key) = (sec.remote, sec.remote_key);
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
    if ok {
        contacts::learn(key);
    }
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

pub(super) fn part_name(uid: u128, rel: &str, size: u64, mtime: &str) -> String {
    format!(
        "{}.part",
        &blake3::hash(format!("{uid:x}\0{rel}\0{size}\0{mtime}").as_bytes()).to_hex()[..32]
    )
}

pub(super) fn rel_components(rel: &str) -> Option<Vec<String>> {
    let v: Vec<String> = rel
        .split('/')
        .map(|c| {
            c.chars()
                .filter(|c| !c.is_control() && *c != '\\')
                .map(|c| if cfg!(windows) && c == ':' { '_' } else { c })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn received_names_stay_inside_the_folder() {
        let base = Path::new("recebidos");
        for rel in ["C:payload.bin", "c:", "pasta/D:x", "a\\C:\\x"] {
            let mut p = base.to_path_buf();
            for part in rel_components(rel).unwrap() {
                let parent = p.clone();
                p.push(part);
                assert_eq!(p.parent(), Some(parent.as_path()), "{rel}");
            }
        }
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
}
