# Input Formats

shelfcheck reads every format into one record model. Each record is one item
(a row, an XML item, or one variant of an ACP product) and keeps every value
as written, with its file, line, and column or JSON pointer.

## Detection

Detection uses the file name, then the headers or root element. `--format`
overrides it.

| Input | Detected by | `--format` |
|---|---|---|
| OpenAI CSV, TSV or JSONL | OpenAI field names such as `item_id`, `target_countries`, `seller_name` | `openai-csv`, `openai-tsv`, `openai-jsonl` |
| Google Merchant Center XML | an `<rss>` or `<feed>` root; items are `<item>` or `<entry>`. Values come from element text; attributes such as Atom's `<link href>` are not read yet | `google-xml` |
| Google CSV or TSV | Google names such as `link`, `image_link`, `item_group_id`, or `g:` prefixes | `google-csv`, `google-tsv` |
| ACP bundle | a folder with `products.jsonl` (and `metadata.json`), or JSONL lines with `variants` | `acp` |
| Shopify product CSV export | `Handle` with `Variant SKU`, `Variant Price` or `Variant Barcode` | `shopify` |
| Anything else | CSV, TSV or JSONL read with the OpenAI names, then the Google names | `csv`, `tsv`, `jsonl` |

## Canonical Fields

Canonical field names are the OpenAI names (`item_id`, `url`, `image_url`,
`additional_image_urls`, `group_id`...). Google-only attributes keep their
Google names (`question_and_answer`, `variant_option`, `popularity_rank`...).
The built-in maps are in `crates/shelfcheck-core/src/map/`.

- Google: `id` is `item_id`, `link` is `url`, `image_link` is `image_url`,
  `additional_image_link` is `additional_image_urls`, `item_group_id` is
  `group_id`, `google_product_category` is `product_category`. Names are read
  with or without `g:`.
- Shopify export: `Variant SKU` is `item_id`, `Handle` is `group_id`,
  `Vendor` is `brand`, `Variant Barcode` is `gtin`, `Variant Price` is
  `price`, `Type` is `product_type`, `Option1 Name`/`Option1 Value` to
  `Option3` are the variant options. Product columns appear only on the first
  row of each Handle, so they are filled down; image-only rows are skipped.
- ACP: each variant becomes one item. `variants[].id` is `item_id`, the
  product `id` is `group_id`, `barcodes[0].value` is `gtin`, `price` in minor
  units becomes `79.99 USD`, `availability.status` is `availability`,
  `variant_options` are the options.

Custom maps go in `shelfcheck.toml`:

```toml
[map]
"My SKU" = "item_id"
"Colour" = "color"
```

`shelfcheck init <feed>` writes a `shelfcheck.toml` listing the columns the
built-in map does not know.

## Variant Options

Options come from `variant_dict` (a JSON object or `name:value` pairs),
`variant_option` (`Memory size:128 GB`), the Shopify option columns, ACP
`variant_options`, and the `color`, `size`, `material` and `pattern`
attributes. `diff` reads critical attributes from the options, and from the
title only when an item has no options.

## Limits

2 GB per file, 5,000,000 rows, 100 KB per cell. Set them lower under
`[limits]` in `shelfcheck.toml`:

```toml
[limits]
max_file_bytes = 500000000
max_rows = 1000000
max_cell_bytes = 20000
```
