//! `greenlit` — the command-line surface for the Greenlit Goods engine.
//!
//! Two commands:
//!   greenlit audit <feed> [--json] [--country US] [--assumed-monthly-sales N] [--strict]
//!   greenlit fix   <feed> [-o out] [--json] [--country US]
//!
//! The CLI is a thin shell: all analysis lives in `greenlit-engine`. Its only
//! jobs are reading files, picking colors when attached to a terminal, printing,
//! and choosing an exit code.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use greenlit_engine::report::render_terminal;
use greenlit_engine::{audit, fix, AuditOptions, Destination, FixOptions, Format};

#[derive(Parser)]
#[command(
    name = "greenlit",
    version,
    about = "Diagnose and fix Google Merchant Center product feeds."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Audit a feed and print a report.
    Audit(AuditArgs),
    /// Apply safe automatic fixes and write a corrected feed.
    Fix(FixArgs),
}

#[derive(Copy, Clone, ValueEnum)]
enum FormatArg {
    Xml,
    Csv,
}

#[derive(Copy, Clone, ValueEnum)]
enum DestinationArg {
    ShoppingAds,
    FreeListings,
}

impl From<DestinationArg> for Destination {
    fn from(destination: DestinationArg) -> Self {
        match destination {
            DestinationArg::ShoppingAds => Destination::ShoppingAds,
            DestinationArg::FreeListings => Destination::FreeListings,
        }
    }
}

impl From<FormatArg> for Format {
    fn from(f: FormatArg) -> Self {
        match f {
            FormatArg::Xml => Format::Xml,
            FormatArg::Csv => Format::Csv,
        }
    }
}

#[derive(Args)]
struct CommonArgs {
    /// Force the input format instead of auto-detecting.
    #[arg(long, value_enum)]
    format: Option<FormatArg>,
    /// ISO 3166-1 alpha-2 target market (affects conditional requirements).
    #[arg(long, default_value = "US")]
    country: String,
    /// Emit machine-readable JSON instead of the human report.
    #[arg(long)]
    json: bool,
    /// Never colorize output.
    #[arg(long)]
    no_color: bool,
    /// Google surface whose conditional requirements should be applied.
    #[arg(long, value_enum, default_value = "shopping-ads")]
    destination: DestinationArg,
}

#[derive(Args)]
struct AuditArgs {
    /// Path to the feed file (XML/RSS or CSV/TSV).
    feed: PathBuf,
    #[command(flatten)]
    common: CommonArgs,
    /// Assumed monthly unit sales per affected product (revenue-at-risk input).
    #[arg(long, default_value_t = 1.0, value_parser = parse_assumed_sales)]
    assumed_monthly_sales: f64,
    /// Exit non-zero if any product would be disapproved (useful in CI).
    #[arg(long)]
    strict: bool,
}

#[derive(Args)]
struct FixArgs {
    /// Path to the feed file (XML/RSS or CSV/TSV).
    feed: PathBuf,
    /// Output path for the corrected feed (default: <input>.fixed.<ext>).
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[command(flatten)]
    common: CommonArgs,
    /// Assumed monthly unit sales per affected product (revenue-at-risk input).
    #[arg(long, default_value_t = 1.0, value_parser = parse_assumed_sales)]
    assumed_monthly_sales: f64,
}

fn resolve_format(common: &CommonArgs, bytes: &[u8]) -> Result<Format> {
    if let Some(f) = common.format {
        return Ok(f.into());
    }
    greenlit_engine::parse::detect_format(bytes)
        .map(Ok)
        .unwrap_or_else(|| {
            Err(anyhow::anyhow!(
                "could not detect format; pass --format xml|csv"
            ))
        })
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        Command::Audit(args) => run_audit(args),
        Command::Fix(args) => run_fix(args),
    }
}

fn run_audit(args: AuditArgs) -> Result<ExitCode> {
    let bytes = read_feed(&args.feed)?;
    let format = resolve_format(&args.common, &bytes)?;

    let opts = AuditOptions {
        target_country: args.common.country.clone(),
        assumed_monthly_sales: args.assumed_monthly_sales,
        destination: args.common.destination.into(),
    };
    let report = audit(&bytes, format, &opts).context("auditing feed")?;

    if args.common.json {
        let json = serde_json::to_string_pretty(&report)?;
        println!("{json}");
    } else {
        let color = use_color(&args.common);
        print!("{}", render_terminal(&report, color));
    }

    if args.strict && strict_failure(&report) {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

fn run_fix(args: FixArgs) -> Result<ExitCode> {
    let bytes = read_feed(&args.feed)?;
    let format = resolve_format(&args.common, &bytes)?;

    let opts = FixOptions {
        audit: AuditOptions {
            target_country: args.common.country.clone(),
            assumed_monthly_sales: args.assumed_monthly_sales,
            destination: args.common.destination.into(),
        },
    };
    let result = fix(&bytes, format, &opts).context("fixing feed")?;

    let out_path = args
        .output
        .unwrap_or_else(|| default_output_path(&args.feed, format));
    write_new_atomic(&out_path, &result.corrected_feed)?;

    if args.common.json {
        // Structured: the change log plus before/after scores.
        let payload = serde_json::json!({
            "schema_version": 1,
            "output": out_path.display().to_string(),
            "fixes_applied": result.log.len(),
            "score_before": result.before.greenlight_score,
            "score_after": result.after.greenlight_score,
            "log": result.log,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        let color = use_color(&args.common);
        print_fix_summary(&result, &out_path, color);
    }

    Ok(ExitCode::SUCCESS)
}

fn print_fix_summary(result: &greenlit_engine::FixResult, out_path: &Path, color: bool) {
    let g = |s: &str| {
        if color {
            format!("\x1b[32m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };
    let b = |s: &str| {
        if color {
            format!("\x1b[1m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    };

    println!();
    println!("  {}", b("GREENLIT GOODS — fix"));
    println!("  ====================");
    println!();
    println!(
        "  Applied {} safe fix(es) across {} field(s).",
        result.log.len(),
        result.log.len()
    );
    println!(
        "  Greenlight Score: {} -> {}",
        display_score(result.before.greenlight_score),
        g(&display_score(result.after.greenlight_score))
    );
    println!(
        "  Disapprovals: {} -> {}   At-risk: {} -> {}",
        result.before.total_disapprovals,
        result.after.total_disapprovals,
        result.before.total_at_risk,
        result.after.total_at_risk,
    );
    println!();

    if !result.log.is_empty() {
        println!("  Changes (first 12):");
        for r in result.log.iter().take(12) {
            let id = terminal_safe(r.product_id.as_deref().unwrap_or("(no id)"));
            println!(
                "  • [{}] {}: \"{}\" -> \"{}\"  ({})",
                id,
                r.field,
                ellipsize(&r.before, 32),
                ellipsize(&r.after, 32),
                r.fixer_id
            );
        }
        if result.log.len() > 12 {
            println!(
                "  … and {} more (use --json for the full log).",
                result.log.len() - 12
            );
        }
        println!();
    }

    println!(
        "  Corrected feed written to: {}",
        terminal_safe(&out_path.display().to_string())
    );
    println!(
        "  Note: only the documented conservative fixes were applied. Remaining issues need review — \
         re-run `greenlit audit` on the output to see them."
    );
    println!();
    let _ = std::io::stdout().flush();
}

fn default_output_path(input: &Path, format: Format) -> PathBuf {
    let ext = match format {
        Format::Xml => "xml",
        Format::Csv => match input.extension().and_then(|value| value.to_str()) {
            Some(value) if value.eq_ignore_ascii_case("tsv") => "tsv",
            Some(value) if value.eq_ignore_ascii_case("txt") => "txt",
            _ => "csv",
        },
    };
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("feed");
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{stem}.fixed.{ext}"))
}

fn parse_assumed_sales(raw: &str) -> std::result::Result<f64, String> {
    let value = raw
        .parse::<f64>()
        .map_err(|_| "must be a number".to_string())?;
    if !value.is_finite() || value < 0.0 {
        return Err("must be finite and non-negative".to_string());
    }
    if value > greenlit_engine::MAX_ASSUMED_MONTHLY_SALES {
        return Err("is too large to keep revenue calculations finite".to_string());
    }
    Ok(value)
}

fn strict_failure(report: &greenlit_engine::Report) -> bool {
    !report.structurally_complete || report.red > 0
}

fn display_score(score: Option<u32>) -> String {
    score.map_or_else(|| "N/A".to_string(), |value| value.to_string())
}

/// Persist a new output without ever replacing an existing path. The temporary
/// file lives beside the destination, so the final persist is same-filesystem.
fn write_new_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        return Err(anyhow!(
            "refusing to overwrite existing output {}; choose a new path",
            path.display()
        ));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating temporary output beside {}", path.display()))?;
    temp.write_all(bytes)
        .with_context(|| format!("writing temporary output for {}", path.display()))?;
    temp.flush()
        .with_context(|| format!("flushing temporary output for {}", path.display()))?;
    temp.as_file()
        .sync_all()
        .with_context(|| format!("syncing temporary output for {}", path.display()))?;
    temp.persist_noclobber(path).map_err(|error| {
        anyhow!(
            "persisting new output {} without overwrite: {}",
            path.display(),
            error.error
        )
    })?;
    Ok(())
}

fn read_feed(path: &Path) -> Result<Vec<u8>> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("reading metadata for {}", path.display()))?;
    if metadata.len() > greenlit_engine::parse::MAX_INPUT_BYTES as u64 {
        return Err(anyhow!(
            "input {} is {} bytes; maximum is {}",
            path.display(),
            metadata.len(),
            greenlit_engine::parse::MAX_INPUT_BYTES
        ));
    }
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

fn use_color(common: &CommonArgs) -> bool {
    !common.no_color && std::io::stdout().is_terminal()
}

fn ellipsize(s: &str, max: usize) -> String {
    let one_line = terminal_safe(s);
    if one_line.chars().count() <= max {
        one_line
    } else {
        let t: String = one_line.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {}", terminal_safe(&format!("{e:#}")));
            ExitCode::from(2)
        }
    }
}

fn terminal_safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            ch if ch.is_control() => out.push_str(&format!("\\u{{{:x}}}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_writer_creates_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("feed.fixed.xml");
        write_new_atomic(&path, b"safe").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"safe");
    }

    #[test]
    fn atomic_writer_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("feed.xml");
        std::fs::write(&path, b"original").unwrap();
        assert!(write_new_atomic(&path, b"replacement").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"original");
    }

    #[test]
    fn terminal_values_escape_control_sequences() {
        assert_eq!(
            terminal_safe("safe\x1b[31mred\nnext"),
            "safe\\u{1b}[31mred next"
        );
    }

    #[test]
    fn default_output_preserves_delimited_extension() {
        assert_eq!(
            default_output_path(Path::new("products.tsv"), Format::Csv),
            PathBuf::from("products.fixed.tsv")
        );
        assert_eq!(
            default_output_path(Path::new("products.csv"), Format::Csv),
            PathBuf::from("products.fixed.csv")
        );
    }
}
