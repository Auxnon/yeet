mod clipboard;
mod dest;
mod net;
mod send;
mod tui;
mod warehouse;

use std::fs::{self, File};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;

use dest::Destination;
use send::{Config, Via};
use warehouse::{Item, Kind};

/// Yeet files and text to other machines on your LAN.
///
///   yeet FILE...        send files/folders
///   yeet -t "text"      send raw text (`-t` alone reads stdin)
///   yeet                pick pending items: files move into the current
///                       directory, text goes to the clipboard
///   yeet | cmd          write the most recent item to stdout
#[derive(Parser)]
#[command(version, about, long_about, verbatim_doc_comment)]
struct Cli {
    /// Files or folders to send
    paths: Vec<PathBuf>,

    /// Send raw text; with no value, read it from stdin
    #[arg(short, long, num_args = 0..=1, default_missing_value = "-")]
    text: Option<String>,

    /// Destination: an ssh host (user@host or ~/.ssh/config alias), or `local`.
    /// Without it, a picker of saved destinations opens
    #[arg(long, env = "YEET_TO")]
    to: Option<String>,

    /// Transfer tool
    #[arg(long, value_enum)]
    via: Option<Via>,

    /// Warehouse path on the remote, relative to its home directory
    #[arg(long)]
    remote_dir: Option<String>,

    /// Write the most recent item to stdout and remove it from the warehouse
    #[arg(short, long, conflicts_with_all = ["paths", "text", "list", "clear"])]
    pop: bool,

    /// With --pop (or when piped): leave the item in the warehouse
    #[arg(short, long, conflicts_with_all = ["paths", "text"])]
    keep: bool,

    /// List pending items without the TUI
    #[arg(short, long, conflicts_with_all = ["paths", "text", "clear"])]
    list: bool,

    /// Delete every pending item
    #[arg(long, conflicts_with_all = ["paths", "text"])]
    clear: bool,
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
    if !cli.paths.is_empty() || cli.text.is_some() {
        return send(cli).map(|()| ExitCode::SUCCESS);
    }

    if cli.list {
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
    } else if cli.clear {
        let items = warehouse::list()?;
        for item in &items {
            item.remove()?;
        }
        eprintln!("cleared {} item(s)", items.len());
        Ok(ExitCode::SUCCESS)
    } else if cli.pop || cli.keep || !io::stdout().is_terminal() {
        pop(cli.keep)
    } else {
        receive()
    }
}

fn send(cli: Cli) -> Result<()> {
    // Resolve everything that can fail before asking where to send it.
    let text = send::resolve_text(cli.text)?;
    for path in &cli.paths {
        if fs::symlink_metadata(path).is_err() {
            bail!("{} not found", path.display());
        }
    }

    let mut config = Config::load()?;
    let to = match cli.to {
        Some(to) => Destination::parse(&to),
        None => {
            if !io::stdout().is_terminal() {
                bail!(
                    "no destination: pass --to <host> or set YEET_TO (the picker needs a terminal)"
                );
            }
            let local = net::detect();
            let pick = tui::pick_destination(
                config.destinations.clone(),
                &send::summary(&cli.paths, text.as_deref()),
                &local
                    .as_ref()
                    .map(net::LocalNet::prefill)
                    .unwrap_or_default(),
                local.as_ref().map(ToString::to_string).as_deref(),
            )?;
            if pick.destinations != config.destinations {
                config.destinations = pick.destinations;
                config.save()?;
            }
            match pick.chosen {
                Some(to) => to,
                None => return Ok(()),
            }
        }
    };

    send::run(send::SendOpts {
        paths: cli.paths,
        text,
        to: to.target(),
        via: cli.via.or(config.via).unwrap_or(Via::Auto),
        remote_dir: cli
            .remote_dir
            .or(config.remote_dir.clone())
            .unwrap_or_else(|| send::DEFAULT_REMOTE_DIR.into()),
    })?;

    // Only remember destinations that actually worked, so typos don't pile up.
    if to.host != "local" {
        config.remember(&to);
        config.save()?;
    }
    Ok(())
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
