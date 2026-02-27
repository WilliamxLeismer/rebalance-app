#!/usr/bin/env python3
"""
Empower PDF Statement → CSV Extractor
Reads the "How is my account invested?" table from an Empower quarterly
statement PDF and writes a two-column CSV: Fund Name, Ending Balance.

Usage:
    python3 scripts/empower_pdf_to_csv.py accounts/empower.pdf accounts/empower_positions.csv

The output CSV is then read by rebalance-app as the Empower account source.
Run this script each quarter after downloading your new statement.
"""

import sys
import csv
import re
import pdfplumber


# X-position thresholds (in PDF points) derived from Empower's layout.
# Fund names start near the left margin; ending balance is in the 5th numeric column.
FUND_NAME_MAX_X = 170      # fund name words are left-aligned (x0 < this)
ENDING_BALANCE_MIN_X = 470 # ending balance column starts around x0=481
ENDING_BALANCE_MAX_X = 530

SECTION_START_PHRASE = "How is my account invested?"
SECTION_END_PHRASE   = "How is my account being funded?"
SKIP_ROW_PREFIXES    = ("totals", "beginning", "balance", "largecap", "smallcap",
                         "bond", "international", "stable", "real")


def find_section_page(pdf):
    """Return the page index that contains the investment table."""
    for i, page in enumerate(pdf.pages):
        text = page.extract_text() or ""
        if SECTION_START_PHRASE in text:
            return i
    raise ValueError(f"Could not find '{SECTION_START_PHRASE}' in any page of the PDF.")


def extract_holdings(page):
    """
    Extract (fund_name, ending_balance) pairs from the investment table page.

    Strategy: group words by their vertical position (top), then for each row:
    - Collect left-side words (x0 < FUND_NAME_MAX_X) as the fund name
    - Find the ending balance value in the x-range for that column
    Skip header rows, section-header rows, and the Totals row.
    """
    words = page.extract_words()

    # Group words by rounded top position (±2pt tolerance)
    rows = {}
    for w in words:
        key = round(w["top"] / 2) * 2   # bucket by 2pt increments
        rows.setdefault(key, []).append(w)

    holdings = []

    # Find the top range of the section we care about
    full_text = page.extract_text() or ""
    section_active = False

    for top_key in sorted(rows.keys()):
        row_words = sorted(rows[top_key], key=lambda w: w["x0"])
        row_text  = " ".join(w["text"] for w in row_words).strip()

        # Activate on section start
        if SECTION_START_PHRASE in row_text:
            section_active = True
            continue

        # Deactivate on section end
        if SECTION_END_PHRASE in row_text:
            break

        if not section_active:
            continue

        # Collect fund name words (left side of page).
        # Exclude financial values: words containing commas (e.g. "8,025.14")
        # or decimals with 2+ digits (e.g. "259.72"), but keep bare integers
        # like "500" which can appear in fund names (e.g. "Fidelity 500 Index").
        name_words = [
            w["text"] for w in row_words
            if w["x0"] < FUND_NAME_MAX_X
            and not re.match(r"^-?[\d,]*,\d{3}", w["text"])          # has thousands comma
            and not re.match(r"^-?\d+\.\d{2,}$", w["text"])          # decimal value (e.g. 259.72)
        ]
        if not name_words:
            continue

        fund_name = " ".join(name_words)

        # Skip header / section-label rows
        if fund_name.lower().replace(" ", "").startswith(SKIP_ROW_PREFIXES):
            continue
        if re.match(r"^(Totals?|Beginning|Ending|Change|Deposits|Withdrawals)$",
                    fund_name, re.IGNORECASE):
            continue

        # Find the ending balance value in the expected x column
        balance_words = [
            w["text"] for w in row_words
            if ENDING_BALANCE_MIN_X <= w["x0"] <= ENDING_BALANCE_MAX_X
        ]
        if not balance_words:
            continue

        raw_balance = balance_words[0].replace(",", "").replace("$", "")
        try:
            balance = float(raw_balance)
        except ValueError:
            continue

        if balance <= 0:
            continue

        holdings.append((fund_name, balance))

    return holdings


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <input.pdf> <output.csv>", file=sys.stderr)
        sys.exit(1)

    pdf_path = sys.argv[1]
    csv_path = sys.argv[2]

    print(f"Reading: {pdf_path}")

    with pdfplumber.open(pdf_path) as pdf:
        page_idx = find_section_page(pdf)
        page     = pdf.pages[page_idx]
        holdings = extract_holdings(page)

    if not holdings:
        print("ERROR: No holdings found. The PDF layout may have changed.", file=sys.stderr)
        sys.exit(1)

    with open(csv_path, "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["Fund Name", "Ending Balance"])
        for name, balance in holdings:
            writer.writerow([name, f"{balance:.2f}"])

    print(f"Wrote {len(holdings)} holding(s) to: {csv_path}")
    for name, balance in holdings:
        print(f"  {name:40s}  ${balance:,.2f}")


if __name__ == "__main__":
    main()
