//! Concurrency sweet-spot sweep — Phase 0 roadmap feature.
//!
//! Runs the existing engine across multiple concurrency levels, picks the one
//! that delivers the best throughput-vs-tail tradeoff. Domain-agnostic: no
//! workload heuristics, no hardcoded categories, no CPU-core presets. The
//! algorithm learns the shape of the curve for whatever endpoint it's given.
//!
//! Output is both human-readable (text table) and script-consumable (JSON).
//! See ROADMAP.md Phase 0 for full design.
use clap::Args;
use std::time::Duration;

use crate::engine::{self, BenchConfig};
use crate::stats::{self, BenchResult};

// ─────────────────────────────────────────────────────────────────────────────
// CLI args
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Args, Debug)]
pub struct SweepArgs {
    /// Target URL
    pub url: String,

    /// Concurrency spec. Formats:
    ///   "10,50,100,500"      explicit list
    ///   "10..1000:log=6"     log-spaced 6 points
    ///   "10..200:step=20"    linear step
    ///   "auto"               algorithm picks (default)
    #[arg(long, default_value = "auto")]
    pub conc: String,

    /// After coarse sweep, bracket ±50% around winner and sweep 4 more points.
    #[arg(long)]
    pub refine: bool,

    /// Repeat each c-level N times, take median (default: 1).
    #[arg(long, default_value = "1")]
    pub repeats: usize,

    /// Selection algorithm.
    #[arg(long, value_enum, default_value_t = PickStrategy::Knee)]
    pub pick: PickStrategy,

    /// For knee mode: smallest c reaching this fraction of peak rps.
    #[arg(long, default_value = "0.95")]
    pub knee_ratio: f64,

    /// Concurrency used as reference for tail-degradation check (default 1).
    #[arg(long, default_value = "1")]
    pub baseline_c: usize,

    /// DQ if p99/p50 > this.
    #[arg(long, default_value = "4.0")]
    pub max_spread: f64,

    /// DQ if p99.9/p50 > this.
    #[arg(long, default_value = "8.0")]
    pub max_p999_ratio: f64,

    /// DQ if error_rate > this (0.01 = 1%).
    #[arg(long, default_value = "0.01")]
    pub max_errors: f64,

    /// DQ if p99 > baseline_p99 × this.
    #[arg(long, default_value = "3.0")]
    pub max_tail_mult: f64,

    // ── Sub-bench pass-through (same semantics as flat `vastar`) ──
    /// Requests per sub-bench (default 2000).
    #[arg(short = 'n', default_value = "2000")]
    pub requests: usize,

    /// Duration per sub-bench (e.g. 5s, 1m) — overrides -n when set.
    #[arg(short = 'z')]
    pub duration: Option<String>,

    /// HTTP method.
    #[arg(short = 'm', default_value = "GET")]
    pub method: String,

    /// Request body.
    #[arg(short = 'd')]
    pub body: Option<String>,

    /// Request body from file.
    #[arg(short = 'D')]
    pub body_file: Option<String>,

    /// Content-Type header.
    #[arg(short = 'T', default_value = "application/json")]
    pub content_type: String,

    /// Custom headers (repeatable).
    #[arg(short = 'H')]
    pub header: Vec<String>,

    /// Accept header.
    #[arg(short = 'A')]
    pub accept: Option<String>,

    /// Basic auth (user:pass).
    #[arg(short = 'a')]
    pub auth: Option<String>,

    /// Per-request timeout in seconds.
    #[arg(short = 't', default_value = "20")]
    pub timeout: u64,

    /// Disable keep-alive.
    #[arg(long)]
    pub disable_keepalive: bool,

    /// Disable compression (accept-encoding: identity).
    #[arg(long)]
    pub disable_compression: bool,

    /// Output format: text | json | ndjson (default: text).
    #[arg(short = 'o', long = "output", default_value = "text")]
    pub output: String,

    /// Also write the JSON result to a file (text still prints to stdout).
    #[arg(long)]
    pub json_path: Option<String>,

    // ── Paired sweep (platform-overhead mode) ──
    //
    // When benchmarking a gateway/proxy/mesh that fronts an upstream, the
    // target's own curve doesn't tell you whether *your platform* is the
    // bottleneck — it could look healthy just because the upstream is doing
    // the heavy lifting. Paired mode runs both endpoints at each concurrency
    // level and picks the sweet spot where the platform still stays close to
    // upstream's own performance.
    /// Reference (upstream) URL. When set, each concurrency is measured
    /// twice — reference and target — and sweet spot is picked where
    /// target's overhead vs reference stays within `--max-overhead-pct`.
    #[arg(long, value_name = "REFERENCE_URL")]
    pub vs: Option<String>,

    /// HTTP method for the reference endpoint (default: same as -m).
    #[arg(long)]
    pub vs_method: Option<String>,

    /// Request body for the reference endpoint (default: same as -d).
    #[arg(long)]
    pub vs_body: Option<String>,

    /// Content-Type for the reference endpoint (default: same as -T).
    #[arg(long)]
    pub vs_content_type: Option<String>,

    /// Load reference curve from a prior `vastar sweep -o json` result file
    /// instead of re-measuring. Skips all reference runs. Mutually
    /// exclusive with `--vs` (JSON file provides the URL).
    #[arg(long, value_name = "FILE", conflicts_with = "vs")]
    pub ref_from_json: Option<String>,

    /// Max acceptable platform overhead: DQ when
    /// `(target_p99 - ref_p99) / ref_p99 × 100 > this`. Default 25%.
    /// Only takes effect in paired mode (`--vs` or `--ref-from-json`).
    #[arg(long, default_value = "25")]
    pub max_overhead_pct: f64,

    /// Max acceptable throughput deficit: DQ when reference delivers
    /// `(ref_rps - target_rps) / ref_rps × 100 > this` more than target.
    /// Catches "platform can't keep up" even when latency looks OK.
    /// Default 50%. Only takes effect in paired mode.
    #[arg(long, default_value = "50")]
    pub max_rps_deficit_pct: f64,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickStrategy {
    /// Smallest c reaching knee_ratio × peak_rps with tail within gates.
    Knee,
    /// argmax(rps / (p99/p50)²) — throughput × consistency².
    Score,
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal types
// ─────────────────────────────────────────────────────────────────────────────

/// One measured point on the sweep curve.
#[derive(Clone)]
struct SweepPoint {
    c: usize,
    repeats: usize,
    rps: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    p999_ms: f64,
    error_rate: f64,
    score: f64,
    disqualified: Option<String>,
    /// Paired-mode only: corresponding reference point measured at same `c`.
    /// None when running single-endpoint sweep (no `--vs`).
    reference: Option<ReferenceSample>,
    /// Paired-mode only: platform overhead vs reference at this `c`.
    overhead_pct: Option<f64>,
    /// Paired-mode only: throughput deficit vs reference (negative = target faster).
    rps_deficit_pct: Option<f64>,
}

/// Snapshot of a reference endpoint measurement — kept minimal since we
/// only need rps + p99 for overhead math and p50/p95 for reporting context.
#[derive(Clone)]
struct ReferenceSample {
    rps: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    p999_ms: f64,
    error_rate: f64,
}

struct SweepResult {
    params: ParamsSnapshot,
    machine: MachineSnapshot,
    baseline: SweepPoint,
    sweep_points: Vec<SweepPoint>,
    sweet_spot: SweetSpot,
    notes: Vec<String>,
    /// Set only in paired mode. None for single-endpoint sweep.
    paired: Option<PairedInfo>,
}

struct PairedInfo {
    reference_url: String,
    reference_method: String,
    reference_baseline: ReferenceSample,
    max_overhead_pct: f64,
    max_rps_deficit_pct: f64,
    source: ReferenceSource,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReferenceSource {
    Live,
    Cached, // from --ref-from-json
}

struct ParamsSnapshot {
    url: String,
    method: String,
    pick: PickStrategy,
    knee_ratio: f64,
    baseline_c: usize,
    repeats: usize,
    requests_per_bench: usize,
    refine: bool,
}

struct MachineSnapshot {
    cpu_cores_logical: usize,
    ram_mb: u64,
}

struct SweetSpot {
    concurrency: usize,
    rps: f64,
    p50_ms: f64,
    p99_ms: f64,
    method: &'static str, // "knee" or "score"
    reasoning: String,
    peak_rps: f64,
    peak_concurrency: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
// Entry point
// ─────────────────────────────────────────────────────────────────────────────

pub async fn run_sweep(args: SweepArgs) {
    validate_args(&args);

    let headers = build_headers(&args);
    let body = build_body(&args);
    let duration = args.duration.as_deref().map(parse_duration);

    // Paired-mode prep (optional second endpoint for overhead comparison).
    let paired_ctx = prepare_paired_context(&args).await;
    let paired_active = paired_ctx.is_some();

    // Step 1 — calibrate target baseline (uncontended reference for target)
    eprintln!("  [sweep] calibrating target baseline at c={}...", args.baseline_c);
    let baseline = measure_point(
        &args, &headers, &body, duration, args.baseline_c, 1,
    ).await;
    eprintln!(
        "  [sweep] target baseline: rps={:.0}  p50={:.2}ms  p99={:.2}ms",
        baseline.rps, baseline.p50_ms, baseline.p99_ms,
    );

    // Step 2 — resolve concurrency spec
    let coarse_levels = resolve_conc_spec(&args.conc, args.baseline_c);
    eprintln!("  [sweep] coarse sweep: c in {:?}", coarse_levels);

    // Step 3 — coarse sweep (paired or single)
    let mut points: Vec<SweepPoint> = Vec::with_capacity(coarse_levels.len());
    for &c in &coarse_levels {
        let pt = measure_one(&args, &headers, &body, duration, c, paired_ctx.as_ref(), &baseline).await;
        log_point(&pt);
        points.push(pt);
    }

    // Step 4 — refine around winner (optional)
    let mut notes: Vec<String> = Vec::new();
    if args.repeats > 1 {
        notes.push(format!("repeats={}", args.repeats));
    }
    if paired_active {
        notes.push("paired=on".into());
    }
    if args.refine {
        notes.push("refine=on".into());
        if let Some(winner_c) = pick_preliminary_winner(&points, args.pick) {
            let bracket_levels = bracket(winner_c, 0.5, 1.5, 4);
            let bracket_levels: Vec<usize> = bracket_levels
                .into_iter()
                .filter(|&c| !coarse_levels.contains(&c) && c > 0)
                .collect();
            if !bracket_levels.is_empty() {
                eprintln!(
                    "  [sweep] refining around c={} with {:?}",
                    winner_c, bracket_levels,
                );
                for &c in &bracket_levels {
                    let pt = measure_one(&args, &headers, &body, duration, c, paired_ctx.as_ref(), &baseline).await;
                    log_point(&pt);
                    points.push(pt);
                }
                points.sort_by_key(|p| p.c);
            }
        }
    }

    // Step 5 — pick sweet spot
    let sweet = pick_sweet_spot(&points, &baseline, &args);

    // Step 6 — emit
    let result = SweepResult {
        params: ParamsSnapshot {
            url: args.url.clone(),
            method: args.method.to_uppercase(),
            pick: args.pick,
            knee_ratio: args.knee_ratio,
            baseline_c: args.baseline_c,
            repeats: args.repeats,
            requests_per_bench: args.requests,
            refine: args.refine,
        },
        machine: capture_machine(),
        baseline,
        sweep_points: points,
        sweet_spot: sweet,
        notes,
        paired: paired_ctx.map(|ctx| PairedInfo {
            reference_url: ctx.url,
            reference_method: ctx.method,
            reference_baseline: ctx.baseline,
            max_overhead_pct: args.max_overhead_pct,
            max_rps_deficit_pct: args.max_rps_deficit_pct,
            source: ctx.source,
        }),
    };

    match args.output.as_str() {
        "json" => {
            let json = format_json(&result);
            println!("{}", json);
        }
        "ndjson" => {
            let ndjson = format_ndjson(&result);
            println!("{}", ndjson);
        }
        _ => {
            print_text(&result);
        }
    }

    if let Some(ref path) = args.json_path {
        let json = format_json(&result);
        if let Err(e) = std::fs::write(path, &json) {
            eprintln!("  [sweep] WARN: could not write --json-path {}: {}", path, e);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Validation + arg prep (mirrors logic in main.rs so sweep can run standalone)
// ─────────────────────────────────────────────────────────────────────────────

fn validate_args(args: &SweepArgs) {
    if args.baseline_c == 0 {
        eprintln!("Error: --baseline-c must be ≥ 1");
        std::process::exit(1);
    }
    if args.repeats == 0 {
        eprintln!("Error: --repeats must be ≥ 1");
        std::process::exit(1);
    }
    if args.knee_ratio <= 0.0 || args.knee_ratio > 1.0 {
        eprintln!("Error: --knee-ratio must be in (0, 1]");
        std::process::exit(1);
    }
    if !matches!(args.output.as_str(), "text" | "json" | "ndjson") {
        eprintln!("Error: --output must be one of: text, json, ndjson");
        std::process::exit(1);
    }
}

fn build_headers(args: &SweepArgs) -> Vec<(String, String)> {
    let mut h: Vec<(String, String)> = Vec::new();
    h.push(("content-type".into(), args.content_type.clone()));
    h.push(("user-agent".into(), format!("vastar-sweep/{}", env!("CARGO_PKG_VERSION"))));
    if let Some(ref a) = args.accept {
        h.push(("accept".into(), a.clone()));
    }
    if args.disable_compression {
        h.push(("accept-encoding".into(), "identity".into()));
    }
    if let Some(ref auth) = args.auth {
        let encoded = base64_encode(auth.as_bytes());
        h.push(("authorization".into(), format!("Basic {}", encoded)));
    }
    for raw in &args.header {
        if let Some((k, v)) = raw.split_once(':') {
            h.push((k.trim().to_lowercase(), v.trim().to_string()));
        }
    }
    h
}

fn build_body(args: &SweepArgs) -> bytes::Bytes {
    if let Some(ref b) = args.body {
        bytes::Bytes::from(b.clone())
    } else if let Some(ref f) = args.body_file {
        bytes::Bytes::from(std::fs::read(f).unwrap_or_else(|e| {
            eprintln!("Error reading body file '{}': {}", f, e);
            std::process::exit(1);
        }))
    } else {
        bytes::Bytes::new()
    }
}

fn parse_duration(s: &str) -> Duration {
    let s = s.trim();
    if let Some(v) = s.strip_suffix('s') {
        Duration::from_secs_f64(v.parse().unwrap_or(0.0))
    } else if let Some(v) = s.strip_suffix('m') {
        Duration::from_secs(v.parse::<u64>().unwrap_or(0) * 60)
    } else if let Some(v) = s.strip_suffix('h') {
        Duration::from_secs(v.parse::<u64>().unwrap_or(0) * 3600)
    } else {
        Duration::from_secs(s.parse().unwrap_or(0))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Concurrency spec resolver
// ─────────────────────────────────────────────────────────────────────────────

/// Parse `--conc` spec into concrete concurrency levels.
/// Formats:
///   "auto"                → log-spaced default range
///   "10,50,100,500"       → explicit list
///   "10..1000:log=6"      → log-spaced 6 points [10, 1000]
///   "10..200:step=20"     → linear step
fn resolve_conc_spec(spec: &str, baseline_c: usize) -> Vec<usize> {
    let s = spec.trim();
    if s == "auto" {
        // Default log-spaced range from 10 to 1000 — wide enough to find knee
        // for sub-ms workloads and heavy-upstream workloads alike.
        return log_spaced(10, 1000, 7)
            .into_iter()
            .filter(|&c| c > baseline_c)
            .collect();
    }
    if let Some((range, opt)) = s.split_once(':') {
        if let Some((lo, hi)) = range.split_once("..") {
            let lo: usize = lo.trim().parse().unwrap_or(10);
            let hi: usize = hi.trim().parse().unwrap_or(1000);
            let (key, val) = opt.split_once('=').unwrap_or(("log", "6"));
            match key.trim() {
                "log" => {
                    let n: usize = val.parse().unwrap_or(6);
                    return log_spaced(lo, hi, n);
                }
                "step" => {
                    let step: usize = val.parse().unwrap_or(50);
                    return (lo..=hi).step_by(step.max(1)).collect();
                }
                _ => {}
            }
        }
    }
    if s.contains(',') {
        return s.split(',')
            .filter_map(|t| t.trim().parse().ok())
            .collect();
    }
    // single number fallback
    if let Ok(n) = s.parse() {
        return vec![n];
    }
    // last resort — auto defaults
    log_spaced(10, 1000, 7)
}

fn log_spaced(lo: usize, hi: usize, n: usize) -> Vec<usize> {
    if n == 0 || lo == 0 {
        return vec![];
    }
    if n == 1 || lo == hi {
        return vec![lo];
    }
    let lo_l = (lo as f64).ln();
    let hi_l = (hi as f64).ln();
    let step = (hi_l - lo_l) / (n - 1) as f64;
    let mut out: Vec<usize> = (0..n)
        .map(|i| ((lo_l + step * i as f64).exp().round() as usize).max(1))
        .collect();
    out.dedup();
    out
}

fn bracket(center: usize, lo_ratio: f64, hi_ratio: f64, n: usize) -> Vec<usize> {
    let lo = ((center as f64) * lo_ratio).max(1.0) as usize;
    let hi = ((center as f64) * hi_ratio).max(1.0) as usize;
    if lo == hi {
        return vec![lo];
    }
    let mut out = log_spaced(lo.max(1), hi, n);
    out.retain(|&c| c != center);
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Paired-mode plumbing
// ─────────────────────────────────────────────────────────────────────────────

/// Runtime context for paired sweep — carries reference URL, headers, body,
/// and the reference baseline (either measured live or loaded from a cache file).
struct PairedCtx {
    url: String,
    method: String,
    headers: Vec<(String, String)>,
    body: bytes::Bytes,
    /// Reference baseline at `--baseline-c`. For live mode this is measured
    /// at prepare time; for cached mode it's loaded from the JSON file.
    baseline: ReferenceSample,
    /// Pre-measured reference curve keyed by concurrency — populated only in
    /// `--ref-from-json` mode. Empty for live mode (measurements happen per-c).
    cached_curve: std::collections::HashMap<usize, ReferenceSample>,
    source: ReferenceSource,
}

async fn prepare_paired_context(args: &SweepArgs) -> Option<PairedCtx> {
    if let Some(ref path) = args.ref_from_json {
        match load_reference_from_json(path) {
            Ok(ctx) => {
                eprintln!(
                    "  [sweep] paired mode: reference loaded from {} ({} points, baseline p99={:.2}ms)",
                    path, ctx.cached_curve.len(), ctx.baseline.p99_ms,
                );
                return Some(ctx);
            }
            Err(e) => {
                eprintln!("  [sweep] ERROR: could not load --ref-from-json {}: {}", path, e);
                std::process::exit(1);
            }
        }
    }

    let url = args.vs.as_ref()?.clone();
    let method = args.vs_method.clone().unwrap_or_else(|| args.method.clone());
    let content_type = args.vs_content_type.clone().unwrap_or_else(|| args.content_type.clone());
    let body_str = args.vs_body.as_ref().or(args.body.as_ref()).cloned();

    // Build minimal headers for reference — same auth/accept if set, but
    // distinct content-type (reference body may be shaped differently).
    let mut headers: Vec<(String, String)> = Vec::new();
    headers.push(("content-type".into(), content_type));
    headers.push(("user-agent".into(), format!("vastar-sweep/{}", env!("CARGO_PKG_VERSION"))));
    if let Some(ref a) = args.accept { headers.push(("accept".into(), a.clone())); }
    if args.disable_compression { headers.push(("accept-encoding".into(), "identity".into())); }
    if let Some(ref auth) = args.auth {
        let encoded = base64_encode(auth.as_bytes());
        headers.push(("authorization".into(), format!("Basic {}", encoded)));
    }

    let body = body_str.map(bytes::Bytes::from).unwrap_or_else(bytes::Bytes::new);

    eprintln!(
        "  [sweep] paired mode: calibrating reference baseline at c={}...",
        args.baseline_c,
    );
    let ref_baseline_pt = run_engine_multi(
        args, &url, &method, &headers, &body, args.baseline_c, 1,
    ).await;
    eprintln!(
        "  [sweep] reference baseline: rps={:.0}  p50={:.2}ms  p99={:.2}ms",
        ref_baseline_pt.rps, ref_baseline_pt.p50_ms, ref_baseline_pt.p99_ms,
    );
    Some(PairedCtx {
        url, method, headers, body,
        baseline: point_to_reference(&ref_baseline_pt),
        cached_curve: std::collections::HashMap::new(),
        source: ReferenceSource::Live,
    })
}

/// Minimal JSON reader — picks the fields we need (baseline.{rps,p99_ms,...},
/// sweep_points[].{concurrency,rps,p99_ms,...}, params.url, params.method)
/// from a prior `vastar sweep -o json` result. Avoids serde dep.
fn load_reference_from_json(path: &str) -> Result<PairedCtx, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;

    let find_number = |obj: &str, key: &str| -> Option<f64> {
        let needle = format!("\"{}\":", key);
        let start = obj.find(&needle)?;
        let after = &obj[start + needle.len()..];
        let end = after.find(|c: char| c == ',' || c == '}' || c == ']').unwrap_or(after.len());
        after[..end].trim().parse().ok()
    };
    let find_string = |obj: &str, key: &str| -> Option<String> {
        let needle = format!("\"{}\":", key);
        let start = obj.find(&needle)?;
        let after = &obj[start + needle.len()..].trim_start();
        let after = after.strip_prefix('"')?;
        let end = after.find('"')?;
        Some(after[..end].to_string())
    };

    let url = find_string(&text, "url").ok_or("missing params.url")?;
    let method = find_string(&text, "method").unwrap_or_else(|| "GET".into());

    // Baseline: look for the first `"baseline": { ... }` object
    let baseline_start = text.find("\"baseline\":").ok_or("missing baseline")?;
    let baseline_section = &text[baseline_start..baseline_start + 400.min(text.len() - baseline_start)];
    let base_rps = find_number(baseline_section, "rps").unwrap_or(0.0);
    let base_p50 = find_number(baseline_section, "p50_ms").unwrap_or(0.0);
    let base_p95 = find_number(baseline_section, "p95_ms").unwrap_or(0.0);
    let base_p99 = find_number(baseline_section, "p99_ms").unwrap_or(0.0);
    let base_p999 = find_number(baseline_section, "p999_ms").unwrap_or(0.0);
    let base_err = find_number(baseline_section, "error_rate").unwrap_or(0.0);

    // sweep_points: find each "{concurrency":N,...}" object and extract
    let mut cached = std::collections::HashMap::new();
    let points_start = text.find("\"sweep_points\":").ok_or("missing sweep_points")?;
    let points_section = &text[points_start..];
    let mut cursor = 0;
    while let Some(off) = points_section[cursor..].find("\"concurrency\":") {
        let abs = cursor + off;
        let tail = &points_section[abs..];
        let end = tail.find('}').unwrap_or(tail.len());
        let point_obj = &tail[..=end];
        if let Some(c) = find_number(point_obj, "concurrency") {
            let c = c as usize;
            let samp = ReferenceSample {
                rps: find_number(point_obj, "rps").unwrap_or(0.0),
                p50_ms: find_number(point_obj, "p50_ms").unwrap_or(0.0),
                p95_ms: find_number(point_obj, "p95_ms").unwrap_or(0.0),
                p99_ms: find_number(point_obj, "p99_ms").unwrap_or(0.0),
                p999_ms: find_number(point_obj, "p999_ms").unwrap_or(0.0),
                error_rate: find_number(point_obj, "error_rate").unwrap_or(0.0),
            };
            // Skip the baseline object (first occurrence) — it's outside sweep_points
            // but our search starts from sweep_points_start so this is safe.
            cached.insert(c, samp);
        }
        cursor = abs + end;
    }

    Ok(PairedCtx {
        url, method,
        headers: vec![],
        body: bytes::Bytes::new(),
        baseline: ReferenceSample {
            rps: base_rps, p50_ms: base_p50, p95_ms: base_p95,
            p99_ms: base_p99, p999_ms: base_p999, error_rate: base_err,
        },
        cached_curve: cached,
        source: ReferenceSource::Cached,
    })
}

/// Fetch the reference sample for a given concurrency — either run it live
/// (PairedCtx::Live) or pick from the cached curve (PairedCtx::Cached).
/// Cached mode falls back to nearest-concurrency interpolation when exact `c` missing.
async fn reference_at(
    ctx: &PairedCtx, args: &SweepArgs,
    _duration: Option<Duration>, c: usize,
) -> ReferenceSample {
    if ctx.source == ReferenceSource::Cached {
        // Exact hit first
        if let Some(s) = ctx.cached_curve.get(&c) {
            return s.clone();
        }
        // Nearest-c fallback — ref curves from cached JSON may not line up exactly
        // with the target's sweep levels, so pick the closest known concurrency.
        let mut keys: Vec<usize> = ctx.cached_curve.keys().copied().collect();
        if keys.is_empty() { return ctx.baseline.clone(); }
        keys.sort();
        let nearest = keys.iter()
            .min_by_key(|&&k| if k > c { k - c } else { c - k })
            .copied()
            .unwrap_or(keys[0]);
        return ctx.cached_curve[&nearest].clone();
    }
    // Live mode — measure now.
    let pt = run_engine_multi(
        args, &ctx.url, &ctx.method, &ctx.headers, &ctx.body, c, args.repeats,
    ).await;
    point_to_reference(&pt)
}

// ─────────────────────────────────────────────────────────────────────────────
// Measurement loop (wraps engine::run with repeats + median aggregation)
// ─────────────────────────────────────────────────────────────────────────────

/// Run the engine `repeats` times against the given URL/method/body/headers and
/// return the median-aggregated sweep point.
async fn run_engine_multi(
    args: &SweepArgs,
    url: &str,
    method: &str,
    headers: &[(String, String)],
    body: &bytes::Bytes,
    concurrency: usize,
    repeats: usize,
) -> SweepPoint {
    let duration = args.duration.as_deref().map(parse_duration);
    let mut samples: Vec<BenchResult> = Vec::with_capacity(repeats);
    for _ in 0..repeats {
        let num_requests = if duration.is_some() {
            usize::MAX / 2
        } else {
            args.requests.max(concurrency)
        };
        let cfg = BenchConfig {
            uri: url.to_string(),
            method: method.to_uppercase(),
            headers: headers.to_vec(),
            body: body.clone(),
            num_requests,
            concurrency,
            duration,
            timeout: Duration::from_secs(args.timeout),
            qps: 0.0,
            disable_keepalive: args.disable_keepalive,
        };
        let (results, elapsed) = engine::run(cfg).await;
        let bench = stats::aggregate(results, elapsed, concurrency);
        samples.push(bench);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    aggregate_samples(concurrency, repeats, &samples)
}

/// Legacy convenience — target-endpoint measurement. Kept for callers that
/// only exercise the target (e.g., baseline calibration).
async fn measure_point(
    args: &SweepArgs,
    headers: &[(String, String)],
    body: &bytes::Bytes,
    _duration: Option<Duration>,
    concurrency: usize,
    repeats: usize,
) -> SweepPoint {
    run_engine_multi(args, &args.url, &args.method, headers, body, concurrency, repeats).await
}

/// Measure one concurrency level for the target. When `paired_ctx` is set,
/// also measures the reference at the same `c` (live) or looks it up from
/// the cached curve (ref-from-json). Applies DQ gates and returns the
/// fully-populated SweepPoint.
async fn measure_one(
    args: &SweepArgs,
    headers: &[(String, String)],
    body: &bytes::Bytes,
    duration: Option<Duration>,
    concurrency: usize,
    paired_ctx: Option<&PairedCtx>,
    baseline: &SweepPoint,
) -> SweepPoint {
    // Reference first (when live) so the target measurement runs against a
    // warm-but-stable upstream — same treatment as sub-bench warmup.
    let ref_sample = match paired_ctx {
        Some(ctx) => Some(reference_at(ctx, args, duration, concurrency).await),
        None => None,
    };

    let mut pt = measure_point(args, headers, body, duration, concurrency, args.repeats).await;

    if let Some(ref r) = ref_sample {
        let overhead = if r.p99_ms > 0.0 {
            (pt.p99_ms - r.p99_ms) / r.p99_ms * 100.0
        } else {
            0.0
        };
        let deficit = if r.rps > 0.0 {
            (r.rps - pt.rps) / r.rps * 100.0
        } else {
            0.0
        };
        pt.reference = Some(r.clone());
        pt.overhead_pct = Some(overhead);
        pt.rps_deficit_pct = Some(deficit);
    }

    pt.disqualified = evaluate_dq(&pt, baseline, args);
    pt
}

fn log_point(p: &SweepPoint) {
    let dq = p.disqualified.as_deref().map(|r| format!("DISQ({})", r)).unwrap_or_default();
    match (&p.reference, p.overhead_pct) {
        (Some(r), Some(ohd)) => {
            eprintln!(
                "  [sweep]  c={:<5} tgt: rps={:<9.0} p99={:.2}ms | ref: rps={:<9.0} p99={:.2}ms | overhead={:+.1}% {}",
                p.c, p.rps, p.p99_ms, r.rps, r.p99_ms, ohd, dq,
            );
        }
        _ => {
            eprintln!(
                "  [sweep]  c={:<5} rps={:<10.0} p50={:.2}ms p99={:.2}ms {}",
                p.c, p.rps, p.p50_ms, p.p99_ms, dq,
            );
        }
    }
}

fn aggregate_samples(c: usize, repeats: usize, samples: &[BenchResult]) -> SweepPoint {
    if samples.is_empty() {
        return SweepPoint {
            c, repeats, rps: 0.0, p50_ms: 0.0, p95_ms: 0.0, p99_ms: 0.0, p999_ms: 0.0,
            error_rate: 1.0, score: 0.0,
            disqualified: Some("no-data".into()),
            reference: None, overhead_pct: None, rps_deficit_pct: None,
        };
    }
    let rps = median_f64(samples.iter().map(|s| s.rps));
    let p50 = median_f64(samples.iter().map(|s| s.percentiles.p50 * 1000.0));
    let p95 = median_f64(samples.iter().map(|s| s.percentiles.p95 * 1000.0));
    let p99 = median_f64(samples.iter().map(|s| s.percentiles.p99 * 1000.0));
    let p999 = median_f64(samples.iter().map(|s| s.percentiles.p999 * 1000.0));
    let err_rate = median_f64(samples.iter().map(|s| {
        if s.total_requests == 0 { 0.0 }
        else { s.total_errors as f64 / s.total_requests as f64 }
    }));
    let spread = if p50 > 0.0 { p99 / p50 } else { f64::INFINITY };
    let score = if spread.is_finite() && spread > 0.0 { rps / (spread * spread) } else { 0.0 };
    SweepPoint {
        c, repeats, rps, p50_ms: p50, p95_ms: p95, p99_ms: p99, p999_ms: p999,
        error_rate: err_rate, score, disqualified: None,
        reference: None, overhead_pct: None, rps_deficit_pct: None,
    }
}

/// Convert a freshly-aggregated sweep point into a compact reference sample.
fn point_to_reference(p: &SweepPoint) -> ReferenceSample {
    ReferenceSample {
        rps: p.rps, p50_ms: p.p50_ms, p95_ms: p.p95_ms,
        p99_ms: p.p99_ms, p999_ms: p.p999_ms,
        error_rate: p.error_rate,
    }
}

fn median_f64<I: IntoIterator<Item = f64>>(iter: I) -> f64 {
    let mut v: Vec<f64> = iter.into_iter().collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if v.is_empty() { return 0.0; }
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

// ─────────────────────────────────────────────────────────────────────────────
// Disqualification gates
// ─────────────────────────────────────────────────────────────────────────────

fn evaluate_dq(pt: &SweepPoint, baseline: &SweepPoint, args: &SweepArgs) -> Option<String> {
    if pt.rps == 0.0 {
        return Some("zero-rps".into());
    }
    if pt.error_rate > args.max_errors {
        return Some(format!("errors={:.1}%", pt.error_rate * 100.0));
    }
    let spread = if pt.p50_ms > 0.0 { pt.p99_ms / pt.p50_ms } else { f64::INFINITY };
    if spread > args.max_spread {
        return Some(format!("spread={:.1}", spread));
    }
    let p999_ratio = if pt.p50_ms > 0.0 { pt.p999_ms / pt.p50_ms } else { f64::INFINITY };
    if p999_ratio > args.max_p999_ratio {
        return Some(format!("p999/p50={:.1}", p999_ratio));
    }
    // Paired-mode gates — only apply when reference data is available.
    // These catch the "target looks healthy but is actually the bottleneck"
    // case by comparing against the upstream's own curve at the same c.
    if let (Some(ohd), Some(_)) = (pt.overhead_pct, &pt.reference) {
        if ohd > args.max_overhead_pct {
            return Some(format!("overhead={:+.0}%", ohd));
        }
    }
    if let Some(deficit) = pt.rps_deficit_pct {
        if deficit > args.max_rps_deficit_pct {
            return Some(format!("rps-deficit={:+.0}%", deficit));
        }
    }
    // Baseline tail-mult gate — intentionally last so paired-mode gates (which
    // carry endpoint-specific semantics) take precedence when triggered.
    if baseline.p99_ms > 0.0 && pt.p99_ms > baseline.p99_ms * args.max_tail_mult {
        return Some(format!("p99>{:.1}xbase", args.max_tail_mult));
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Sweet-spot picker
// ─────────────────────────────────────────────────────────────────────────────

fn pick_preliminary_winner(points: &[SweepPoint], strategy: PickStrategy) -> Option<usize> {
    let candidates: Vec<&SweepPoint> = points.iter().filter(|p| p.disqualified.is_none()).collect();
    match strategy {
        PickStrategy::Knee => {
            let peak = candidates.iter().map(|p| p.rps).fold(0.0_f64, f64::max);
            candidates.iter()
                .filter(|p| p.rps >= peak * 0.90)
                .min_by_key(|p| p.c)
                .map(|p| p.c)
        }
        PickStrategy::Score => {
            candidates.iter()
                .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
                .map(|p| p.c)
        }
    }
}

fn pick_sweet_spot(points: &[SweepPoint], baseline: &SweepPoint, args: &SweepArgs) -> SweetSpot {
    let qualified: Vec<&SweepPoint> = points.iter().filter(|p| p.disqualified.is_none()).collect();
    // Peak from qualified points when available; fall back to all points so that
    // in all-DQ'd runs the summary still shows meaningful peak/pct numbers.
    let peak_source: Vec<&SweepPoint> = if qualified.is_empty() {
        points.iter().collect()
    } else {
        qualified.clone()
    };
    let peak_rps = peak_source.iter().map(|p| p.rps).fold(0.0_f64, f64::max);
    let peak_c = peak_source.iter()
        .filter(|p| (p.rps - peak_rps).abs() < f64::EPSILON)
        .map(|p| p.c)
        .next()
        .unwrap_or(0);

    let selected: Option<&SweepPoint> = match args.pick {
        PickStrategy::Knee => {
            let threshold = peak_rps * args.knee_ratio;
            let tail_cap = baseline.p99_ms * args.max_tail_mult;
            qualified.iter()
                .filter(|p| p.rps >= threshold)
                .filter(|p| baseline.p99_ms == 0.0 || p.p99_ms <= tail_cap)
                .min_by_key(|p| p.c)
                .copied()
        }
        PickStrategy::Score => {
            qualified.iter()
                .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
                .copied()
        }
    };

    // Fallback: if neither strategy found a qualified point, pick argmax(rps) over qualified,
    // or over all points if everything is DQ'd (nothing we can do — emit a warning reason).
    let (chosen, method, reasoning) = match (selected, args.pick) {
        (Some(p), PickStrategy::Knee) => {
            let pct = if peak_rps > 0.0 { p.rps / peak_rps * 100.0 } else { 0.0 };
            let base_mult = if baseline.p99_ms > 0.0 { p.p99_ms / baseline.p99_ms } else { 0.0 };
            (
                p,
                "knee",
                format!(
                    "smallest c reaching {:.1}% of peak rps={:.0} @ c={}, p99 at {:.1}× baseline",
                    pct, peak_rps, peak_c, base_mult,
                ),
            )
        }
        (Some(p), PickStrategy::Score) => (
            p,
            "score",
            format!("argmax(rps / (p99/p50)²) = {:.0}", p.score),
        ),
        (None, _) => {
            // All qualified points fail the chosen strategy's gates. Pick
            // depends on which information we have:
            //
            //   Paired mode (overhead_pct available on any point):
            //     pick point with SMALLEST overhead_pct — that is the
            //     least-stressful-for-the-gateway c. Picking argmax(rps)
            //     in paired-mode always lands on the most-overloaded point
            //     (the one with the MOST fast 502s inflating rps).
            //
            //   Non-paired mode:
            //     keep the historical argmax(rps) behavior — it's the
            //     expected "pick the fastest" fallback when only target
            //     data is available.
            let has_overhead = points.iter().any(|p| p.overhead_pct.is_some());
            let fallback: Option<&SweepPoint> = if has_overhead {
                // Lowest overhead_pct first; ties broken by smaller c
                // (safer, less resource pressure).
                points.iter()
                    .filter(|p| p.overhead_pct.is_some())
                    .min_by(|a, b| {
                        let oa = a.overhead_pct.unwrap();
                        let ob = b.overhead_pct.unwrap();
                        oa.partial_cmp(&ob).unwrap_or(std::cmp::Ordering::Equal)
                            .then(a.c.cmp(&b.c))
                    })
            } else {
                qualified.iter()
                    .max_by(|a, b| a.rps.partial_cmp(&b.rps).unwrap_or(std::cmp::Ordering::Equal))
                    .copied()
                    .or_else(|| points.iter().max_by(|a, b| a.rps.partial_cmp(&b.rps).unwrap_or(std::cmp::Ordering::Equal)))
            };
            match fallback {
                Some(p) => {
                    let reason = if has_overhead {
                        let ohd = p.overhead_pct.unwrap_or(0.0);
                        format!(
                            "no point met primary gates; paired-mode fallback = smallest overhead ({:+.1}%)",
                            ohd,
                        )
                    } else {
                        "no point met primary strategy gates; argmax(rps) over all samples".into()
                    };
                    (p, "fallback", reason)
                }
                None => {
                    return SweetSpot {
                        concurrency: 0, rps: 0.0, p50_ms: 0.0, p99_ms: 0.0,
                        method: "none",
                        reasoning: "all sweep points disqualified — endpoint unhealthy".into(),
                        peak_rps: 0.0, peak_concurrency: 0,
                    };
                }
            }
        }
    };

    SweetSpot {
        concurrency: chosen.c,
        rps: chosen.rps,
        p50_ms: chosen.p50_ms,
        p99_ms: chosen.p99_ms,
        method,
        reasoning,
        peak_rps,
        peak_concurrency: peak_c,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Machine snapshot
// ─────────────────────────────────────────────────────────────────────────────

fn capture_machine() -> MachineSnapshot {
    let cpu_cores_logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    let ram_mb = read_ram_mb().unwrap_or(0);
    MachineSnapshot { cpu_cores_logical, ram_mb }
}

fn read_ram_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb: u64 = rest.trim().split_whitespace().next()?.parse().ok()?;
            return Some(kb / 1024);
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Output — text (pretty table for humans)
// ─────────────────────────────────────────────────────────────────────────────

fn print_text(r: &SweepResult) {
    println!();
    println!("━━━ vastar sweep — {} {} ━━━", r.params.method, r.params.url);
    println!();
    println!(
        "  Calibration (c={}):      rps={:<10.0} p50={:.2}ms   p99={:.2}ms",
        r.params.baseline_c, r.baseline.rps, r.baseline.p50_ms, r.baseline.p99_ms,
    );
    if let Some(ref pi) = r.paired {
        let src = match pi.source {
            ReferenceSource::Live => "live",
            ReferenceSource::Cached => "cached",
        };
        println!(
            "  Reference (c={}, {}): rps={:<10.0} p50={:.2}ms   p99={:.2}ms   {} {}",
            r.params.baseline_c, src,
            pi.reference_baseline.rps, pi.reference_baseline.p50_ms, pi.reference_baseline.p99_ms,
            pi.reference_method, pi.reference_url,
        );
        println!(
            "  Overhead gate:           max_overhead_pct={}%, max_rps_deficit_pct={}%",
            pi.max_overhead_pct as i64, pi.max_rps_deficit_pct as i64,
        );
    }
    println!(
        "  Machine:                 {} logical cores, {} MB RAM",
        r.machine.cpu_cores_logical, r.machine.ram_mb,
    );
    println!();
    println!(
        "  Sweep ({} points{}):",
        r.sweep_points.len(),
        if r.params.repeats > 1 { format!(", {} repeats each, median", r.params.repeats) } else { String::new() },
    );
    if r.paired.is_some() {
        println!(
            "    {:<6} {:<10} {:<9} {:<10} {:<9} {:<10}  {}",
            "c", "tgt rps", "tgt p99", "ref rps", "ref p99", "overhead", "verdict",
        );
        println!(
            "    {:<6} {:<10} {:<9} {:<10} {:<9} {:<10}  {}",
            "──────", "──────────", "─────────", "──────────", "─────────", "──────────", "──────────────",
        );
        for p in &r.sweep_points {
            let is_sweet = r.sweet_spot.concurrency == p.c;
            let verdict = match (&p.disqualified, is_sweet) {
                (Some(reason), _) => format!("DISQ ({})", reason),
                (None, true) => "← sweet spot".into(),
                (None, false) => String::new(),
            };
            let (r_rps, r_p99, ohd) = match (&p.reference, p.overhead_pct) {
                (Some(refp), Some(o)) => (
                    format!("{:.0}", refp.rps),
                    format!("{:.2}ms", refp.p99_ms),
                    format!("{:+.1}%", o),
                ),
                _ => ("—".into(), "—".into(), "—".into()),
            };
            println!(
                "    {:<6} {:<10.0} {:<9} {:<10} {:<9} {:<10}  {}",
                p.c, p.rps, format!("{:.2}ms", p.p99_ms),
                r_rps, r_p99, ohd, verdict,
            );
        }
    } else {
        println!(
            "    {:<7} {:<12} {:<10} {:<10} {:<10} {:<10} {:<9}  {}",
            "c", "rps", "p50", "p95", "p99", "p99.9", "score", "verdict",
        );
        println!(
            "    {:<7} {:<12} {:<10} {:<10} {:<10} {:<10} {:<9}  {}",
            "───────", "────────────", "──────────", "──────────", "──────────", "──────────", "─────────", "──────────────",
        );
        for p in &r.sweep_points {
            let is_sweet = r.sweet_spot.concurrency == p.c;
            let verdict = match (&p.disqualified, is_sweet) {
                (Some(reason), _) => format!("DISQ ({})", reason),
                (None, true) => "← sweet spot".into(),
                (None, false) => String::new(),
            };
            println!(
                "    {:<7} {:<12.0} {:<10} {:<10} {:<10} {:<10} {:<9.0}  {}",
                p.c,
                p.rps,
                format!("{:.2}ms", p.p50_ms),
                format!("{:.2}ms", p.p95_ms),
                format!("{:.2}ms", p.p99_ms),
                format!("{:.2}ms", p.p999_ms),
                p.score,
                verdict,
            );
        }
    }
    println!();
    if r.sweet_spot.concurrency == 0 {
        println!("  ━━━ Sweet spot: NONE ━━━");
        println!("  {}", r.sweet_spot.reasoning);
    } else {
        let pct = if r.sweet_spot.peak_rps > 0.0 {
            r.sweet_spot.rps / r.sweet_spot.peak_rps * 100.0
        } else { 0.0 };
        let tail_mult = if r.baseline.p99_ms > 0.0 {
            r.sweet_spot.p99_ms / r.baseline.p99_ms
        } else { 0.0 };
        println!("  ━━━ Sweet spot: c={} ━━━", r.sweet_spot.concurrency);
        println!(
            "  Throughput:   {:.0} req/s   ({:.1}% of peak {:.0} @ c={})",
            r.sweet_spot.rps, pct, r.sweet_spot.peak_rps, r.sweet_spot.peak_concurrency,
        );
        println!(
            "  Latency p99:  {:.2}ms        ({:.1}× baseline c={})",
            r.sweet_spot.p99_ms, tail_mult, r.params.baseline_c,
        );
        println!("  Strategy:     {}", r.sweet_spot.method);
        println!("  Reasoning:    {}", r.sweet_spot.reasoning);
    }
    if !r.notes.is_empty() {
        println!("  Notes:        {}", r.notes.join(", "));
    }
    println!();
}

// ─────────────────────────────────────────────────────────────────────────────
// Output — JSON (hand-formatted, avoids serde dependency)
// ─────────────────────────────────────────────────────────────────────────────

fn format_json(r: &SweepResult) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"schema_version\": \"1.0\",\n");
    // params
    s.push_str("  \"params\": {\n");
    s.push_str(&format!("    \"url\": {},\n", json_string(&r.params.url)));
    s.push_str(&format!("    \"method\": {},\n", json_string(&r.params.method)));
    s.push_str(&format!("    \"pick\": {},\n", json_string(&format!("{:?}", r.params.pick).to_lowercase())));
    s.push_str(&format!("    \"knee_ratio\": {},\n", r.params.knee_ratio));
    s.push_str(&format!("    \"baseline_c\": {},\n", r.params.baseline_c));
    s.push_str(&format!("    \"repeats\": {},\n", r.params.repeats));
    s.push_str(&format!("    \"requests_per_bench\": {},\n", r.params.requests_per_bench));
    s.push_str(&format!("    \"refine\": {}\n", r.params.refine));
    s.push_str("  },\n");
    // machine
    s.push_str("  \"machine\": {\n");
    s.push_str(&format!("    \"cpu_cores_logical\": {},\n", r.machine.cpu_cores_logical));
    s.push_str(&format!("    \"ram_mb\": {}\n", r.machine.ram_mb));
    s.push_str("  },\n");
    // baseline
    s.push_str("  \"baseline\": ");
    s.push_str(&json_point(&r.baseline));
    s.push_str(",\n");
    // sweep_points
    s.push_str("  \"sweep_points\": [\n");
    for (i, p) in r.sweep_points.iter().enumerate() {
        s.push_str("    ");
        s.push_str(&json_point(p));
        if i + 1 < r.sweep_points.len() { s.push(','); }
        s.push('\n');
    }
    s.push_str("  ],\n");
    // sweet_spot
    s.push_str("  \"sweet_spot\": {\n");
    s.push_str(&format!("    \"concurrency\": {},\n", r.sweet_spot.concurrency));
    s.push_str(&format!("    \"rps\": {:.2},\n", r.sweet_spot.rps));
    s.push_str(&format!("    \"p50_ms\": {:.3},\n", r.sweet_spot.p50_ms));
    s.push_str(&format!("    \"p99_ms\": {:.3},\n", r.sweet_spot.p99_ms));
    s.push_str(&format!("    \"method\": {},\n", json_string(r.sweet_spot.method)));
    s.push_str(&format!("    \"reasoning\": {},\n", json_string(&r.sweet_spot.reasoning)));
    s.push_str(&format!("    \"peak_rps\": {:.2},\n", r.sweet_spot.peak_rps));
    s.push_str(&format!("    \"peak_concurrency\": {}\n", r.sweet_spot.peak_concurrency));
    s.push_str("  },\n");
    // notes
    s.push_str("  \"notes\": [");
    for (i, n) in r.notes.iter().enumerate() {
        if i > 0 { s.push_str(", "); }
        s.push_str(&json_string(n));
    }
    s.push_str("]");
    // paired
    if let Some(ref pi) = r.paired {
        s.push_str(",\n  \"paired\": {\n");
        s.push_str(&format!("    \"reference_url\": {},\n", json_string(&pi.reference_url)));
        s.push_str(&format!("    \"reference_method\": {},\n", json_string(&pi.reference_method)));
        s.push_str(&format!(
            "    \"reference_source\": {},\n",
            json_string(match pi.source { ReferenceSource::Live => "live", ReferenceSource::Cached => "cached" }),
        ));
        s.push_str(&format!("    \"max_overhead_pct\": {},\n", pi.max_overhead_pct));
        s.push_str(&format!("    \"max_rps_deficit_pct\": {},\n", pi.max_rps_deficit_pct));
        s.push_str(&format!(
            "    \"reference_baseline\": {{\"rps\":{:.2},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"p999_ms\":{:.3},\"error_rate\":{:.4}}}\n",
            pi.reference_baseline.rps, pi.reference_baseline.p50_ms, pi.reference_baseline.p95_ms,
            pi.reference_baseline.p99_ms, pi.reference_baseline.p999_ms, pi.reference_baseline.error_rate,
        ));
        s.push_str("  }");
    }
    s.push_str("\n}");
    s
}

fn format_ndjson(r: &SweepResult) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "{{\"event\":\"baseline\",\"point\":{}}}\n",
        json_point(&r.baseline),
    ));
    for p in &r.sweep_points {
        s.push_str(&format!("{{\"event\":\"point\",\"point\":{}}}\n", json_point(p)));
    }
    s.push_str(&format!(
        "{{\"event\":\"sweet_spot\",\"concurrency\":{},\"rps\":{:.2},\"p50_ms\":{:.3},\"p99_ms\":{:.3},\"method\":{},\"reasoning\":{}}}",
        r.sweet_spot.concurrency, r.sweet_spot.rps, r.sweet_spot.p50_ms, r.sweet_spot.p99_ms,
        json_string(r.sweet_spot.method), json_string(&r.sweet_spot.reasoning),
    ));
    s
}

fn json_point(p: &SweepPoint) -> String {
    let dq = match &p.disqualified {
        Some(r) => json_string(r),
        None => "null".into(),
    };
    let mut s = format!(
        "{{\"concurrency\":{},\"repeats\":{},\"rps\":{:.2},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"p999_ms\":{:.3},\"error_rate\":{:.4},\"score\":{:.0},\"disqualified\":{}",
        p.c, p.repeats, p.rps, p.p50_ms, p.p95_ms, p.p99_ms, p.p999_ms, p.error_rate, p.score, dq,
    );
    if let Some(ref r) = p.reference {
        s.push_str(&format!(
            ",\"reference\":{{\"rps\":{:.2},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"p999_ms\":{:.3},\"error_rate\":{:.4}}}",
            r.rps, r.p50_ms, r.p95_ms, r.p99_ms, r.p999_ms, r.error_rate,
        ));
    }
    if let Some(o) = p.overhead_pct {
        s.push_str(&format!(",\"overhead_pct\":{:.2}", o));
    }
    if let Some(d) = p.rps_deficit_pct {
        s.push_str(&format!(",\"rps_deficit_pct\":{:.2}", d));
    }
    s.push('}');
    s
}

/// Minimal JSON string escaper — handles `"`, `\\`, `\n`, `\r`, `\t`, control chars.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Base64 — duplicated from main.rs to keep sweep self-contained.
// ─────────────────────────────────────────────────────────────────────────────

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

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_spaced_monotonic() {
        let v = log_spaced(10, 1000, 5);
        assert_eq!(v.len(), 5);
        assert_eq!(v[0], 10);
        assert_eq!(v[4], 1000);
        for i in 1..v.len() {
            assert!(v[i] >= v[i - 1]);
        }
    }

    #[test]
    fn resolve_explicit_list() {
        assert_eq!(resolve_conc_spec("10,50,200", 1), vec![10, 50, 200]);
    }

    #[test]
    fn resolve_step_range() {
        assert_eq!(
            resolve_conc_spec("10..50:step=10", 1),
            vec![10, 20, 30, 40, 50],
        );
    }

    #[test]
    fn json_escape_basic() {
        assert_eq!(json_string("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_string("a\nb"), "\"a\\nb\"");
    }
}
