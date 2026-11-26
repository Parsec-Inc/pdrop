use crate::{net, relay};
use std::{
    io,
    path::{Path, PathBuf},
};

pub const NAME_MAX: usize = 32;
const FILE: &str = "relays";
const OLD_ADDR: &str = "relay";
const OLD_SECRET: &str = "sala";

#[derive(Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub addr: String,
    pub secret: String,
}

impl Profile {
    pub fn new(name: &str, addr: &str, secret: &str) -> Result<Self, String> {
        let (addr, secret) = (addr.trim(), secret.trim());
        if addr.is_empty() {
            return Err("informe o servidor, no formato host:porta".into());
        }
        if addr.chars().chain(secret.chars()).any(char::is_control) {
            return Err("o servidor e a senha não podem ter tabulação nem quebra de linha".into());
        }
        relay::config(Some(addr), Some(secret))?;
        let mut name = short_name(name);
        if name.is_empty() {
            name = short_name(addr.rsplit_once(':').map_or(addr, |(host, _)| host));
        }
        Ok(Self {
            name,
            addr: addr.into(),
            secret: secret.into(),
        })
    }

    fn same_room(&self, cfg: &(String, String)) -> bool {
        self.addr == cfg.0 && relay::room_of(&self.secret) == cfg.1
    }
}

fn short_name(s: &str) -> String {
    let s: String = net::clean(s.trim()).chars().take(NAME_MAX).collect();
    s.trim().into()
}

#[derive(Clone, Default, PartialEq)]
pub struct Book {
    pub list: Vec<Profile>,
    pub active: Option<usize>,
}

impl Book {
    pub fn current(&self) -> Option<&Profile> {
        self.list.get(self.active?)
    }

    fn parse(text: &str) -> Result<Self, String> {
        let mut book = Self::default();
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let bad = || format!("linha {} inválida", n + 1);
            let f: Vec<&str> = line.split('\t').collect();
            let &[mark @ ("*" | "-"), name, addr, secret] = f.as_slice() else {
                return Err(bad());
            };
            let p = Profile::new(name, addr, secret).map_err(|e| format!("{}: {e}", bad()))?;
            if mark == "*" {
                book.active = Some(book.list.len());
            }
            book.list.push(p);
        }
        Ok(book)
    }

    fn render(&self) -> String {
        self.list
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mark = if self.active == Some(i) { '*' } else { '-' };
                format!("{mark}\t{}\t{}\t{}\n", p.name, p.addr, p.secret)
            })
            .collect()
    }

    fn put(&mut self, at: Option<usize>, mut p: Profile) -> usize {
        let at = at.filter(|i| *i < self.list.len());
        let taken = |name: &str| {
            self.list
                .iter()
                .enumerate()
                .any(|(i, o)| Some(i) != at && o.name == name)
        };
        if taken(&p.name) {
            let base = p.name.clone();
            p.name = (2..)
                .map(|n| {
                    let suffix = format!(" {n}");
                    let keep = NAME_MAX - suffix.len();
                    base.chars().take(keep).collect::<String>() + &suffix
                })
                .find(|name| !taken(name))
                .unwrap();
        }
        match at {
            Some(i) => {
                self.list[i] = p;
                i
            }
            None => {
                self.list.push(p);
                self.list.len() - 1
            }
        }
    }

    fn remove(&mut self, i: usize) {
        if i >= self.list.len() {
            return;
        }
        self.list.remove(i);
        self.active = match self.active {
            Some(a) if a == i => None,
            Some(a) if a > i => Some(a - 1),
            a => a,
        };
    }

    fn adopt(&mut self, p: Profile) {
        let found = self
            .list
            .iter()
            .position(|o| o.addr == p.addr && o.secret == p.secret);
        self.active = Some(found.unwrap_or_else(|| self.put(None, p)));
    }
}

#[derive(Default)]
pub struct Saved {
    pub book: Book,
    broken: Option<String>,
    old: (Option<String>, Option<String>),
    migrate: bool,
}

fn read(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn load(dir: &Path) -> Saved {
    let mut saved = Saved::default();
    let path = dir.join(FILE);
    match read(&path) {
        Ok(Some(text)) => {
            let _ = crate::owner_only(&path);
            match Book::parse(&text) {
                Ok(book) => saved.book = book,
                Err(e) => saved.broken = Some(format!("{}: {e}", path.display())),
            }
        }
        Ok(None) => {
            let old = |name| {
                read(&dir.join(name))
                    .ok()
                    .flatten()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            };
            let (addr, secret) = (old(OLD_ADDR), old(OLD_SECRET));
            if secret.is_some() {
                let _ = crate::owner_only(&dir.join(OLD_SECRET));
            }
            match (&addr, &secret) {
                (Some(a), Some(s)) if Profile::new("", a, s).is_ok() => {
                    saved.book.adopt(Profile::new("", a, s).unwrap());
                    saved.migrate = true;
                }
                _ => saved.old = (addr, secret),
            }
        }
        Err(e) => saved.broken = Some(format!("{}: {e}", path.display())),
    }
    saved
}

fn store(dir: &Path, book: &Book) -> io::Result<()> {
    crate::replace_file(&dir.join(FILE), &book.render(), true)?;
    for old in [OLD_ADDR, OLD_SECRET] {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(())
}

#[derive(Default)]
pub struct Flags {
    pub addr: Option<String>,
    pub room: Option<String>,
}

impl Flags {
    fn any(&self) -> bool {
        self.addr.is_some() || self.room.is_some()
    }
}

#[derive(Default)]
pub struct Env {
    pub addr: Option<String>,
    pub room: Option<String>,
}

impl Env {
    pub fn read() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        Self {
            addr: var("PDROP_RELAY"),
            room: var("PDROP_ROOM"),
        }
    }

    pub fn any(&self) -> bool {
        self.addr.is_some() || self.room.is_some()
    }
}

fn saved_pair(saved: &Saved) -> (Option<String>, Option<String>) {
    match saved.book.current() {
        Some(p) => (Some(p.addr.clone()), Some(p.secret.clone())),
        None => saved.old.clone(),
    }
}

pub type Resolved = (Option<relay::Config>, Option<String>);

pub fn resolve(flags: &Flags, env: &Env, saved: &Saved) -> Result<Resolved, String> {
    let (addr, secret) = saved_pair(saved);
    let addr = flags.addr.clone().or_else(|| env.addr.clone()).or(addr);
    let secret = flags.room.clone().or_else(|| env.room.clone()).or(secret);
    let off = |e: String| {
        eprintln!("relay desligado: {e}");
        Ok((None, Some(e)))
    };
    match relay::config(addr.as_deref(), secret.as_deref()) {
        Ok(None) => match &saved.broken {
            Some(e) => off(e.clone()),
            None => Ok((None, None)),
        },
        Err(e) if !flags.any() => off(saved.broken.clone().unwrap_or(e)),
        r => r.map(|c| (c, None)),
    }
}

pub fn remember(dir: &Path, flags: &Flags, saved: &mut Saved) -> Result<(), String> {
    if let Some(e) = &saved.broken {
        return if flags.any() {
            Err(format!("não salvei o relay: {e}"))
        } else {
            Ok(())
        };
    }
    let mut book = saved.book.clone();
    if flags.any() {
        let (addr, secret) = saved_pair(saved);
        let (addr, secret) = (flags.addr.clone().or(addr), flags.room.clone().or(secret));
        if let (Some(a), Some(s)) = (addr, secret) {
            book.adopt(Profile::new("", &a, &s)?);
        }
    }
    if book != saved.book || saved.migrate {
        store(dir, &book).map_err(|e| format!("não consegui salvar o relay: {e}"))?;
        (saved.book, saved.migrate, saved.old) = (book, false, Default::default());
    }
    Ok(())
}

pub struct Control {
    dir: PathBuf,
    saved: Saved,
    me: relay::Me,
    peers: net::Peers,
    incoming: relay::Incoming,
    handle: Option<relay::Handle>,
    running: Option<(String, String)>,
    off: Option<String>,
}

impl Control {
    pub fn new(
        dir: PathBuf,
        saved: Saved,
        me: relay::Me,
        peers: net::Peers,
        incoming: relay::Incoming,
        (cfg, off): Resolved,
    ) -> Self {
        let mut c = Self {
            dir,
            saved,
            me,
            peers,
            incoming,
            handle: None,
            running: None,
            off,
        };
        if cfg.is_some() {
            c.connect(cfg);
        }
        c
    }

    pub fn book(&self) -> &Book {
        &self.saved.book
    }

    pub fn link(&self) -> Option<relay::Link> {
        match &self.handle {
            Some(h) => Some(h.status.lock().unwrap().clone()),
            None => self.off.clone().map(relay::Link::Off),
        }
    }

    pub fn in_use(&self) -> Option<usize> {
        let running = self.running.as_ref()?;
        self.saved
            .book
            .current()
            .filter(|p| p.same_room(running))
            .and(self.saved.book.active)
    }

    pub fn external(&self) -> bool {
        self.running.is_some() && self.in_use().is_none()
    }

    fn connect(&mut self, cfg: Option<relay::Config>) {
        if let Some(h) = self.handle.take() {
            h.stop();
        }
        self.running = cfg.as_ref().map(|c| (c.addr.clone(), c.room.clone()));
        self.handle = cfg.map(|c| {
            relay::watch(
                c,
                self.me.clone(),
                self.peers.clone(),
                self.incoming.clone(),
            )
        });
    }

    fn change(&mut self, edit: impl FnOnce(&mut Book)) -> Result<(), String> {
        if let Some(e) = &self.saved.broken {
            return Err(format!("corrija ou apague o arquivo antes: {e}"));
        }
        let mut book = self.saved.book.clone();
        edit(&mut book);
        store(&self.dir, &book).map_err(|e| format!("não consegui salvar: {e}"))?;
        (self.saved.book, self.saved.migrate, self.saved.old) = (book, false, Default::default());
        Ok(())
    }

    fn reconnect(&mut self) {
        let cfg = self
            .saved
            .book
            .current()
            .and_then(|p| relay::config(Some(&p.addr), Some(&p.secret)).ok()?);
        self.off = None;
        self.connect(cfg);
    }

    pub fn activate(&mut self, i: Option<usize>) -> Result<(), String> {
        self.change(|b| b.active = i.filter(|i| *i < b.list.len()))?;
        if i.is_none() || self.in_use() != i {
            self.reconnect();
        }
        Ok(())
    }

    pub fn save(&mut self, at: Option<usize>, p: Profile) -> Result<(), String> {
        let before = self.saved.book.current().cloned();
        self.change(|b| {
            let i = b.put(at, p);
            if at.is_none() {
                b.active = Some(i);
            }
        })?;
        let now = self.saved.book.current();
        let room = |p: &Profile| (p.addr.clone(), p.secret.clone());
        if now.map(room) != before.as_ref().map(room) {
            self.reconnect();
        }
        Ok(())
    }

    pub fn remove(&mut self, i: usize) -> Result<(), String> {
        let was_connected = self.in_use() == Some(i);
        self.change(|b| b.remove(i))?;
        if was_connected {
            self.reconnect();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S1: &str = "0123456789abcdef";
    const S2: &str = "fedcba9876543210";

    fn profile(name: &str, addr: &str, secret: &str) -> Profile {
        Profile::new(name, addr, secret).unwrap()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pdrop-relays-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn control(dir: &Path) -> Control {
        let me = relay::Me {
            id: 1,
            uid: 2,
            name: Default::default(),
            color: Default::default(),
            policy: Default::default(),
        };
        Control::new(
            dir.into(),
            load(dir),
            me,
            Default::default(),
            std::sync::Arc::new(|_| {}),
            (None, None),
        )
    }

    #[test]
    fn profiles_are_checked_and_named() {
        assert!(Profile::new("a", "", S1).is_err());
        assert!(Profile::new("a", "sem-porta", S1).is_err());
        assert!(Profile::new("a", "ex.com:1", "curta").is_err());
        assert!(Profile::new("a", "ex.com:1", "0123456789\tabcdef").is_err());
        assert!(profile("  ", "ex.com:1", S1).name == "ex.com");
        assert!(
            profile("a\tb\n", " ex.com:1 ", S1)
                .name
                .chars()
                .all(|c| !c.is_control())
        );
        assert!(profile(&"x".repeat(99), "ex.com:1", S1).name.len() == NAME_MAX);
    }

    #[test]
    fn the_book_survives_a_round_trip() {
        let mut b = Book::default();
        b.put(None, profile("casa", "a.com:1", S1));
        b.put(
            None,
            profile("casa", "b.com:2", "senha com espaços no meio"),
        );
        b.active = Some(1);
        assert_eq!(b.list[1].name, "casa 2");
        assert!(Book::parse(&b.render()).unwrap() == b);
        b.put(Some(1), b.list[1].clone());
        assert_eq!((b.list.len(), b.list[1].name.as_str()), (2, "casa 2"));
        b.remove(0);
        assert_eq!(b.active, Some(0));
        b.remove(0);
        assert_eq!(b.active, None);
        assert!(Book::parse("lixo").is_err());
        assert!(Book::parse("*\ta\ta.com:1\tcurta").is_err());
    }

    #[test]
    fn the_old_files_become_a_saved_relay() {
        let dir = temp("old");
        std::fs::write(dir.join(OLD_ADDR), "a.com:1\n").unwrap();
        std::fs::write(dir.join(OLD_SECRET), format!("{S1}\n")).unwrap();
        let mut saved = load(&dir);
        assert!(!dir.join(FILE).exists(), "só ler não grava");
        assert!(saved.book.current() == Some(&profile("", "a.com:1", S1)));
        remember(&dir, &Flags::default(), &mut saved).unwrap();
        assert!(load(&dir).book == saved.book);
        assert!(!dir.join(OLD_ADDR).exists() && !dir.join(OLD_SECRET).exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn options_beat_the_environment_which_beats_the_saved_relay() {
        let mut saved = Saved::default();
        saved.book.adopt(profile("", "salvo.com:1", S1));
        let addr = |f: &Flags, e: &Env| resolve(f, e, &saved).unwrap().0.unwrap().addr;
        let env = Env {
            addr: Some("env.com:1".into()),
            room: None,
        };
        let flags = Flags {
            addr: Some("flag.com:1".into()),
            room: None,
        };
        assert_eq!(addr(&Flags::default(), &Env::default()), "salvo.com:1");
        assert_eq!(addr(&Flags::default(), &env), "env.com:1");
        assert_eq!(addr(&flags, &env), "flag.com:1");

        let bad = Flags {
            addr: Some("ruim".into()),
            room: None,
        };
        assert!(resolve(&bad, &Env::default(), &saved).is_err());
        let bad = Env {
            addr: Some("ruim".into()),
            room: None,
        };
        let (cfg, off) = resolve(&Flags::default(), &bad, &saved).unwrap();
        assert!(cfg.is_none() && off.is_some());
    }

    #[test]
    fn options_are_remembered_and_the_environment_is_not() {
        let dir = temp("flags");
        let mut saved = load(&dir);
        let flags = |a: &str, s: Option<&str>| Flags {
            addr: Some(a.into()),
            room: s.map(Into::into),
        };
        remember(&dir, &flags("a.com:1", None), &mut saved).unwrap();
        assert!(!dir.join(FILE).exists());
        remember(&dir, &flags("a.com:1", Some(S1)), &mut saved).unwrap();
        remember(&dir, &flags("b.com:1", Some(S2)), &mut saved).unwrap();
        remember(&dir, &flags("a.com:1", Some(S1)), &mut saved).unwrap();
        let b = load(&dir).book;
        assert_eq!((b.list.len(), b.active), (2, Some(0)));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_file_that_cannot_be_read_is_never_overwritten() {
        let dir = temp("broken");
        std::fs::write(dir.join(FILE), "editado à mão e quebrado").unwrap();
        let mut c = control(&dir);
        assert!(c.save(None, profile("", "a.com:1", S1)).is_err());
        let mut saved = load(&dir);
        assert!(remember(&dir, &Flags::default(), &mut saved).is_ok());
        let (cfg, off) = resolve(&Flags::default(), &Env::default(), &saved).unwrap();
        assert!(cfg.is_none() && off.unwrap().contains("linha 1"));
        assert_eq!(
            std::fs::read_to_string(dir.join(FILE)).unwrap(),
            "editado à mão e quebrado"
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_window_switches_between_saved_relays() {
        let dir = temp("control");
        let mut c = control(&dir);
        assert!(c.link().is_none());
        c.save(None, profile("um", "127.0.0.1:1", S1)).unwrap();
        c.save(None, profile("dois", "127.0.0.1:2", S2)).unwrap();
        assert_eq!(c.in_use(), Some(1), "o novo já entra em uso");
        c.activate(Some(0)).unwrap();
        assert_eq!(c.in_use(), Some(0));

        let status = c.handle.as_ref().unwrap().status.clone();
        let same =
            |c: &Control| std::sync::Arc::ptr_eq(&status, &c.handle.as_ref().unwrap().status);
        c.save(Some(1), profile("outro", "127.0.0.1:3", S2))
            .unwrap();
        c.save(Some(0), profile("renomeado", "127.0.0.1:1", S1))
            .unwrap();
        c.activate(Some(0)).unwrap();
        assert!(same(&c));
        c.save(Some(0), profile("renomeado", "127.0.0.1:4", S1))
            .unwrap();
        assert!(!same(&c));
        assert_eq!(c.in_use(), Some(0));

        c.remove(1).unwrap();
        assert_eq!(c.in_use(), Some(0));
        c.activate(None).unwrap();
        assert!(c.link().is_none() && c.running.is_none());
        c.activate(Some(0)).unwrap();
        c.remove(0).unwrap();
        assert!(c.link().is_none() && c.book().list.is_empty());
        assert!(load(&dir).book == *c.book());
        std::fs::remove_dir_all(dir).ok();
    }
}
