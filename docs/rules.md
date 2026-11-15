# Rules

Generated from the rule packs in `rules/` on 2026-11-15. `shelfcheck rules list` prints the same list, and `shelfcheck rules describe <rule-id>` prints one rule in full.

## OpenAI product feed

- Pack id: `openai-product-feed`, file `rules/openai-product-feed/checked-2026-11-14.toml`
- Source: https://developers.openai.com/commerce/specs/file-upload/products
- Version: as published, checked 2026-11-14; checked 2026-11-14
- Note: Conditionally required but not encoded, because the condition is not in the feed itself: is_ads_eligible (for ads) and marketplace_seller.
- Note: mpn goes with brand. brand is one of the eleven required fields, so an mpn without a brand is reported as a missing brand.
- Note: Never invent an mpn to replace a missing gtin.

| Rule | Field | Kind | Severity | Cites |
|---|---|---|---|---|
| `openai.required.item_id` | `item_id` | required | error | item_id: required (one of the 11 required fields) |
| `openai.required.title` | `title` | required | error | title: required (one of the 11 required fields) |
| `openai.required.description` | `description` | required | error | description: required (one of the 11 required fields) |
| `openai.required.url` | `url` | required | error | url: required (one of the 11 required fields) |
| `openai.required.brand` | `brand` | required | error | brand: required (one of the 11 required fields) |
| `openai.required.seller_name` | `seller_name` | required | error | seller_name: required (one of the 11 required fields) |
| `openai.required.image_url` | `image_url` | required | error | image_url: required (one of the 11 required fields) |
| `openai.required.availability` | `availability` | required | error | availability: required (one of the 11 required fields) |
| `openai.required.price` | `price` | required | error | price: required (one of the 11 required fields) |
| `openai.required.target_countries` | `target_countries` | required | error | target_countries: required (one of the 11 required fields) |
| `openai.required.store_country` | `store_country` | required | error | store_country: required (one of the 11 required fields) |
| `openai.price.form` | `price` | price | error | price: major units and an uppercase ISO 4217 code, such as 79.99 USD |
| `openai.sale_price.form` | `sale_price` | price | warning | sale_price: above 0, below price, same currency; otherwise ignored |
| `openai.sale_price.below_price` | `sale_price` | less_than_field | warning | sale_price: above 0, below price, same currency; otherwise ignored |
| `openai.availability.enum` | `availability` | enum | error | availability: in_stock, out_of_stock, pre_order, backorder or unknown |
| `openai.url.absolute` | `url` | url_absolute | error | url: public, absolute http(s) URL |
| `openai.image_url.absolute` | `image_url` | url_absolute | error | image_url: public, absolute http(s) URL |
| `openai.additional_image_urls.absolute` | `additional_image_urls` | url_absolute | error | additional_image_urls: public, absolute http(s) URLs; in CSV a comma inside a URL is written %2C |
| `openai.title.length` | `title` | max_len | warning | title: aim for up to 150 characters |
| `openai.description.length` | `description` | max_len | warning | description: aim for up to 5,000 characters |
| `openai.listing_has_variations.enum` | `listing_has_variations` | enum | error | listing_has_variations: true or false |
| `openai.variants.listing_has_variations` | `listing_has_variations` | required_if | warning | listing_has_variations: Set true on every variant row. Omitted, empty, or false: no variant options are used. |
| `openai.variants.group_id` | `group_id` | required_if | error | Variants: group_id, listing_has_variations and variant_dict go together |
| `openai.variants.variant_dict` | `variant_dict` | required_if | error | Variants: group_id, listing_has_variations and variant_dict go together |
| `openai.checkout.seller_privacy_policy` | `seller_privacy_policy` | required_if | error | Checkout: is_eligible_checkout needs seller_privacy_policy and seller_tos |
| `openai.checkout.seller_tos` | `seller_tos` | required_if | error | Checkout: is_eligible_checkout needs seller_privacy_policy and seller_tos |
| `openai.dimensions_unit.required` | `dimensions_unit` | required_if | error | length, width, height: each requires dimensions_unit (the dimensions object carries its own unit) |
| `openai.dimensions_unit.enum` | `dimensions_unit` | enum | error | dimensions_unit: in, cm, ft, m or mm |
| `openai.item_weight_unit.required` | `item_weight_unit` | required_if | error | weight: Requires item_weight_unit |
| `openai.item_weight_unit.enum` | `item_weight_unit` | enum | error | item_weight_unit: g, kg, oz or lb |
| `openai.condition.enum` | `condition` | enum | error | condition: new, refurbished or used |
| `openai.target_countries.form` | `target_countries` | pattern | error | target_countries: Uppercase ISO 3166-1 alpha-2 codes |
| `openai.store_country.form` | `store_country` | pattern | error | store_country: ISO 3166-1 alpha-2 code |

## ACP feed schema

- Pack id: `acp-feed`, file `rules/acp-feed/2026-04-17.toml`
- Source: https://github.com/agentic-commerce-protocol/agentic-commerce-protocol/tree/main/spec/2026-04-17/json-schema
- Version: 2026-04-17 (Beta); checked 2026-11-14
- Note: Ingestion is metadata.json plus products.jsonl, as a full replacement.
- Note: Format assertions (uri, date-time) are on.

| Rule | Field | Kind | Severity | Cites |
|---|---|---|---|---|
| `acp.schema.product` |  | json_schema | error | ACP feed schema 2026-04-17, $defs/Product |
| `acp.schema.metadata` |  | json_schema | error | ACP feed schema 2026-04-17, $defs/FeedMetadata |

## Google Merchant Center

- Pack id: `google-merchant`, file `rules/google-merchant/checked-2026-11-14.toml`
- Source: https://support.google.com/merchants/answer/7052112
- Version: product data specification, checked 2026-11-14; checked 2026-11-14
- Note: Images must be at least 500 x 500 pixels, enforced from 31 January 2027 (a warning until then). Checked only with --images, from the local file named by the last segment of image_link; otherwise reported as not checked.
- Note: question_and_answer is not read in this version. Google's example writes quoted pairs separated by commas ("Q":"A","Q2":"A2"); that form, and any XML groups nested inside the element, are left for a later version. Its limits (30 pairs, 1,000 characters per question and per answer, 10,000 in total) are not checked; a run lists the attribute under What This Run Did Not Check when a feed has it.
- Note: Apparel rules match the category path or its top-level numeric id (166, 1604 Clothing, 187 Shoes); a numeric id of a subcategory is not recognised.
- Note: mpn is required when there is no gtin. identifier_exists = no is not modelled in this version.

| Rule | Field | Kind | Severity | Cites |
|---|---|---|---|---|
| `google.required.id` | `item_id` | required | error | id: required for all products |
| `google.required.title` | `title` | required | error | title: required for all products |
| `google.required.description` | `description` | required | error | description: required for all products |
| `google.required.link` | `url` | required | error | link: required for all products |
| `google.required.image_link` | `image_url` | required | error | image_link: required for all products |
| `google.required.availability` | `availability` | required | error | availability: required for all products |
| `google.required.price` | `price` | required | error | price: required for all products |
| `google.id.length` | `item_id` | max_len | error | id: up to 50 characters |
| `google.title.length` | `title` | max_len | error | title: up to 150 characters |
| `google.description.length` | `description` | max_len | error | description: up to 5,000 characters |
| `google.link.absolute` | `url` | url_absolute | error | link: absolute URL starting with http or https |
| `google.image_link.absolute` | `image_url` | url_absolute | error | image_link: absolute URL starting with http or https |
| `google.availability.enum` | `availability` | enum | error | availability: in_stock, out_of_stock, preorder or backorder |
| `google.price.form` | `price` | price | error | price: amount and ISO 4217 currency, such as 15.00 USD |
| `google.brand.required_new` | `brand` | required_if | error | brand: required for all new products (except movies, books and music) |
| `google.mpn.without_gtin` | `mpn` | required_if | error | mpn: required if the product has no manufacturer-assigned GTIN |
| `google.condition.enum` | `condition` | enum | error | condition: new, refurbished or used |
| `google.availability_date.required` | `availability_date` | required_if | error | availability_date: Required if product availability is set to preorder |
| `google.availability_date.backorder` | `availability_date` | required_if | warning | availability: Provide the availability date attribute if you submit preorder or backorder |
| `google.apparel.color` | `color` | required_if | error | color: required for apparel products |
| `google.apparel.size` | `size` | required_if | error | size: Required for all apparel products in Apparel & Accessories > Clothing (ID:1604) and Apparel & Accessories > Shoes (ID:187) |
| `google.apparel.gender` | `gender` | required_if | error | gender: required for apparel products |
| `google.apparel.age_group` | `age_group` | required_if | error | age_group: required for apparel products |
| `google.related_product.form` | `related_product` | pattern | error | related_product: relationship_type:identifier_type:identifier |
| `google.document_link.count` | `document_link` | list_max | error | document_link: up to 5 PDF links |
| `google.document_link.absolute` | `document_link` | url_absolute | error | document_link: up to 5 PDF links |
| `google.document_link.pdf` | `document_link` | pattern | warning | document_link: up to 5 PDF links |
| `google.item_group_title.length` | `item_group_title` | max_len | error | item_group_title: up to 150 characters |
| `google.variant_option.count` | `variant_option` | list_max | error | variant_option: up to 30 values, such as Memory size:128 GB |
| `google.variant_option.form` | `variant_option` | pattern | error | variant_option: name:value, such as Memory size:128 GB |
| `google.image_link.min_width` | `image_width` | range | warning | image_link: at least 500 x 500 pixels, enforced from 31 January 2027 (checked only with --images) |
| `google.image_link.min_height` | `image_height` | range | warning | image_link: at least 500 x 500 pixels, enforced from 31 January 2027 (checked only with --images) |
| `google.popularity_rank.range` | `popularity_rank` | range | error | popularity_rank: from 0 to 100 |
| `google.gender.enum` | `gender` | enum | error | gender: male, female or unisex |
| `google.age_group.enum` | `age_group` | enum | error | age_group: newborn, infant, toddler, kids or adult |
| `google.related_product.count` | `related_product` | list_max | error | related_product: Max 30 values |
| `google.brand.length` | `brand` | max_len | error | brand: Max 70 characters |
| `google.mpn.length` | `mpn` | max_len | error | mpn: Max 70 characters |

Listed under "What This Run Did Not Check":

- Questions and answers: when a feed has `question_and_answer`, the run says how many items have it and that it was not read.
- Images: No image folder was given, so Google's 500 x 500 minimum, enforced from 31 January 2027, was not checked. Add --images.

## GS1 GTIN rules

- Pack id: `gs1-gtin`, file `rules/gs1-gtin/genspecs-26.0.toml`
- Source: https://www.gs1.org/standards/barcodes-epcrfid-id-keys/gs1-general-specifications
- Version: General Specifications 26.0; checked 2026-11-14
- Note: Spaces and dashes in gtin: an error for the OpenAI product feed ("no spaces or dashes") and for ACP, which keeps the raw value. For Google every rule here checks the value with them removed, because Google's help says "spaces and dashes are accepted, but ignored".

| Rule | Field | Kind | Severity | Cites |
|---|---|---|---|---|
| `gs1.gtin.form` | `gtin` | gtin | error | GTIN-8, 12, 13 or 14: digits only, leading zeros kept, no spaces or dashes |
| `gs1.gtin.check_digit` | `gtin` | gtin | error | GS1 General Specifications 26.0, check digit calculation for GTIN-8, 12, 13 and 14 |
| `gs1.gtin.restricted_prefix` | `gtin` | gtin_prefix | error | Prefixes 02, 04 and 20 to 29: restricted circulation numbers |
| `gs1.gtin.restricted_prefix_8` | `gtin` | gtin_prefix | error | GTIN-8 prefixes 0 and 2: restricted circulation numbers (RCN-8) |
| `gs1.gtin.refund_prefix` | `gtin` | gtin_prefix | error | Prefix 980: refund receipts |
| `gs1.gtin.coupon_prefix` | `gtin` | gtin_prefix | error | Prefixes 981 to 983 and 99: coupons |
| `gs1.gtin.demo_prefix` | `gtin` | gtin_prefix | error | Prefix 952: demonstration numbers |
| `gs1.gtin.unused_prefix` | `gtin` | gtin_prefix | error | Prefixes 0000001 to 0000099: unused |
| `gs1.gtin.prefix_05` | `gtin` | gtin_prefix | warning | Prefix 05: coupons in Google's help, company prefixes in GS1 General Specifications 26.0 |

## Cross-format consistency

- Pack id: `cross-format`, file `rules/cross-format/checked-2026-11-14.toml`
- Source: shelfcheck
- Version: 1; checked 2026-11-14

| Rule | Field | Kind | Severity | Cites |
|---|---|---|---|---|
| `cross.price` | `price` | same_across_formats | error | The same item must have the same price in every feed |
| `cross.availability` | `availability` | same_across_formats | error | The same item must have the same availability in every feed |
| `cross.brand` | `brand` | same_across_formats | error | The same item must have the same brand in every feed |
| `cross.gtin` | `gtin` | same_across_formats | error | The same item must have the same gtin in every feed |
