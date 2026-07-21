#!/usr/bin/env python3
"""
Update the @hone Hive account's json_metadata with mainnet and testnet bootstrap peers.

Usage:
    python3 hive-register-peers.py --key 5K...WIF

The posting key is the WIF for the @hone Hive account.
Reads HONE_HIVE_POSTING_KEY from env if --key is not given.

json_metadata structure:
  {
    "hone_peers":         [...mainnet multiaddrs...],
    "hone_testnet_peers": [...testnet multiaddrs...]
  }
"""

import sys, os, json, argparse

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--key", default=os.environ.get("HONE_HIVE_POSTING_KEY", ""),
                        help="WIF posting key for @hone Hive account")
    parser.add_argument("--mainnet-peers", nargs="*", default=[
        "/dns4/bootstrap1.honemesh.net/tcp/6942",
        "/dns4/bootstrap2.honemesh.net/tcp/6942",
    ], help="Mainnet multiaddrs")
    parser.add_argument("--testnet-peers", nargs="*", default=[
        "/ip4/192.168.68.72/tcp/6943",
        "/ip4/100.90.146.17/tcp/6943",
    ], help="Testnet (hone-testnet) multiaddrs")
    parser.add_argument("--dry-run", action="store_true", help="Print metadata without posting")
    args = parser.parse_args()

    if not args.key and not args.dry_run:
        print("Error: provide --key <WIF> or set HONE_HIVE_POSTING_KEY", file=sys.stderr)
        sys.exit(1)

    metadata = {
        "hone_peers": args.mainnet_peers,
        "hone_testnet_peers": args.testnet_peers,
    }

    print("json_metadata to post:")
    print(json.dumps(metadata, indent=2))

    if args.dry_run:
        print("\n[dry-run] not posting")
        return

    try:
        from beem import Hive
        from beem.account import Account
    except ImportError:
        print("beem not installed. Run: uv pip install beem --python /tmp/hone-hive-venv", file=sys.stderr)
        sys.exit(1)

    hive = Hive(keys=[args.key])
    try:
        from beembase.operations import Account_update2
    except ImportError:
        print("beembase not found. Reinstall beem.", file=sys.stderr)
        sys.exit(1)

    # account_update2 writes posting_json_metadata — posting key is sufficient.
    op = Account_update2(**{
        "account": "hone",
        "json_metadata": "",
        "posting_json_metadata": json.dumps(metadata),
        "extensions": [],
    })
    hive.finalizeOp(op, "hone", "posting")
    print("\nPosted to Hive. Verify at: https://hiveblocks.com/@hone")

if __name__ == "__main__":
    main()
