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
    let o = quiet(Command::new(cmd).args(args))
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some((
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    ))
}

#[cfg(target_os = "linux")]
fn check() -> Option<Warning> {
    use crate::net::{DISCOVERY_PORT, TRANSFER_PORT};
    if run("firewall-cmd", &["--state"]).is_some_and(|(ok, _)| ok) {
        let open = |arg: String| run("firewall-cmd", &[&arg]).is_some_and(|(ok, _)| ok);
        if open("--query-service=pdrop".into())
            || (open(format!("--query-port={DISCOVERY_PORT}/udp"))
                && open(format!("--query-port={TRANSFER_PORT}/tcp")))
        {
            return None;
        }
        let fix = if std::path::Path::new("/usr/lib/firewalld/services/pdrop.xml").exists() {
            "sudo firewall-cmd --permanent --add-service=pdrop && sudo firewall-cmd --reload"
                .to_string()
        } else {
            format!(
                "sudo firewall-cmd --permanent --add-port={DISCOVERY_PORT}/udp --add-port={TRANSFER_PORT}/tcp && sudo firewall-cmd --reload"
            )
        };
        return Some(Warning {
            text: "o firewalld está bloqueando o pdrop".into(),
            fix,
        });
    }
    let ufw = std::fs::read_to_string("/etc/ufw/ufw.conf").unwrap_or_default();
    if ufw.lines().any(|l| l.trim() == "ENABLED=yes") {
        let fix = if std::path::Path::new("/etc/ufw/applications.d/pdrop").exists() {
            "sudo ufw allow pdrop".to_string()
        } else {
            format!("sudo ufw allow {DISCOVERY_PORT}/udp && sudo ufw allow {TRANSFER_PORT}/tcp")
        };
        return Some(Warning {
            text: "o ufw está ligado e pode bloquear o pdrop".into(),
            fix,
        });
    }
    None
}

#[cfg(windows)]
fn check() -> Option<Warning> {
    let exe = std::env::current_exe().ok()?;
    let exe = exe.to_string_lossy().replace('\'', "''");
    let script = format!(
        "@(Get-NetFirewallApplicationFilter -Program '{exe}' | Get-NetFirewallRule | \
         Where-Object {{ $_.Enabled -eq 'True' -and $_.Direction -eq 'Inbound' -and $_.Action -eq 'Block' }}).Count"
    );
    let (_, out) = run(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )?;
    let blocked = out.trim().parse::<u32>().unwrap_or(0) > 0;
    blocked.then(|| Warning {
        text: "o Firewall do Windows está bloqueando o pdrop".into(),
        fix: "Configurações > Privacidade e segurança > Segurança do Windows > Firewall > Permitir um aplicativo: marque o pdrop em Particular".into(),
    })
}

#[cfg(target_os = "macos")]
fn check() -> Option<Warning> {
    const FW: &str = "/usr/libexec/ApplicationFirewall/socketfilterfw";
    let exe = std::env::current_exe().ok()?;
    let on = run(FW, &["--getglobalstate"]).is_some_and(|(_, out)| out.contains("enabled"));
    let blocked = on
        && run(FW, &["--getappblocked", &exe.to_string_lossy()])
            .is_some_and(|(_, out)| out.contains("blocked") && !out.contains("permitted"));
    blocked.then(|| Warning {
        text: "o firewall do macOS está bloqueando o pdrop".into(),
        fix: "Ajustes do Sistema > Rede > Firewall > Opções: permita conexões de entrada para o pdrop".into(),
    })
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
fn check() -> Option<Warning> {
    None
}
