#!/usr/bin/env python3
"""Standing hardware grant for Find My (no hide key on phones).

Runs on the host that holds the owner's hide key (beastly). Input: the phone's
hone-hwkey.json (app: Find my things > Export hardware key; adb pull from
/sdcard/Android/data/<pkg>/files/). It

  1. verifies the Android Key Attestation chain (Google root, not revoked, TEE/StrongBox,
     PURPOSE_WRAP_KEY, our challenge, this app's package + signing-cert digest),
  2. builds FIND_GRANT and signs it with the owner's CURRENT hide key,
  3. per device derives id_seed_g, takes PRK = HKDF-Extract(salt="", id_seed_g) and wraps it
     to the phone's attested key (Android ImportWrappedKey / SecureKeyWrapper) as a
     non-exportable HMAC-SHA256 key,
  4. writes hone-grant.json for the phone (adb push to the same folder, then Import grant).

Private keys stay in memory: never printed, logged or written. The bundle holds only
RSA-OAEP/AES-GCM ciphertext that only the phone's hardware can open.

  grant   --hwkey hone-hwkey.json (--dry-run --test-owner NAME | --wallet W --account A)
          [--device PUB[:label] ...] [--generation PUB=G ...] [--package P] [--app-cert SHA256HEX ...]
          [--out hone-grant.json] [--grant-out find_grant_entry.json] [--allow-unlocked] [--selftest]
  revoke  --hwkey-id HEX (--dry-run --test-owner NAME | --wallet W --account A) [--grant-out F]

--dry-run uses a deterministic TEST hide key derived from --test-owner (never a real key),
adds self-test vectors (reference lids + one sealed test sighting) and submits nothing.
Signed entries go to --grant-out; nothing is submitted by this tool.
"""
import argparse, base64, hashlib, json, os, sys, time, urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hone_locshare as hl  # key schedule + ECIES, mirrors IdKeys.kt

from cryptography import x509
from cryptography.hazmat.primitives import hashes, hmac, serialization
from cryptography.hazmat.primitives.asymmetric import padding
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.ciphers.aead import AESGCM

HERE = os.path.dirname(os.path.abspath(__file__))
ROOTS_FILE = os.path.join(HERE, "android_attestation_roots.json")
ATT_OID = "1.3.6.1.4.1.11129.2.1.17"
ROOTS_URL = "https://android.googleapis.com/attestation/root"
STATUS_URL = "https://android.googleapis.com/attestation/status"
REAL = ("josh", "shindevlin", "natoshisakamoto")


# ── minimal DER ────────────────────────────────────────────────────────────────
def der_read(b, i=0):
    t = b[i]; i += 1
    cls, cons, tag = t >> 6, bool(t & 0x20), t & 0x1F
    if tag == 0x1F:
        tag = 0
        while True:
            c = b[i]; i += 1; tag = (tag << 7) | (c & 0x7F)
            if not c & 0x80: break
    ln = b[i]; i += 1
    if ln & 0x80:
        n = ln & 0x7F; ln = int.from_bytes(b[i:i + n], "big"); i += n
    return cls, cons, tag, b[i:i + ln], i + ln


def der_items(b):
    i, out = 0, []
    while i < len(b):
        cls, cons, tag, v, i = der_read(b, i); out.append((cls, cons, tag, v))
    return out


def der_int(v): return int.from_bytes(v, "big", signed=True)


def enc_len(n):
    if n < 0x80: return bytes([n])
    s = n.to_bytes((n.bit_length() + 7) // 8, "big"); return bytes([0x80 | len(s)]) + s


def tlv(t, v): return bytes(t) + enc_len(len(v)) + v
def seq(*xs): return tlv([0x30], b"".join(xs))
def set_(*xs): return tlv([0x31], b"".join(sorted(xs)))
def octet(v): return tlv([0x04], v)
def null(): return b"\x05\x00"
def integer(n): return tlv([0x02], n.to_bytes(max(1, (n.bit_length() + 8) // 8), "big", signed=True))


def ctx(n, v):  # [n] EXPLICIT, high-tag form for n >= 31
    if n < 31: return tlv([0xA0 | n], v)
    t = []
    while True:
        t.insert(0, n & 0x7F); n >>= 7
        if not n: break
    return tlv([0xBF] + [x | 0x80 for x in t[:-1]] + [t[-1]], v)


# ── attestation ────────────────────────────────────────────────────────────────
def load_roots():
    try:
        pems = json.load(urllib.request.urlopen(ROOTS_URL, timeout=20))
        json.dump(pems, open(ROOTS_FILE, "w"), indent=1)
    except Exception:
        pems = json.load(open(ROOTS_FILE))
    return {x509.load_pem_x509_certificate(p.encode()).public_key().public_bytes(
        serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo) for p in pems}


def revoked_serials():
    try:
        st = json.load(urllib.request.urlopen(STATUS_URL, timeout=30)).get("entries", {})
        return {k.lower().lstrip("0") for k, v in st.items() if v.get("status") == "REVOKED"}
    except Exception as e:
        sys.exit(f"cannot fetch the attestation revocation list ({e}); refusing to grant")


def auth_list(b):
    return {tag: v for (_, _, tag, v) in der_items(b)}

def verify_attestation(hw, package, app_certs, allow_unlocked):
    chain = [x509.load_der_x509_certificate(base64.b64decode(c)) for c in hw["chain"]]
    for a, b in zip(chain, chain[1:]):
        a.verify_directly_issued_by(b)
    spki_of = lambda c: c.public_key().public_bytes(serialization.Encoding.DER, serialization.PublicFormat.SubjectPublicKeyInfo)
    if spki_of(chain[-1]) not in load_roots():
        sys.exit("attestation root is not a Google hardware attestation root")
    rev = revoked_serials()
    for c in chain:
        if format(c.serial_number, "x") in rev:
            sys.exit(f"attestation certificate {c.serial_number:x} is REVOKED")
    leaf = chain[0]; spki = spki_of(leaf)
    if base64.b64decode(hw["wrap_spki"]) != spki:
        sys.exit("exported wrapping key does not match the attested key")
    ext = next((e for e in leaf.extensions if e.oid.dotted_string == ATT_OID), None)
    if ext is None:
        sys.exit("leaf certificate has no key attestation extension")
    f = der_items(der_read(ext.value.value)[3])
    att_ver, att_lvl, km_ver, km_lvl = (der_int(f[i][3]) for i in range(4))
    challenge, sw, hwe = f[4][3], auth_list(f[6][3]), auth_list(f[7][3])
    levels = {0: "software", 1: "tee", 2: "strongbox"}
    if att_lvl < 1 or km_lvl < 1:
        sys.exit(f"key is not in secure hardware (attestation {levels.get(att_lvl)}, keymint {levels.get(km_lvl)})")
    if challenge != hashlib.sha256(f"hone-find-v1|{hw['account']}|{hw['device']}".encode()).digest():
        sys.exit("attestation challenge does not match this phone's account/device")
    purposes = {der_int(v) for (_, _, _, v) in der_items(der_items(hwe[1])[0][3])} if 1 in hwe else set()
    if 5 not in purposes:
        sys.exit("attested key is not a hardware PURPOSE_WRAP_KEY key")
    app = hwe.get(709) or sw.get(709)
    if app is None:
        sys.exit("no attestationApplicationId")
    a = der_items(der_items(der_items(app)[0][3])[0][3])  # [709] -> OCTET -> SEQUENCE{SET pkgs, SET digests}
    pkgs = [der_items(p[3])[0][3].decode() for p in der_items(a[0][3])]
    digests = [x[3].hex() for x in der_items(a[1][3])]
    if package not in pkgs:
        sys.exit(f"attested package {pkgs} is not {package}")
    if app_certs and not set(digests) & {c.lower() for c in app_certs}:
        sys.exit(f"app signing cert {digests} not in the allowed list")
    locked, boot = None, None
    if 704 in hwe:
        r = der_items(der_items(hwe[704])[0][3])
        locked = r[1][3] != b"\x00"
        boot = {0: "verified", 1: "self-signed", 2: "unverified", 3: "failed"}.get(der_int(r[2][3]))
    if (locked is False or boot not in (None, "verified")) and not allow_unlocked:
        sys.exit(f"bootloader unlocked or boot {boot}; pass --allow-unlocked to accept")
    digest = hashlib.sha256(b"".join(c.public_bytes(serialization.Encoding.DER) for c in chain)).hexdigest()
    return {"hw_key": hashlib.sha256(spki).hexdigest(), "attestation_digest": digest, "wrap_pub": leaf.public_key(),
            "security_level": levels[km_lvl], "keymint_version": km_ver, "attestation_version": att_ver,
            "package": pkgs, "app_cert_sha256": digests, "device_locked": locked, "verified_boot": boot}


# ── ImportWrappedKey (SecureKeyWrapper) ────────────────────────────────────────
def hmac_key_description():
    al = seq(ctx(1, set_(integer(2))),   # purpose SIGN
             ctx(2, integer(128)),       # algorithm HMAC
             ctx(3, integer(256)),       # key size
             ctx(5, set_(integer(4))),   # digest SHA-256
             ctx(8, integer(256)),       # min MAC length
             ctx(503, null()))           # no auth required
    return seq(integer(3), al)           # keyFormat RAW


def wrap_hmac_key(wrap_pub, key):
    transport, iv = os.urandom(32), os.urandom(12)
    kd = hmac_key_description()
    ct = AESGCM(transport).encrypt(iv, key, kd)
    enc_t = wrap_pub.encrypt(transport, padding.OAEP(mgf=padding.MGF1(hashes.SHA1()), algorithm=hashes.SHA256(), label=None))
    return seq(integer(0), octet(enc_t), octet(iv), kd, octet(ct[:-16]), octet(ct[-16:]))


def seed_g(hide, dev, g):
    return hl.hkdf(hide, b"hone-id-seed" + dev + (g.to_bytes(4, "big") if g else b""))


def prk(seed):
    h = hmac.HMAC(b"\x00" * 32, hashes.SHA256()); h.update(seed); return h.finalize()


# ── owner key ──────────────────────────────────────────────────────────────────
def owner_keys(a):
    if a.dry_run:
        if not a.test_owner or a.test_owner in REAL:
            sys.exit("--dry-run needs --test-owner NAME (not a real account)")
        return a.test_owner, hashlib.sha256(b"hone-findgrant-TEST|" + a.test_owner.encode()).digest(), 1
    if not (a.wallet and a.account):
        sys.exit("real grants need --wallet and --account (or use --dry-run)")
    k = hl.load_wallet(a.wallet, a.account, a.node)
    keys = hl.account(a.node, a.account).get("keys", {})
    hide = k.get("hide") or sys.exit("wallet has no hide key")
    if (keys or {}).get("hide") != hl.ed_pub(hide).hex():
        sys.exit("wallet hide key is not the account's current on-chain hide key")
    return a.account, hide, hl.account(a.node, a.account)["nonce"] + 1


def sign(hide, msg):
    return Ed25519PrivateKey.from_private_bytes(hide).sign(msg.encode()).hex()


def cmd_grant(a):
    hw = json.load(open(a.hwkey))
    att = verify_attestation(hw, a.package or hw["package"], a.app_cert, a.allow_unlocked)
    print(f"attestation ok: {att['security_level']} · keymint v{att['keymint_version']} · package {att['package']} · "
          f"app cert {[x[:16] for x in att['app_cert_sha256']]} · locked {att['device_locked']} · boot {att['verified_boot']}")
    owner, hide, nonce = owner_keys(a)
    gens = {k.lower(): int(v) for k, v in (x.split("=") for x in a.generation)}
    devs = {}
    for d in (a.device or [hw["device"]]):
        pub, _, label = d.partition(":"); pub = pub.lower()
        devs[pub] = label or ("this phone" if pub == hw["device"] else pub[:8])
    order = sorted(devs)
    dl = [{"device": d, "generation": gens.get(d, 0)} for d in order]
    msg = json.dumps({"chain_id": "hone", "type": "FIND_GRANT", "owner": owner, "hw_key": att["hw_key"],
                      "attestation_digest": att["attestation_digest"], "devices": dl, "nonce": nonce}, separators=(",", ":"))
    entry = {"owner": owner, "hw_key": att["hw_key"], "attestation_digest": att["attestation_digest"], "devices": dl,
             "epoch": 0, "nonce": nonce, "signed_by": owner, "signature": sign(hide, msg)}
    bundle = {"v": 1, "owner": owner, "hw_key": att["hw_key"], "dry_run": bool(a.dry_run), "grant": entry, "devices": []}
    for i, d in enumerate(order):
        g = gens.get(d, 0); s = seed_g(hide, bytes.fromhex(d), g)
        bundle["devices"].append({"device": d, "generation": g, "label": devs[d],
                                  "wrapped": base64.b64encode(wrap_hmac_key(att["wrap_pub"], prk(s))).decode()})
        if i == 0 and (a.dry_run or a.selftest):   # public reference vectors only
            w0 = hl.window()
            lids = [{"w": w, "lid": hl.window_key(hl.share_key(s, hl.period(w)), w)["lid"].hex()} for w in range(w0 - 4, w0 + 96)]
            wk = hl.window_key(hl.share_key(s, hl.period(w0)), w0)
            pt = json.dumps({"w": w0, "c": "TEST", "t": int(time.time() * 1000), "r": -42}).encode()
            bundle["selftest"] = {"device": d, "generation": g, "lids": lids, "sighting": {
                "w": w0, "blob": base64.b64encode(hl.seal(wk["pk"], hl.INFO_SIGHTING, pt, aad=wk["lid"])).decode()}}
    json.dump(bundle, open(a.out, "w"), indent=1)
    json.dump(entry, open(a.grant_out, "w"), indent=1)
    print(f"{'TEST ' if a.dry_run else ''}grant for @{owner}: hw_key {att['hw_key'][:16]}… · {len(order)} device(s) -> {a.out}; "
          f"signed FIND_GRANT -> {a.grant_out} (not submitted)")


def cmd_revoke(a):
    owner, hide, nonce = owner_keys(a)
    msg = json.dumps({"chain_id": "hone", "type": "FIND_REVOKE", "owner": owner, "hw_key": a.hwkey_id, "nonce": nonce},
                     separators=(",", ":"))
    json.dump({"owner": owner, "hw_key": a.hwkey_id, "epoch": 0, "nonce": nonce, "signed_by": owner,
               "signature": sign(hide, msg)}, open(a.grant_out, "w"), indent=1)
    print(f"signed FIND_REVOKE for {a.hwkey_id[:16]}… -> {a.grant_out} (not submitted)")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--node", default=os.environ.get("HONE_NODE", "http://127.0.0.1:4242"))
    sp = ap.add_subparsers(dest="cmd", required=True)
    for name in ("grant", "revoke"):
        p = sp.add_parser(name)
        p.add_argument("--dry-run", action="store_true"); p.add_argument("--test-owner")
        p.add_argument("--wallet"); p.add_argument("--account")
        p.add_argument("--grant-out", default="find_grant_entry.json")
    g = sp.choices["grant"]
    g.add_argument("--hwkey", required=True); g.add_argument("--device", action="append")
    g.add_argument("--generation", action="append", default=[]); g.add_argument("--package")
    g.add_argument("--app-cert", action="append", default=[]); g.add_argument("--out", default="hone-grant.json")
    g.add_argument("--allow-unlocked", action="store_true"); g.add_argument("--selftest", action="store_true")
    sp.choices["revoke"].add_argument("--hwkey-id", required=True)
    a = ap.parse_args()
    {"grant": cmd_grant, "revoke": cmd_revoke}[a.cmd](a)


if __name__ == "__main__":
    main()