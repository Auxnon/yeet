//! Staging items locally and shipping them to another machine's warehouse.

use std::fs;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use serde::Deserialize;

use crate::warehouse::{self, Kind, Meta};

pub const DEFAULT_REMOTE_DIR: &str = ".yeet/warehouse";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    /// rsync if installed, falling back to scp
    Auto,
    Rsync,
    Scp,
}

/// `~/.config/yeet/config.toml`
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub to: Option<String>,
    pub via: Option<Via>,
    pub remote_dir: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let Some(path) = dirs::config_dir().map(|d| d.join("yeet").join("config.toml")) else {
            return Ok(Self::default());
        };
        match fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", path.display())),
            Err(_) => Ok(Self::default()),
        }
    }
}

pub struct SendOpts {
    pub paths: Vec<PathBuf>,
    pub text: Option<String>,
    pub to: String,
    pub via: Via,
    pub remote_dir: String,
}

pub fn run(opts: SendOpts) -> Result<()> {
    let mut text = opts.text.clone();
    // `-t` with no value (or `-t -`) means read the text from stdin.
    if text.as_deref() == Some("-") {
        let mut stdin = std::io::stdin();
        if stdin.is_terminal() {
            bail!("-t with no text reads stdin, but stdin is a terminal");
        }
        let mut buf = String::new();
        stdin.read_to_string(&mut buf).context("reading stdin")?;
        text = Some(buf);
    }
    if text.as_deref() == Some("") {
        bail!("refusing to send empty text");
    }

    let batch = warehouse::new_id();
    let stage = warehouse::yeet_home()?.join("outbox").join(&batch);
    fs::create_dir_all(&stage)?;
    let result = stage_and_ship(&stage, &batch, &opts.paths, text.as_deref(), &opts);
    // The outbox copy is disposable either way.
    let _ = fs::remove_dir_all(&stage);
    result
}

fn stage_and_ship(
    stage: &Path,
    batch: &str,
    paths: &[PathBuf],
    text: Option<&str>,
    opts: &SendOpts,
) -> Result<()> {
    let from = gethostname::gethostname().to_string_lossy().into_owned();
    let mut labels = Vec::new();
    // Items within one batch get distinct, ordered ids: batch id + sequence.
    let mut seq = 0;
    let mut next_dir = || {
        seq += 1;
        stage.join(format!("{batch}-{seq:03}"))
    };

    for path in paths {
        let name = path
            .canonicalize()
            .with_context(|| format!("{} not found", path.display()))?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .context("path has no file name")?;
        let dir = next_dir();
        fs::create_dir_all(dir.join("payload"))?;
        warehouse::copy_tree(path, &dir.join("payload").join(&name), true)?;
        let meta = Meta {
            kind: Kind::File,
            name: Some(name.clone()),
            is_dir: path.is_dir(),
            size: warehouse::tree_size(path),
            from: from.clone(),
            sent_at: warehouse::now_millis(),
            preview: None,
        };
        write_meta(&dir, &meta)?;
        labels.push(name);
    }

    if let Some(text) = text {
        let dir = next_dir();
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("text"), text)?;
        let preview: String = text
            .trim()
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(80)
            .collect();
        let meta = Meta {
            kind: Kind::Text,
            name: None,
            is_dir: false,
            size: text.len() as u64,
            from: from.clone(),
            sent_at: warehouse::now_millis(),
            preview: Some(preview),
        };
        write_meta(&dir, &meta)?;
        labels.push(format!(
            "text ({})",
            warehouse::human_size(text.len() as u64)
        ));
    }

    if opts.to == "local" {
        let root = warehouse::root()?;
        fs::create_dir_all(&root)?;
        for entry in fs::read_dir(stage)? {
            let entry = entry?;
            warehouse::move_path(&entry.path(), &root.join(entry.file_name()))?;
        }
    } else {
        ship(stage, batch, opts)?;
    }

    for label in labels {
        eprintln!("yeeted {label} → {}", opts.to);
    }
    Ok(())
}

fn write_meta(dir: &Path, meta: &Meta) -> Result<()> {
    fs::write(dir.join("meta.json"), serde_json::to_vec_pretty(meta)?)?;
    Ok(())
}

/// Upload the staged batch into `<remote_dir>/.incoming/<batch>` and then move
/// its items into the warehouse in one step, so a receiver never sees a
/// half-transferred item.
fn ship(stage: &Path, batch: &str, opts: &SendOpts) -> Result<()> {
    let rd = opts
        .remote_dir
        .strip_prefix("~/")
        .unwrap_or(&opts.remote_dir);
    let incoming = format!("{rd}/.incoming");

    let via = match opts.via {
        Via::Auto if which("rsync") => Via::Rsync,
        Via::Auto => Via::Scp,
        v => v,
    };

    let uploaded = match via {
        Via::Rsync => {
            let ok = rsync(stage, &incoming, opts)?;
            if !ok && opts.via == Via::Auto {
                eprintln!("yeet: rsync failed, retrying with scp");
                scp(stage, &incoming, opts)?
            } else {
                ok
            }
        }
        _ => scp(stage, &incoming, opts)?,
    };
    if !uploaded {
        bail!("transfer to {} failed", opts.to);
    }

    let finalize = format!(
        "cd {} && mv .incoming/{batch}/* . && rmdir .incoming/{batch}",
        sh_quote(rd)
    );
    if !ssh(&opts.to, &finalize)? {
        bail!("upload succeeded but moving items into the remote warehouse failed");
    }
    Ok(())
}

fn rsync(stage: &Path, incoming: &str, opts: &SendOpts) -> Result<bool> {
    let mut cmd = Command::new("rsync");
    cmd.arg("-a")
        .arg("--rsync-path")
        .arg(format!("mkdir -p {} && rsync", sh_quote(incoming)));
    if std::io::stderr().is_terminal() {
        cmd.arg("--info=progress2");
    }
    cmd.arg(stage).arg(format!("{}:{incoming}/", opts.to));
    Ok(cmd.status().context("running rsync")?.success())
}

fn scp(stage: &Path, incoming: &str, opts: &SendOpts) -> Result<bool> {
    if !ssh(&opts.to, &format!("mkdir -p {}", sh_quote(incoming)))? {
        return Ok(false);
    }
    let status = Command::new("scp")
        .args(["-r", "-S", "ssh"])
        .arg(stage)
        .arg(format!("{}:{incoming}/", opts.to))
        .status()
        .context("running scp")?;
    Ok(status.success())
}

fn ssh(host: &str, remote_cmd: &str) -> Result<bool> {
    let status = Command::new("ssh")
        .arg(host)
        .arg(remote_cmd)
        .status()
        .context("running ssh")?;
    Ok(status.success())
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::sh_quote;

    #[test]
    fn quoting() {
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }
}
