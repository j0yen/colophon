//! `colophon` CLI — decode wintermute provfs provenance xattrs.
//!
//! # Usage
//!
//! ```text
//! colophon parse <file>
//! colophon parse --from-string '<session-xattr-value>'
//! colophon parse --format json <file>
//! colophon attribute <dir>
//! colophon attribute <dir> --format json --top 5 --by skill
//! colophon stale <dir>
//! colophon stale <dir> --consumer /usr/bin/colophon --format json
//! colophon digest
//! colophon digest --cruft-root ~/.cache/build-worktrees --config-root ~/.claude --format markdown
//! ```
#![allow(clippy::print_stdout, clippy::print_stderr)]

use clap::{Parser, Subcommand};
use colophon::attribute::{attribute, print_text as print_attribution, AttributeOpts, GroupBy};
use colophon::digest::{digest, render_markdown, DigestOpts};
use colophon::stale::{print_text as print_stale, stale, StaleOpts};
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

    /// Walk a directory tree and report which sessions/skills wrote each file.
    Attribute {
        /// Root directory to walk.
        #[arg(value_name = "DIR")]
        dir: PathBuf,

        /// Output format.
        #[arg(long, value_enum, default_value = "text")]
        format: Format,

        /// Maximum actor buckets to display.
        #[arg(long, default_value_t = 10)]
        top: usize,

        /// Suppress actors with fewer total bytes than this.
        #[arg(long, default_value_t = 0)]
        min_bytes: u64,

        /// Grouping key for actor buckets.
        #[arg(long, value_enum, default_value = "skill")]
        by: GroupBy,
    },

    /// Walk a directory and flag files whose writer is dead or stale.
    Stale {
        /// Root directory to walk.
        #[arg(value_name = "DIR")]
        dir: PathBuf,

        /// Output format.
        #[arg(long, value_enum, default_value = "text")]
        format: Format,

        /// Optional consumer binary path; files older than its mtime are flagged OlderThanConsumer.
        #[arg(long, value_name = "PATH")]
        consumer: Option<PathBuf>,

        /// Minimum age in seconds before a file with a dead pid is flagged (pid-reuse guard).
        #[arg(long, default_value_t = 3600)]
        min_age: u64,
    },

    /// Compose attribution + staleness into a single provenance digest block.
    Digest {
        /// Output format.
        #[arg(long, value_enum, default_value = "markdown")]
        format: DigestFormat,

        /// Root(s) to attribute (cruft dirs). Repeatable.
        #[arg(long, value_name = "PATH")]
        cruft_root: Vec<PathBuf>,

        /// Root(s) to check for staleness (config/state dirs). Repeatable.
        #[arg(long, value_name = "PATH")]
        config_root: Vec<PathBuf>,

        /// Maximum actor buckets and stale entries in the output.
        #[arg(long, default_value_t = 10)]
        top: usize,

        /// Optional consumer binary path for the OlderThanConsumer staleness check.
        #[arg(long, value_name = "PATH")]
        consumer: Option<PathBuf>,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Clone, clap::ValueEnum)]
enum DigestFormat {
    Markdown,
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
        } => run_parse(file, from_string, format),

        Commands::Attribute {
            dir,
            format,
            top,
            min_bytes,
            by,
        } => run_attribute(dir, format, top, min_bytes, by),

        Commands::Stale {
            dir,
            format,
            consumer,
            min_age,
        } => run_stale(dir, format, consumer, min_age),

        Commands::Digest {
            format,
            cruft_root,
            config_root,
            top,
            consumer,
        } => run_digest(format, cruft_root, config_root, top, consumer),
    }
}

// ── Subcommand handlers ───────────────────────────────────────────────────────

fn run_parse(
    file: Option<PathBuf>,
    from_string: Option<String>,
    format: Format,
) -> std::process::ExitCode {
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
        Format::Json => match serde_json::to_string_pretty(&prov) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("colophon: serialization error: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
        Format::Text => print_text(&prov),
    }

    std::process::ExitCode::SUCCESS
}

fn run_attribute(
    dir: PathBuf,
    format: Format,
    top: usize,
    min_bytes: u64,
    by: GroupBy,
) -> std::process::ExitCode {
    let opts = AttributeOpts {
        top_n: top,
        min_bytes,
        group_by: by,
        ..AttributeOpts::default()
    };
    let report = attribute(&dir, &opts);

    match format {
        Format::Json => match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("colophon: serialization error: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
        Format::Text => print_attribution(&report, top),
    }

    std::process::ExitCode::SUCCESS
}

fn run_stale(
    dir: PathBuf,
    format: Format,
    consumer: Option<PathBuf>,
    min_age: u64,
) -> std::process::ExitCode {
    let opts = StaleOpts {
        min_age_secs: min_age,
        consumer,
        ..StaleOpts::default()
    };
    let entries = stale(&dir, &opts);

    match format {
        Format::Json => match serde_json::to_string_pretty(&entries) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("colophon: serialization error: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
        Format::Text => print_stale(&entries),
    }

    std::process::ExitCode::SUCCESS
}

fn run_digest(
    format: DigestFormat,
    cruft_root: Vec<PathBuf>,
    config_root: Vec<PathBuf>,
    top: usize,
    consumer: Option<PathBuf>,
) -> std::process::ExitCode {
    // Default roots if none provided.
    let cruft_roots = if cruft_root.is_empty() {
        default_cruft_roots()
    } else {
        cruft_root
    };
    let config_roots = if config_root.is_empty() {
        default_config_roots()
    } else {
        config_root
    };

    let opts = DigestOpts {
        cruft_roots,
        config_roots,
        top,
        consumer,
    };

    let d = digest(&opts);

    match format {
        DigestFormat::Markdown => {
            let md = render_markdown(&d, top);
            print!("{md}");
        }
        DigestFormat::Json => match serde_json::to_string_pretty(&d) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("colophon: serialization error: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
    }

    std::process::ExitCode::SUCCESS
}

// ── Default roots ─────────────────────────────────────────────────────────────

fn default_cruft_roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/jsy".to_owned());
    vec![
        PathBuf::from(&home).join(".cache/build-worktrees"),
        PathBuf::from(&home).join(".claude/projects"),
    ]
}

fn default_config_roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/jsy".to_owned());
    // ~/.claude top-level state only (projects dir is excluded via config to keep it fast).
    vec![PathBuf::from(&home).join(".claude")]
}

// ── Parse text printer ────────────────────────────────────────────────────────

fn print_text(p: &Provenance) {
    println!("form:        {:?}", p.form);
    if !p.comm_chain.is_empty() {
        let chain = p.comm_chain.join(" > ");
        println!("comm-chain:  {chain}");
    }
    if let Some(skill) = p.originating_skill() {
        println!("skill:       {skill:?}");
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
