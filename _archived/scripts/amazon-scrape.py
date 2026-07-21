#!/usr/bin/env python3
"""
HONE Amazon store scraper — camoufox backend
Shin Devlin

Accepts a single Amazon URL as argv[1].
Outputs a JSON object to stdout: { products: [...] } or { error: "..." }
Errors/debug go to stderr only.
"""

import sys
import json
import re
import time

def die(error, message=None):
    obj = {"error": error}
    if message:
        obj["message"] = message
    print(json.dumps(obj))
    sys.stdout.flush()
    sys.exit(0)  # exit 0 so Node reads the JSON error output

if len(sys.argv) < 2:
    die("usage", "usage: amazon-scrape.py <url>")

url = sys.argv[1].strip()
if "amazon" not in url:
    die("invalid_url", "URL must be an amazon.com link")

try:
    from camoufox.sync_api import Camoufox
except ImportError:
    die("camoufox_not_installed", "camoufox Python package not found")

def _extract_price(text):
    m = re.search(r'[\$£€]?([\d,]+\.?\d*)', text or "")
    if m:
        try:
            return float(m.group(1).replace(",", ""))
        except Exception:
            pass
    return 0.0

def _clean(s):
    if not s:
        return ""
    return re.sub(r'\s+', ' ', str(s).strip())

def _is_captcha(page):
    try:
        content = page.content()
        return any(x in content for x in [
            "Type the characters you see",
            "Enter the characters you see",
            "api-services-support@amazon.com",
            "robot or automated service",
            "Sorry, we just need to make sure",
        ])
    except Exception:
        return False

def _parse_jsonld(page):
    products = []
    try:
        snippets = page.evaluate(
            "() => Array.from(document.querySelectorAll('script[type=\"application/ld+json\"]')).map(s => s.textContent)"
        )
        for raw in (snippets or []):
            try:
                data = json.loads(raw)
                items = data if isinstance(data, list) else [data]
                for item in items:
                    if item.get("@type") == "Product" and item.get("name"):
                        price = 0.0
                        offers = item.get("offers", {})
                        if isinstance(offers, list):
                            offers = offers[0]
                        price = float(offers.get("price", 0) or 0)
                        products.append({
                            "title": _clean(item.get("name", "")),
                            "price": price,
                            "description": _clean(item.get("description", ""))[:200],
                            "image": "",
                            "asin": item.get("sku") or item.get("productID") or "",
                            "category": str(item.get("category", "General")),
                            "condition": "New",
                            "qty": 1,
                        })
            except Exception:
                pass
    except Exception as e:
        print(f"[scraper] jsonld error: {e}", file=sys.stderr)
    return products

def _parse_search_results(page):
    products = []
    try:
        items = page.query_selector_all('[data-asin][data-index]')
        for el in items:
            asin = el.get_attribute("data-asin") or ""
            if not asin or len(asin) != 10:
                continue
            title_el = (
                el.query_selector('h2 a span') or
                el.query_selector('[data-cy="title-recipe"] span') or
                el.query_selector('.a-size-medium') or
                el.query_selector('.a-size-base-plus')
            )
            title = _clean(title_el.inner_text() if title_el else "") or f"Product {asin}"
            price_whole = el.query_selector('.a-price-whole')
            price_frac  = el.query_selector('.a-price-fraction')
            if price_whole:
                whole = _clean(price_whole.inner_text()).replace(",", "").rstrip(".")
                frac  = _clean(price_frac.inner_text()) if price_frac else "00"
                try:
                    price = float(f"{whole}.{frac}")
                except Exception:
                    price = 0.0
            else:
                price_el = el.query_selector('.a-offscreen')
                price = _extract_price(price_el.inner_text() if price_el else "")
            img_el = el.query_selector('img.s-image')
            image = img_el.get_attribute("src") if img_el else ""
            products.append({
                "title": title, "price": price, "description": "",
                "image": image, "asin": asin,
                "category": "General", "condition": "New", "qty": 1,
            })
    except Exception as e:
        print(f"[scraper] search parse error: {e}", file=sys.stderr)
    return products

def _parse_single_product(page):
    try:
        title_el = page.query_selector('#productTitle')
        title = _clean(title_el.inner_text() if title_el else "")
        if not title:
            return []
        price_el = (
            page.query_selector('.a-price .a-offscreen') or
            page.query_selector('#priceblock_ourprice') or
            page.query_selector('#priceblock_dealprice')
        )
        price = _extract_price(price_el.inner_text() if price_el else "")
        desc_el = page.query_selector('#productDescription p') or page.query_selector('#feature-bullets')
        desc = _clean(desc_el.inner_text() if desc_el else "")[:200]
        asin_m = re.search(r'/dp/([A-Z0-9]{10})', page.url)
        asin = asin_m.group(1) if asin_m else ""
        img_el = page.query_selector('#landingImage') or page.query_selector('#imgTagWrapperId img')
        image = img_el.get_attribute("src") if img_el else ""
        return [{"title": title, "price": price, "description": desc,
                 "image": image, "asin": asin,
                 "category": "General", "condition": "New", "qty": 1}]
    except Exception as e:
        print(f"[scraper] single product parse error: {e}", file=sys.stderr)
        return []

def scrape(url):
    with Camoufox(headless=True, geoip=True) as browser:
        page = browser.new_page()
        # Block images/media to speed up load
        page.route("**/*.{png,jpg,jpeg,gif,webp,svg,woff,woff2,ttf,mp4,mp3}",
                   lambda r: r.abort())
        print(f"[scraper] navigating to {url}", file=sys.stderr)
        page.goto(url, wait_until="domcontentloaded", timeout=25000)
        time.sleep(2)

        if _is_captcha(page):
            return {"error": "amazon_captcha",
                    "message": "Amazon returned a CAPTCHA. Use the CSV export fallback."}

        products = _parse_jsonld(page)
        if products:
            print(f"[scraper] jsonld: {len(products)} products", file=sys.stderr)
            return {"products": products, "count": len(products)}

        products = _parse_search_results(page)
        if products:
            print(f"[scraper] search grid: {len(products)} products", file=sys.stderr)
            return {"products": products, "count": len(products)}

        products = _parse_single_product(page)
        if products:
            print(f"[scraper] single product: {len(products)}", file=sys.stderr)
            return {"products": products, "count": len(products)}

        return {"error": "no_products",
                "message": "No products found. Try amazon.com/s?me=SELLERID or use the CSV export."}

try:
    result = scrape(url)
    print(json.dumps(result))
    sys.stdout.flush()
except Exception as e:
    die("scraper_error", str(e))
