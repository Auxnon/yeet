//! Copy text to the system clipboard.
//!
//! On Linux we shell out to wl-copy / xclip / xsel / termux-clipboard-set:
//! they keep serving the selection after yeet exits, which an in-process
//! clipboard owner can't do. Elsewhere arboard is fine.

use anyhow::{Result, bail};

#[cfg(target_os = "linux")]
pub fn copy(text: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let x11 = std::env::var_os("DISPLAY").is_some();
    let candidates: &[(&str, &[&str], bool)] = &[
        ("wl-copy", &[], wayland),
        ("xclip", &["-selection", "clipboard"], x11),
        ("xsel", &["--clipboard", "--input"], x11),
        ("termux-clipboard-set", &[], true),
    ];

    let mut tried = Vec::new();
    for (bin, args, usable) in candidates {
        if !usable {
            continue;
        }
        // stdout/stderr must not be pipes: these tools fork a background
        // server that would hold them open.
        let Ok(mut child) = Command::new(bin)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        tried.push(*bin);
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        if child.wait()?.success() {
            return Ok(());
        }
    }

    if !wayland && !x11 {
        bail!("no graphical session (neither WAYLAND_DISPLAY nor DISPLAY is set)");
    }
    if tried.is_empty() {
        bail!("no clipboard tool found (install wl-clipboard, xclip, or xsel)");
    }
    bail!("clipboard tool failed: {}", tried.join(", "))
}

#[cfg(not(target_os = "linux"))]
pub fn copy(text: &str) -> Result<()> {
    match arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_owned())) {
        Ok(()) => Ok(()),
        Err(e) => bail!("{e}"),
    }
}
