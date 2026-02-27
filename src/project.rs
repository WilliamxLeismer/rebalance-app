use std::collections::HashMap;

use anyhow::Result;
use chrono::{Duration, Local, NaiveDate};

use crate::config::{Config, TargetSnapshot};
use crate::display::{self, ProjectionRow};
use crate::portfolio;
use crate::rebalance::{new_lazy_rebalance, PortfolioAsset};

/// Run the `project` subcommand.
///
/// Simulates `months * 2` semi-monthly contributions, iteratively advancing
/// the portfolio state after each one.  If [targets.end] is set, target
/// allocations are linearly interpolated period-by-period.
pub fn run(cfg: &Config, months: usize, contribution_override: Option<f64>) -> Result<()> {
    let contribution = cfg.contribution(contribution_override);
    let periods = months * 2;

    // Load initial portfolio state
    let holdings = portfolio::load_all_accounts(cfg)?;
    let totals = portfolio::aggregate(&holdings, cfg);

    let start_date = Local::now().date().naive_local();
    let categories = cfg.categories();

    // Build initial assets using the start target
    let mut current_assets =
        portfolio::to_portfolio_assets(&totals, &cfg.targets.start);

    let mut rows: Vec<ProjectionRow> = Vec::with_capacity(periods);

    for period in 0..periods {
        // 1. Compute interpolated target for this period
        let target = interpolate_targets(cfg, period, periods);

        // 2. Update each asset's target allocation
        for asset in &current_assets {
            let _ = asset; // satisfy unused warning — rebuilt below
        }
        current_assets = rebuild_with_target(&current_assets, &target);

        // 3. Run one rebalance step (zero-target assets excluded to avoid ÷0)
        let (mut zero_target, nonzero): (Vec<_>, Vec<_>) =
            current_assets.clone().into_iter().partition(|a| a.target_pct() == 0.0);
        let mut result = new_lazy_rebalance(contribution, nonzero);
        result.append(&mut zero_target);

        // 4. Record this period's output
        let period_date = period_date(start_date, period);
        let port_value_after: f64 =
            result.iter().map(|a| a.value() + a.contribution_amount()).sum();
        let total_contribution: f64 = result.iter().map(|a| a.contribution_amount()).sum();

        let contributions: HashMap<String, f64> = result
            .iter()
            .map(|a| (a.name().to_string(), a.contribution_amount()))
            .collect();

        let allocs_after: HashMap<String, f64> = result
            .iter()
            .map(|a| {
                let new_val = a.value() + a.contribution_amount();
                (a.name().to_string(), new_val / port_value_after * 100.0)
            })
            .collect();

        rows.push(ProjectionRow {
            period: period + 1,
            date: period_date.format("%Y-%m-%d").to_string(),
            contributions,
            total_contribution,
            portfolio_value_after: port_value_after,
            allocations_after: allocs_after,
        });

        // 5. Advance portfolio state for next period
        current_assets = advance_state(result);
    }

    display::print_projection_table(&rows, &categories, contribution, months);
    Ok(())
}

// ── Target interpolation ──────────────────────────────────────────────────────

/// Linearly interpolate between start and end targets based on the actual
/// calendar dates in the config, not the projection period count.
///
/// `t` is the fraction of the way from `targets.start.date` to
/// `targets.end.date` that this period's date falls on (clamped 0.0–1.0).
/// If there is no end target, returns start allocations unchanged.
fn interpolate_targets(cfg: &Config, period: usize, _total_periods: usize) -> TargetSnapshot {
    let start = &cfg.targets.start;

    let end = match &cfg.targets.end {
        Some(e) => e,
        None => return start.clone(),
    };

    let today = Local::now().date().naive_local();
    let period_date = today + Duration::days(15 * period as i64);
    let total_days = (end.date - start.date).num_days();
    if total_days <= 0 {
        return start.clone();
    }
    let elapsed = (period_date - start.date).num_days();
    let t = (elapsed as f64 / total_days as f64).clamp(0.0, 1.0);

    let mut allocations: HashMap<String, f64> = HashMap::new();
    for (cat, &start_pct) in &start.allocations {
        let end_pct = end.allocations.get(cat).copied().unwrap_or(start_pct);
        allocations.insert(cat.clone(), start_pct + t * (end_pct - start_pct));
    }

    // Normalise to ensure exact 100.0 sum (guards against floating-point drift)
    let sum: f64 = allocations.values().sum();
    if sum > 0.0 {
        for v in allocations.values_mut() {
            *v = *v / sum * 100.0;
        }
    }

    TargetSnapshot {
        date: start.date,
        allocations,
    }
}

// ── State helpers ─────────────────────────────────────────────────────────────

/// Re-build the asset list with new target allocations from `snapshot`.
/// Values are carried forward unchanged; targets are replaced.
fn rebuild_with_target(
    assets: &[PortfolioAsset],
    snapshot: &TargetSnapshot,
) -> Vec<PortfolioAsset> {
    assets
        .iter()
        .map(|a| {
            let target_pct = snapshot.allocations.get(a.name()).copied().unwrap_or(0.0);
            PortfolioAsset::new(a.name().to_string(), a.value(), target_pct / 100.0)
        })
        .collect()
}

/// Advance the portfolio state by adding each asset's contribution to its value.
/// Returns a fresh Vec<PortfolioAsset> ready for the next period.
fn advance_state(result: Vec<PortfolioAsset>) -> Vec<PortfolioAsset> {
    result
        .iter()
        .map(|a| {
            let new_value = a.value() + a.contribution_amount();
            // target_pct will be updated by rebuild_with_target next iteration
            PortfolioAsset::new(a.name().to_string(), new_value, a.target_pct() / 100.0)
        })
        .collect()
}

/// Approximate semi-monthly dates: period 0 → start, then +15 days per period.
fn period_date(start: NaiveDate, period: usize) -> NaiveDate {
    start + Duration::days(15 * period as i64)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn make_cfg(start: &[(&str, f64)], end: Option<&[(&str, f64)]>) -> Config {
        let end_section = match end {
            Some(e) => {
                let entries: Vec<String> =
                    e.iter().map(|(k, v)| format!("{} = {}", k, v)).collect();
                format!(
                    "\n[targets.end]\ndate = \"2027-01-01\"\n{}",
                    entries.join("\n")
                )
            }
            None => String::new(),
        };

        let start_entries: Vec<String> = start
            .iter()
            .map(|(k, v)| format!("{} = {}", k, v))
            .collect();
        let toml = format!(
            r#"
[settings]
contribution_per_paycheck = 1000.0
drift_warn_threshold = 5.0

[accounts.v]
name = "V"
csv_path = "x.csv"
format = "vanguard"

[fund_categories]

[targets.start]
date = "2026-01-01"
{}
{}
"#,
            start_entries.join("\n"),
            end_section,
        );
        toml::from_str(&toml).unwrap()
    }

    #[test]
    fn interpolate_at_zero() {
        let cfg = make_cfg(&[("stocks", 80.0), ("bonds", 20.0)],
                           Some(&[("stocks", 60.0), ("bonds", 40.0)]));
        let result = interpolate_targets(&cfg, 0, 10);
        assert!((result.allocations["stocks"] - 80.0).abs() < 0.01);
        assert!((result.allocations["bonds"]  - 20.0).abs() < 0.01);
    }

    #[test]
    fn interpolate_at_end() {
        let cfg = make_cfg(&[("stocks", 80.0), ("bonds", 20.0)],
                           Some(&[("stocks", 60.0), ("bonds", 40.0)]));
        let result = interpolate_targets(&cfg, 9, 10);
        assert!((result.allocations["stocks"] - 60.0).abs() < 0.5);
        assert!((result.allocations["bonds"]  - 40.0).abs() < 0.5);
    }

    #[test]
    fn interpolate_midpoint() {
        let cfg = make_cfg(&[("stocks", 80.0), ("bonds", 20.0)],
                           Some(&[("stocks", 60.0), ("bonds", 40.0)]));
        let result = interpolate_targets(&cfg, 5, 10);
        // t = 5/9 ≈ 0.556: stocks ≈ 80 - 0.556*20 ≈ 68.9
        assert!(result.allocations["stocks"] > 60.0);
        assert!(result.allocations["stocks"] < 80.0);
    }

    #[test]
    fn interpolate_no_end_returns_start() {
        let cfg = make_cfg(&[("stocks", 80.0), ("bonds", 20.0)], None);
        let result = interpolate_targets(&cfg, 5, 10);
        assert!((result.allocations["stocks"] - 80.0).abs() < 0.01);
    }

    #[test]
    fn advance_state_adds_contribution() {
        let assets = vec![
            PortfolioAsset::new("stocks".to_string(), 8000.0, 0.80),
            PortfolioAsset::new("bonds".to_string(),  2000.0, 0.20),
        ];
        let result = new_lazy_rebalance(1000.0, assets);
        let next = advance_state(result);
        let total: f64 = next.iter().map(|a| a.value()).sum();
        assert!((total - 11000.0).abs() < 0.01);
    }

    #[test]
    fn interpolate_normalises_to_100() {
        let cfg = make_cfg(&[("stocks", 80.0), ("bonds", 20.0)],
                           Some(&[("stocks", 60.0), ("bonds", 40.0)]));
        for period in 0..10 {
            let result = interpolate_targets(&cfg, period, 10);
            let sum: f64 = result.allocations.values().sum();
            assert!((sum - 100.0).abs() < 0.001);
        }
    }

    #[test]
    fn period_date_advances_by_15_days() {
        let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        assert_eq!(period_date(start, 0), start);
        assert_eq!(
            period_date(start, 1),
            NaiveDate::from_ymd_opt(2026, 3, 16).unwrap()
        );
        assert_eq!(
            period_date(start, 2),
            NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()
        );
    }
}
