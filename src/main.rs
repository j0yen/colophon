//! `colophon` CLI — decode wintermute provfs provenance xattrs.
//!
//! # Usage
//!
//! ```text
//! colophon parse <file>
//! colophon parse --from-string '<session-xattr-value>'
//! colophon parse --format json <file>
//! colophon stale <dir>
//! colophon stale --consumer <binary> --format json <dir>
//! colophon attribute <dir>
//! colophon attribute <dir> --format json --top 5 --by skill
//! ```
#![allow(clippy::print_stdout, clippy::print_stderr)]

use clap::{Parser, Subcommand};
use colophon::attribute::{attribute, AttributeOpts, GroupBy};
use colophon::{parse, read_file, stale, Provenance, StaleOpts, StaleReason, Staleness};
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

    /// Walk a directory and flag files whose provenance shows the writing
    /// session is dead or whose prov.ts predates the consuming binary.
    Stale {
        /// Directory to walk.
        #[arg(value_name = "DIR")]
        dir: PathBuf,

        /// Output format.
        #[arg(long, value_enum, default_value = "text")]
        format: Format,

        /// Path to a consumer binary; enables the OlderThanConsumer check.
        #[arg(long, value_name = "BINARY")]
        consumer: Option<PathBuf>,

        /// Minimum age (in seconds) a file's prov.ts must be before the writing
        /// pid is checked for liveness.  Default: 3600 (1 h).
        #[arg(long, value_name = "SECS", default_value = "3600")]
        min_age: u64,

        /// Show only verdicts matching this reason.
        #[arg(long, value_enum, value_name = "REASON")]
        reason: Option<ReasonFilter>,
    },

    /// Walk a directory tree and report which sessions/skills wrote each file.
    Attribute {
        /// Root directory to walk.
        #[arg(value_name = "DIR")]
        dir: PathBuf,

        /// Output format.
        #[arg(long, value_enum, default_value = "text")]
        format: Format,

        /// Show only the top-N actor buckets (headline still counts all files).
        #[arg(long, default_value_t = 10)]
        top: usize,

        /// Suppress buckets smaller than this many bytes.
        #[arg(long, default_value_t = 0)]
        min_bytes: u64,

        /// Grouping key for actor buckets.
        #[arg(long, value_enum, default_value = "skill")]
        by: GroupBy,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum Format {
    Text,
    Json,
}

/// Filter for the `--reason` flag of the `stale` subcommand.
#[derive(Clone, clap::ValueEnum)]
enum ReasonFilter {
    WriterDead,
    Older,
    Any,
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

        Commands::Stale {
            dir,
            format,
            consumer,
            min_age,
            reason,
        } => run_stale(dir, format, consumer, min_age, reason),

        Commands::Attribute {
            dir,
            format,
            top,
            min_bytes,
            by,
        } => run_attribute(dir, format, top, min_bytes, by),
    }
}

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
        Format::Text => print_prov_text(&prov),
    }

    std::process::ExitCode::SUCCESS
}

fn run_stale(
    dir: PathBuf,
    format: Format,
    consumer: Option<PathBuf>,
    min_age: u64,
    reason_filter: Option<ReasonFilter>,
) -> std::process::ExitCode {
    let opts = StaleOpts {
        min_age_secs: min_age,
        consumer,
        skip_prefixes: vec![],
    };

    let mut verdicts = stale(&dir, &opts);

    // Apply reason filter.
    if let Some(filter) = reason_filter {
        verdicts.retain(|v| match filter {
            ReasonFilter::WriterDead => {
                matches!(v.reason, StaleReason::WriterDead | StaleReason::Both)
            }
            ReasonFilter::Older => {
                matches!(v.reason, StaleReason::OlderThanConsumer | StaleReason::Both)
            }
            ReasonFilter::Any => true,
        });
    }

    match format {
        Format::Json => match serde_json::to_string_pretty(&verdicts) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("colophon: serialization error: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
        Format::Text => {
            for v in &verdicts {
                print_staleness_text(v);
            }
        }
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
    if !dir.exists() {
        eprintln!("colophon attribute: directory does not exist: {}", dir.display());
        return std::process::ExitCode::FAILURE;
    }

    let opts = AttributeOpts {
        max_depth: 64,
        top_n: top,
        min_bytes,
        group_by: by,
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
        Format::Text => colophon::attribute::print_text(&report, top),
    }

    std::process::ExitCode::SUCCESS
}

fn print_prov_text(p: &Provenance) {
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

fn print_staleness_text(v: &Staleness) {
    println!(
        "{}: {:?} (age {}d)",
        v.path.display(),
        v.reason,
        v.age_days
    );
    if let Some(pid) = v.prov.pid {
        println!("  pid:  {pid}");
    }
    if let Some(ts) = v.prov.ts {
        println!("  ts:   {ts}");
    }
    if !v.prov.comm_chain.is_empty() {
        let chain = v.prov.comm_chain.join(" > ");
        println!("  chain: {chain}");
    }
}
