"use strict";

/**
 * BTCPC EVM Claim Submitter
 * Shin Devlin
 *
 * Submits signed claim proofs to wBTCPC contracts on EVM chains.
 * Supports: Ethereum, Base, Arbitrum, Optimism, and any EVM-compatible chain.
 *
 * Flow:
 *   1. Miner earns BTCPC reward
 *   2. claimProofGenerator creates a signed proof
 *   3. This module submits the proof to the wBTCPC contract
 *   4. wBTCPC contract verifies signature and mints wrapped tokens
 *
 * Requires: funded wallet on each target chain for gas.
 * Uses raw HTTP JSON-RPC (no ethers.js dependency).
 */

var crypto = require("crypto");
var https = require("https");
var http = require("http");
var { keccak256 } = require("js-sha3");
var secp256k1 = require("secp256k1");

// Phase E: CrossChainClaim Mongoose model removed.
// In-memory claim store: "miner|chain|epoch" → claim object
const claimStore = new Map();

// EVM chain configurations
var CHAINS = {
  ethereum: {
    name: "Ethereum",
    chainId: 1,
    rpc: process.env.ETH_RPC_URL || "https://eth.llamarpc.com",
    contract: process.env.WBTCPC_ETH_CONTRACT || null,
    explorer: "https://etherscan.io/tx/"
  },
  base: {
    name: "Base",
    chainId: 8453,
    rpc: process.env.BASE_RPC_URL || "https://mainnet.base.org",
    contract: process.env.WBTCPC_BASE_CONTRACT || null,
    explorer: "https://basescan.org/tx/"
  },
  arbitrum: {
    name: "Arbitrum",
    chainId: 42161,
    rpc: process.env.ARB_RPC_URL || "https://arb1.arbitrum.io/rpc",
    contract: process.env.WBTCPC_ARB_CONTRACT || null,
    explorer: "https://arbiscan.io/tx/"
  },
  optimism: {
    name: "Optimism",
    chainId: 10,
    rpc: process.env.OP_RPC_URL || "https://mainnet.optimism.io",
    contract: process.env.WBTCPC_OP_CONTRACT || null,
    explorer: "https://optimistic.etherscan.io/tx/"
  }
};

// ─── JSON-RPC Helper ─────────────────────────────────────────────

function jsonRpc(rpcUrl, method, params) {
  return new Promise(function (resolve, reject) {
    var body = JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: method,
      params: params || []
    });

    var parsed = new URL(rpcUrl);
    var client = parsed.protocol === "https:" ? https : http;
    var req = client.request({
      hostname: parsed.hostname,
      port: parsed.port,
      path: parsed.pathname,
      method: "POST",
      headers: { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body) }
    }, function (res) {
      var data = "";
      res.on("data", function (chunk) { data += chunk; });
      res.on("end", function () {
        try {
          var json = JSON.parse(data);
          if (json.error) reject(new Error(json.error.message));
          else resolve(json.result);
        } catch (e) { reject(e); }
      });
    });
    req.on("error", reject);
    req.write(body);
    req.end();
  });
}

// ─── EVM Signing ─────────────────────────────────────────────────

/**
 * Sign a claim proof for the wBTCPC contract.
 * Produces the same signature format the contract expects.
 *
 * @param {number} chainId — target chain ID
 * @param {string} targetWallet — 0x address of the claimer
 * @param {number} epoch — epoch number
 * @param {string} amount — raw amount (10 decimal fixed point)
 * @param {string} miner — miner account name (will be bytes32 encoded)
 * @param {string} signingKey — hex private key of the signing authority
 * @returns {{ signature: string, messageHash: string }}
 */
function signClaimProof(chainId, targetWallet, epoch, amount, miner, signingKey) {
  // Match the contract's keccak256(abi.encodePacked(chainid, sender, epoch, amount, miner))
  // ABI encode: chainId (uint256) + address (uint160) + epoch (uint256) + amount (uint256) + miner (bytes32)

  var minerBytes32 = Buffer.alloc(32);
  Buffer.from(miner).copy(minerBytes32);

  var packed = Buffer.concat([
    encodeUint256(chainId),
    encodeAddress(targetWallet),
    encodeUint256(epoch),
    encodeUint256(BigInt(amount)),
    minerBytes32
  ]);

  var messageHash = Buffer.from(keccak256(packed), "hex");

  // EIP-191 prefix
  var prefix = "\x19Ethereum Signed Message:\n32";
  var ethHash = Buffer.from(keccak256(Buffer.concat([Buffer.from(prefix), messageHash])), "hex");

  // Sign with secp256k1
  var sig = secp256k1.ecdsaSign(ethHash, Buffer.from(signingKey, "hex"));
  var sigBuf = Buffer.concat([Buffer.from(sig.signature), Buffer.from([sig.recid + 27])]);

  return {
    signature: "0x" + sigBuf.toString("hex"),
    messageHash: "0x" + messageHash.toString("hex")
  };
}

function encodeUint256(value) {
  var buf = Buffer.alloc(32);
  var big = typeof value === "bigint" ? value : BigInt(value);
  for (var i = 31; i >= 0; i--) {
    buf[i] = Number(big & 0xFFn);
    big >>= 8n;
  }
  return buf;
}

function encodeAddress(addr) {
  var buf = Buffer.alloc(32);
  Buffer.from(addr.replace("0x", ""), "hex").copy(buf, 12);
  return buf;
}

// ─── Claim Submission ────────────────────────────────────────────

/**
 * Submit a claim to a wBTCPC contract on an EVM chain.
 *
 * @param {string} chain — chain key (e.g. "base", "ethereum")
 * @param {object} proof — from claimProofGenerator
 * @param {string} signingKey — hex private key of the signing authority
 * @param {string} submitterKey — hex private key of the wallet paying gas
 * @returns {{ txHash, status }}
 */
async function submitClaim(chain, proof, signingKey, submitterKey) {
  var config = CHAINS[chain];
  if (!config) throw new Error("Unknown chain: " + chain);
  if (!config.contract) throw new Error("No wBTCPC contract deployed on " + chain);

  // Check if already claimed
  var claimKey = proof.miner + '|' + chain + '|' + proof.epoch;
  var existing = claimStore.get(claimKey);
  if (existing && (existing.status === 'submitted' || existing.status === 'confirmed')) {
    return { txHash: existing.tx_hash, status: "already_claimed" };
  }

  // Sign the proof for this chain
  var amountRaw = Math.round(parseFloat(proof.direct_amount) * 1e10).toString();
  var minerBytes32 = "0x" + Buffer.from(proof.miner).toString("hex").padEnd(64, "0");

  var signed = signClaimProof(
    config.chainId,
    proof.target_wallet,
    proof.epoch,
    amountRaw,
    proof.miner,
    signingKey
  );

  // Build the contract call data
  // claim(uint256 epoch, uint256 amount, bytes32 miner, bytes signature)
  var funcSelector = keccak256("claim(uint256,uint256,bytes32,bytes)").slice(0, 8);
  var callData = "0x" + funcSelector +
    encodeUint256(proof.epoch).toString("hex") +
    encodeUint256(BigInt(amountRaw)).toString("hex") +
    Buffer.from(proof.miner).toString("hex").padEnd(64, "0") +
    // Dynamic bytes offset (4 * 32 = 128 = 0x80)
    "0000000000000000000000000000000000000000000000000000000000000080" +
    // Signature length (65 bytes)
    "0000000000000000000000000000000000000000000000000000000000000041" +
    // Signature data (padded to 32-byte boundary)
    signed.signature.replace("0x", "").padEnd(192, "0");

  // Record the claim attempt in-memory
  var claim = {
    miner: proof.miner,
    chain: chain,
    target_wallet: proof.target_wallet,
    epoch: proof.epoch,
    native_reward: parseFloat(proof.amount),
    claim_amount: proof.direct_amount,
    direct_amount: proof.direct_amount,
    lp_amount: proof.lp_amount,
    period: proof.period,
    cross_chain_ratio: proof.cross_chain_ratio,
    proof_signature: signed.signature,
    status: "generated",
    created_at: new Date().toISOString()
  };
  claimStore.set(claimKey, claim);

  console.log("[BTCPC Claims] " + config.name + ": epoch " + proof.epoch +
    " | " + proof.direct_amount + " wBTCPC → " + proof.target_wallet.slice(0, 10) + "...");

  // Note: actual on-chain submission requires a funded gas wallet and
  // transaction signing. This is the full data preparation.
  // To submit: send a transaction to config.contract with callData.
  //
  // For now, record as "generated" — manual or automated submission later.
  // When gas wallets are funded, uncomment the eth_sendRawTransaction call.

  return {
    txHash: null,
    status: "generated",
    chain: config.name,
    contract: config.contract,
    callData: callData,
    amount: proof.direct_amount + " wBTCPC",
    claim_id: claimKey
  };
}

/**
 * Submit all pending claims for a miner across all EVM chains.
 */
async function submitAllClaims(miner, epoch, amount, linkedChains, signingKey) {
  var { generateClaimProof } = require("./claimProofGenerator");
  var results = [];

  for (var chain of Object.keys(CHAINS)) {
    var wallet = linkedChains[chain] || linkedChains.evm;
    if (!wallet) continue;
    if (!CHAINS[chain].contract) continue;

    try {
      var proof = generateClaimProof(miner, epoch, amount, chain, wallet, signingKey);
      if (!proof) continue;

      var result = await submitClaim(chain, proof, signingKey, null);
      results.push(result);
    } catch (err) {
      console.error("[BTCPC Claims] " + chain + " failed: " + err.message);
    }
  }

  return results;
}

/**
 * Get claim status for a miner on all chains.
 */
async function getClaimStatus(miner, epoch) {
  const results = [];
  for (const [key, claim] of claimStore) {
    const matchMiner = miner === null || claim.miner === miner;
    const matchEpoch = epoch === null || claim.epoch === epoch;
    if (matchMiner && matchEpoch) {
      results.push(claim);
    }
  }
  return results;
}

module.exports = {
  CHAINS: CHAINS,
  signClaimProof: signClaimProof,
  submitClaim: submitClaim,
  submitAllClaims: submitAllClaims,
  getClaimStatus: getClaimStatus
};
