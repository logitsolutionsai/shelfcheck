# Security

shelfcheck reads local files only. It makes no network requests, never
fetches the URLs in a feed, and never changes a store.

## Limits

- 2 GB per file, 5,000,000 rows, 100 KB per cell. All can be set lower in
  `shelfcheck.toml` under `[limits]`.
- XML is read without DTDs or external entities. Only the five XML entities
  and character references are resolved; any other entity is an error.
- Regular expressions come from rule packs only, never from feed data.

## Sensitive Data

Feeds can hold supplier costs or unreleased products. The report and SARIF
hold only the fields a problem needs, and `--redact field,field` removes the
values of more fields.

## Reporting a Vulnerability

Please report it privately through this repository's GitHub security
advisories ("Report a vulnerability" on the Security tab), not in a public
issue.
