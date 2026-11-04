#!/usr/bin/env python3
"""HONE location sharing + sighting crypto (reference implementation of
clients/hone-android-native/docs/SIGHTINGS.md; mirrors sightings/IdKeys.kt).

Runs on the host that holds the grantor's wallet file. Private keys are read from the
wallet JSON in memory and never printed, logged, written or sent anywhere.

  grant   --wallet W --account A --to B --device PUB [--label L] [--months N] [--out F]
  revoke  --wallet W --account A --to B --device PUB [--out F]
  list    --account A                                   (key names only; values are ciphertext)
  inbox   --wallet W --account B --from A[,A2]          (grantee: decrypt grants addressed to B)
  find    --wallet W --device PUB --db sightings.db     (owner: open sealed sightings of a device)
  device-seed-provision --wallet W --device PUB         (sealed id_seed for a device, for its claim)
  (global: --node URL, default $HONE_NODE or http://127.0.0.1:4242)

Grants live in the grantor's on-chain Memory store (MemorySet, value <= 4096 bytes):
  key   = "ls1-" + sha256("hone-locshare|" + grantee + "|" + device_pub_hex)[0..16].hex()
  value = base64(ECIES(grantee memo key, JSON{v,from,to,device,label,keys{period: share_k}}), aad = grantor)
"""
import argparse, base64, calendar, datetime, hashlib, json, os, sys, time, urllib.request

from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.asymmetric.x25519 import X25519PrivateKey, X25519PublicKey
from cryptography.hazmat.primitives.ciphers.aead import AESGCM
from cryptography.hazmat.primitives.kdf.hkdf import HKDF

WINDOW_S = 900
INFO_SIGHTING = "hone-sighting-v1"
INFO_GRANT = "hone-locshare-v1"
INFO_PROVISION = "hone-seed-provision-v1"
P = 2**255 - 19


def hkdf(ikm, info, salt=b"", n=32):
    return HKDF(hashes.SHA256(), n, salt or None, info).derive(ikm)


def window(t=None):
    return int(t if t is not None else time.time()) // WINDOW_S


def period(w):
    d = datetime.datetime.fromtimestamp(w * WINDOW_S, datetime.timezone.utc)
    return d.year * 12 + d.month - 1


def period_windows(p):
    y, m = divmod(p, 12)
    s = calendar.timegm((y, m + 1, 1, 0, 0, 0))
    y2, m2 = divmod(p + 1, 12)
    e = calendar.timegm((y2, m2 + 1, 1, 0, 0, 0))
    return range(s // WINDOW_S, e // WINDOW_S)


def device_seed(hide_priv, device_pub):
    return hkdf(hide_priv, b"hone-id-seed" + device_pub)


def share_key(seed, p):
    return hkdf(seed, b"hone-share" + p.to_bytes(4, "big"))


def xpub(sk):
    return X25519PrivateKey.from_private_bytes(sk).public_key().public_bytes_raw()


def window_key(share_k, w):
    sk = hkdf(share_k, b"hone-pkw" + w.to_bytes(8, "big"))
    pk = xpub(sk)
    return {"sk": sk, "pk": pk, "lid": hashlib.sha256(pk).digest()[:16],
            "tag": hkdf(share_k, b"hone-tag" + w.to_bytes(8, "big"), n=2)}


def seal(pk, info, pt, aad=b"", esk=None, nonce=None):
    esk = esk or os.urandom(32)
    e = X25519PrivateKey.from_private_bytes(esk)
    epk = e.public_key().public_bytes_raw()
    shared = e.exchange(X25519PublicKey.from_public_bytes(pk))
    key = hkdf(shared, info.encode(), salt=epk + pk)
    nonce = nonce or os.urandom(12)
    return epk + nonce + AESGCM(key).encrypt(nonce, pt, aad or None)


def open_(sk, pk, info, blob, aad=b""):
    epk = blob[:32]
    shared = X25519PrivateKey.from_private_bytes(sk).exchange(X25519PublicKey.from_public_bytes(epk))
    key = hkdf(shared, info.encode(), salt=epk + pk)
    try:
        return AESGCM(key).decrypt(blob[32:44], blob[44:], aad or None)
    except Exception:
        return None


def ed_pub_to_x(ed_pub):
    y = int.from_bytes(ed_pub, "little") & ((1 << 255) - 1)
    u = (1 + y) * pow((1 - y) % P, P - 2, P) % P
    return u.to_bytes(32, "little")


def ed_priv_to_x(ed_priv):
    return hashlib.sha512(ed_priv).digest()[:32]


def ed_pub(priv):
    return Ed25519PrivateKey.from_private_bytes(priv).public_key().public_bytes_raw()


def grant_key_name(grantee, device_hex):
    return "ls1-" + hashlib.sha256(f"hone-locshare|{grantee}|{device_hex}".encode()).digest()[:16].hex()


# ── chain I/O ─────────────────────────────────────────────────────────────────
def http(node, path, body=None, method=None):
    req = urllib.request.Request(node + path, method=method or ("POST" if body is not None else "GET"),
                                 data=None if body is None else json.dumps(body).encode(),
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as r:
        return json.loads(r.read() or b"{}")


def account(node, a):
    return http(node, f"/api/account/{a}")


def canonical(fields, chain_id, typ):
    # Same message hone-cli's sign_entry builds (serde_json: sorted keys, compact).
    m = dict(fields, chain_id=chain_id, type=typ)
    return json.dumps(m, sort_keys=True, separators=(",", ":"))


def load_wallet(path, acct, node):
    w = json.load(open(os.path.expanduser(path)))
    keys = account(node, acct).get("keys", {})
    k = {r: bytes.fromhex(w[f"hone_{r}_private_key"]) for r in ("active", "memo", "hide") if w.get(f"hone_{r}_private_key")}
    for r, v in k.items():  # refuse a wallet that is not this account's
        if keys.get(r) and keys[r] != ed_pub(v).hex():
            sys.exit(f"wallet {r} key does not match {acct}'s on-chain {r} key")
    if "hide" in k and not keys.get("hide"):
        print(f"note: {acct} has no hide key registered on chain; using the wallet's", file=sys.stderr)
    return k


def submit(node, acct, key, value, active, out, delete=False):
    chain_id = http(node, "/api/node/info").get("chain_id", "hone")
    nonce = int(account(node, acct).get("nonce", 0)) + 1
    fields = {"account": acct, "key": key, "nonce": nonce}
    if not delete:
        fields["value"] = value
    msg = canonical(fields, chain_id, "MEMORY_DELETE" if delete else "MEMORY_SET")
    sig = Ed25519PrivateKey.from_private_bytes(active).sign(msg.encode()).hex()
    body = {"nonce": nonce, "signature": sig}
    if not delete:
        body["value"] = value
    if out:
        json.dump({"method": "DELETE" if delete else "POST", "path": f"/api/memory/{acct}/{key}", "body": body}, open(out, "w"))
        print(f"signed request written to {out} (not submitted)")
        return
    r = http(node, f"/api/memory/{acct}/{key}", body, method="DELETE" if delete else "POST")
    print("submitted:", json.dumps(r))


# ── commands ──────────────────────────────────────────────────────────────────
def cmd_grant(a):
    k = load_wallet(a.wallet, a.account, a.node)
    if "hide" not in k or "active" not in k:
        sys.exit("wallet needs hide + active keys")
    memo = account(a.node, a.to).get("keys", {}).get("memo")
    if not memo:
        sys.exit(f"{a.to} has no memo key on chain: they must register one before anyone can share with them")
    dev = bytes.fromhex(a.device)
    seed = device_seed(k["hide"], dev)
    p0 = period(window())
    keys = {str(p): share_key(seed, p).hex() for p in range(p0, p0 + a.months)}
    pt = json.dumps({"v": 1, "from": a.account, "to": a.to, "device": a.device,
                     "label": a.label or a.device[:8], "keys": keys}, separators=(",", ":")).encode()
    blob = seal(ed_pub_to_x(bytes.fromhex(memo)), INFO_GRANT, pt, aad=a.account.encode())
    value = base64.b64encode(blob).decode()
    assert len(value) <= 4096
    name = grant_key_name(a.to, a.device)
    print(f"grant {a.account} -> {a.to} device {a.device[:16]}.. periods {sorted(keys)} key {name}")
    submit(a.node, a.account, name, value, k["active"], a.out)


def cmd_revoke(a):
    k = load_wallet(a.wallet, a.account, a.node)
    name = grant_key_name(a.to, a.device)
    print(f"revoke {a.account} -> {a.to} device {a.device[:16]}.. key {name}")
    submit(a.node, a.account, name, None, k["active"], a.out, delete=True)


def cmd_list(a):
    mem = http(a.node, f"/api/memory/{a.account}").get("memory", {})
    for name in sorted(mem):
        if name.startswith("ls1-"):
            print(name, len(mem[name]), "bytes")


def decrypt_grant(memo_priv, grantor, value):
    sk = ed_priv_to_x(memo_priv)
    pt = open_(sk, xpub(sk), INFO_GRANT, base64.b64decode(value), aad=grantor.encode())
    return json.loads(pt) if pt else None


def cmd_inbox(a):
    k = load_wallet(a.wallet, a.account, a.node)
    for g in a.frm.split(","):
        mem = http(a.node, f"/api/memory/{g}").get("memory", {})
        for name, v in mem.items():
            if name.startswith("ls1-"):
                d = decrypt_grant(k["memo"], g, v)
                if d and d.get("to") == a.account:
                    print(f"from {g}: device {d['device'][:16]}.. '{d['label']}' periods {sorted(d['keys'])}")


def cmd_find(a):
    """Owner side: open sealed sightings of one of my devices from a sightings.db copy."""
    import sqlite3
    w = json.load(open(os.path.expanduser(a.wallet)))
    seed = device_seed(bytes.fromhex(w["hone_hide_private_key"]), bytes.fromhex(a.device))
    now = window()
    keys = {}
    for wi in range(now - a.days * 96, now + 1):
        k = window_key(share_key(seed, period(wi)), wi)
        keys[k["lid"].hex()] = k
    db = sqlite3.connect(a.db)
    n = 0
    for lid, wi, blob, ts in db.execute("SELECT lid, window, blob, ts FROM enc_sighting"):
        k = keys.get(lid)
        if not k:
            continue
        pt = open_(k["sk"], k["pk"], INFO_SIGHTING, bytes(blob), aad=k["lid"])
        if pt:
            n += 1
            print("sighting", json.loads(pt))
    print(f"{n} sighting(s) of device {a.device[:16]}.. opened")


def cmd_provision(a):
    w = json.load(open(os.path.expanduser(a.wallet)))
    dev = bytes.fromhex(a.device)
    blob = seal(ed_pub_to_x(dev), INFO_PROVISION, device_seed(bytes.fromhex(w["hone_hide_private_key"]), dev))
    print(base64.b64encode(blob).decode())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--node", default=os.environ.get("HONE_NODE", "http://127.0.0.1:4242"))
    sp = ap.add_subparsers(dest="cmd", required=True)
    for n in ("grant", "revoke", "inbox", "device-seed-provision"):
        p = sp.add_parser(n); p.add_argument("--wallet", required=True)
        if n != "device-seed-provision":
            p.add_argument("--account", required=True)
    for n in ("grant", "revoke"):
        p = sp.choices[n]; p.add_argument("--to", required=True); p.add_argument("--device", required=True)
        p.add_argument("--out")
    sp.choices["grant"].add_argument("--label"); sp.choices["grant"].add_argument("--months", type=int, default=2)
    sp.choices["inbox"].add_argument("--from", dest="frm", required=True)
    sp.choices["device-seed-provision"].add_argument("--device", required=True)
    sp.add_parser("list").add_argument("--account", required=True)
    p = sp.add_parser("find"); p.add_argument("--wallet", required=True); p.add_argument("--device", required=True)
    p.add_argument("--db", required=True); p.add_argument("--days", type=int, default=7)
    a = ap.parse_args()
    {"grant": cmd_grant, "revoke": cmd_revoke, "list": cmd_list, "inbox": cmd_inbox,
     "device-seed-provision": cmd_provision, "find": cmd_find}[a.cmd](a)


if __name__ == "__main__":
    main()
