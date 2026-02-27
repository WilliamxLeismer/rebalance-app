use anyhow::Result;
use chrono::Local;

use crate::config::Config;
use crate::display;
use crate::portfolio;
use crate::rebalance::new_lazy_rebalance;

/// Run the `status` subcommand.
///
/// 1. Loads all account CSVs
/// 2. Aggregates holdings into category totals (splitting blend funds)
/// 3. Runs one rebalance step for the next contribution
/// 4. Prints status table, drift warnings, next contribution allocation,
///    account breakdown, and any blend-fund staleness warnings
pub fn run(cfg: &Config, contribution_override: Option<f64>) -> Result<()> {
    let contribution = cfg.contribution(contribution_override);

    // Load and aggregate
    let holdings = portfolio::load_all_accounts(cfg)?;
    let totals = portfolio::aggregate(&holdings, cfg);
    let assets = portfolio::to_portfolio_assets(&totals, &cfg.targets.start);

    // Zero-target categories (e.g. cash = 0.0) cause division by zero in the
    // algorithm; split them out, run the rebalancer on the rest, then merge back.
    let (mut zero_target, nonzero): (Vec<_>, Vec<_>) =
        assets.into_iter().partition(|a| a.target_pct() == 0.0);
    let mut balanced = new_lazy_rebalance(contribution, nonzero);
    balanced.append(&mut zero_target);

    // Display
    display::print_status_table(&balanced, cfg.settings.drift_warn_threshold, contribution);
    display::print_contribution_allocation(&balanced);

    let breakdown = portfolio::account_breakdown(&holdings);
    display::print_account_breakdown(&breakdown);

    // Blend fund staleness check
    let today = Local::now().date().naive_local();
    let stale = cfg.stale_blend_funds(today, 90);
    display::print_blend_staleness_warning(&stale);

    Ok(())
}
