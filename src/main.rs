use clap::{Parser, Subcommand};
use std::time::Duration;

mod engine;
mod report;
mod stats;
mod sweep;

#[derive(Parser)]
#[command(
    name = "vastar",
    about = "vastar — HTTP load generator. Fast, zero-copy, Rust.",
    version,
    // Allow `vastar <URL> [flags]` (flat form, unchanged) alongside
    // `vastar <subcommand> ...`. `subcommand_negates_reqs` tells clap that
    // when a subcommand is present, positional `url` is not required.
    subcommand_negates_reqs = true,
)]
struct Cli {
    /// Target URL (flat bench mode — not needed when a subcommand is used).
    #[arg(required = true)]
    url: Option<String>,

    /// Number of requests to run. Default is 200.
    #[arg(short = 'n', default_value = "200")]
    requests: usize,

    /// Number of workers to run concurrently. Default is 50.
    #[arg(short = 'c', default_value = "50")]
    concurrency: usize,

    /// Duration (e.g. 10s, 1m). When set, -n is ignored.
    #[arg(short = 'z')]
    duration: Option<String>,

    /// Rate limit in QPS per worker. 0 = no limit.
    #[arg(short = 'q', default_value = "0")]
    qps: f64,

    /// HTTP method. Default is GET.
    #[arg(short = 'm', default_value = "GET")]
    method: String,

    /// HTTP request body.
    #[arg(short = 'd')]
    body: Option<String>,

    /// HTTP request body from file.
    #[arg(short = 'D')]
    body_file: Option<String>,

    /// Content-type header. Default is "text/html".
    #[arg(short = 'T', default_value = "text/html")]
    content_type: String,

    /// Custom HTTP headers (repeatable). e.g. -H "Accept: application/json"
    #[arg(short = 'H')]
    header: Vec<String>,

    /// Timeout for each request in seconds. Default is 20.
    #[arg(short = 't', default_value = "20")]
    timeout: u64,

    /// HTTP Accept header.
    #[arg(short = 'A')]
    accept: Option<String>,

    /// Basic authentication (user:pass).
    #[arg(short = 'a')]
    auth: Option<String>,

    /// Output type. "csv" for CSV output.
    #[arg(short = 'o')]
    output: Option<String>,

    /// Disable keep-alive, prevents re-use of TCP connections.
    #[arg(long)]
    disable_keepalive: bool,

    /// Disable compression.
    #[arg(long)]
    disable_compression: bool,

    /// Disable following of HTTP redirects.
    #[arg(long)]
    disable_redirects: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Adaptive concurrency sweep — finds the sweet-spot `c` empirically.
    Sweep(sweep::SweepArgs),
}

fn parse_duration(s: &str) -> Duration {
    let s = s.trim();
    if let Some(v) = s.strip_suffix('s') {
        Duration::from_secs_f64(v.parse().expect("invalid duration"))
    } else if let Some(v) = s.strip_suffix('m') {
        Duration::from_secs(v.parse::<u64>().expect("invalid duration") * 60)
    } else if let Some(v) = s.strip_suffix('h') {
        Duration::from_secs(v.parse::<u64>().expect("invalid duration") * 3600)
    } else {
        Duration::from_secs(s.parse().expect("invalid duration"))
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Dispatch subcommand if present.
    if let Some(cmd) = cli.command {
        match cmd {
            Command::Sweep(args) => {
                sweep::run_sweep(args).await;
                return;
            }
        }
    }

    // Flat bench mode (original behavior).
    let url = cli.url.expect("URL required for flat bench mode");

    // Validate
    if cli.concurrency == 0 {
        eprintln!("Error: -c cannot be smaller than 1.");
        std::process::exit(1);
    }
    if cli.duration.is_none() && cli.requests == 0 {
        eprintln!("Error: -n cannot be 0.");
        std::process::exit(1);
    }
    if cli.duration.is_none() && cli.requests < cli.concurrency {
        eprintln!("Error: -n cannot be less than -c.");
        std::process::exit(1);
    }

    // Build headers
    let mut headers: Vec<(String, String)> = Vec::new();
    headers.push(("content-type".into(), cli.content_type.clone()));
    headers.push(("user-agent".into(), "vastar/0.1.0".into()));

    if let Some(ref accept) = cli.accept {
        headers.push(("accept".into(), accept.clone()));
    }

    if cli.disable_compression {
        headers.push(("accept-encoding".into(), "identity".into()));
    }

    // Basic auth
    if let Some(ref auth) = cli.auth {
        let encoded = base64_encode(auth.as_bytes());
        headers.push(("authorization".into(), format!("Basic {}", encoded)));
    }

    // Custom headers
    for h in &cli.header {
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_lowercase(), v.trim().to_string()));
        } else {
            eprintln!("Warning: invalid header format '{}', expected 'Key: Value'", h);
        }
    }

    // Build body
    let body = if let Some(ref b) = cli.body {
        bytes::Bytes::from(b.clone())
    } else if let Some(ref f) = cli.body_file {
        bytes::Bytes::from(std::fs::read(f).unwrap_or_else(|e| {
            eprintln!("Error reading body file '{}': {}", f, e);
            std::process::exit(1);
        }))
    } else {
        bytes::Bytes::new()
    };

    // Parse duration
    let duration = cli.duration.as_ref().map(|d| parse_duration(d));
    let num_requests = if duration.is_some() {
        usize::MAX / 2
    } else {
        cli.requests
    };

    let config = engine::BenchConfig {
        uri: url,
        method: cli.method.to_uppercase(),
        headers,
        body,
        num_requests,
        concurrency: cli.concurrency,
        duration,
        timeout: Duration::from_secs(cli.timeout),
        qps: cli.qps,
        disable_keepalive: cli.disable_keepalive,
    };

    let (results, elapsed) = engine::run(config).await;
    let bench_result = stats::aggregate(results, elapsed, cli.concurrency);
    report::print_report(&bench_result);

    // Silence dead-code warnings for flags not yet wired through.
    let _ = cli.output;
    let _ = cli.disable_redirects;
}

/// Minimal base64 encoder — avoids adding a dependency just for auth.
fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
