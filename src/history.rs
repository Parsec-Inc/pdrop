use crate::config::setting_path;

#[derive(Clone)]
pub struct Rec {
    pub ts: u64,
    pub up: bool,
    pub kind: char,
    pub peer: String,
    pub label: String,
    pub size: u64,
    pub ok: bool,
    pub detail: String,
}

pub const HIST_MAX: usize = 200;

pub fn load_hist() -> Vec<Rec> {
    let raw = std::fs::read_to_string(setting_path("historico")).unwrap_or_default();
    let mut v: Vec<Rec> = raw
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let &[ts, up, kind, peer, label, size, ok, detail] = f.as_slice() else {
                return None;
            };
            Some(Rec {
                ts: ts.parse().ok()?,
                up: up == "1",
                kind: kind.chars().next()?,
                peer: peer.into(),
                label: label.into(),
                size: size.parse().ok()?,
                ok: ok == "1",
                detail: unesc(detail),
            })
        })
        .collect();
    v.drain(..v.len().saturating_sub(HIST_MAX));
    v
}

pub fn append_hist(r: &Rec) {
    use std::io::Write;
    let p = setting_path("historico");
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let _ = writeln!(
            f,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.ts,
            r.up as u8,
            r.kind,
            r.peer,
            r.label,
            r.size,
            r.ok as u8,
            esc(&r.detail)
        );
    }
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn unesc(s: &str) -> String {
    let (mut out, mut it) = (String::new(), s.chars());
    while let Some(c) = it.next() {
        match (c, c == '\\') {
            (_, true) => out.push(match it.next() {
                Some('n') => '\n',
                Some('t') => '\t',
                Some(o) => o,
                None => '\\',
            }),
            _ => out.push(c),
        }
    }
    out
}
