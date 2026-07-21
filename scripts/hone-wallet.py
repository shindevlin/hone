#!/usr/bin/env python3
"""
hone-wallet.py — offline BIP39 multi-chain wallet generator
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

NO NETWORK CALLS. Every function is shown in plain Python so you can read
exactly what happens to your mnemonic. Run this air-gapped if you want.
Produces output byte-for-byte identical to the Rust hone-node wallet.rs.

Standards implemented — read the linked specs to verify this code yourself:
  BIP-39  https://github.com/bitcoin/bips/blob/master/bip-0039.mediawiki
  BIP-32  https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki
  BIP-44  https://github.com/bitcoin/bips/blob/master/bip-0044.mediawiki
  SLIP-10 https://github.com/satoshilabs/slips/blob/master/slip-0010.md
  EIP-55  https://eips.ethereum.org/EIPS/eip-55

Derivation map (mirrors wallet.rs exactly):
  HONE    ed25519   seed[0:32]  (raw BIP-39 seed — no HD path)
  Bitcoin  secp256k1 BIP-44  m/44'/0'/0'/0/0
  Ethereum secp256k1 BIP-44  m/44'/60'/0'/0/0  (same addr: Polygon/BSC/Avax/Arb/Op/Base)
  XRP      secp256k1 BIP-44  m/44'/144'/0'/0/0
  Cosmos   secp256k1 BIP-44  m/44'/118'/0'/0/0
  Tron     secp256k1 BIP-44  m/44'/195'/0'/0/0
  Solana   ed25519   SLIP-10 m/44'/501'/0'/0'
  TON      ed25519   SLIP-10 m/44'/607'/0'
  Stellar  ed25519   SLIP-10 m/44'/148'/0'/0'
  NEAR     ed25519   SLIP-10 m/44'/397'/0'/0'
  Sui      ed25519   SLIP-10 m/44'/784'/0'/0'
  Aptos    ed25519   SLIP-10 m/44'/637'/0'/0'

Install dependencies (one-time):
  pip install mnemonic ecdsa cryptography base58 bech32 pycryptodome

Usage:
  python3 hone-wallet.py generate              # new random 12-word wallet
  python3 hone-wallet.py derive                # enter your existing mnemonic
  python3 hone-wallet.py show    [wallet.key]  # display a saved wallet.key
  python3 hone-wallet.py verify  [wallet.key]  # prove mnemonic matches wallet.key
"""

import sys
import os
import hmac as _hmac
import hashlib
import json
import secrets
import base64
import getpass

# ── Dependency check ──────────────────────────────────────────────────────────
_MISSING = []
try:
    from mnemonic import Mnemonic as _Mnemonic
except ImportError:
    _MISSING.append("mnemonic")
try:
    from ecdsa import SigningKey as _EcdsaSK, SECP256k1 as _SECP256K1
except ImportError:
    _MISSING.append("ecdsa")
try:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey as _Ed25519PK
except ImportError:
    _MISSING.append("cryptography")
try:
    import base58 as _base58
except ImportError:
    _MISSING.append("base58")
try:
    import bech32 as _bech32
except ImportError:
    _MISSING.append("bech32")
try:
    from Crypto.Hash import keccak as _keccak_mod
except ImportError:
    _MISSING.append("pycryptodome")

if _MISSING:
    print("Missing dependencies. Install with:")
    print(f"  pip install {' '.join(_MISSING)}")
    sys.exit(1)

# ── Constants ─────────────────────────────────────────────────────────────────

# secp256k1 group order — from SEC 2 v2 §2.4.1
SECP256K1_N = 0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFEBAAEDCE6AF48A03BBFD25E8CD0364141

# BIP-32 / SLIP-10 hardened key flag (index >= 2^31 is hardened)
HARDENED = 0x80000000

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# BIP-39: Mnemonic → 64-byte seed
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Seed = PBKDF2-HMAC-SHA512(password=mnemonic, salt="mnemonic"+passphrase,
#                           iterations=2048, dklen=64)
# We always use passphrase="" — the Rust node never prompts for one.

def mnemonic_to_seed(words: str) -> bytes:
    """BIP-39: validate mnemonic and derive 64-byte seed (empty passphrase)."""
    mnemo = _Mnemonic("english")
    if not mnemo.check(words):
        raise ValueError("Invalid mnemonic — bad checksum or unknown words")
    return _Mnemonic.to_seed(words, passphrase="")

def generate_mnemonic() -> str:
    """Generate a random 12-word mnemonic from 128 bits of OS entropy."""
    return _Mnemonic("english").to_mnemonic(secrets.token_bytes(16))

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# BIP-32: secp256k1 hierarchical deterministic key derivation
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def _H(key: bytes, data: bytes) -> bytes:
    """HMAC-SHA512."""
    return _hmac.new(key, data, hashlib.sha512).digest()

def _compressed_pubkey(privkey: bytes) -> bytes:
    """SEC1 compressed public key: 0x02/0x03 || x (33 bytes)."""
    point = _EcdsaSK.from_string(privkey, curve=_SECP256K1).get_verifying_key().pubkey.point
    prefix = b'\x02' if point.y() % 2 == 0 else b'\x03'
    return prefix + point.x().to_bytes(32, 'big')

def _add_mod_n(a: bytes, b: bytes) -> bytes:
    """(a + b) mod secp256k1_N, both as 32-byte big-endian."""
    return ((int.from_bytes(a, 'big') + int.from_bytes(b, 'big')) % SECP256K1_N).to_bytes(32, 'big')

def bip32_master(seed: bytes) -> tuple:
    """BIP-32 master key: HMAC-SHA512(key="Bitcoin seed", data=seed)."""
    I = _H(b"Bitcoin seed", seed)
    return I[:32], I[32:]  # (master_privkey, master_chaincode)

def _child_hard(sk: bytes, chain: bytes, idx: int) -> tuple:
    """
    BIP-32 hardened child (index >= 2^31).
    Data = 0x00 || sk || ser32(idx | HARDENED)
    child = (HMAC_IL + parent_sk) mod N
    """
    data = b'\x00' + sk + (idx | HARDENED).to_bytes(4, 'big')
    I = _H(chain, data)
    return _add_mod_n(I[:32], sk), I[32:]

def _child_normal(sk: bytes, chain: bytes, idx: int) -> tuple:
    """
    BIP-32 normal child (index < 2^31).
    Data = compressed_pubkey(sk) || ser32(idx)
    child = (HMAC_IL + parent_sk) mod N
    """
    data = _compressed_pubkey(sk) + idx.to_bytes(4, 'big')
    I = _H(chain, data)
    return _add_mod_n(I[:32], sk), I[32:]

def bip44(seed: bytes, coin_type: int) -> bytes:
    """
    BIP-44: m/44'/coin_type'/0'/0/0
    Levels 44', coin_type', 0' are hardened; final 0/0 are normal.
    """
    sk, ch = bip32_master(seed)
    sk, ch = _child_hard(sk, ch, 44)
    sk, ch = _child_hard(sk, ch, coin_type)
    sk, ch = _child_hard(sk, ch, 0)
    sk, ch = _child_normal(sk, ch, 0)
    sk, ch = _child_normal(sk, ch, 0)
    return sk

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# SLIP-10: ed25519 hierarchical deterministic key derivation
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Unlike BIP-32, ALL ed25519 derivation levels MUST be hardened (SLIP-10 §2.1.2).
# Data per level = 0x00 || key || ser32(index | HARDENED)

def slip10(seed: bytes, path: list) -> tuple:
    """
    SLIP-10 ed25519 derivation.
    path: list of raw indices (hardened flag added automatically).
    Returns (private_key_32_bytes, public_key_32_bytes).
    """
    I = _H(b"ed25519 seed", seed)
    key, chain = I[:32], I[32:]
    for idx in path:
        data = b'\x00' + key + (idx | HARDENED).to_bytes(4, 'big')
        I = _H(chain, data)
        key, chain = I[:32], I[32:]
    pub = _Ed25519PK.from_private_bytes(key).public_key().public_bytes_raw()
    return key, pub

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Address encoding — one function per chain, each shown in full
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def _hash160(data: bytes) -> bytes:
    """HASH160 = RIPEMD-160(SHA-256(data)) — used by Bitcoin, XRP, Cosmos."""
    sha = hashlib.sha256(data).digest()
    return hashlib.new('ripemd160', sha).digest()

def _checksum4(data: bytes) -> bytes:
    """First 4 bytes of double-SHA-256 — used in base58check formats."""
    return hashlib.sha256(hashlib.sha256(data).digest()).digest()[:4]

def _keccak256(data: bytes) -> bytes:
    """
    Keccak-256 as used by Ethereum and Tron.
    NOT the same as NIST SHA3-256 — different padding (0x01 vs 0x06).
    Do not replace with hashlib.sha3_256.
    """
    k = _keccak_mod.new(digest_bits=256)
    k.update(data)
    return k.digest()

# ── Bitcoin ───────────────────────────────────────────────────────────────────
# Public key: SEC1 compressed hex
# WIF: Wallet Import Format = base58check(0x80 || privkey || 0x01)
# The 0x01 suffix signals a compressed public key will be used.

def _btc_wif(sk: bytes) -> str:
    payload = b'\x80' + sk + b'\x01'
    return _base58.b58encode(payload + _checksum4(payload)).decode()

def _btc_pubkey(sk: bytes) -> str:
    return _compressed_pubkey(sk).hex()

# ── Ethereum / all EVM chains ─────────────────────────────────────────────────
# Address = EIP-55 checksum encoding of last 20 bytes of keccak256(uncompressed_pubkey[1:])
# The same private key and address works on every EVM chain (Polygon, BSC, Avalanche,
# Arbitrum, Optimism, Base) — only the chain_id in the transaction differs.

def _eth_addr(sk: bytes) -> str:
    vk = _EcdsaSK.from_string(sk, curve=_SECP256K1).get_verifying_key()
    raw = keccak256_addr = _keccak256(vk.to_string())  # to_string() = x||y (64 bytes), no prefix
    addr_bytes = raw[12:]  # last 20 bytes

    # EIP-55 mixed-case checksum
    hex_lower = addr_bytes.hex()
    h = _keccak256(hex_lower.encode())
    result = "0x"
    for i, c in enumerate(hex_lower):
        nibble = (h[i // 2] >> (4 if i % 2 == 0 else 0)) & 0x0F
        result += c.upper() if c.isalpha() and nibble >= 8 else c
    return result

# ── XRP ───────────────────────────────────────────────────────────────────────
# base58check(0x00 || HASH160(compressed_pubkey)) using Bitcoin alphabet

def _xrp_addr(sk: bytes) -> str:
    h160 = _hash160(_compressed_pubkey(sk))
    payload = b'\x00' + h160
    return _base58.b58encode(payload + _checksum4(payload)).decode()

# ── Cosmos / ATOM ─────────────────────────────────────────────────────────────
# bech32("cosmos", HASH160(compressed_pubkey))

def _cosmos_addr(sk: bytes) -> str:
    h160 = _hash160(_compressed_pubkey(sk))
    data5 = _bech32.convertbits(h160, 8, 5)
    return _bech32.bech32_encode("cosmos", data5)

# ── Tron ──────────────────────────────────────────────────────────────────────
# Same computation as Ethereum but with 0x41 prefix, then base58check

def _tron_addr(sk: bytes) -> str:
    vk = _EcdsaSK.from_string(sk, curve=_SECP256K1).get_verifying_key()
    h = _keccak256(vk.to_string())
    payload = b'\x41' + h[12:]  # 0x41 = Tron mainnet version byte
    return _base58.b58encode(payload + _checksum4(payload)).decode()

# ── Solana ────────────────────────────────────────────────────────────────────
# Address = base58(ed25519_public_key)

def _sol_addr(pub: bytes) -> str:
    return _base58.b58encode(pub).decode()

# ── TON ───────────────────────────────────────────────────────────────────────
# wallet.rs stores raw public key hex — full TON address derivation requires
# contract state init hash, which is wallet-type-specific (V4R2 etc.).
# The hex public key is the portable, wallet-agnostic identifier.

def _ton_pubkey(pub: bytes) -> str:
    return pub.hex()

# ── Stellar / XLM ─────────────────────────────────────────────────────────────
# StrKey encoding: version_byte=0x30 (produces 'G' prefix) || pub_key,
# then CRC-16/XMODEM checksum (little-endian 2 bytes), then RFC-4648 base32 (no padding).
# Spec: https://developers.stellar.org/docs/fundamentals-and-concepts/stellar-data-structures/accounts

def _crc16_xmodem(data: bytes) -> int:
    """CRC-16/XMODEM: poly=0x1021, init=0x0000, no reflection, xorout=0x0000."""
    crc = 0
    for b in data:
        crc ^= b << 8
        for _ in range(8):
            crc = ((crc << 1) ^ 0x1021 if crc & 0x8000 else crc << 1) & 0xFFFF
    return crc

def _stellar_addr(pub: bytes) -> str:
    payload = bytes([0x30]) + pub
    crc = _crc16_xmodem(payload).to_bytes(2, 'little')
    return base64.b32encode(payload + crc).decode().rstrip('=')

# ── NEAR ──────────────────────────────────────────────────────────────────────
# Address = base58(ed25519_public_key) — same encoding as Solana

def _near_addr(pub: bytes) -> str:
    return _base58.b58encode(pub).decode()

# ── Sui ───────────────────────────────────────────────────────────────────────
# Address = Blake2b-256(0x00 || pub_key), hex with "0x" prefix
# 0x00 = ed25519 scheme flag (Sui supports multiple signature schemes)

def _sui_addr(pub: bytes) -> str:
    h = hashlib.blake2b(b'\x00' + pub, digest_size=32).digest()
    return "0x" + h.hex()

# ── Aptos ─────────────────────────────────────────────────────────────────────
# Address = SHA3-256(pub_key || 0x00), hex with "0x" prefix
# 0x00 = single ed25519 authentication scheme flag
# This IS NIST SHA3-256 (not Keccak) — hashlib.sha3_256 is correct here.

def _aptos_addr(pub: bytes) -> str:
    h = hashlib.sha3_256(pub + b'\x00').digest()
    return "0x" + h.hex()

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Full wallet derivation
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def derive_all(words: str) -> dict:
    """
    Derive every chain key from a BIP-39 mnemonic.
    Returns a dict matching the WalletKeys struct in wallet.rs — drop this
    directly into ~/.hone/wallet.key and the Rust node will load it.
    """
    seed = mnemonic_to_seed(words)

    # HONE role keys: SLIP-10 m/44'/6942'/role'/0'
    # coin 6942 = HONE. role 0=owner,1=active,2=posting,3=memo,4=hide,5=seek
    HONE_COIN = 6942
    owner_priv,   owner_pub   = slip10(seed, [44, HONE_COIN, 0, 0])
    active_priv,  active_pub  = slip10(seed, [44, HONE_COIN, 1, 0])
    post_priv,    post_pub    = slip10(seed, [44, HONE_COIN, 2, 0])
    memo_priv,    memo_pub    = slip10(seed, [44, HONE_COIN, 3, 0])
    hide_priv,    hide_pub    = slip10(seed, [44, HONE_COIN, 4, 0])
    seek_priv,    seek_pub    = slip10(seed, [44, HONE_COIN, 5, 0])

    # secp256k1 BIP-44 chains
    btc = bip44(seed, 0)    # Bitcoin
    eth = bip44(seed, 60)   # Ethereum (all EVM)
    xrp = bip44(seed, 144)  # XRP
    atm = bip44(seed, 118)  # Cosmos
    trx = bip44(seed, 195)  # Tron

    # ed25519 SLIP-10 chains
    sol_priv,  sol_pub  = slip10(seed, [44, 501, 0, 0])  # Solana  m/44'/501'/0'/0'
    ton_priv,  ton_pub  = slip10(seed, [44, 607, 0])     # TON     m/44'/607'/0'
    xlm_priv,  xlm_pub  = slip10(seed, [44, 148, 0, 0]) # Stellar m/44'/148'/0'/0'
    near_priv, near_pub = slip10(seed, [44, 397, 0, 0]) # NEAR    m/44'/397'/0'/0'
    sui_priv,  sui_pub  = slip10(seed, [44, 784, 0, 0]) # Sui     m/44'/784'/0'/0'
    apt_priv,  apt_pub  = slip10(seed, [44, 637, 0, 0]) # Aptos   m/44'/637'/0'/0'

    return {
        "mnemonic":            words,

        # HONE role keys (SLIP-10 m/44'/6942'/role'/0')
        "hone_owner_private_key":   owner_priv.hex(),
        "hone_owner_public_key":    owner_pub.hex(),
        "hone_active_private_key":  active_priv.hex(),
        "hone_active_public_key":   active_pub.hex(),
        "hone_private_key":         post_priv.hex(),   # posting (back-compat field name)
        "hone_public_key":          post_pub.hex(),
        "hone_memo_private_key":    memo_priv.hex(),
        "hone_memo_public_key":     memo_pub.hex(),
        "hone_hide_private_key":    hide_priv.hex(),
        "hone_hide_public_key":     hide_pub.hex(),
        "hone_seek_private_key":    seek_priv.hex(),
        "hone_seek_public_key":     seek_pub.hex(),

        "bitcoin_wif":         _btc_wif(btc),
        "bitcoin_pubkey":      _btc_pubkey(btc),

        "ethereum_address":     _eth_addr(eth),
        "ethereum_private_key": eth.hex(),

        "xrp_address":          _xrp_addr(xrp),
        "xrp_private_key":      xrp.hex(),

        "cosmos_address":       _cosmos_addr(atm),
        "cosmos_private_key":   atm.hex(),

        "tron_address":         _tron_addr(trx),
        "tron_private_key":     trx.hex(),

        "solana_address":       _sol_addr(sol_pub),
        "solana_private_key":   sol_priv.hex(),

        "ton_public_key":       _ton_pubkey(ton_pub),
        "ton_private_key":      ton_priv.hex(),

        "stellar_address":      _stellar_addr(xlm_pub),
        "stellar_private_key":  xlm_priv.hex(),

        "near_address":         _near_addr(near_pub),
        "near_private_key":     near_priv.hex(),

        "sui_address":          _sui_addr(sui_pub),
        "sui_private_key":      sui_priv.hex(),

        "aptos_address":        _aptos_addr(apt_pub),
        "aptos_private_key":    apt_priv.hex(),
    }

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Save / load wallet.key
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

DEFAULT_PATH = os.path.expanduser("~/.hone/wallet.key")

def save_wallet(wallet: dict, path: str) -> None:
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, 'w') as f:
        json.dump(wallet, f, indent=2)
    os.chmod(path, 0o600)
    print(f"\n  Saved to: {path}  (chmod 600)")

def load_wallet(path: str) -> dict:
    with open(path) as f:
        return json.load(f)

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Display
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def print_wallet(w: dict, show_private: bool = False) -> None:
    W = 80
    priv = lambda v: v if show_private else "(hidden — use --private to show)"

    print("\n" + "═" * W)
    print("  HONE WALLET — generated offline, no data left this machine")
    print("═" * W)
    print(f"\n  MNEMONIC  ← write these 12 words down. they regenerate everything below.")
    print(f"\n  {w['mnemonic']}\n")
    print("─" * W)

    chains = [
        ("HONE OWNER KEY  (SLIP-10 m/44'/6942'/0'/0') — key rotation & governance",
         [("public key",   w["hone_owner_public_key"]),
          ("private key",  priv(w["hone_owner_private_key"]))]),

        ("HONE ACTIVE KEY  (SLIP-10 m/44'/6942'/1'/0') — transfers & staking",
         [("public key",   w["hone_active_public_key"]),
          ("private key",  priv(w["hone_active_private_key"]))]),

        ("HONE POSTING KEY  (SLIP-10 m/44'/6942'/2'/0') — daily activity",
         [("public key",   w["hone_public_key"]),
          ("private key",  priv(w["hone_private_key"]))]),

        ("HONE MEMO KEY  (SLIP-10 m/44'/6942'/3'/0') — encrypted messages",
         [("public key",   w["hone_memo_public_key"]),
          ("private key",  priv(w["hone_memo_private_key"]))]),

        ("HONE HIDE KEY  (SLIP-10 m/44'/6942'/4'/0') — private content",
         [("public key",   w["hone_hide_public_key"]),
          ("private key",  priv(w["hone_hide_private_key"]))]),

        ("HONE SEEK KEY  (SLIP-10 m/44'/6942'/5'/0') — encrypted delivery",
         [("public key",   w["hone_seek_public_key"]),
          ("private key",  priv(w["hone_seek_private_key"]))]),

        ("BITCOIN  (BIP-44  m/44'/0'/0'/0/0)",
         [("pubkey",       w["bitcoin_pubkey"]),
          ("WIF",          priv(w["bitcoin_wif"]))]),

        ("ETHEREUM / POLYGON / BSC / AVALANCHE / ARBITRUM / OPTIMISM / BASE\n"
         "  (BIP-44  m/44'/60'/0'/0/0 — same address on every EVM chain)",
         [("address",      w["ethereum_address"]),
          ("private key",  priv(w["ethereum_private_key"]))]),

        ("XRP  (BIP-44  m/44'/144'/0'/0/0)",
         [("address",      w["xrp_address"]),
          ("private key",  priv(w["xrp_private_key"]))]),

        ("COSMOS / ATOM  (BIP-44  m/44'/118'/0'/0/0)",
         [("address",      w["cosmos_address"]),
          ("private key",  priv(w["cosmos_private_key"]))]),

        ("TRON  (BIP-44  m/44'/195'/0'/0/0)",
         [("address",      w["tron_address"]),
          ("private key",  priv(w["tron_private_key"]))]),

        ("SOLANA  (SLIP-10  m/44'/501'/0'/0')",
         [("address",      w["solana_address"]),
          ("private key",  priv(w["solana_private_key"]))]),

        ("TON  (SLIP-10  m/44'/607'/0')",
         [("public key",   w["ton_public_key"]),
          ("private key",  priv(w["ton_private_key"]))]),

        ("STELLAR / XLM  (SLIP-10  m/44'/148'/0'/0')",
         [("address",      w["stellar_address"]),
          ("private key",  priv(w["stellar_private_key"]))]),

        ("NEAR  (SLIP-10  m/44'/397'/0'/0')",
         [("address",      w["near_address"]),
          ("private key",  priv(w["near_private_key"]))]),

        ("SUI  (SLIP-10  m/44'/784'/0'/0')",
         [("address",      w["sui_address"]),
          ("private key",  priv(w["sui_private_key"]))]),

        ("APTOS  (SLIP-10  m/44'/637'/0'/0')",
         [("address",      w["aptos_address"]),
          ("private key",  priv(w["aptos_private_key"]))]),
    ]

    for name, fields in chains:
        print(f"\n  {name}")
        for label, value in fields:
            print(f"    {label:<14} {value}")

    print("\n" + "═" * W + "\n")
    if not show_private:
        print("  Private keys hidden. Run with --private to display them.\n")

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Commands
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def cmd_generate(path, show_private):
    print("\n  Generating new wallet from OS random bytes...")
    words = generate_mnemonic()
    wallet = derive_all(words)
    print_wallet(wallet, show_private)
    save_wallet(wallet, path)
    print("  WARNING: Your mnemonic is above. Write it down before closing this window.")
    print("  Anyone with those 12 words can derive every key in this file.\n")

def cmd_derive(path, show_private):
    print("\n  Enter your BIP-39 mnemonic (12 or 24 words).")
    print("  Input is hidden and never stored by this script.\n")
    words = getpass.getpass("  Mnemonic: ").strip()
    if not words:
        sys.exit("  No mnemonic entered.")
    print("\n  Deriving keys (this takes a moment — PBKDF2 with 2048 rounds)...")
    wallet = derive_all(words)
    print_wallet(wallet, show_private)
    if os.path.exists(path):
        ans = input(f"  {path} already exists. Overwrite? [y/N] ").strip().lower()
        if ans != 'y':
            print("  Aborted — file not written.")
            return
    save_wallet(wallet, path)

def cmd_show(path, show_private):
    if not os.path.exists(path):
        sys.exit(f"  Not found: {path}")
    print_wallet(load_wallet(path), show_private)

def cmd_verify(path, show_private):
    if not os.path.exists(path):
        sys.exit(f"  Not found: {path}")
    saved = load_wallet(path)
    print("\n  Enter your mnemonic to verify it matches the saved wallet:")
    words = getpass.getpass("  Mnemonic: ").strip()
    derived = derive_all(words)

    check_keys = [
        "hone_owner_public_key", "hone_active_public_key",
        "hone_public_key", "hone_memo_public_key",
        "hone_hide_public_key", "hone_seek_public_key",
        "bitcoin_pubkey", "ethereum_address",
        "solana_address", "near_address", "sui_address", "aptos_address",
    ]
    mismatches = [k for k in check_keys if saved.get(k) != derived.get(k)]

    if mismatches:
        print(f"\n  MISMATCH on: {', '.join(mismatches)}")
        print("  This mnemonic does NOT match the wallet.key file.\n")
        sys.exit(1)
    else:
        print("\n  Verified — mnemonic matches wallet.key exactly.\n")

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Entry point
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

USAGE = """
  python3 hone-wallet.py generate [path]   new random wallet
  python3 hone-wallet.py derive   [path]   derive from existing mnemonic
  python3 hone-wallet.py show     [path]   display a wallet.key file
  python3 hone-wallet.py verify   [path]   check mnemonic matches wallet.key

  Default path: ~/.hone/wallet.key
  Add --private to any command to display private keys and WIF.
"""

def main():
    args   = [a for a in sys.argv[1:] if not a.startswith('--')]
    flags  = [a for a in sys.argv[1:] if a.startswith('--')]
    show_p = '--private' in flags

    if not args:
        print(USAGE)
        sys.exit(0)

    cmd  = args[0]
    path = args[1] if len(args) > 1 else DEFAULT_PATH

    dispatch = {
        "generate": cmd_generate,
        "derive":   cmd_derive,
        "show":     cmd_show,
        "verify":   cmd_verify,
    }
    if cmd not in dispatch:
        print(f"  Unknown command: {cmd}")
        print(USAGE)
        sys.exit(1)

    dispatch[cmd](path, show_p)

if __name__ == "__main__":
    main()
