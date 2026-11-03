"""Fetch openly licensed bird audio from Wikimedia Commons for speech-gate testing."""
import json, os, re, sys, urllib.parse, urllib.request

OUT = sys.argv[1]
WANT = int(sys.argv[2]) if len(sys.argv) > 2 else 40
UA = "HoneSpeechGateTest/0.1 (open-source bird detector test; https://github.com/shindevlin/hone)"
OK_LIC = re.compile(r"^(CC0|Public domain|PD|CC BY(-SA)? [0-9.]+)", re.I)
os.makedirs(OUT, exist_ok=True)


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()


got, manifest = 0, []
for query in ["bird song", "bird call", "birdsong recording", "owl call", "dove cooing", "songbird"]:
    if got >= WANT:
        break
    params = {
        "action": "query", "format": "json", "generator": "search", "gsrnamespace": "6",
        "gsrsearch": f"filetype:audio {query}", "gsrlimit": "50",
        "prop": "imageinfo", "iiprop": "url|size|extmetadata|mime",
    }
    data = json.loads(get("https://commons.wikimedia.org/w/api.php?" + urllib.parse.urlencode(params)))
    for page in (data.get("query", {}).get("pages", {}) or {}).values():
        if got >= WANT:
            break
        ii = (page.get("imageinfo") or [{}])[0]
        meta = ii.get("extmetadata", {})
        lic = meta.get("LicenseShortName", {}).get("value", "")
        title = page.get("title", "")
        if not OK_LIC.match(lic) or ii.get("size", 0) > 6_000_000 or ii.get("mime") not in ("application/ogg", "audio/ogg", "audio/x-flac", "audio/flac", "audio/wav", "audio/x-wav"):
            continue
        if re.search(r"(?i)speech|spoken|pronunc|voice|human|music|song of|choir|interview", title):
            continue
        name = re.sub(r"[^A-Za-z0-9._-]", "_", title.split(":", 1)[1])[:120]
        path = os.path.join(OUT, name)
        if any(m["file"] == name for m in manifest):
            continue
        try:
            open(path, "wb").write(get(ii["url"]))
        except Exception as e:
            print("skip", title, e)
            continue
        artist = re.sub("<[^>]+>", "", meta.get("Artist", {}).get("value", ""))[:120]
        manifest.append({"file": name, "title": title, "license": lic, "artist": artist, "url": ii.get("descriptionurl")})
        got += 1
json.dump(manifest, open(os.path.join(OUT, "MANIFEST.json"), "w"), indent=1)
print("downloaded", got)
