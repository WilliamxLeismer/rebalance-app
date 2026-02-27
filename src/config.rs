use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

// ── Top-level config ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct Config {
    pub settings: Settings,
    pub accounts: HashMap<String, AccountConfig>,
    pub fund_categories: HashMap<String, String>,
    #[serde(default)]
    pub blend_funds: HashMap<String, BlendFund>,
    pub targets: TargetsConfig,
}

/// A fund whose value is split across multiple asset categories
/// (e.g. a target-date fund like VFFVX).
#[derive(Debug, Clone)]
pub struct BlendFund {
    pub last_updated: NaiveDate,
    /// Category name → percentage of fund value (must sum to ~100.0)
    pub allocations: HashMap<String, f64>,
}

impl BlendFund {
    /// Returns true if last_updated is older than `warn_days` days.
    pub fn is_stale(&self, today: NaiveDate, warn_days: i64) -> bool {
        (today - self.last_updated).num_days() > warn_days
    }
}

impl<'de> Deserialize<'de> for BlendFund {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(BlendFundVisitor)
    }
}

struct BlendFundVisitor;

impl<'de> Visitor<'de> for BlendFundVisitor {
    type Value = BlendFund;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a TOML table with a `last_updated` string and f64 allocation values")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<BlendFund, M::Error> {
        let mut last_updated: Option<NaiveDate> = None;
        let mut allocations: HashMap<String, f64> = HashMap::new();

        while let Some(key) = map.next_key::<String>()? {
            if key == "last_updated" {
                let raw: String = map.next_value()?;
                last_updated = Some(
                    NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
                        .map_err(|e| de::Error::custom(format!("invalid date '{}': {}", raw, e)))?,
                );
            } else {
                let val: f64 = map.next_value()?;
                allocations.insert(key, val);
            }
        }

        let last_updated =
            last_updated.ok_or_else(|| de::Error::missing_field("last_updated"))?;
        Ok(BlendFund { last_updated, allocations })
    }
}

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub contribution_per_paycheck: f64,
    pub drift_warn_threshold: f64,
}

#[derive(Debug, Deserialize)]
pub struct AccountConfig {
    pub name: String,
    pub csv_path: String,
    pub format: String,
}

#[derive(Debug, Deserialize)]
pub struct TargetsConfig {
    pub start: TargetSnapshot,
    pub end: Option<TargetSnapshot>,
}

// ── TargetSnapshot ────────────────────────────────────────────────────────────
// A TOML section that contains a `date` key plus one f64 key per category.
// Example:
//   [targets.start]
//   date        = "2026-02-27"
//   us_stocks   = 50.0
//   intl_stocks = 20.0
//   ...
//
// We use a manual Deserialize impl to pull `date` out and collect the rest
// into the `allocations` HashMap.

#[derive(Debug, Clone)]
pub struct TargetSnapshot {
    pub date: NaiveDate,
    pub allocations: HashMap<String, f64>,
}

impl<'de> Deserialize<'de> for TargetSnapshot {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(TargetSnapshotVisitor)
    }
}

struct TargetSnapshotVisitor;

impl<'de> Visitor<'de> for TargetSnapshotVisitor {
    type Value = TargetSnapshot;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a TOML table with a `date` string and f64 allocation values")
    }

    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<TargetSnapshot, M::Error> {
        let mut date: Option<NaiveDate> = None;
        let mut allocations: HashMap<String, f64> = HashMap::new();

        while let Some(key) = map.next_key::<String>()? {
            if key == "date" {
                let raw: String = map.next_value()?;
                date = Some(
                    NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
                        .map_err(|e| de::Error::custom(format!("invalid date '{}': {}", raw, e)))?,
                );
            } else {
                let val: f64 = map.next_value()?;
                allocations.insert(key, val);
            }
        }

        let date = date.ok_or_else(|| de::Error::missing_field("date"))?;
        Ok(TargetSnapshot { date, allocations })
    }
}

// ── Config loading & validation ───────────────────────────────────────────────

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("could not read config file: {}", path.display()))?;
        let config: Config =
            toml::from_str(&raw).with_context(|| "failed to parse config.toml")?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        self.validate_targets(&self.targets.start, "targets.start")?;
        if let Some(end) = &self.targets.end {
            self.validate_targets(end, "targets.end")?;
            // Both snapshots must define the same set of categories.
            let start_keys: std::collections::HashSet<&String> =
                self.targets.start.allocations.keys().collect();
            let end_keys: std::collections::HashSet<&String> =
                end.allocations.keys().collect();
            if start_keys != end_keys {
                bail!(
                    "[targets.start] and [targets.end] must define the same categories.\n\
                     start has: {:?}\n\
                     end has:   {:?}",
                    start_keys,
                    end_keys
                );
            }
        }

        // All fund_categories values must be valid category keys.
        let valid_categories: std::collections::HashSet<&String> =
            self.targets.start.allocations.keys().collect();
        for (fund, category) in &self.fund_categories {
            if !valid_categories.contains(category) {
                bail!(
                    "fund '{}' maps to unknown category '{}'. \
                     Valid categories: {:?}",
                    fund,
                    category,
                    valid_categories
                );
            }
        }

        // Blend fund allocations must sum to ~100 and use valid categories.
        for (ticker, blend) in &self.blend_funds {
            // last_updated is a separate field deserialized by serde; the
            // remaining keys in `allocations` are category→percentage pairs.
            // Filter out the "last_updated" key if it leaked through flatten.
            let alloc_sum: f64 = blend.allocations.values().sum();
            if (alloc_sum - 100.0).abs() > 0.5 {
                bail!(
                    "[blend_funds.{}] allocations must sum to 100.0, but sum to {:.2}",
                    ticker,
                    alloc_sum
                );
            }
            for cat in blend.allocations.keys() {
                if !valid_categories.contains(cat) {
                    bail!(
                        "[blend_funds.{}] references unknown category '{}'. \
                         Valid categories: {:?}",
                        ticker,
                        cat,
                        valid_categories
                    );
                }
            }
        }

        Ok(())
    }

    /// Returns ticker symbols for blend funds whose `last_updated` date is
    /// older than `warn_days` days.  Called by the `status` command to print
    /// a reminder to refresh the allocation percentages.
    pub fn stale_blend_funds(&self, today: NaiveDate, warn_days: i64) -> Vec<&str> {
        let mut stale = Vec::new();
        for (ticker, blend) in &self.blend_funds {
            if blend.is_stale(today, warn_days) {
                stale.push(ticker.as_str());
            }
        }
        stale.sort();
        stale
    }

    fn validate_targets(&self, snapshot: &TargetSnapshot, label: &str) -> Result<()> {
        if snapshot.allocations.is_empty() {
            bail!("[{}] has no allocation entries", label);
        }
        let sum: f64 = snapshot.allocations.values().sum();
        if (sum - 100.0).abs() > 0.01 {
            bail!(
                "[{}] allocations must sum to 100.0, but sum to {:.4}.\n\
                 Entries: {:?}",
                label,
                sum,
                snapshot.allocations
            );
        }
        Ok(())
    }

    /// Returns the ordered list of category names from [targets.start].
    /// The order is sorted alphabetically for consistent display.
    pub fn categories(&self) -> Vec<String> {
        let mut cats: Vec<String> = self.targets.start.allocations.keys().cloned().collect();
        cats.sort();
        cats
    }

    /// Returns the contribution amount, optionally overridden by the caller.
    pub fn contribution(&self, override_amount: Option<f64>) -> f64 {
        override_amount.unwrap_or(self.settings.contribution_per_paycheck)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(toml_str: &str) -> Result<Config> {
        let config: Config = toml::from_str(toml_str)?;
        config.validate()?;
        Ok(config)
    }

    const VALID_TOML: &str = r#"
[settings]
contribution_per_paycheck = 1500.0
drift_warn_threshold = 5.0

[accounts.vanguard]
name     = "Vanguard Roth IRA"
csv_path = "data/vanguard.csv"
format   = "vanguard"

[fund_categories]
"VTSAX" = "us_stocks"
"VTIAX" = "intl_stocks"

[targets.start]
date        = "2026-02-27"
us_stocks   = 70.0
intl_stocks = 30.0
"#;

    #[test]
    fn valid_config_parses() {
        let cfg = make_config(VALID_TOML).unwrap();
        assert_eq!(cfg.settings.contribution_per_paycheck, 1500.0);
        assert_eq!(cfg.targets.start.allocations["us_stocks"], 70.0);
        assert_eq!(cfg.targets.start.allocations["intl_stocks"], 30.0);
        assert!(cfg.targets.end.is_none());
    }

    #[test]
    fn bad_target_sum_errors() {
        let toml = r#"
[settings]
contribution_per_paycheck = 500.0
drift_warn_threshold = 5.0

[accounts.v]
name = "X"
csv_path = "x.csv"
format = "vanguard"

[fund_categories]

[targets.start]
date      = "2026-01-01"
us_stocks = 60.0
bonds     = 30.0
"#;
        let err = make_config(toml).unwrap_err();
        assert!(err.to_string().contains("sum to 90"));
    }

    #[test]
    fn unknown_category_in_fund_map_errors() {
        let toml = r#"
[settings]
contribution_per_paycheck = 500.0
drift_warn_threshold = 5.0

[accounts.v]
name = "X"
csv_path = "x.csv"
format = "vanguard"

[fund_categories]
"VTSAX" = "typo_category"

[targets.start]
date      = "2026-01-01"
us_stocks = 100.0
"#;
        let err = make_config(toml).unwrap_err();
        assert!(err.to_string().contains("unknown category"));
    }

    #[test]
    fn start_end_category_mismatch_errors() {
        let toml = r#"
[settings]
contribution_per_paycheck = 500.0
drift_warn_threshold = 5.0

[accounts.v]
name = "X"
csv_path = "x.csv"
format = "vanguard"

[fund_categories]

[targets.start]
date      = "2026-01-01"
us_stocks = 100.0

[targets.end]
date  = "2027-01-01"
bonds = 100.0
"#;
        let err = make_config(toml).unwrap_err();
        assert!(err.to_string().contains("same categories"));
    }

    #[test]
    fn contribution_override() {
        let cfg = make_config(VALID_TOML).unwrap();
        assert_eq!(cfg.contribution(None), 1500.0);
        assert_eq!(cfg.contribution(Some(999.0)), 999.0);
    }

    const BLEND_TOML: &str = r#"
[settings]
contribution_per_paycheck = 1500.0
drift_warn_threshold = 5.0

[accounts.v]
name = "Vanguard"
csv_path = "x.csv"
format = "vanguard"

[fund_categories]
"VTSAX" = "us_stocks"

[blend_funds.VFFVX]
last_updated = "2026-02-27"
us_stocks    = 54.4
intl_stocks  = 37.1
us_bonds     = 7.9
cash         = 0.6
real_estate  = 0.0

[targets.start]
date        = "2026-02-27"
us_stocks   = 50.0
intl_stocks = 20.0
us_bonds    = 15.0
cash        = 5.0
real_estate = 10.0
"#;

    #[test]
    fn blend_fund_parses() {
        let cfg = make_config(BLEND_TOML).unwrap();
        let blend = &cfg.blend_funds["VFFVX"];
        assert_eq!(blend.allocations["us_stocks"], 54.4);
        assert_eq!(blend.allocations["intl_stocks"], 37.1);
        assert_eq!(
            blend.last_updated,
            NaiveDate::from_ymd_opt(2026, 2, 27).unwrap()
        );
    }

    #[test]
    fn blend_fund_staleness() {
        let cfg = make_config(BLEND_TOML).unwrap();
        let blend = &cfg.blend_funds["VFFVX"];
        let today = NaiveDate::from_ymd_opt(2026, 2, 27).unwrap();
        assert!(!blend.is_stale(today, 90));

        let future = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap(); // 103 days later
        assert!(blend.is_stale(future, 90));
    }

    #[test]
    fn stale_blend_funds_helper() {
        let cfg = make_config(BLEND_TOML).unwrap();
        let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let stale = cfg.stale_blend_funds(today, 90);
        assert_eq!(stale, vec!["VFFVX"]);

        let recent = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        assert!(cfg.stale_blend_funds(recent, 90).is_empty());
    }

    #[test]
    fn blend_fund_bad_category_errors() {
        let toml = r#"
[settings]
contribution_per_paycheck = 500.0
drift_warn_threshold = 5.0

[accounts.v]
name = "X"
csv_path = "x.csv"
format = "vanguard"

[fund_categories]

[blend_funds.VFFVX]
last_updated = "2026-01-01"
us_stocks    = 100.0

[targets.start]
date      = "2026-01-01"
bonds     = 100.0
"#;
        let err = make_config(toml).unwrap_err();
        assert!(err.to_string().contains("unknown category"));
    }
}
