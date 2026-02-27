# rebalance-app

## TL;DR

```bash
# 1. Build
cargo build --release

# 2. Download your position CSVs from Vanguard and Alight into accounts/
#    For Empower (PDF only), run:
python3 scripts/empower_pdf_to_csv.py accounts/empower.pdf accounts/empower_positions.csv

# 3. Edit config.toml — set your contribution amount, fund mappings, and target %s

# 4. See where you stand and what to do next paycheck
./target/release/rebalance-app status

# 5. Plan the next 6 months of contributions
./target/release/rebalance-app project --months 6
```

---

Retirement portfolio rebalancer. Tells you exactly how to allocate each paycheck contribution across your asset categories to reach your target allocation over time.

Supports multiple brokerage accounts (Vanguard, Empower, Alight/Ford SSIP) combined into a single portfolio view.

## Setup

**1. Install Rust** (if not already installed)
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**2. Build**
```bash
cargo build --release
```

**3. Configure** — copy and edit `config.toml`:
- Set your `contribution_per_paycheck` amount
- Point each account at its CSV file under `accounts/`
- Map your fund tickers/names to the five asset categories
- Set your target allocation percentages in `[targets.start]`

**4. Download your position CSVs** from each brokerage and place them in `accounts/`.
Empower only provides PDF statements — run the extraction script first:
```bash
python3 scripts/empower_pdf_to_csv.py accounts/empower.pdf accounts/empower_positions.csv
```

## Usage

```bash
# Show current portfolio vs targets, drift warnings, and next contribution
./target/release/rebalance-app status

# Plan contributions over the next N months (2 contributions/month assumed)
./target/release/rebalance-app project --months 6

# Override the contribution amount from config
./target/release/rebalance-app --contribution 800 status
./target/release/rebalance-app --contribution 800 project --months 12

# Use a different config file
./target/release/rebalance-app --config ~/my-portfolio.toml status
```

## Asset categories

The five supported categories (defined in `config.toml`):

| Category | Key |
|----------|-----|
| US Stocks | `us_stocks` |
| International Stocks | `intl_stocks` |
| US Bonds | `us_bonds` |
| Cash | `cash` |
| Real Estate | `real_estate` |

## Blend funds

Target-date funds (e.g. VFFVX) can be split across categories in `config.toml`:

```toml
[blend_funds.VFFVX]
last_updated = "2026-02-27"
us_stocks    = 54.4
intl_stocks  = 37.1
us_bonds     = 7.9
cash         = 0.6
real_estate  = 0.0
```

The app warns you when blend fund data is older than 90 days.

## Gradual target changes

Set a future target allocation and the `project` command will interpolate month by month:

```toml
[targets.start]
date        = "2026-02-27"
us_stocks   = 50.0
intl_stocks = 20.0
us_bonds    = 15.0
cash        = 5.0
real_estate = 10.0

[targets.end]
date        = "2027-02-27"
us_stocks   = 45.0
intl_stocks = 20.0
us_bonds    = 20.0
cash        = 5.0
real_estate = 10.0
```

## Files that are gitignored (contain personal data)

```
accounts/    # brokerage CSV exports and Empower PDF
data/        # alternative location for brokerage files
config.toml  # your actual allocation targets and contribution amount
```

Keep these locally only — never commit them.

## Running tests

```bash
cargo test
```

## Algorithm

Uses optimal lazy rebalancing: contributions are distributed to bring underweight assets toward their targets without selling existing holdings, minimising tax events and transaction costs. Based on [optimalrebalancing.tk](http://optimalrebalancing.tk).
