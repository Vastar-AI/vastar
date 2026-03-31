use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::engine::Progress;
use crate::stats::{BenchResult, Percentiles, PhaseDetails};

const PROGRESS_LINES: usize = 6;

/// Live progress display — reads atomics at 10 FPS, zero lock.
/// ASCII only. No emoji. No aggressive clear screen.
/// Uses ANSI cursor-up + line-clear to overwrite in place.
pub async fn render_progress(
    progress: Arc<Progress>,
    total: usize,
    is_duration_mode: bool,
    stop: Arc<AtomicBool>,
) {
    // Skip rendering if stderr is not a terminal (piped/redirected)
    if !std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        // Just wait for stop signal, no output
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if stop.load(Ordering::Relaxed) {
                return;
            }
        }
    }

    let start = Instant::now();
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    let mut first = true;

    // Hide cursor
    eprint!("\x1b[?25l");
    let _ = std::io::stderr().flush();

    loop {
        interval.tick().await;
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let completed = progress.completed.load(Ordering::Relaxed) as usize;
        let errors = progress.errors.load(Ordering::Relaxed);
        let total_ns = progress.total_ns.load(Ordering::Relaxed);
        let elapsed = start.elapsed();

        // Move cursor up to overwrite previous frame
        if !first {
            eprint!("\x1b[{}A", PROGRESS_LINES);
        }
        first = false;

        let rps = if elapsed.as_secs_f64() > 0.001 {
            completed as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };

        let avg_ms = if completed > 0 {
            total_ns as f64 / completed as f64 / 1_000_000.0
        } else {
            0.0
        };

        // Line 1: header
        eprintln!("\x1b[2K  \x1b[38;5;34mvastar\x1b[0m -- running");

        // Line 2: blank
        eprintln!("\x1b[2K");

        // Line 3: progress bar or request count
        if is_duration_mode {
            eprintln!(
                "\x1b[2K  Requests: \x1b[38;5;34m{}\x1b[0m  Errors: {}",
                completed, errors
            );
        } else {
            let pct = if total > 0 { completed * 100 / total } else { 0 };
            let bar_width = 40;
            let filled = (pct * bar_width / 100).min(bar_width);
            // Colored bar: each ■ gets a color from green gradient based on position
            let mut bar = String::with_capacity(filled * 20);
            for i in 0..filled {
                // Gradient: dark green(22) → green(28) → bright green(34) → lime(40) → cyan(34)
                let color = match i * 5 / bar_width.max(1) {
                    0 => 22,
                    1 => 28,
                    2 => 34,
                    3 => 40,
                    _ => 82,
                };
                bar.push_str(&format!("\x1b[38;5;{}m\u{25A0}", color));
            }
            if !bar.is_empty() { bar.push_str("\x1b[0m"); }
            let space = " ".repeat(bar_width.saturating_sub(filled));
            eprintln!(
                "\x1b[2K  [{}{}] \x1b[38;5;34m{}%\x1b[0m  {}/{}",
                bar, space, pct, completed, total
            );
        }

        // Line 4: timing
        eprintln!(
            "\x1b[2K  Elapsed {:.1}s    RPS \x1b[38;5;34m{:.0}\x1b[0m/s    Avg \x1b[38;5;34m{:.2}\x1b[0mms",
            elapsed.as_secs_f64(),
            rps,
            avg_ms
        );

        // Line 5: errors (if any)
        if errors > 0 {
            eprintln!("\x1b[2K  \x1b[38;5;124mErrors: {}\x1b[0m", errors);
        } else {
            eprintln!("\x1b[2K");
        }

        // Line 6: blank
        eprintln!("\x1b[2K");

        let _ = std::io::stderr().flush();
    }

    // Clear progress area
    if !first {
        eprint!("\x1b[{}A", PROGRESS_LINES);
        for _ in 0..PROGRESS_LINES {
            eprintln!("\x1b[2K");
        }
        eprint!("\x1b[{}A", PROGRESS_LINES);
    }

    // Show cursor
    eprint!("\x1b[?25h");
    let _ = std::io::stderr().flush();
}

/// Print final benchmark report to stdout.
pub fn print_report(r: &BenchResult) {
    let successful = r.total_requests as u64 - r.total_errors;

    println!();
    println!("Summary:");
    println!();
    println!("  Total:        {:.4} secs", r.total_duration.as_secs_f64());
    if successful == 0 {
        println!("  Requests/sec: 0.00");
        if r.total_errors > 0 {
            println!();
            println!("Errors: {} total", r.total_errors);
            println!();
            println!("All {} requests failed. Is the target running?", r.total_errors);
            println!();
        }
        return;
    }
    println!("  Slowest:      {:.4} secs", r.max_latency);
    println!("  Fastest:      {:.4} secs", r.min_latency);
    println!("  Average:      {:.4} secs", r.avg_latency);
    println!("  Requests/sec: {:.2}", r.rps);

    if r.total_bytes > 0 {
        let size_per = if r.total_requests > 0 {
            r.total_bytes / r.total_requests as u64
        } else {
            0
        };
        println!("  Total data:   {} bytes", r.total_bytes);
        println!("  Size/request: {} bytes", size_per);
    }
    println!();

    // Response time distribution — colored by SLO level (like oha)
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stdout());
    println!("Response time distribution:");
    println!();
    // Key percentiles highlighted with (ms) — p50, p95, p99, p99.9
    // These 4 are what performance engineers focus on.
    let pcts: &[(&str, f64, bool)] = &[
        ("10.00%", r.percentiles.p10,   false),
        ("25.00%", r.percentiles.p25,   false),
        ("50.00%", r.percentiles.p50,   true),   // median — typical UX
        ("75.00%", r.percentiles.p75,   false),
        ("90.00%", r.percentiles.p90,   false),
        ("95.00%", r.percentiles.p95,   true),   // SLO target
        ("99.00%", r.percentiles.p99,   true),   // tail latency
        ("99.90%", r.percentiles.p999,  true),   // worst case
        ("99.99%", r.percentiles.p9999, false),
    ];
    for (pct, val, key) in pcts {
        if use_color {
            let (color, _) = slo_color(*val, &r.percentiles);
            if *key {
                println!("  {} in {}{:.4}{} secs  ({}{:.2}ms{})", pct, color, val, RESET, color, val * 1000.0, RESET);
            } else {
                println!("  {} in {}{:.4}{} secs", pct, color, val, RESET);
            }
        } else if *key {
            println!("  {} in {:.4} secs  ({:.2}ms)", pct, val, val * 1000.0);
        } else {
            println!("  {} in {:.4} secs", pct, val);
        }
    }

    println!();

    // Histogram — 11-level SLO color gradient
    if !r.histogram.is_empty() {
        let use_color = std::io::IsTerminal::is_terminal(&std::io::stdout());
        println!("Response time histogram:");
        println!();
        let max_count = r.histogram.iter().map(|b| b.count).max().unwrap_or(1).max(1);
        let bar_max = 48;
        for bucket in &r.histogram {
            let bar_len = bucket.count * bar_max / max_count;
            if use_color {
                let (color, _) = slo_color(bucket.mark, &r.percentiles);
                let bar = "\u{25A0}".repeat(bar_len);
                println!("  {:.4} [{}]\t{}{}{}", bucket.mark, bucket.count, color, bar, RESET);
            } else {
                let bar = "#".repeat(bar_len);
                println!("  {:.4} [{}]\t{}", bucket.mark, bucket.count, bar);
            }
        }
        if use_color {
            println!();
            print_slo_legend(&r.percentiles);
        }
        println!();
    }

    // Status code distribution — colored by status class
    if !r.status_dist.is_empty() {
        let use_color = std::io::IsTerminal::is_terminal(&std::io::stdout());
        println!("Status code distribution:");
        println!();
        let mut codes: Vec<_> = r.status_dist.iter().collect();
        codes.sort_by_key(|(k, _)| **k);
        for (code, count) in codes {
            let (color, desc) = status_info(*code, use_color);
            if *code >= 200 && *code < 300 {
                println!("  {}[{}] {} responses{}", color, code, count, RESET);
            } else {
                println!("  {}[{}] {} responses{} -- {}", color, code, count, RESET, desc);
            }
        }
        println!();
    }

    // Details (like hey)
    print_details(&r.details);

    // Errors
    if r.total_errors > 0 {
        println!("Errors: {} total", r.total_errors);
        println!();
    }

    // Insight — include non-2xx as errors
    let non_2xx: usize = r.status_dist.iter()
        .filter(|(k, _)| **k < 200 || **k >= 300)
        .map(|(_, v)| *v)
        .sum();
    let total_failures = r.total_errors as usize + non_2xx;
    let successful = r.total_requests.saturating_sub(total_failures);
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let error_rate = if r.total_requests > 0 {
        total_failures as f64 / r.total_requests as f64
    } else { 0.0 };
    if successful > 0 && use_color {
        print_insight(&r.percentiles, r.rps, r.concurrency, r.avg_latency, error_rate, &r.status_dist);
    } else if successful == 0 && r.total_errors > 0 {
        println!("All {} requests failed. Is the target running?", r.total_errors);
    }
    println!();
}

// Insight severity colors
const C_OK: &str = "\x1b[38;5;34m";    // green — healthy
const C_WARN: &str = "\x1b[38;5;178m"; // dark amber — warning
const C_CRIT: &str = "\x1b[38;5;124m"; // dark red — serious

fn print_insight(
    p: &Percentiles, _rps: f64, _concurrency: usize, _avg_latency: f64,
    error_rate: f64, status_dist: &std::collections::HashMap<u16, usize>,
) {
    let p99_p95 = if p.p95 > 0.0 { p.p99 / p.p95 } else { 1.0 };
    let p999_p99 = if p.p99 > 0.0 { p.p999 / p.p99 } else { 1.0 };
    let spread = if p.p50 > 0.0 { p.p99 / p.p50 } else { 1.0 };
    let p95_p50 = if p.p50 > 0.0 { p.p95 / p.p50 } else { 1.0 };

    println!("Insight:");
    println!();

    // Error rate — most critical, show first
    if error_rate > 0.0 {
        let pct = error_rate * 100.0;
        let non_200: Vec<String> = status_dist.iter()
            .filter(|(k, _)| **k < 200 || **k >= 300)
            .map(|(k, v)| format!("{}x{}", k, v))
            .collect();
        let codes = if non_200.is_empty() { String::new() } else { format!(" ({})", non_200.join(", ")) };
        if pct > 50.0 {
            println!("  {}Error rate: {:.1}%{} -- CRITICAL{}", C_CRIT, pct, RESET, codes);
        } else if pct > 10.0 {
            println!("  {}Error rate: {:.1}%{} -- HIGH{}", C_CRIT, pct, RESET, codes);
        } else if pct > 1.0 {
            println!("  {}Error rate: {:.1}%{} -- elevated{}", C_WARN, pct, RESET, codes);
        } else {
            println!("  {}Error rate: {:.1}%{} -- low{}", C_WARN, pct, RESET, codes);
        }
    }

    // Latency consistency
    if spread <= 1.5 {
        println!("  {}Latency spread p99/p50 = {:.1}x{} -- excellent consistency", C_OK, spread, RESET);
    } else if spread <= 3.0 {
        println!("  {}Latency spread p99/p50 = {:.1}x{} -- good consistency", C_OK, spread, RESET);
    } else if spread <= 5.0 {
        println!("  {}Latency spread p99/p50 = {:.1}x{} -- moderate variance", C_WARN, spread, RESET);
    } else {
        println!("  {}Latency spread p99/p50 = {:.1}x{} -- high variance, investigate slow path", C_CRIT, spread, RESET);
    }

    // Tail latency
    if p99_p95 > 2.0 {
        println!("  {}Tail ratio p99/p95 = {:.1}x{} -- tail latency problem (>2x)", C_CRIT, p99_p95, RESET);
    } else if p99_p95 > 1.5 {
        println!("  {}Tail ratio p99/p95 = {:.1}x{} -- mild tail latency", C_WARN, p99_p95, RESET);
    } else {
        println!("  {}Tail ratio p99/p95 = {:.1}x{} -- clean tail", C_OK, p99_p95, RESET);
    }

    // Outlier detection
    if p999_p99 > 3.0 {
        println!("  {}Outlier ratio p99.9/p99 = {:.1}x{} -- severe outliers (>3x), check GC/infra", C_CRIT, p999_p99, RESET);
    } else if p999_p99 > 2.0 {
        println!("  {}Outlier ratio p99.9/p99 = {:.1}x{} -- outliers present", C_WARN, p999_p99, RESET);
    } else {
        println!("  {}Outlier ratio p99.9/p99 = {:.1}x{} -- no significant outliers", C_OK, p999_p99, RESET);
    }

    // Queuing
    if p95_p50 > 3.0 {
        println!("  {}Queue ratio p95/p50 = {:.1}x{} -- queuing/contention detected (>3x)", C_CRIT, p95_p50, RESET);
    } else if p95_p50 > 2.0 {
        println!("  {}Queue ratio p95/p50 = {:.1}x{} -- mild queuing", C_WARN, p95_p50, RESET);
    }
}

/// HTTP status code color + description
fn status_info(code: u16, use_color: bool) -> (&'static str, &'static str) {
    let desc = match code {
        200 => "OK", 201 => "Created", 204 => "No Content",
        301 => "Moved Permanently", 302 => "Redirect", 304 => "Not Modified",
        400 => "Bad Request", 401 => "Unauthorized", 403 => "Forbidden",
        404 => "Not Found", 405 => "Method Not Allowed", 408 => "Request Timeout",
        429 => "Too Many Requests",
        500 => "Internal Server Error", 501 => "Not Implemented",
        502 => "Bad Gateway", 503 => "Service Unavailable", 504 => "Gateway Timeout",
        _ => "Unknown",
    };
    if !use_color { return ("", desc); }
    let color = match code {
        200..=299 => "\x1b[38;5;34m",  // green
        300..=399 => "\x1b[38;5;178m", // amber
        400..=499 => "\x1b[38;5;202m", // orange-red
        500..=599 => "\x1b[38;5;124m", // dark red
        _ => "",
    };
    (color, desc)
}

fn print_details(d: &PhaseDetails) {
    println!("Details (average, fastest, slowest):");
    println!();
    println!("  req write:\t{:.4} secs, {:.4} secs, {:.4} secs",
        d.req_write.avg, d.req_write.min, d.req_write.max);
    println!("  resp wait:\t{:.4} secs, {:.4} secs, {:.4} secs",
        d.resp_wait.avg, d.resp_wait.min, d.resp_wait.max);
    println!("  resp read:\t{:.4} secs, {:.4} secs, {:.4} secs",
        d.resp_read.avg, d.resp_read.min, d.resp_read.max);
    println!();
}

// ---------------------------------------------------------------------------
// 11-Level SLO Color Gradient
// ---------------------------------------------------------------------------
//
// Maps response latency to SLO health levels using ANSI 256-color.
// Tight quantization: green → lime → yellow → orange → red
//
// | Level | Threshold     | Color        | SLO Status  |
// |-------|---------------|--------------|-------------|
// |  1    | <= p25×0.5    | bright green | elite       |
// |  2    | <= p25        | green        | excellent   |
// |  3    | <= p50        | dark green   | good        |
// |  4    | <= p75        | lime         | normal      |
// |  5    | <= p90        | yellow       | acceptable  |
// |  6    | <= p95        | gold         | degraded    |
// |  7    | <= p99        | orange       | slow        |
// |  8    | <= p99×1.5    | dark orange  | very slow   |
// |  9    | <= p99×2.0    | red-orange   | critical    |
// | 10    | <= p99×3.0    | red          | severe      |
// | 11    | > p99×3.0     | dark red     | violation   |

const RESET: &str = "\x1b[0m";

// 11-level color gradient: deep green → deep red, all solid █ bars.
//
//  Level  Color               ANSI 256
//  ─────  ──────────────────  ────────
//   1     deep green          22
//   2     green               28
//   3     bright green        34
//   4     lime                76
//   5     yellow              184
//   6     gold                220
//   7     orange              208
//   8     dark orange         202
//   9     red-orange          196
//  10     red                 160
//  11     deep red            124

// 11-level gradient using ANSI 256-color cube (index = 16 + 36r + 6g + b).
// Symmetric path through the color cube:
//
//   Green phase    : r=0, g ascending   → dark green to bright green
//   Transition up  : g=5, r ascending   → lime to yellow-green
//   Center         : r=5, g=5           → yellow
//   Transition down: r=5, g descending  → orange
//   Red phase      : g=0, r descending  → red to dark red
//
//  Lvl  (r,g,b)  ANSI   Color
//  ───  ───────  ────   ─────────────
//   1   (0,1,0)   22    dark green
//   2   (0,2,0)   28    green
//   3   (0,3,0)   34    medium green
//   4   (0,4,0)   40    bright green
//   5   (1,5,0)   82    lime
//   6   (3,5,0)  154    yellow-green
//   7   (5,5,0)  226    yellow (center)
//   8   (5,3,0)  214    orange-yellow
//   9   (5,1,0)  202    orange-red
//  10   (4,0,0)  160    red
//  11   (2,0,0)   88    dark red

const SLO_LEVELS: [(&str, &str); 11] = [
    ("\x1b[38;5;22m",  "elite"),      //  (0,1,0) dark green
    ("\x1b[38;5;28m",  "excellent"),   //  (0,2,0) green
    ("\x1b[38;5;34m",  "good"),        //  (0,3,0) medium green
    ("\x1b[38;5;40m",  "normal"),      //  (0,4,0) bright green
    ("\x1b[38;5;82m",  "acceptable"),  //  (1,5,0) lime
    ("\x1b[38;5;154m", "degraded"),    //  (3,5,0) yellow-green
    ("\x1b[38;5;226m", "slow"),        //  (5,5,0) yellow
    ("\x1b[38;5;214m", "very slow"),   //  (5,3,0) orange-yellow
    ("\x1b[38;5;202m", "critical"),    //  (5,1,0) orange-red
    ("\x1b[38;5;160m", "severe"),      //  (4,0,0) red
    ("\x1b[38;5;88m",  "violation"),   //  (2,0,0) dark red
];

fn slo_color(latency: f64, p: &Percentiles) -> (&'static str, &'static str) {
    let level = if latency <= p.p25 * 0.5 {
        0
    } else if latency <= p.p25 {
        1
    } else if latency <= p.p50 {
        2
    } else if latency <= p.p75 {
        3
    } else if latency <= p.p90 {
        4
    } else if latency <= p.p95 {
        5
    } else if latency <= p.p99 {
        6
    } else if latency <= p.p99 * 1.5 {
        7
    } else if latency <= p.p99 * 2.0 {
        8
    } else if latency <= p.p99 * 3.0 {
        9
    } else {
        10
    };
    (SLO_LEVELS[level].0, SLO_LEVELS[level].1)
}

fn print_slo_legend(p: &Percentiles) {
    let fmt_ms = |v: f64| -> String {
        let ms = v * 1000.0;
        if ms < 1.0 { format!("{:.2}ms", ms) }
        else if ms < 100.0 { format!("{:.1}ms", ms) }
        else { format!("{:.0}ms", ms) }
    };

    let thresholds: [String; 11] = [
        format!("<={}", fmt_ms(p.p25 * 0.5)),
        format!("<={}", fmt_ms(p.p25)),
        format!("<={}", fmt_ms(p.p50)),
        format!("<={}", fmt_ms(p.p75)),
        format!("<={}", fmt_ms(p.p90)),
        format!("<={}", fmt_ms(p.p95)),
        format!("<={}", fmt_ms(p.p99)),
        format!("<={}", fmt_ms(p.p99 * 1.5)),
        format!("<={}", fmt_ms(p.p99 * 2.0)),
        format!("<={}", fmt_ms(p.p99 * 3.0)),
        format!(">{}", fmt_ms(p.p99 * 3.0)),
    ];

    // 4 rows × 3 columns (last row has 2)
    const CELL: usize = 26;
    let items: Vec<String> = (0..11)
        .map(|i| format!("{}\u{2588}\u{2588}{} {:<11}{}", SLO_LEVELS[i].0, RESET, SLO_LEVELS[i].1, thresholds[i]))
        .collect();

    println!("  SLO:");
    println!();
    for row in items.chunks(3) {
        print!("  ");
        for item in row {
            let visible_len = strip_ansi_len(item);
            let pad = if CELL > visible_len { CELL - visible_len } else { 0 };
            print!("{}{}", item, " ".repeat(pad));
        }
        println!();
    }
    println!("  \x1b[38;5;242mNote: SLO levels are relative to this run's own percentile distribution,\x1b[0m");
    println!("  \x1b[38;5;242mnot absolute latency thresholds. Define custom SLO targets per your system.\x1b[0m");
}

fn strip_ansi_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c == 'm' { in_esc = false; }
        } else if c == '\x1b' {
            in_esc = true;
        } else {
            len += 1;
        }
    }
    len
}
