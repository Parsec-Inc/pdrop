//! Clipboard via ferramentas do sistema (sem dependência): wl-clipboard, xclip/xsel, pbcopy/pbpaste, PowerShell (Windows).
//! Com wl-clipboard e xclip o conteúdo vai com o tipo (texto, imagem…); nas outras, só texto.
use std::{
    io::{self, Read, Write},
    process::{Command, Stdio},
    sync::{Mutex, OnceLock},
};

/// Maior conteúdo de clipboard aceito (fica todo na memória dos dois lados).
pub const MAX: u64 = 64 << 20;
pub const TEXT: &str = "text/plain;charset=utf-8";

/// Conteúdo do clipboard com o tipo MIME.
#[derive(Clone, PartialEq)]
pub struct Clip {
    pub mime: String,
    pub data: Vec<u8>,
}

impl Clip {
    pub fn is_text(&self) -> bool {
        is_text(&self.mime)
    }
}

fn is_text(m: &str) -> bool {
    m.starts_with("text/") || matches!(m, "UTF8_STRING" | "STRING" | "TEXT" | "COMPOUND_TEXT")
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Wl,
    Xclip,
    Xsel,
    Mac,
    Win,
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
}

fn tool() -> Option<Tool> {
    static T: OnceLock<Option<Tool>> = OnceLock::new();
    *T.get_or_init(|| {
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        match () {
            _ if cfg!(windows) => which("powershell.exe").then_some(Tool::Win),
            _ if which("pbpaste") => Some(Tool::Mac),
            _ if wayland && which("wl-paste") && which("wl-copy") => Some(Tool::Wl),
            _ if which("xclip") => Some(Tool::Xclip),
            _ if which("xsel") => Some(Tool::Xsel),
            _ => None,
        }
    })
}

pub fn available() -> bool {
    tool().is_some()
}

/// Última coisa que o pdrop viu ou escreveu no clipboard: evita reenviar o que acabou de chegar (eco).
static LAST: Mutex<Option<Clip>> = Mutex::new(None);

/// Windows: sem isso cada leitura do clipboard pisca uma janela de console.
fn quiet(c: &mut Command) -> &mut Command {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(c, 0x0800_0000);
    c
}

/// Roda e lê a saída inteira, até MAX; maior que isso é recusado (o processo é morto, não fica preso no pipe).
fn read(cmd: &[&str]) -> Option<Vec<u8>> {
    let mut child = quiet(Command::new(cmd[0]).args(&cmd[1..])).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut buf = Vec::new();
    child.stdout.take()?.take(MAX + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > MAX {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    child.wait().ok()?.success().then_some(buf)
}

/// Entre os tipos oferecidos, o que melhor representa o conteúdo: imagem se houver, senão texto.
fn pick(types: &str) -> Option<String> {
    let ts: Vec<&str> = types.lines().map(str::trim).filter(|t| !t.is_empty()).collect();
    let has = |t: &str| ts.contains(&t);
    if has("image/png") {
        return Some("image/png".into());
    }
    if let Some(t) = ts.iter().find(|t| t.starts_with("image/")) {
        return Some(t.to_string());
    }
    [TEXT, "UTF8_STRING", "text/plain", "STRING", "TEXT"].into_iter().find(|t| has(t)).map(|t| if t == "UTF8_STRING" { TEXT.into() } else { t.into() })
}

pub fn get() -> Option<Clip> {
    let t = tool()?;
    let mime = match t {
        Tool::Wl => pick(&String::from_utf8_lossy(&read(&["wl-paste", "--list-types"])?))?,
        Tool::Xclip => pick(&String::from_utf8_lossy(&read(&["xclip", "-selection", "clipboard", "-t", "TARGETS", "-o"])?))?,
        _ => TEXT.into(),
    };
    let data = match t {
        Tool::Wl => read(&["wl-paste", "-n", "-t", &mime])?,
        Tool::Xclip => read(&["xclip", "-selection", "clipboard", "-t", if mime == TEXT { "UTF8_STRING" } else { &mime }, "-o"])?,
        Tool::Xsel => read(&["xsel", "-ob"])?,
        Tool::Mac => read(&["pbpaste"])?,
        Tool::Win => read(&["powershell.exe", "-NoProfile", "-Command", "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Get-Clipboard -Raw"])?,
    };
    Some(Clip { mime, data })
}

pub fn set(c: &Clip) -> io::Result<()> {
    let t = tool().ok_or_else(|| io::Error::other("sem ferramenta de clipboard"))?;
    let text = c.is_text();
    let cmd: Vec<&str> = match t {
        // texto sem -t: wl-copy/xclip oferecem todos os tipos de texto, e qualquer app cola
        Tool::Wl if text => vec!["wl-copy"],
        Tool::Wl => vec!["wl-copy", "-t", &c.mime],
        Tool::Xclip if text => vec!["xclip", "-selection", "clipboard", "-i"],
        Tool::Xclip => vec!["xclip", "-selection", "clipboard", "-t", &c.mime, "-i"],
        _ if !text => return Err(io::Error::other("este sistema só recebe texto no clipboard")),
        Tool::Xsel => vec!["xsel", "-ib"],
        Tool::Mac => vec!["pbcopy"],
        Tool::Win => vec!["powershell.exe", "-NoProfile", "-Command", "[Console]::InputEncoding=[Text.Encoding]::UTF8; Set-Clipboard -Value ([Console]::In.ReadToEnd())"],
    };
    *LAST.lock().unwrap() = Some(c.clone());
    let mut child = quiet(Command::new(cmd[0]).args(&cmd[1..])).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    child.stdin.take().unwrap().write_all(&c.data)?; // o stdin fecha aqui: EOF para a ferramenta
    if !child.wait()?.success() {
        return Err(io::Error::other(format!("{} falhou", cmd[0])));
    }
    Ok(())
}

/// Marca o conteúdo atual como "já visto": a ponte só envia o que for copiado depois de ligada.
pub fn mark_current() {
    *LAST.lock().unwrap() = get();
}

/// true se `cur` é novo (e passa a ser o último visto).
pub fn changed(cur: &Clip) -> bool {
    let mut l = LAST.lock().unwrap();
    (l.as_ref() != Some(cur)).then(|| *l = Some(cur.clone())).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_prefers_image_then_text() {
        assert_eq!(pick("text/html\nimage/png\ntext/plain").as_deref(), Some("image/png"));
        assert_eq!(pick("image/jpeg\nTEXT").as_deref(), Some("image/jpeg"));
        assert_eq!(pick("TARGETS\nUTF8_STRING\nSTRING").as_deref(), Some(TEXT));
        assert_eq!(pick("text/html\ntext/plain").as_deref(), Some("text/plain"));
        assert_eq!(pick("x-special/gnome-copied-files"), None);
    }
}
