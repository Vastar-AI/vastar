use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::engine::Progress;
use crate::stats::BenchResult;

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
        eprintln!("\x1b[2K  jude -- running");

        // Line 2: blank
        eprintln!("\x1b[2K");

        // Line 3: progress bar or request count
        if is_duration_mode {
            eprintln!(
                "\x1b[2K  Requests: {}  Errors: {}",
                completed, errors
            );
        } else {
            let pct = if total > 0 { completed * 100 / total } else { 0 };
            let bar_width = 40;
            let filled = (pct * bar_width / 100).min(bar_width);
            let bar = "=".repeat(filled);
            let arrow = if filled < bar_width { ">" } else { "" };
            let space = " ".repeat(bar_width.saturating_sub(filled + if filled < bar_width { 1 } else { 0 }));
            eprintln!(
                "\x1b[2K  [{}{}{}] {}%  {}/{}",
                bar, arrow, space, pct, completed, total
            );
        }

        // Line 4: timing
        eprintln!(
            "\x1b[2K  Elapsed {:.1}s    RPS {:.0}/s    Avg {:.2}ms",
            elapsed.as_secs_f64(),
            rps,
            avg_ms
        );

        // Line 5: errors (if any)
        if errors > 0 {
            eprintln!("\x1b[2K  Errors: {}", errors);
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
    println!();
    println!("Summary:");
    println!("  Total:        {:.4} secs", r.total_duration.as_secs_f64());
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

    // Latency distribution
    println!("Latency distribution:");
    println!("  10% in {:.4} secs", r.percentiles.p10);
    println!("  25% in {:.4} secs", r.percentiles.p25);
    println!("  50% in {:.4} secs", r.percentiles.p50);
    println!("  75% in {:.4} secs", r.percentiles.p75);
    println!("  90% in {:.4} secs", r.percentiles.p90);
    println!("  95% in {:.4} secs", r.percentiles.p95);
    println!("  99% in {:.4} secs", r.percentiles.p99);
    println!();

    // Histogram
    if !r.histogram.is_empty() {
        println!("Response time histogram:");
        let max_count = r.histogram.iter().map(|b| b.count).max().unwrap_or(1).max(1);
        let bar_max = 48;
        for bucket in &r.histogram {
            let bar_len = bucket.count * bar_max / max_count;
            let bar = "|".repeat(bar_len);
            println!("  {:.4} [{}]\t{}", bucket.mark, bucket.count, bar);
        }
        println!();
    }

    // Status code distribution
    if !r.status_dist.is_empty() {
        println!("Status code distribution:");
        let mut codes: Vec<_> = r.status_dist.iter().collect();
        codes.sort_by_key(|(k, _)| **k);
        for (code, count) in codes {
            println!("  [{}] {} responses", code, count);
        }
        println!();
    }

    // Errors
    if r.total_errors > 0 {
        println!("Errors: {} total", r.total_errors);
        println!();
    }
}
