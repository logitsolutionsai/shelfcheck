# Contributing

## Build and Test

```text
cargo test --workspace
cargo test --release -p shelfcheck --test perf -- --ignored --nocapture
```

The gold tests compare `check` and `diff` on the fixtures with
`fixtures/gold.json`, exactly. If you change a fixture, regenerate both:

```text
python fixtures/generate.py
```

## Rule Packs

Rules are data in `rules/`. Each pack records its source and the date it was
checked against that source, and each rule cites the source's wording.

- When a source changes, update the rule, its `cites` text and the pack's
  `checked` date in the same change.
- New rules use one of the existing kinds. There is no expression language.
- Re-check every pack monthly and before each release.
- `rules/` and `assets/fonts/` are copied into the crates that embed them, so
  `cargo package` includes them. After changing either, run
  `scripts/sync-crate-data.sh`; the `crate_data` tests fail until the copies
  match.

## Snapshots and Licences

- The demo's terminal output and report.html are insta snapshots
  (`crates/shelfcheck-cli/tests/snapshots/`). After an intended change, run
  `INSTA_UPDATE=always cargo test -p shelfcheck --test snapshots` and read the
  diff of the `.snap` files before committing them.
- After any dependency change, regenerate the third-party licence notice:
  `cargo about generate --offline about.hbs -o THIRD-PARTY-LICENSES.html`.
  CI fails while it is out of date.

## Reporting a Stale Rule

Open an issue with the rule id (`shelfcheck rules describe <rule-id>`), a link
to the source page, the text that changed and the date you read it.

## Style

`cargo fmt` and `cargo clippy -- -D warnings`. No `unwrap` or `expect` outside
tests. Output must stay deterministic: sorted, stable ordering.
