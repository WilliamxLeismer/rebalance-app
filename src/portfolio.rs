use std::collections::HashMap;

use crate::config::{Config, TargetSnapshot};
use crate::csv_import::{normalise, BrokerageFormat, RawHolding};
use crate::rebalance::PortfolioAsset;
use anyhow::Result;

// ── Load all accounts ─────────────────────────────────────────────────────────

/// Parse every configured account CSV and return the combined raw holdings.
/// Unmapped funds are printed to stderr so the user can add them to config.
pub fn load_all_accounts(cfg: &Config) -> Result<Vec<RawHolding>> {
    let mut all: Vec<RawHolding> = Vec::new();
    for (key, acct) in &cfg.accounts {
        let fmt = BrokerageFormat::from_str(&acct.format)
            .map_err(|e| anyhow::anyhow!("account '{}': {}", key, e))?;
        let path = std::path::Path::new(&acct.csv_path);
        let holdings =
            crate::csv_import::parse_csv(path, fmt, &acct.name)?;
        all.extend(holdings);
    }
    Ok(all)
}

// ── Aggregate to category values ──────────────────────────────────────────────

/// Aggregate raw holdings into category totals.
///
/// Blend funds are split across categories using the percentages in
/// `cfg.blend_funds`.  Unmapped funds are reported to stderr and excluded.
/// Returns a map of category → total dollar value.
pub fn aggregate(holdings: &[RawHolding], cfg: &Config) -> HashMap<String, f64> {
    // Initialise all categories at 0 so even empty ones appear in the output.
    let mut totals: HashMap<String, f64> = cfg
        .categories()
        .into_iter()
        .map(|c| (c, 0.0))
        .collect();

    for h in holdings {
        let key = normalise(&h.fund_name);

        // Check blend funds first (e.g. VFFVX).
        if let Some(blend) = cfg.blend_funds.get(&key) {
            for (cat, pct) in &blend.allocations {
                *totals.entry(cat.clone()).or_insert(0.0) += h.value * pct / 100.0;
            }
            continue;
        }

        // Check direct fund→category mapping.
        if let Some(cat) = cfg.fund_categories.get(&key) {
            *totals.entry(cat.clone()).or_insert(0.0) += h.value;
            continue;
        }

        // Unmapped — warn the user so they can add it to config.
        eprintln!(
            "WARNING: '{}' (${:.2}, account: {}) has no category mapping — skipped.\n\
             Add it to [fund_categories] in config.toml.",
            h.fund_name, h.value, h.account_name
        );
    }

    totals
}

// ── Convert to PortfolioAsset vec ─────────────────────────────────────────────

/// Convert category totals into the Vec<PortfolioAsset> that the rebalancing
/// algorithm expects.  Target allocations are taken from `snapshot`.
pub fn to_portfolio_assets(
    totals: &HashMap<String, f64>,
    snapshot: &TargetSnapshot,
) -> Vec<PortfolioAsset> {
    let mut assets: Vec<PortfolioAsset> = totals
        .iter()
        .map(|(cat, &value)| {
            let target_pct = snapshot.allocations.get(cat).copied().unwrap_or(0.0);
            PortfolioAsset::new(cat.clone(), value, target_pct / 100.0)
        })
        .collect();

    // Consistent ordering for display (alphabetical by category name).
    assets.sort_by(|a, b| a.name().cmp(b.name()));
    assets
}

// ── Account breakdown ─────────────────────────────────────────────────────────

/// Returns a map of account_name → Vec<(fund_name, value)> for display.
pub fn account_breakdown(holdings: &[RawHolding]) -> HashMap<String, Vec<(String, f64)>> {
    let mut map: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for h in holdings {
        map.entry(h.account_name.clone())
            .or_default()
            .push((h.fund_name.clone(), h.value));
    }
    map
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TargetSnapshot;
    use chrono::NaiveDate;

    fn make_snapshot(allocs: &[(&str, f64)]) -> TargetSnapshot {
        TargetSnapshot {
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            allocations: allocs.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
        }
    }

    fn make_holding(account: &str, fund: &str, value: f64) -> RawHolding {
        RawHolding {
            account_name: account.to_string(),
            fund_name: fund.to_string(),
            value,
        }
    }

    fn make_cfg_from_toml(s: &str) -> Config {
        toml::from_str(s).unwrap()
    }

    const BASE_TOML: &str = r#"
[settings]
contribution_per_paycheck = 500.0
drift_warn_threshold = 5.0

[accounts.v]
name = "Vanguard"
csv_path = "x.csv"
format = "vanguard"

[fund_categories]
"VTSAX" = "us_stocks"
"VTIAX" = "intl_stocks"

[targets.start]
date        = "2026-01-01"
us_stocks   = 70.0
intl_stocks = 30.0
"#;

    #[test]
    fn aggregate_maps_funds_to_categories() {
        let cfg = make_cfg_from_toml(BASE_TOML);
        let holdings = vec![
            make_holding("Vanguard", "VTSAX", 7000.0),
            make_holding("Vanguard", "VTIAX", 3000.0),
        ];
        let totals = aggregate(&holdings, &cfg);
        assert_eq!(totals["us_stocks"], 7000.0);
        assert_eq!(totals["intl_stocks"], 3000.0);
    }

    #[test]
    fn aggregate_initialises_zero_categories() {
        let cfg = make_cfg_from_toml(BASE_TOML);
        let holdings = vec![make_holding("Vanguard", "VTSAX", 5000.0)];
        let totals = aggregate(&holdings, &cfg);
        // intl_stocks not in holdings but still present at 0
        assert_eq!(totals.get("intl_stocks"), Some(&0.0));
    }

    #[test]
    fn aggregate_splits_blend_fund() {
        let toml = r#"
[settings]
contribution_per_paycheck = 500.0
drift_warn_threshold = 5.0

[accounts.v]
name = "Vanguard"
csv_path = "x.csv"
format = "vanguard"

[fund_categories]

[blend_funds.VFFVX]
last_updated = "2026-01-01"
us_stocks    = 60.0
intl_stocks  = 40.0

[targets.start]
date        = "2026-01-01"
us_stocks   = 70.0
intl_stocks = 30.0
"#;
        let cfg = make_cfg_from_toml(toml);
        let holdings = vec![make_holding("Vanguard", "VFFVX", 1000.0)];
        let totals = aggregate(&holdings, &cfg);
        assert!((totals["us_stocks"] - 600.0).abs() < 0.01);
        assert!((totals["intl_stocks"] - 400.0).abs() < 0.01);
    }

    #[test]
    fn to_portfolio_assets_sets_targets() {
        let snapshot = make_snapshot(&[("us_stocks", 70.0), ("intl_stocks", 30.0)]);
        let totals = HashMap::from([
            ("us_stocks".to_string(), 7000.0),
            ("intl_stocks".to_string(), 3000.0),
        ]);
        let assets = to_portfolio_assets(&totals, &snapshot);
        assert_eq!(assets.len(), 2);
        let us = assets.iter().find(|a| a.name() == "us_stocks").unwrap();
        assert_eq!(us.value(), 7000.0);
        assert!((us.target_pct() - 70.0).abs() < 0.001);
    }
}
