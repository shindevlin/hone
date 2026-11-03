"""Reference verifier for hone-decide jury reveals (docs/JURY.md).
  python3 jury_verify.py reveal.json [expected_root_hex]
Recomputes leaf = sha256(vote_bytes || salt) and folds the Merkle proof; exits 1 on mismatch."""
import hashlib, json, struct, sys


def vote_bytes(v):
    b = b"hone-decide-vote-v1" + bytes.fromhex(v["entry_hash"]) + struct.pack("<Q", v["epoch"])
    for s in (v["task"], v["option"]):
        e = s.encode(); b += bytes([len(e)]) + e
    return b + bytes([v["conf_q8"]])


def verify(rev, root=None):
    leaf = hashlib.sha256(vote_bytes(rev["vote"]) + bytes.fromhex(rev["salt"])).digest()
    if leaf.hex() != rev["leaf"]:
        return False, "leaf mismatch"
    acc = leaf
    for p in rev["proof"]:
        h = bytes.fromhex(p["hash"])
        acc = hashlib.sha256(b"\x01" + (h + acc if p["side"] == "L" else acc + h)).digest()
    want = root or rev["committed_root"]
    return acc.hex() == want, acc.hex()


if __name__ == "__main__":
    rev = json.load(open(sys.argv[1]))
    ok, info = verify(rev, sys.argv[2] if len(sys.argv) > 2 else None)
    print("OK" if ok else "FAIL", info)
    sys.exit(0 if ok else 1)
