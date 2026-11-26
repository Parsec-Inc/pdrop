use crate::{secure, ui};
use std::{path::PathBuf, sync::atomic::Ordering};

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| ".".into())
}

pub fn config_dir() -> PathBuf {
    home().join(".config/pdrop")
}

pub fn setting_path(k: &str) -> PathBuf {
    config_dir().join(k)
}

pub fn load_setting(k: &str) -> Option<String> {
    std::fs::read_to_string(setting_path(k))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn save_setting(k: &str, v: &str) {
    let p = setting_path(k);
    let _ = std::fs::create_dir_all(p.parent().unwrap()).and_then(|_| std::fs::write(p, v));
}

pub fn replace_file(p: &std::path::Path, v: &str, private: bool) -> std::io::Result<()> {
    use std::io::Write;
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    std::fs::create_dir_all(p.parent().unwrap())?;
    let tmp = p.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let write = || {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create_new(true);
        #[cfg(unix)]
        if private {
            std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        }
        let mut f = o.open(&tmp)?;
        if private {
            owner_only(&tmp)?;
        }
        f.write_all(v.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, p)
    };
    write().inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub fn owner_only(_p: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(_p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn load_identity() -> ([u8; 32], u128) {
    let saved = load_setting("chave").and_then(|s| {
        let (k, p) = s.split_once('\n')?;
        Some((unhex(k.trim())?, unhex(p.trim())?))
    });
    let (k, p) = saved.unwrap_or_else(|| {
        let (k, p) = secure::generate();
        save_setting("chave", &format!("{}\n{}", hex(&k), hex(&p)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                setting_path("chave"),
                std::fs::Permissions::from_mode(0o600),
            );
        }
        (k, p)
    });
    (k, secure::id_of(&p))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    let b: Vec<u8> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect::<Option<_>>()?;
    b.try_into().ok()
}

pub fn load_trust() -> std::collections::HashMap<u128, String> {
    let raw = load_setting("confiaveis").unwrap_or_default();
    raw.lines()
        .filter_map(|l| l.split_once('\t'))
        .filter_map(|(u, n)| Some((u128::from_str_radix(u, 16).ok()?, n.to_string())))
        .collect()
}

pub fn load_routes() -> Vec<(String, PathBuf)> {
    let raw = load_setting("destinos").unwrap_or_default();
    let real_home = std::env::var_os("SNAP_REAL_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(home);
    raw.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(r, p)| {
            let p = p.trim();
            (
                r.trim().to_string(),
                p.strip_prefix("~/")
                    .map_or_else(|| PathBuf::from(p), |rest| real_home.join(rest)),
            )
        })
        .collect()
}

pub fn saved_peers() -> Vec<std::net::IpAddr> {
    load_setting("peers")
        .map(|s| {
            s.lines()
                .filter_map(|l| l.split('#').next()?.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

pub fn load_color() -> u8 {
    let c = load_setting("cor").and_then(|s| s.parse::<u8>().ok());
    c.filter(|&c| (c as usize) < ui::COLORS.len()).unwrap_or(0)
}

pub fn load_icon() -> u8 {
    let i = load_setting("icone").and_then(|s| s.parse::<u8>().ok());
    i.filter(|&i| (i as usize) < ui::ICONS.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {

    #[test]
    fn replace_file_swaps_the_whole_file() {
        let dir = std::env::temp_dir().join(format!("pdrop-replace-{}", std::process::id()));
        let p = dir.join("relays");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, "um segredo antigo bem mais comprido").unwrap();
        super::replace_file(&p, "novo", true).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "novo");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "sobrou temporário"
        );
        std::fs::remove_dir_all(dir).ok();
    }
}
