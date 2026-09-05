use std::env;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// User-Agent sent on every outbound HTTP request unless `ct_log.user_agent`
/// overrides it. Built from the package version at compile time so the CT log
/// fetch client and the TLS-pinned Apple catalog client can never drift apart.
pub const DEFAULT_USER_AGENT: &str = concat!("certstream-server-rust/", env!("CARGO_PKG_VERSION"));

/// A finite replay of one log's index range, written out as JSONL. Parsed
/// here rather than in `backfill` so a malformed invocation is rejected
/// before anything reaches the network.
#[derive(Debug, Clone)]
pub struct BackfillArgs {
    pub log: String,
    pub start: u64,
    pub end: u64,
    pub out: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CliArgs {
    pub validate_config: bool,
    pub dry_run: bool,
    pub export_metrics: bool,
    pub show_version: bool,
    pub show_help: bool,
    pub backfill: Option<Result<BackfillArgs, String>>,
}

impl CliArgs {
    pub fn parse() -> Self {
        let args: Vec<String> = env::args().collect();

        Self {
            validate_config: args.iter().any(|a| a == "--validate-config"),
            dry_run: args.iter().any(|a| a == "--dry-run"),
            export_metrics: args.iter().any(|a| a == "--export-metrics"),
            show_version: args.iter().any(|a| a == "--version" || a == "-V"),
            show_help: args.iter().any(|a| a == "--help" || a == "-h"),
            backfill: args
                .iter()
                .any(|a| a == "--backfill")
                .then(|| parse_backfill(&args)),
        }
    }

    pub fn print_help() {
        println!("certstream-server-rust {}", VERSION);
        println!();
        println!("High-performance Certificate Transparency log streaming server");
        println!();
        println!("USAGE:");
        println!("    certstream-server-rust [OPTIONS]");
        println!();
        println!("OPTIONS:");
        println!("    --validate-config    Validate configuration and exit");
        println!("    --dry-run            Start server without connecting to CT logs");
        println!("    --export-metrics     Export current metrics and exit (output is empty on cold start)");
        println!("    --backfill           Replay one log's index range as JSONL and exit");
        println!("        --log <ID>       CT log URL, log ID, or a substring of its name");
        println!("        --start <N>      First entry index, inclusive");
        println!("        --end <N>        Last entry index, inclusive");
        println!("        --out <PATH>     Output file (default: stdout)");
        println!("    -V, --version        Print version information");
        println!("    -h, --help           Print help information");
        println!();
        println!("ENVIRONMENT VARIABLES:");
        println!("    CERTSTREAM_CONFIG              Path to config file");
        println!("    CERTSTREAM_HOST                Server host (default: 0.0.0.0)");
        println!("    CERTSTREAM_PORT                Server port (default: 8080)");
        println!("    CERTSTREAM_LOG_LEVEL           Log level (default: info)");
        println!("    CERTSTREAM_BUFFER_SIZE         Broadcast buffer size (default: 1000)");
        println!("    CERTSTREAM_USER_AGENT          Override the outbound HTTP User-Agent");
        println!();
        println!("For more information, see: https://github.com/reloading01/certstream-server-rust");
    }

    pub fn print_version() {
        println!("certstream-server-rust {}", VERSION);
    }
}

/// `--key value` lookup. Returns `None` when the flag is absent or trailing.
fn value_of<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    let pos = args.iter().position(|a| a == key)?;
    args.get(pos + 1).map(String::as_str)
}

fn parse_backfill(args: &[String]) -> Result<BackfillArgs, String> {
    let log = value_of(args, "--log")
        .ok_or("--backfill needs --log <url|log-id|name>")?
        .to_string();

    let number = |key: &str| -> Result<u64, String> {
        let raw = value_of(args, key).ok_or_else(|| format!("--backfill needs {key} <N>"))?;
        raw.parse::<u64>()
            .map_err(|_| format!("{key} must be a non-negative integer, got `{raw}`"))
    };
    let start = number("--start")?;
    let end = number("--end")?;
    if end < start {
        return Err(format!("--end ({end}) is before --start ({start})"));
    }

    Ok(BackfillArgs {
        log,
        start,
        end,
        // `-` is the conventional spelling of stdout, and is not a filename.
        out: value_of(args, "--out")
            .filter(|p| *p != "-")
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("certstream-server-rust")
            .chain(list.iter().copied())
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn a_complete_backfill_invocation_parses() {
        let parsed = parse_backfill(&args(&[
            "--backfill", "--log", "argon", "--start", "10", "--end", "20", "--out", "x.jsonl",
        ]))
        .unwrap();
        assert_eq!(parsed.log, "argon");
        assert_eq!((parsed.start, parsed.end), (10, 20));
        assert_eq!(parsed.out.as_deref(), Some("x.jsonl"));
    }

    #[test]
    fn stdout_is_the_default_and_dash_means_stdout() {
        let implicit =
            parse_backfill(&args(&["--backfill", "--log", "a", "--start", "0", "--end", "1"]))
                .unwrap();
        assert!(implicit.out.is_none());

        let explicit = parse_backfill(&args(&[
            "--backfill", "--log", "a", "--start", "0", "--end", "1", "--out", "-",
        ]))
        .unwrap();
        assert!(explicit.out.is_none());
    }

    /// A bad invocation must fail here, before anything reaches a CT log.
    #[test]
    fn incomplete_or_inverted_invocations_are_rejected() {
        for (bad, expect) in [
            (vec!["--backfill", "--start", "0", "--end", "1"], "--log"),
            (vec!["--backfill", "--log", "a", "--end", "1"], "--start"),
            (vec!["--backfill", "--log", "a", "--start", "0"], "--end"),
            (
                vec!["--backfill", "--log", "a", "--start", "9", "--end", "1"],
                "before",
            ),
            (
                vec!["--backfill", "--log", "a", "--start", "x", "--end", "1"],
                "non-negative",
            ),
        ] {
            let err = parse_backfill(&args(&bad)).unwrap_err();
            assert!(err.contains(expect), "{bad:?} → {err}");
        }
    }

    #[test]
    fn a_trailing_flag_has_no_value() {
        assert!(value_of(&args(&["--log"]), "--log").is_none());
        assert!(value_of(&args(&["--start", "5"]), "--log").is_none());
    }
}
