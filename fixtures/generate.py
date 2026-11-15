"""Generate shelfcheck's synthetic fixtures and gold.json.

Run from the repository root:  python fixtures/generate.py

Everything is deterministic (fixed seed). Brands, barcodes and stores are
made up. Barcodes have valid check digits unless a problem is seeded on
purpose. gold.json lists every problem and change the seeds should produce,
with its location, computed here from the seeds, independently of the Rust
code. The gold tests (crates/shelfcheck-cli/tests/gold.rs) require shelfcheck
to produce exactly these: no missed problems, no extra ones.
"""

import csv
import io
import json
import os
import random

ROOT = os.path.dirname(os.path.abspath(__file__))
SEED = 20261114
STORE = "https://store-a.example"

CSV_COLUMNS = [
    "item_id", "title", "description", "url", "brand", "seller_name", "image_url",
    "additional_image_urls", "availability", "availability_date", "price", "sale_price",
    "target_countries", "store_country", "gtin", "mpn", "group_id", "listing_has_variations",
    "variant_dict", "product_category", "color", "size", "gender", "age_group", "offer_id",
]
COL = {name: i + 1 for i, name in enumerate(CSV_COLUMNS)}
ALL = ["openai", "acp", "google"]

APPAREL = "Apparel & Accessories > Clothing > Shirts & Tops"
CATEGORIES = [
    # (kind, product_category, brand, option name, option values)
    ("Phone", "Electronics > Communications > Telephony > Mobile Phones", "Nova", "storage", ["128 GB", "256 GB", "512 GB"]),
    ("Tablet", "Electronics > Computers > Tablet Computers", "Slate", "storage", ["64 GB", "128 GB", "256 GB"]),
    ("Mug", "Home & Garden > Kitchen & Dining > Tableware > Drinkware > Mugs", "Harbor & Co", "color", ["White", "Black", "Blue", "Green"]),
    ("Tee", APPAREL, "Linden", "size", ["S", "M", "L", "XL", "XXL"]),
    ("Lamp", "Home & Garden > Lighting > Lamps", "Brightwell", "color", ["White", "Black", "Brass"]),
    ("Frame", "Home & Garden > Decor > Picture Frames", "Fernhill", "size", ["20 cm", "30 cm", "40 cm", "50 cm"]),
    ("Kettle", "Home & Garden > Kitchen & Dining > Kitchen Appliances > Kettles", "Corvid", "color", ["Steel", "Black", "Cream"]),
    ("Bag", "Apparel & Accessories > Handbags, Wallets & Cases > Handbags", "Marlow", "color", ["Tan", "Black", "Navy"]),
    ("Towel", "Home & Garden > Linens & Bedding > Towels", "Quillon", "color", ["White", "Grey", "Navy", "Sand", "Sage"]),
    ("Sock", "Apparel & Accessories > Clothing > Underwear & Socks > Socks", "Tessel", "pack quantity", ["3-pack", "6-pack"]),
]


def check_digit(body):
    total = 0
    for i, ch in enumerate(reversed(body)):
        total += int(ch) * (3 if i % 2 == 0 else 1)
    return str((10 - total % 10) % 10)


def gtin13(prefix, n):
    body = prefix + str(n).rjust(12 - len(prefix), "0")
    return body + check_digit(body)


def money(cents):
    return f"{cents // 100}.{cents % 100:02d} USD"


# ---------------------------------------------------------------- store-a

def build_store_a():
    rng = random.Random(SEED)
    products = 1400
    target_variants = 3912
    sizes = [rng.randint(1, 5) for _ in range(products)]
    # Adjust deterministically so the total is exactly 3,912.
    i = 0
    while sum(sizes) != target_variants:
        if sum(sizes) < target_variants and sizes[i] < 5:
            sizes[i] += 1
        elif sum(sizes) > target_variants and sizes[i] > 1:
            sizes[i] -= 1
        i = (i + 1) % products
    rows = []
    gtin_n = 1
    for p in range(products):
        kind, category, brand, opt_name, opt_values = CATEGORIES[p % len(CATEGORIES)]
        group = f"P{p + 1:04d}"
        n = sizes[p]
        model = f"{kind[:3].upper()}{p + 1:04d}"
        base_cents = rng.randint(9, 400) * 100 + rng.choice([49, 95, 99])
        for v in range(n):
            value = opt_values[v % len(opt_values)]
            if v >= len(opt_values):
                value = f"{value} {v // len(opt_values) + 1}"
            item_id = f"{model}-{v + 1}"
            gtin = gtin13("50", gtin_n)
            gtin_n += 1
            title = f"{brand} {kind} {model}, {value}"
            desc = f"{brand} {kind.lower()} {model} in {value}. Synthetic fixture product."
            if (p * 7 + v) % 97 == 0:
                desc = f"{brand} {kind.lower()} {model}.\nSecond line of the description."
            cents = base_cents + v * 100
            availability = "in_stock"
            availability_date = ""
            r = (p * 13 + v * 5) % 50
            if r == 0:
                availability = "out_of_stock"
            elif r == 1:
                availability = "backorder"
                availability_date = "2026-12-20"
            elif r == 2:
                availability = "pre_order"
                availability_date = "2026-12-15"
            row = {
                "item_id": item_id,
                "title": title,
                "description": desc,
                "url": f"{STORE}/products/{group.lower()}?variant={v + 1}",
                "brand": brand,
                "seller_name": "Store A",
                "image_url": f"{STORE}/img/{item_id.lower()}.jpg",
                "additional_image_urls": f"{STORE}/img/{item_id.lower()}-2.jpg,{STORE}/img/{item_id.lower()}-3.jpg" if v == 0 else "",
                "availability": availability,
                "availability_date": availability_date,
                "price": money(cents),
                "sale_price": money(cents - 500) if (p + v) % 11 == 0 else "",
                "target_countries": "US",
                "store_country": "US",
                "gtin": gtin,
                "mpn": f"{model}-{v + 1:02d}",
                "group_id": group,
                "listing_has_variations": "true" if n > 1 else "false",
                "variant_dict": json.dumps({opt_name: value}) if n > 1 else "",
                "product_category": category,
                "color": "",
                "size": "",
                "gender": "",
                "age_group": "",
                "offer_id": "",
                "_cents": cents,
                "_kind": kind,
                "_seeds": [],
            }
            if category.startswith("Apparel & Accessories"):
                # Google requires these four for apparel.
                row["color"] = value if opt_name == "color" else "Black"
                row["size"] = value if opt_name == "size" else ("One size" if kind == "Bag" else "M")
                row["gender"] = "unisex"
                row["age_group"] = "adult"
            rows.append(row)
    assert len(rows) == target_variants, len(rows)

    # Seed problems on distinct rows, each row at most one seed.
    free = list(range(len(rows)))
    rng.shuffle(free)

    def take(k):
        out = sorted(free[:k])
        del free[:k]
        return out

    seeds = {
        "check_digit": take(84),
        "target_countries": take(61),
        "preorder": take(23),
        "sale_price": take(15),
        "long_title": take(12),
        "restricted": take(9),
        "no_brand": take(7),
        "cross_price": take(11),
        "acp_decimal": take(17),
    }
    for name, idxs in seeds.items():
        for i in idxs:
            rows[i]["_seeds"].append(name)
    for i in seeds["check_digit"]:
        g = rows[i]["gtin"]
        rows[i]["_expected"] = g[-1]
        rows[i]["gtin"] = g[:-1] + str((int(g[-1]) + 1) % 10)
    for i in seeds["target_countries"]:
        rows[i]["target_countries"] = ""
    for i in seeds["preorder"]:
        rows[i]["availability"] = "preorder"
        rows[i]["availability_date"] = "2026-12-15"
    for i in seeds["sale_price"]:
        rows[i]["sale_price"] = rows[i]["price"]
    for i in seeds["long_title"]:
        t = rows[i]["title"] + ", with a very long title that keeps going past the limit both formats set"
        while len(t) <= 150:
            t += " and more"
        rows[i]["title"] = t
    for k, i in enumerate(seeds["restricted"]):
        body = f"2{k + 1}" + str(900000 + i).rjust(10, "0")
        rows[i]["gtin"] = body + check_digit(body)
    for i in seeds["no_brand"]:
        rows[i]["brand"] = ""
    return rows, seeds


def write_csv(path, columns, rows):
    """Write rows and return the 1-based line each row starts on."""
    buf = io.StringIO()
    w = csv.writer(buf, lineterminator="\n")
    w.writerow(columns)
    line = 2
    starts = []
    for r in rows:
        starts.append(line)
        one = io.StringIO()
        csv.writer(one, lineterminator="\n").writerow([r.get(c, "") for c in columns])
        text = one.getvalue()
        buf.write(text)
        line += text.count("\n")
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(buf.getvalue())
    return starts


def xml_escape(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


GOOGLE_AVAIL = {"in_stock": "in_stock", "out_of_stock": "out_of_stock", "pre_order": "preorder",
                "preorder": "preorder", "backorder": "backorder"}


def write_google_xml(path, rows):
    """Return per-row dict: item line and element lines."""
    out = []
    lines = {}
    cur = [1]

    def emit(text):
        out.append(text)
        cur[0] += text.count("\n")

    emit('<?xml version="1.0" encoding="UTF-8"?>\n')
    emit('<rss version="2.0" xmlns:g="http://base.google.com/ns/1.0">\n')
    emit("<channel>\n")
    emit("<title>Store A</title>\n")
    emit(f"<link>{STORE}</link>\n")
    emit("<description>Synthetic fixture feed</description>\n")
    for i, r in enumerate(rows):
        info = {"item": cur[0]}
        emit("<item>\n")

        def el(name, value, key=None, cdata=False):
            info[key or name] = cur[0]
            body = f"<![CDATA[{value}]]>" if cdata else xml_escape(value)
            emit(f"<{name}>{body}</{name}>\n")

        el("g:id", r["item_id"])
        el("title", r["title"])
        el("description", r["description"], cdata=("\n" in r["description"]))
        el("link", r["url"])
        el("g:image_link", r["image_url"])
        for extra in [u for u in r["additional_image_urls"].split(",") if u]:
            el("g:additional_image_link", extra)
        el("g:availability", GOOGLE_AVAIL[r["availability"]])
        if r["availability_date"]:
            el("g:availability_date", r["availability_date"])
        price = r["price"]
        if "cross_price" in r["_seeds"]:
            price = money(r["_cents"] + 500)
        el("g:price", price)
        if r["brand"]:
            el("g:brand", r["brand"])
        el("g:gtin", r["gtin"])
        el("g:mpn", r["mpn"])
        el("g:item_group_id", r["group_id"])
        el("g:google_product_category", r["product_category"])
        for a in ["color", "size", "gender", "age_group"]:
            if r[a]:
                el(f"g:{a}", r[a])
        emit("<g:shipping>\n<g:country>US</g:country>\n<g:price>0.00 USD</g:price>\n</g:shipping>\n")
        emit("</item>\n")
        lines[i] = info
    emit("</channel>\n</rss>\n")
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write("".join(out))
    return lines


ACP_STATUS = {"in_stock": "in_stock", "out_of_stock": "out_of_stock", "pre_order": "preorder",
              "preorder": "preorder", "backorder": "backorder"}


def write_acp(dirpath, rows):
    os.makedirs(dirpath, exist_ok=True)
    with open(os.path.join(dirpath, "metadata.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump({"id": "feed_store_a", "target_country": "US", "updated_at": "2026-11-14T00:00:00Z"}, f, indent=2)
        f.write("\n")
    groups = []
    by_group = {}
    for i, r in enumerate(rows):
        g = r["group_id"]
        if g not in by_group:
            by_group[g] = []
            groups.append(g)
        by_group[g].append(i)
    where = {}
    with open(os.path.join(dirpath, "products.jsonl"), "w", encoding="utf-8", newline="\n") as f:
        for line_no, g in enumerate(groups, start=1):
            idxs = by_group[g]
            first = rows[idxs[0]]
            variants = []
            for vi, i in enumerate(idxs):
                r = rows[i]
                amount = r["_cents"]
                if "acp_decimal" in r["_seeds"]:
                    amount = float(f"{r['_cents'] // 100}.{r['_cents'] % 100:02d}")
                v = {
                    "id": r["item_id"],
                    "title": r["title"],
                    "description": {"plain": r["description"]},
                    "url": r["url"],
                    "barcodes": [{"type": "GTIN", "value": r["gtin"]}],
                    "price": {"amount": amount, "currency": "USD"},
                    "availability": {"available": r["availability"] == "in_stock", "status": ACP_STATUS[r["availability"]]},
                    "media": [{"type": "image", "url": r["image_url"]}],
                    "seller": {"name": r["seller_name"]},
                }
                if r["variant_dict"]:
                    for k, val in json.loads(r["variant_dict"]).items():
                        v["variant_options"] = [{"name": k, "value": val}]
                variants.append(v)
                where[i] = (line_no, vi)
            product = {
                "id": g,
                "title": first["title"].split(",")[0],
                "description": {"plain": f"{first['brand'] or 'Unbranded'} product {g}."},
                "url": f"{STORE}/products/{g.lower()}",
                "variants": variants,
            }
            f.write(json.dumps(product, ensure_ascii=False) + "\n")
    return where


def check_gold(rows, seeds, csv_lines, xml_lines, acp_where):
    csv_file = "store-a/products.csv"
    xml_file = "store-a/products.xml"
    acp_file = "store-a/acp/products.jsonl"

    def p(rule, file, line, item, formats, column=None, pointer=None, expected=None):
        d = {"rule": rule, "file": file, "line": line, "column": column, "pointer": pointer,
             "item_id": item, "formats": formats}
        if expected is not None:
            d["expected"] = expected
        return d

    alone = []
    for i, r in enumerate(rows):
        line = csv_lines[i]
        s = r["_seeds"]
        item = r["item_id"]
        if "check_digit" in s:
            alone.append(p("gs1.gtin.check_digit", csv_file, line, item, ALL, COL["gtin"], expected=r["_expected"]))
        if "restricted" in s:
            alone.append(p("gs1.gtin.restricted_prefix", csv_file, line, item, ALL, COL["gtin"]))
        if "target_countries" in s:
            alone.append(p("openai.required.target_countries", csv_file, line, item, ["openai"], COL["target_countries"]))
        if "preorder" in s:
            alone.append(p("openai.availability.enum", csv_file, line, item, ["openai"], COL["availability"]))
        if "sale_price" in s:
            alone.append(p("openai.sale_price.below_price", csv_file, line, item, ["openai"], COL["sale_price"]))
        if "long_title" in s:
            alone.append(p("openai.title.length", csv_file, line, item, ["openai"], COL["title"]))
            alone.append(p("google.title.length", csv_file, line, item, ["google"], COL["title"]))
        if "no_brand" in s:
            alone.append(p("openai.required.brand", csv_file, line, item, ["openai"], COL["brand"]))
            alone.append(p("google.brand.required_new", csv_file, line, item, ["google"], COL["brand"]))

    together = []
    for i, r in enumerate(rows):
        s = r["_seeds"]
        item = r["item_id"]
        line = csv_lines[i]
        x = xml_lines[i]
        acp_line, vi = acp_where[i]
        bc = f"/variants/{vi}/barcodes/0/value"
        if "check_digit" in s:
            together.append(p("gs1.gtin.check_digit", csv_file, line, item, ["openai"], COL["gtin"], expected=r["_expected"]))
            together.append(p("gs1.gtin.check_digit", xml_file, x["g:gtin"], item, ["google"], expected=r["_expected"]))
            together.append(p("gs1.gtin.check_digit", acp_file, acp_line, item, ["acp"], pointer=bc, expected=r["_expected"]))
        if "restricted" in s:
            together.append(p("gs1.gtin.restricted_prefix", csv_file, line, item, ["openai"], COL["gtin"]))
            together.append(p("gs1.gtin.restricted_prefix", xml_file, x["g:gtin"], item, ["google"]))
            together.append(p("gs1.gtin.restricted_prefix", acp_file, acp_line, item, ["acp"], pointer=bc))
        if "target_countries" in s:
            together.append(p("openai.required.target_countries", csv_file, line, item, ["openai"], COL["target_countries"]))
        if "preorder" in s:
            together.append(p("openai.availability.enum", csv_file, line, item, ["openai"], COL["availability"]))
        if "sale_price" in s:
            together.append(p("openai.sale_price.below_price", csv_file, line, item, ["openai"], COL["sale_price"]))
        if "long_title" in s:
            together.append(p("openai.title.length", csv_file, line, item, ["openai"], COL["title"]))
            together.append(p("google.title.length", xml_file, x["title"], item, ["google"]))
        if "no_brand" in s:
            together.append(p("openai.required.brand", csv_file, line, item, ["openai"], COL["brand"]))
            together.append(p("google.brand.required_new", xml_file, x["item"], item, ["google"]))
        if "cross_price" in s:
            together.append(p("cross.price", xml_file, x["g:price"], item, ["openai", "google"]))
        if "acp_decimal" in s:
            together.append(p("acp.schema.product", acp_file, acp_line, item, ["acp"], pointer=f"/variants/{vi}/price/amount"))

    groups = len({r["group_id"] for r in rows})
    return [
        {
            "name": "openai-csv-alone",
            "inputs": [csv_file],
            "formats": ALL,
            "summary": {"products": groups, "items": len(rows)},
            "problems": alone,
        },
        {
            "name": "store-a-three-feeds",
            "inputs": [csv_file, xml_file, "store-a/acp"],
            "formats": ALL,
            "summary": {"products": groups, "items": len(rows) * 3},
            "problems": together,
        },
    ]


# ---------------------------------------------------------------- diff snapshots

DIFF_COLUMNS = ["item_id", "title", "description", "url", "brand", "seller_name", "image_url", "availability",
                "price", "target_countries", "store_country", "gtin", "mpn", "group_id",
                "listing_has_variations", "variant_dict", "product_category", "offer_id"]


def drow(item_id, title, gtin, brand, mpn, group, options=None, category="Home & Garden",
         price="19.99 USD", availability="in_stock", offer=""):
    return {
        "item_id": item_id, "title": title, "description": f"{title}. Synthetic fixture product.",
        "url": f"{STORE}/products/{group.lower()}", "brand": brand, "seller_name": "Store A",
        "image_url": f"{STORE}/img/{item_id.lower()}.jpg", "availability": availability, "price": price,
        "target_countries": "US", "store_country": "US", "gtin": gtin, "mpn": mpn, "group_id": group,
        "listing_has_variations": "true" if options else "false",
        "variant_dict": json.dumps(options) if options else "", "product_category": category,
        "offer_id": offer,
    }


def diff_fixtures():
    g = lambda n: gtin13("761", n)  # noqa: E731
    phone = "0194253012344"
    case = "0194253098768"
    tab = "5012345678900"
    mug = "0012345678905"
    common_before = [
        drow("TAB-S-256", "Slate Tab S, 256 GB, Grey", g(2), "Slate", "TS-256-GRY", "TAB-S",
             {"storage": "256 GB", "color": "Grey"}, "Tablets", "449.99 USD"),
        drow("TEE-BLK-L", "Linden Tee, Black, L", g(4), "Linden", "LT-BLK-L", "TEE-BLK",
             {"color": "Black", "size": "L"}, "Shirts", "24.99 USD"),
        drow("TAB-S-512", "Slate Tab S, 512 GB, Grey", g(5), "Slate", "TS-512-GRY", "TAB-S",
             {"storage": "512 GB", "color": "Grey"}, "Tablets", "549.99 USD"),
    ]
    fillers = [
        drow(f"FILL-{k:02d}", f"Fernhill Coaster {k}", g(100 + k), "Fernhill", f"FH-C-{k:02d}", f"FILL-{k:02d}",
             None, "Coasters", f"{5 + k}.49 USD")
        for k in range(1, 16)
    ]
    oct_rows = [
        drow("PHN-X2-BLK", "Nova X2 Phone, 128 GB, Black", phone, "Nova", "NX2-128-BLK", "PHN-X2",
             {"storage": "128 GB", "color": "Black"}, "Phones", "699.99 USD"),
        drow("TAB-S-128", "Slate Tab S, 128 GB, Grey", tab, "Slate", "TS-128-GRY", "TAB-S",
             {"storage": "128 GB", "color": "Grey"}, "Tablets", "399.99 USD"),
        common_before[0],
        drow("MUG-12-WHT", "Harbor Mug 12 oz, White", mug, "Harbor & Co", "HC-MUG-12", "MUG-12",
             None, "Mugs", "14.99 USD"),
        drow("TEE-BLK-M", "Linden Tee, Black, M", g(3), "Linden", "LT-BLK-M", "TEE-BLK",
             {"color": "Black", "size": "M"}, "Shirts", "24.99 USD"),
        common_before[1],
        drow("FRAME-30", "Fernhill Frame", g(6), "Fernhill", "FH-FR-30", "FRAME",
             {"size": "30 cm"}, "Frames", "29.99 USD"),
        drow("LAMP-01", "Brightwell Desk Lamp", g(7), "Brightwell", "BW-LMP-01", "LAMP",
             None, "Lamps", "59.99 USD"),
        drow("CUP-S", "Fernhill Cup", g(8), "Fernhill", "FH-CUP-S", "CUP-S", {"size": "S"}, "Cups", "9.99 USD"),
        drow("CUP-L", "Fernhill Cup", g(9), "Fernhill", "FH-CUP-L", "CUP-L", {"size": "L"}, "Cups", "12.99 USD"),
        drow("KETTLE-1", "Corvid Kettle, Steel", g(10), "Corvid", "CV-KT-1", "KETTLE", None, "Kettles", "39.99 USD"),
        drow("SOCK-3PK", "Tessel Socks, 3-pack", g(11), "Tessel", "TS-SK-3", "SOCK", None, "Socks", "12.49 USD"),
        drow("TOWEL-1", "Quillon Towel, Sand", g(12), "Quillon", "QL-TW-1", "TOWEL", None, "Towels", "18.99 USD"),
        drow("BAG-1", "Marlow Tote, Tan", g(13), "Marlow", "MR-BG-1", "BAG", None, "Bags", "49.99 USD",
             offer="BAG-1-4999"),
        drow("OLD-1", "Corvid Toaster", g(14), "Corvid", "CV-TS-1", "OLD", None, "Toasters", "44.99 USD"),
        common_before[2],
    ] + fillers
    nov_rows = [
        drow("PHN-X2-BLK", "Nova X2 Phone Case, Black", case, "Nova", "NX2-CASE-BLK", "PHN-X2",
             {"color": "Black"}, "Phone cases", "29.99 USD"),
        drow("TAB-S-128", "Slate Tab S, 256 GB, Grey", tab, "Slate", "TS-128-GRY", "TAB-S",
             {"storage": "256 GB", "color": "Grey"}, "Tablets", "399.99 USD"),
        common_before[0],
        drow("MUG-12OZ-WHT", "Harbor Mug 12 oz, White", mug, "Harbor & Co", "HC-MUG-12", "MUG-12",
             None, "Mugs", "14.99 USD"),
        drow("TEE-BLK-M", "Linden Tee, Black, M", g(3), "Linden", "LT-BLK-M", "TEE-BLK",
             {"color": "BLK", "size": "M"}, "Shirts", "24.99 USD"),
        common_before[1],
        drow("FRAME-30", "Fernhill Frame", g(6), "Fernhill", "FH-FR-30", "FRAME",
             {"size": "300 mm"}, "Frames", "29.99 USD"),
        drow("LAMP-01-WHT", "Brightwell Desk Lamp, White", g(7), "Brightwell", "BW-LMP-01-WHT", "LAMP",
             {"color": "White"}, "Lamps", "59.99 USD"),
        drow("LAMP-01-BLK", "Brightwell Desk Lamp, Black", g(15), "Brightwell", "BW-LMP-01-BLK", "LAMP",
             {"color": "Black"}, "Lamps", "59.99 USD"),
        drow("CUP-S", "Fernhill Cup", g(8), "Fernhill", "FH-CUP-S", "CUP", {"size": "S"}, "Cups", "9.99 USD"),
        drow("CUP-L", "Fernhill Cup", g(9), "Fernhill", "FH-CUP-L", "CUP", {"size": "L"}, "Cups", "12.99 USD"),
        drow("KETTLE-1", "Corvid Kettle, Steel", g(10), "Corvid", "CV-KT-1", "KETTLE", None, "Kettles", "34.99 USD"),
        drow("SOCK-3PK", "Tessel Socks, 3-pack", g(11), "Tessel", "TS-SK-3", "SOCK", None, "Socks", "12.49 USD"),
        drow("SOCK-3PK", "Tessel Socks, 3-pack", g(11), "Tessel", "TS-SK-3", "SOCK", None, "Socks", "12.49 USD"),
        drow("TOWEL-1", "Quillon Towel, Sand", g(12), "Quillon", "QL-TW-1", "TOWEL", None, "Towels", "18.99 USD",
             availability="out_of_stock"),
        drow("BAG-1", "Marlow Tote, Tan", g(13), "Marlow", "MR-BG-1", "BAG", None, "Bags", "49.99 USD",
             offer="BAG-1-4999"),
        drow("NEW-1", "Corvid Milk Frother", g(16), "Corvid", "CV-MF-1", "NEW", None, "Frothers", "34.99 USD"),
        common_before[2],
    ] + fillers
    os.makedirs(os.path.join(ROOT, "store-a-oct"), exist_ok=True)
    os.makedirs(os.path.join(ROOT, "store-a-nov"), exist_ok=True)
    write_csv(os.path.join(ROOT, "store-a-oct", "products.csv"), DIFF_COLUMNS, oct_rows)
    nov_lines = write_csv(os.path.join(ROOT, "store-a-nov", "products.csv"), DIFF_COLUMNS, nov_rows)

    export_cols = ["Handle", "Title", "Body (HTML)", "Vendor", "Type", "Option1 Name", "Option1 Value",
                   "Option2 Name", "Option2 Value", "Variant SKU", "Variant Price", "Variant Barcode", "Image Src"]
    export = [
        {"Handle": "nova-x2-case", "Title": "Nova X2 Phone Case, Black", "Body (HTML)": "<p>Case.</p>", "Vendor": "Nova",
         "Type": "Phone cases", "Option1 Name": "Color", "Option1 Value": "Black", "Variant SKU": "PHN-X2-BLK",
         "Variant Price": "29.99", "Variant Barcode": case, "Image Src": f"{STORE}/img/phn-x2-blk.jpg"},
        {"Handle": "slate-tab-s", "Title": "Slate Tab S, 128 GB, Grey", "Body (HTML)": "<p>Tablet.</p>", "Vendor": "Slate",
         "Type": "Tablets", "Option1 Name": "Storage", "Option1 Value": "256 GB", "Option2 Name": "Color",
         "Option2 Value": "Grey", "Variant SKU": "TAB-S-128", "Variant Price": "399.99", "Variant Barcode": tab,
         "Image Src": f"{STORE}/img/tab-s-128.jpg"},
        {"Handle": "slate-tab-s", "Option1 Value": "256 GB", "Option2 Value": "Grey", "Variant SKU": "TAB-S-256",
         "Variant Price": "449.99", "Variant Barcode": g(2)},
        {"Handle": "slate-tab-s", "Image Src": f"{STORE}/img/tab-s-2.jpg"},
        {"Handle": "harbor-mug-12", "Title": "Harbor Mug 12 oz, White", "Body (HTML)": "<p>Mug.</p>",
         "Vendor": "Harbor & Co", "Type": "Mugs", "Option1 Name": "Title", "Option1 Value": "Default Title",
         "Variant SKU": "MUG-12OZ-WHT", "Variant Price": "14.99", "Variant Barcode": mug},
    ]
    write_csv(os.path.join(ROOT, "store-a-export.csv"), export_cols, export)

    def ch(cid, kind, item, fp, status="open"):
        return {"id": cid, "kind": kind, "item_id": item, "fingerprint": fp, "status": status}

    sock_lines = [nov_lines[i] for i, r in enumerate(nov_rows) if r["item_id"] == "SOCK-3PK"]
    changes = [
        ch("D1", "id_reused", "PHN-X2-BLK", f"gtin:{phone}->{case}"),
        ch("D2", "critical_attribute_changed", "TAB-S-128", "capacity:128 GB->256 GB"),
        ch("D3", "id_changed", "LAMP-01", "id:LAMP-01->LAMP-01-WHT"),
        ch("D4", "id_changed", "MUG-12-WHT", "id:MUG-12-WHT->MUG-12OZ-WHT"),
        ch("W1", "store_stale", "TAB-S-128", "title:capacity:128 GB"),
        ch("W2", "group_changed", "CUP-L", "group:CUP-L->CUP"),
        ch("W3", "group_changed", "CUP-S", "group:CUP-S->CUP"),
        ch("W4", "offer_id_has_price", "BAG-1", "offer:BAG-1-4999"),
        ch("W5", "duplicate_id", "SOCK-3PK", f"rows:{len(sock_lines)}"),
        ch("I1", "new", "LAMP-01-BLK", "new"),
        ch("I2", "new", "NEW-1", "new"),
        ch("I3", "retired", "OLD-1", "retired"),
        ch("I4", "price_only", "KETTLE-1", "price:39.99 USD->34.99 USD"),
        ch("I5", "stock_only", "TOWEL-1", "availability:in_stock->out_of_stock"),
    ]
    decided = []
    decisions = []
    for c in changes:
        d = dict(c)
        if c["id"].startswith("D"):
            d["status"] = "decided"
            verdict = {"id_reused": "different_product", "critical_attribute_changed": "data_error_fixed",
                       "id_changed": "same_product"}[c["kind"]]
            decisions.append((c, verdict))
        decided.append(d)
    with open(os.path.join(ROOT, "store-a-decisions.toml"), "w", encoding="utf-8", newline="\n") as f:
        f.write("# Decisions for the store-a-oct to store-a-nov fixture changes.\n\n")
        for c, verdict in decisions:
            f.write("[[decision]]\n")
            f.write(f'id = "{c["id"]}"\nchange = "{c["kind"]}"\nitem_id = "{c["item_id"]}"\n')
            f.write(f'fingerprint = "{c["fingerprint"]}"\nverdict = "{verdict}"\n')
            f.write('by = "dev@example.com"\nat = "2026-11-14"\n\n')

    old_n = len(oct_rows)
    new_n = len({r["item_id"] for r in nov_rows})
    # Every old item is matched by gtin except the reused phone id and the retired toaster.
    pairs = old_n - 2
    stats = {"old_items": old_n, "new_items": new_n, "in_both": pairs, "by_gtin": pairs, "by_brand_mpn": 0, "by_id": 0}
    base = {"old": "store-a-oct/products.csv", "new": "store-a-nov/products.csv", "store": "store-a-export.csv",
            "stats": stats}
    return [
        dict(base, name="oct-to-nov", decisions=None, changes=changes, needs_decision=4),
        dict(base, name="oct-to-nov-decided", decisions="store-a-decisions.toml", changes=decided, needs_decision=0),
    ]


def main():
    rows, seeds = build_store_a()
    os.makedirs(os.path.join(ROOT, "store-a"), exist_ok=True)
    public = [{k: v for k, v in r.items() if not k.startswith("_")} for r in rows]
    csv_lines = write_csv(os.path.join(ROOT, "store-a", "products.csv"), CSV_COLUMNS, public)
    xml_lines = write_google_xml(os.path.join(ROOT, "store-a", "products.xml"), rows)
    acp_where = write_acp(os.path.join(ROOT, "store-a", "acp"), rows)
    gold = {
        "generated_by": "fixtures/generate.py",
        "seed": SEED,
        "seeded": {k: len(v) for k, v in seeds.items()},
        "check": check_gold(rows, seeds, csv_lines, xml_lines, acp_where),
        "diff": diff_fixtures(),
    }
    with open(os.path.join(ROOT, "gold.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(gold, f, indent=1, ensure_ascii=False)
        f.write("\n")
    print("check cases:", [(c["name"], len(c["problems"])) for c in gold["check"]])
    print("diff cases:", [(d["name"], len(d["changes"])) for d in gold["diff"]])


if __name__ == "__main__":
    main()
