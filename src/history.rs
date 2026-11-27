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
    pub uid: u128,
}

pub const HIST_MAX: usize = 200;

fn parse_line(l: &str) -> Option<Rec> {
    let f: Vec<&str> = l.split('\t').collect();
    if !(8..=9).contains(&f.len()) {
        return None;
    }
    Some(Rec {
        ts: f[0].parse().ok()?,
        up: f[1] == "1",
        kind: f[2].chars().next()?,
        peer: f[3].into(),
        label: f[4].into(),
        size: f[5].parse().ok()?,
        ok: f[6] == "1",
        detail: unesc(f[7]),
        uid: f.get(8).map_or(Some(0), |u| u128::from_str_radix(u, 16).ok())?,
    })
}

pub fn load_hist() -> Vec<Rec> {
    let raw = std::fs::read_to_string(setting_path("historico")).unwrap_or_default();
    let mut v: Vec<Rec> = raw.lines().filter_map(parse_line).collect();
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
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:x}",
            r.ts,
            r.up as u8,
            r.kind,
            r.peer,
            r.label,
            r.size,
            r.ok as u8,
            esc(&r.detail),
            r.uid
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_and_new_lines_load() {
        let old = parse_line("1\t1\tt\tana\tmensagem\t3\t1\toi\\nx").unwrap();
        assert_eq!((old.uid, old.detail.as_str()), (0, "oi\nx"));
        let new = parse_line("1\t0\tt\tana\tmensagem\t3\t1\toi\tff").unwrap();
        assert_eq!(new.uid, 0xff);
        assert!(parse_line("1\t1\tt\tana").is_none());
    }
}
