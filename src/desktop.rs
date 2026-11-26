use crate::{net, net::Msg};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::Duration,
};

// Devolve um flag: ligue-o para fechar o prompt (pedido já respondido no app).
pub fn notify(body: String, ask: Option<(u64, Sender<Msg>)>) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        let asking = ask.is_some();
        #[cfg(target_os = "macos")]
        let (mut c, parse) = {
            let mut c = std::process::Command::new("osascript");
            c.args(["-e", "on run {t, b}"]);
            if asking {
                c.args(["-e", "beep", "-e", "display dialog b with title t buttons {\"Recusar\", \"Aceitar\"} default button \"Recusar\" giving up after 120"]);
            } else {
                c.args([
                    "-e",
                    "display notification b with title t sound name \"Ping\"",
                ]);
            }
            c.args(["-e", "end run", "pdrop", &body]);
            (
                c,
                (|o: &str| {
                    o.contains("button returned:Aceitar")
                        .then_some(true)
                        .or(o.contains("button returned:Recusar").then_some(false))
                }) as fn(&str) -> Option<bool>,
            )
        };
        #[cfg(windows)]
        let (mut c, parse) = {
            let mut c = std::process::Command::new("powershell.exe");
            crate::clip::quiet(c.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                if asking { WIN_ASK } else { WIN_TOAST },
            ]))
            .env("PDROP_BODY", &body);
            (
                c,
                (|o: &str| {
                    o.contains("Yes")
                        .then_some(true)
                        .or(o.contains("No").then_some(false))
                }) as fn(&str) -> Option<bool>,
            )
        };
        #[cfg(not(any(target_os = "macos", windows)))]
        let (mut c, parse) = {
            let mut c = std::process::Command::new("notify-send");
            // nada de `-u critical`: pela especificação, notificação crítica nunca expira.
            // `transient` também a tira da central de notificações quando some.
            c.args(["-a", "pdrop", "-h", "int:transient:1"]);
            if asking {
                c.args(["-t", "120000", "-w"]);
                c.args(["-A", "y=Aceitar", "-A", "n=Recusar"]);
            } else {
                c.args(["-t", "6000"]);
            }
            c.args(["pdrop", &body.replace('&', "&amp;").replace('<', "&lt;")]);
            (
                c,
                (|o: &str| match o.trim() {
                    "y" => Some(true),
                    "n" => Some(false),
                    _ => None,
                }) as fn(&str) -> Option<bool>,
            )
        };
        let Ok(mut ch) = c.stdout(std::process::Stdio::piped()).spawn() else {
            return;
        };
        while ch.try_wait().ok().flatten().is_none() {
            if stop.load(Ordering::Relaxed) {
                ch.kill().ok();
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut out = String::new();
        if let Some(mut so) = ch.stdout.take() {
            so.read_to_string(&mut out).ok();
        }
        if let (Some((id, tx)), Some(y)) = (ask, parse(&out)) {
            tx.send(Msg::Reply(id, y)).ok();
        }
    });
    cancel
}

#[cfg(windows)]
const WIN_TOAST: &str = "[void][Windows.UI.Notifications.ToastNotificationManager,Windows.UI.Notifications,ContentType=WindowsRuntime];\
$x=[Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent('ToastText02');\
$t=$x.GetElementsByTagName('text');$t[0].AppendChild($x.CreateTextNode('pdrop'))|Out-Null;$t[1].AppendChild($x.CreateTextNode($env:PDROP_BODY))|Out-Null;\
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($x))";

#[cfg(windows)]
const WIN_ASK: &str = "Add-Type -AssemblyName System.Windows.Forms;\
$f=New-Object Windows.Forms.Form -Property @{TopMost=$true};\
[Windows.Forms.MessageBox]::Show($f,$env:PDROP_BODY,'pdrop','YesNo','Question','Button2')";

pub fn reveal(p: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{}", p.display()))
        .spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = std::process::Command::new("xdg-open")
        .arg(if p.is_dir() {
            p
        } else {
            p.parent().unwrap_or(p)
        })
        .spawn();
}

pub fn hostname() -> String {
    let h = std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        })
        .unwrap_or_default();
    let h = net::clean(h.trim().split('.').next().unwrap_or(""));
    if h.is_empty() { "pdrop".into() } else { h }
}
