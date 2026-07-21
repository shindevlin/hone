#!/usr/bin/env python3
"""
Update the hone Hive account's peer list.

Usage:
    python3 hive_update_peers.py

Requires:  pip install beem

Run this whenever bootstrap node addresses change.
The HONE node reads this list at startup via the discovery module.
"""

import json
import sys

try:
    from beem import Hive
    from beem.account import Account
except ImportError:
    print("Install beem first:  pip install beem")
    sys.exit(1)

# ── Config ────────────────────────────────────────────────────────────────────

HIVE_ACCOUNT  = "hone"
POSTING_KEY   = ""  # set via env var or prompt — do not hardcode

# Update this list whenever bootstrap node addresses change.
# Format: libp2p multiaddr strings.
HONE_PEERS = [
    "/dns4/bootstrap1.honemesh.net/tcp/6942",
    "/dns4/bootstrap2.honemesh.net/tcp/6942",
    # Add more as the network grows:
    # "/ip4/1.2.3.4/tcp/6942",
]

# ── Main ──────────────────────────────────────────────────────────────────────

def main():
    import os
    key = POSTING_KEY or os.environ.get("HIVE_POSTING_KEY") or input("Posting key: ").strip()
    if not key:
        print("No posting key provided.")
        sys.exit(1)

    h   = Hive(keys=[key])
    acc = Account(HIVE_ACCOUNT, blockchain_instance=h)

    # Preserve any existing metadata fields, only overwrite hone_peers.
    try:
        existing = json.loads(acc["json_metadata"] or "{}")
    except Exception:
        existing = {}

    existing["hone_peers"] = HONE_PEERS
    new_meta = json.dumps(existing, separators=(",", ":"))

    print(f"Updating @{HIVE_ACCOUNT} json_metadata:")
    print(f"  {new_meta}")

    acc.update_account_profile(existing)
    print("Done.")

if __name__ == "__main__":
    main()
