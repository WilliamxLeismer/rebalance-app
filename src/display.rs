use std::collections::HashMap;
use std::io::Write;
use tabwriter::TabWriter;

use crate::rebalance::PortfolioAsset;

// ── ANSI colour helpers ───────────────────────────────────────────────────────

const RED:   &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

fn red(s: &str)    -> String { format!("{}{}{}", RED, s, RESET) }
fn yellow(s: &str) -> String { format!("{}{}{}", YELLOW, s, RESET) }

// ── Formatters ────────────────────────────────────────────────────────────────

pub fn fmt_dollars(v: f64) -> String {
    // Manual thousands separator for f64.
    let cents = (v * 100.0).round() as i64;
    let negative = cents < 0;
    let cents = cents.unsigned_abs();
    let dollars = cents / 100;
    let frac = cents % 100;

    let mut s = format!("{}", dollars);
    // Insert commas every 3 digits from the right.
    let mut result = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 { result.push(','); }
        result.push(ch);
    }
    s = result.chars().rev().collect();
    if negative { format!("-${}.{:02}", s, frac) } else { format!("${}.{:02}", s, frac) }
}

pub fn fmt_pct(v: f64) -> String { format!("{:.1}%", v) }

fn fmt_drift(drift_pct: f64, threshold: f64) -> String {
    let s = format!("{:+.1}%", drift_pct);
    if drift_pct.abs() >= threshold {
        red(&s)
    } else if drift_pct.abs() >= threshold * 0.7 {
        yellow(&s)
    } else {
        s
    }
}

// ── Status table ──────────────────────────────────────────────────────────────

pub fn print_status_table(
    assets: &[PortfolioAsset],
    drift_threshold: f64,
    contribution: f64,
) {
    let total_value: f64 = assets.iter().map(|a| a.value()).sum();

    println!(
        "\nPortfolio Status  —  Total: {}  |  Next contribution: {}",
        fmt_dollars(total_value),
        fmt_dollars(contribution),
    );

    // Build a tab-separated string for tabwriter alignment.
    let mut buf = String::from(
        "Category\tCurrent $\tCurrent %\tTarget %\tDrift\tStatus\n",
    );

    let mut any_warn = false;

    for asset in assets {
        let drift_pct = asset.actual_pct() - asset.target_pct();
        let is_warn = drift_pct.abs() >= drift_threshold;
        if is_warn { any_warn = true; }

        let status = if is_warn {
            red("⚠ WARNING")
        } else {
            "OK".to_string()
        };

        buf.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            asset.name(),
            fmt_dollars(asset.value()),
            fmt_pct(asset.actual_pct()),
            fmt_pct(asset.target_pct()),
            fmt_drift(drift_pct, drift_threshold),
            status,
        ));
    }

    let tw_output = tabwrite(&buf);
    print!("{}", tw_output);

    if any_warn {
        println!(
            "\n{}Drift threshold is {:.1}%. Update contributions to reduce drift.{}",
            RED, drift_threshold, RESET
        );
    }
}

// ── Next-contribution allocation ──────────────────────────────────────────────

pub fn print_contribution_allocation(assets: &[PortfolioAsset]) {
    println!("\nNext contribution allocation:");
    let contributing: Vec<_> = assets
        .iter()
        .filter(|a| a.contribution_amount() > 0.01)
        .collect();

    if contributing.is_empty() {
        println!("  (portfolio is at target — no allocation needed)");
        return;
    }

    let mut buf = String::new();
    for asset in contributing {
        buf.push_str(&format!(
            "  {}\t{}\n",
            asset.name(),
            fmt_dollars(asset.contribution_amount()),
        ));
    }
    print!("{}", tabwrite(&buf));
}

// ── Account breakdown ─────────────────────────────────────────────────────────

pub fn print_account_breakdown(
    breakdown: &HashMap<String, Vec<(String, f64)>>,
) {
    println!("\nAccount breakdown:");
    let mut accounts: Vec<&String> = breakdown.keys().collect();
    accounts.sort();

    for acct in accounts {
        let mut funds = breakdown[acct].clone();
        funds.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        let entries: Vec<String> = funds
            .iter()
            .map(|(name, val)| format!("{} {}", name, fmt_dollars(*val)))
            .collect();

        println!("  {}:  {}", acct, entries.join("  |  "));
    }
}

// ── Staleness warning ─────────────────────────────────────────────────────────

pub fn print_blend_staleness_warning(stale: &[&str]) {
    if stale.is_empty() { return; }
    println!(
        "\n{}NOTE: Blend fund allocation data is outdated (>90 days) for: {}.{}",
        YELLOW,
        stale.join(", "),
        RESET,
    );
    println!(
        "{}Update the percentages and last_updated date in [blend_funds] in config.toml.{}",
        YELLOW, RESET
    );
}

// ── Projection table ──────────────────────────────────────────────────────────

pub struct ProjectionRow {
    pub period: usize,
    pub date: String,
    pub contributions: HashMap<String, f64>,
    pub total_contribution: f64,
    pub portfolio_value_after: f64,
    pub allocations_after: HashMap<String, f64>,
}

pub fn print_projection_table(
    rows: &[ProjectionRow],
    categories: &[String],
    contribution: f64,
    months: usize,
) {
    println!(
        "\nProjection — {} contributions over {} month(s)  (contribution: {})\n",
        rows.len(),
        months,
        fmt_dollars(contribution),
    );

    // Header
    let mut header = "Period\tDate".to_string();
    for cat in categories {
        header.push('\t');
        header.push_str(cat);
    }
    header.push_str("\tTotal\n");

    let mut buf = header;

    for row in rows {
        let mut line = format!("{}\t{}", row.period, row.date);
        for cat in categories {
            let amt = row.contributions.get(cat).copied().unwrap_or(0.0);
            line.push('\t');
            line.push_str(&fmt_dollars(amt));
        }
        line.push('\t');
        line.push_str(&fmt_dollars(row.total_contribution));
        line.push('\n');
        buf.push_str(&line);
    }

    print!("{}", tabwrite(&buf));

    // Summary
    if let Some(last) = rows.last() {
        println!("\nEnd state: {}", fmt_dollars(last.portfolio_value_after));
        print!("Allocation: ");
        let parts: Vec<String> = categories
            .iter()
            .map(|cat| {
                let pct = last.allocations_after.get(cat).copied().unwrap_or(0.0);
                format!("{} {}", cat, fmt_pct(pct))
            })
            .collect();
        println!("{}", parts.join("  "));
    }
}

// ── tabwriter helper ──────────────────────────────────────────────────────────

fn tabwrite(s: &str) -> String {
    let mut tw = TabWriter::new(vec![]);
    tw.write_all(s.as_bytes()).unwrap();
    tw.flush().unwrap();
    String::from_utf8(tw.into_inner().unwrap()).unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_dollars_basic() {
        assert_eq!(fmt_dollars(1234567.89), "$1,234,567.89");
        assert_eq!(fmt_dollars(0.0), "$0.00");
        assert_eq!(fmt_dollars(500.0), "$500.00");
        assert_eq!(fmt_dollars(-250.50), "-$250.50");
    }

    #[test]
    fn fmt_pct_basic() {
        assert_eq!(fmt_pct(18.9), "18.9%");
        assert_eq!(fmt_pct(100.0), "100.0%");
    }
}
