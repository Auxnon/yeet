mod clipboard;
mod send;
mod tui;
mod warehouse;

use std::fs::File;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use send::{Config, Via};
use warehouse::{Item, Kind};

/// Yeet files and text to other machines on your LAN.
///
/// Run with no arguments to pick pending items: files move into the current
/// directory, text goes to the clipboard. When stdout is piped, the most recent
/// item is written to stdout instead (`yeet | jq .`).
#[derive(Parser)]
#[command(version, about, long_about)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Send files, folders, or text to another machine's warehouse
    #[command(visible_alias = "s")]
    Send {
        /// Files or folders to send
        paths: Vec<PathBuf>,
        /// Send this text (stdin is used when no paths or text are given)
        #[arg(short, long)]
        text: Option<String>,
        /// Destination: an ssh host (user@host or ~/.ssh/config alias), or `local`
        #[arg(long, env = "YEET_TO")]
        to: Option<String>,
        /// Transfer tool
        #[arg(long, value_enum)]
        via: Option<Via>,
        /// Warehouse path on the remote, relative to its home directory
        #[arg(long)]
        remote_dir: Option<String>,
    },
    /// Write the most recent item to stdout and remove it from the warehouse
    Pop {
        /// Leave the item in the warehouse
        #[arg(short, long)]
        keep: bool,
    },
    /// List pending items without the TUI
    #[command(visible_alias = "ls")]
    List,
    /// Delete every pending item
    Clear,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("yeet: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.cmd {
        Some(Cmd::Send {
            paths,
            text,
            to,
            via,
            remote_dir,
        }) => {
            let config = Config::load()?;
            let to = to.or(config.to).context(
                "no destination: pass --to <host>, set YEET_TO, or set `to` in ~/.config/yeet/config.toml",
            )?;
            send::run(send::SendOpts {
                paths,
                text,
                to,
                via: via.or(config.via).unwrap_or(Via::Auto),
                remote_dir: remote_dir
                    .or(config.remote_dir)
                    .unwrap_or_else(|| send::DEFAULT_REMOTE_DIR.into()),
            })?;
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Pop { keep }) => pop(keep),
        Some(Cmd::List) => {
            for item in warehouse::list()? {
                println!(
                    "{}\t{}\t{}\t{}",
                    item.id,
                    warehouse::human_age(item.meta.sent_at),
                    item.meta.from,
                    warehouse::describe(&item)
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Some(Cmd::Clear) => {
            let items = warehouse::list()?;
            for item in &items {
                item.remove()?;
            }
            eprintln!("cleared {} item(s)", items.len());
            Ok(ExitCode::SUCCESS)
        }
        None if io::stdout().is_terminal() => receive(),
        None => pop(false),
    }
}

fn receive() -> Result<ExitCode> {
    let items = warehouse::list()?;
    if items.is_empty() {
        eprintln!("nothing pending in {}", warehouse::root()?.display());
        return Ok(ExitCode::SUCCESS);
    }
    let Some(chosen) = tui::pick(items)? else {
        return Ok(ExitCode::SUCCESS);
    };

    let cwd = std::env::current_dir()?;
    let mut failed = false;
    let mut texts = Vec::new();
    // Oldest first, so files and joined text keep their send order.
    for item in chosen.into_iter().rev() {
        match item.meta.kind {
            Kind::File => match item.move_into(&cwd) {
                Ok(dest) => eprintln!("→ {}", dest.display()),
                Err(e) => {
                    eprintln!("yeet: {}: {e:#}", warehouse::describe(&item));
                    failed = true;
                }
            },
            Kind::Text => texts.push(item),
        }
    }

    if !texts.is_empty() {
        let joined = texts
            .iter()
            .map(Item::read_text)
            .collect::<Result<Vec<_>>>()?
            .join("\n");
        match clipboard::copy(&joined) {
            Ok(()) => {
                for item in &texts {
                    item.remove()?;
                }
                eprintln!(
                    "→ copied {} to clipboard",
                    warehouse::human_size(joined.len() as u64)
                );
            }
            Err(e) => {
                eprintln!("yeet: could not copy to clipboard: {e:#}");
                eprintln!("yeet: text left in the warehouse; run `yeet pop` to print it");
                failed = true;
            }
        }
    }

    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn pop(keep: bool) -> Result<ExitCode> {
    let Some(item) = warehouse::list()?.into_iter().next() else {
        eprintln!("yeet: nothing pending");
        return Ok(ExitCode::FAILURE);
    };
    if item.meta.is_dir {
        bail!(
            "most recent item `{}` is a folder; run `yeet` in a terminal to receive it",
            warehouse::describe(&item)
        );
    }

    let mut src = File::open(item.payload_path()).context("opening item")?;
    let mut out = io::stdout().lock();
    match io::copy(&mut src, &mut out).and_then(|_| out.flush()) {
        Ok(()) => {}
        // Downstream closed early (`yeet | head`): leave the item so nothing is lost.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(ExitCode::FAILURE),
        Err(e) => return Err(e.into()),
    }
    if !keep {
        item.remove()?;
    }
    Ok(ExitCode::SUCCESS)
}
