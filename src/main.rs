//! `colophon` CLI — decode wintermute provfs provenance xattrs.
//!
//! # Usage
//!
//! ```text
//! colophon parse <file>
//! colophon parse --from-string '<session-xattr-value>'
//! colophon parse --format json <file>
//! ```
#![allow(clippy::print_stdout, clippy::print_stderr)]

use clap::{Parser, Subcommand};
use colophon::{parse, read_file, Provenance};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "colophon",
    about = "Decode wintermute kernel provenance xattrs (user.prov.session / user.prov.ts)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse and display provenance for a file or a literal xattr string.
    Parse {
        /// Path to the file to read xattrs from. Mutually exclusive with `--from-string`.
        #[arg(value_name = "FILE", conflicts_with = "from_string")]
        file: Option<PathBuf>,

        /// Parse a literal `user.prov.session` value (for testing / piped `getfattr` output).
        #[arg(long, value_name = "XATTR_VALUE")]
        from_string: Option<String>,

        /// Output format.
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum Format {
    Text,
    Json,
}

fn main() -> std::process::ExitCode {
    // SIGPIPE: must be the very first act before any I/O.
    // Prevents panics when stdout is closed (e.g. `colophon parse <file> | head`).
    sigpipe::reset();

    let cli = Cli::parse();

    match cli.command {
        Commands::Parse {
            file,
            from_string,
            format,
        } => {
            let prov: Provenance = if let Some(val) = from_string {
                parse(&val, None)
            } else if let Some(path) = file {
                match read_file(&path) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("colophon: error reading {}: {e}", path.display());
                        return std::process::ExitCode::FAILURE;
                    }
                }
            } else {
                eprintln!("colophon parse: provide FILE or --from-string");
                return std::process::ExitCode::FAILURE;
            };

            match format {
                Format::Json => {
                    match serde_json::to_string_pretty(&prov) {
                        Ok(s) => println!("{s}"),
                        Err(e) => {
                            eprintln!("colophon: serialization error: {e}");
                            return std::process::ExitCode::FAILURE;
                        }
                    }
                }
                Format::Text => print_text(&prov),
            }

            std::process::ExitCode::SUCCESS
        }
    }
}

fn print_text(p: &Provenance) {
    println!("form:        {:?}", p.form);
    if !p.comm_chain.is_empty() {
        println!("comm-chain:  {}", p.comm_chain.join(" > "));
    }
    if let Some(skill) = p.originating_skill() {
        println!("skill:       {:?}", skill);
    }
    if let Some(cwd) = &p.cwd {
        println!("cwd:         {cwd}");
    }
    if let Some(pid) = p.pid {
        println!("pid:         {pid}");
    }
    if let Some(uid) = p.uid {
        println!("uid:         {uid}");
    }
    if let Some(id) = &p.agent_session {
        println!("agent-id:    {id}");
    }
    if let Some(ts) = p.ts {
        println!("ts:          {ts}");
    }
    if !p.env.is_empty() {
        for (k, v) in &p.env {
            println!("env:         {k}={v}");
        }
    }
    println!("raw:         {}", p.raw);
}
