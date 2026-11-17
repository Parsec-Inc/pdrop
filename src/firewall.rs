
use crate::clip::quiet;
use std::{
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[derive(Clone, PartialEq)]
pub struct Warning {
    pub text: String,
    pub fix: String,
}

pub type Shared = Arc<Mutex<Option<Warning>>>;

const RECHECK: Duration = Duration::from_secs(30);

pub fn watch() -> Shared {
    let shared = Shared::default();
    let out = shared.clone();
    thread::spawn(move || {
        loop {
            *out.lock().unwrap() = check();
            thread::sleep(RECHECK);
        }
    });
    shared
}

fn run(cmd: &str, args: &[&str]) -> Option<(bool, String)> {
    let o = quiet(Command::new(cmd).args(args)).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    Some((o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()))
}

#[cfg(target_os = "linux")]
fn check() -> Option<Warning> {
    use crate::net::{DISCOVERY_PORT, TRANSFER_PORT};
    if run("firewall-cmd", &["--state"]).is_some_and(|(ok, _)| ok) {
        let open = |arg: String| run("firewall-cmd", &[&arg]).is_some_and(|(ok, _)| ok);
        if open("--query-service=pdrop".into()) || (open(format!("--query-port={DISCOVERY_PORT}/udp")) && open(format!("--query-port={TRANSFER_PORT}/tcp"))) {
            return None;
        }
        let fix = if std::path::Path::new("/usr/lib/firewalld/services/pdrop.xml").exists() {
            "sudo firewall-cmd --permanent --add-service=pdrop && sudo firewall-cmd --reload".to_string()
        } else {
            format!("sudo firewall-cmd --permanent --add-port={DISCOVERY_PORT}/udp --add-port={TRANSFER_PORT}/tcp && sudo firewall-cmd --reload")
        };
        return Some(Warning { text: "o firewalld está bloqueando o pdrop".into(), fix });
    }
    if ufw_enabled(&std::fs::read_to_string("/etc/ufw/ufw.conf").unwrap_or_default()) {
        let fix = if std::path::Path::new("/etc/ufw/applications.d/pdrop").exists() {
            "sudo ufw allow pdrop".to_string()
        } else {
            format!("sudo ufw allow {DISCOVERY_PORT}/udp && sudo ufw allow {TRANSFER_PORT}/tcp")
        };
        return Some(Warning { text: "o ufw está ligado e pode bloquear o pdrop".into(), fix });
    }
    None
}

#[cfg(any(target_os = "linux", test))]
fn ufw_enabled(conf: &str) -> bool {
    conf.lines().any(|l| l.trim() == "ENABLED=yes")
}

#[cfg(windows)]
fn block_query(exe: &std::path::Path) -> String {
    let exe = exe.to_string_lossy().replace('\'', "''");
    format!(
        "@(Get-NetFirewallApplicationFilter -Program '{exe}' | Get-NetFirewallRule | \
         Where-Object {{ $_.Enabled -eq 'True' -and $_.Direction -eq 'Inbound' -and $_.Action -eq 'Block' }}).Count"
    )
}

#[cfg(windows)]
fn check() -> Option<Warning> {
    let exe = std::env::current_exe().ok()?;
    let (_, out) = run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &block_query(&exe)])?;
    let blocked = out.trim().parse::<u32>().unwrap_or(0) > 0;
    blocked.then(|| Warning {
        text: "o Firewall do Windows está bloqueando o pdrop".into(),
        fix: "Configurações > Privacidade e segurança > Segurança do Windows > Firewall > Permitir um aplicativo: marque o pdrop em Particular".into(),
    })
}

#[cfg(target_os = "macos")]
const FW: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";

#[cfg(any(target_os = "macos", test))]
fn app_blocked(out: &str) -> bool {
    out.contains("blocked") && !out.contains("permitted") && !out.contains("not blocked")
}

#[cfg(target_os = "macos")]
fn check() -> Option<Warning> {
    let exe = std::env::current_exe().ok()?;
    let on = run(FW, &["--getglobalstate"]).is_some_and(|(_, out)| out.contains("enabled"));
    let blocked = on && run(FW, &["--getappblocked", &exe.to_string_lossy()]).is_some_and(|(_, out)| app_blocked(&out));
    blocked.then(|| Warning {
        text: "o firewall do macOS está bloqueando o pdrop".into(),
        fix: "Ajustes do Sistema > Rede > Firewall > Opções: permita conexões de entrada para o pdrop".into(),
    })
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn check() -> Option<Warning> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ufw_and_macos_answers_are_read() {
        assert!(ufw_enabled("# ufw\nENABLED=yes\n"));
        assert!(!ufw_enabled("ENABLED=no\n"));
        assert!(app_blocked("The application /x is blocked from incoming connections."));
        assert!(!app_blocked("The application /x is permitted."));
        assert!(!app_blocked("The application /x is not part of the firewall"));
    }

    #[test]
    fn check_finishes_on_this_computer() {
        let started = std::time::Instant::now();
        if let Some(w) = check() {
            assert!(!w.text.is_empty() && !w.fix.is_empty());
        }
        assert!(started.elapsed() < Duration::from_secs(30));
    }

    #[cfg(windows)]
    #[test]
    fn windows_block_query_answers_a_count() {
        let exe = std::env::current_exe().unwrap();
        let (ok, out) = run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &block_query(&exe)]).unwrap();
        assert!(ok && out.trim().parse::<u32>().is_ok(), "{out:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn socketfilterfw_answers_in_the_words_check_reads() {
        for arg in ["--getglobalstate", "--getblockall"] {
            let (_, out) = run(FW, &[arg]).unwrap();
            assert!(out.contains("enabled") || out.contains("disabled"), "{arg}: {out:?}");
        }
        let exe = std::env::current_exe().unwrap();
        let (_, out) = run(FW, &["--getappblocked", &exe.to_string_lossy()]).unwrap();
        assert!(!app_blocked(&out), "{out:?}");
    }
}
