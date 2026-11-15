# Changelog

## 0.1.0 (unreleased)

- `shelfcheck check`: OpenAI product feed, ACP feed schema 2026-04-17, Google
  Merchant Center (with the conversational attributes) and GS1 GTIN rules,
  plus cross-format consistency.
- `shelfcheck diff` and `shelfcheck decide`: identity keys, critical
  attribute parsing, change rules and a decisions file.
- Outputs: terminal, report.html, results.json, SARIF 2.1.0,
  failing-products.csv and JUnit XML.
- `openai.variants.listing_has_variations` is a warning: OpenAI keeps such a
  row without its variant options.
- Barcodes with spaces or dashes are read without them for Google, which
  ignores them; they stay an error for the OpenAI product feed and ACP.
- `diff` reports a critical attribute change with no barcode on either side
  when the id, brand and MPN are the same.
- Google's `question_and_answer` is not read in this version; a run that sees
  it says so under "What This Run Did Not Check".
- report.html has a dark theme, and command boxes wrap to show the whole
  command.
- Release archives include THIRD-PARTY-LICENSES.html (cargo-about).
