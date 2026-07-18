"use strict";

/**
 * HONE Sparse Merkle Tree
 * Shin Devlin
 *
 * Fixed-depth (32-level) Sparse Merkle Tree for account state commitments.
 * Provides O(32 hashes) updates, constant-size root (32 bytes), and
 * ~1KB inclusion proofs for any account.
 *
 * Leaf index = first 4 bytes of SHA256(username) → uint32 position
 * Leaf value = SHA256(canonical JSON of account state)
 * Empty leaves use precomputed default hashes per level.
 */

var crypto = require("crypto");

var TREE_DEPTH = 32;

// Precompute default hashes for empty subtrees at each level.
// Level 0 (leaf) default = SHA256("")
// Level N default = SHA256(level[N-1] + level[N-1])
var DEFAULT_HASHES = new Array(TREE_DEPTH + 1);
DEFAULT_HASHES[0] = crypto.createHash("sha256").update("").digest("hex");
for (var i = 1; i <= TREE_DEPTH; i++) {
  DEFAULT_HASHES[i] = crypto.createHash("sha256")
    .update(DEFAULT_HASHES[i - 1] + DEFAULT_HASHES[i - 1])
    .digest("hex");
}

/**
 * SparseMerkleTree — fixed-depth tree with lazy population.
 * Only stores nodes along populated paths.
 */
function SparseMerkleTree() {
  // nodes: Map<string, string> where key = "depth:index" and value = hash
  // Only stores non-default nodes.
  this.nodes = new Map();
  // Track populated leaves: username → { index, stateHash }
  this.leaves = new Map();
}

/**
 * Compute the leaf index for a username.
 * Returns a uint32 derived from the first 4 bytes of SHA256(username).
 */
SparseMerkleTree.usernameToIndex = function (username) {
  var hash = crypto.createHash("sha256").update(username).digest();
  return hash.readUInt32BE(0);
};

/**
 * Compute the canonical hash of an account state object.
 * State must have deterministic serialization: sorted keys, no undefined values.
 */
SparseMerkleTree.hashState = function (state) {
  var canonical = {
    balance: state.balance || 0,
    delegated: state.delegated || 0,
    nonce: state.nonce || 0,
    staked: state.staked || 0
  };
  return crypto.createHash("sha256")
    .update(JSON.stringify(canonical))
    .digest("hex");
};

/**
 * Get a node hash at a specific position.
 * Returns the default hash for that level if the node is not populated.
 */
SparseMerkleTree.prototype._getNode = function (depth, index) {
  var key = depth + ":" + index;
  return this.nodes.get(key) || DEFAULT_HASHES[depth];
};

/**
 * Set a node hash at a specific position.
 * If the value equals the default hash, remove it (keep storage sparse).
 */
SparseMerkleTree.prototype._setNode = function (depth, index, hash) {
  var key = depth + ":" + index;
  if (hash === DEFAULT_HASHES[depth]) {
    this.nodes.delete(key);
  } else {
    this.nodes.set(key, hash);
  }
};

/**
 * Update the tree with a new account state.
 * Rehashes one path from leaf to root (32 hashes).
 */
SparseMerkleTree.prototype.update = function (username, accountState) {
  var leafIndex = SparseMerkleTree.usernameToIndex(username);
  var stateHash = SparseMerkleTree.hashState(accountState);

  // Set the leaf (depth 0)
  this._setNode(0, leafIndex, stateHash);
  this.leaves.set(username, { index: leafIndex, stateHash: stateHash });

  // Rehash up the tree
  var idx = leafIndex;
  for (var d = 0; d < TREE_DEPTH; d++) {
    var parentIndex = idx >>> 1;
    var leftChild, rightChild;

    if (idx % 2 === 0) {
      leftChild = this._getNode(d, idx);
      rightChild = this._getNode(d, idx + 1);
    } else {
      leftChild = this._getNode(d, idx - 1);
      rightChild = this._getNode(d, idx);
    }

    var parentHash = crypto.createHash("sha256")
      .update(leftChild + rightChild)
      .digest("hex");

    this._setNode(d + 1, parentIndex, parentHash);
    idx = parentIndex;
  }
};

/**
 * Remove an account from the tree (set leaf to default).
 */
SparseMerkleTree.prototype.remove = function (username) {
  this.update(username, { balance: 0, staked: 0, delegated: 0, nonce: 0 });
  this.leaves.delete(username);
  // Clean up: set the leaf node to default explicitly
  var leafIndex = SparseMerkleTree.usernameToIndex(username);
  this._setNode(0, leafIndex, DEFAULT_HASHES[0]);
  // Rehash up
  var idx = leafIndex;
  for (var d = 0; d < TREE_DEPTH; d++) {
    var parentIndex = idx >>> 1;
    var leftChild = this._getNode(d, idx % 2 === 0 ? idx : idx - 1);
    var rightChild = this._getNode(d, idx % 2 === 0 ? idx + 1 : idx);
    var parentHash = crypto.createHash("sha256")
      .update(leftChild + rightChild)
      .digest("hex");
    this._setNode(d + 1, parentIndex, parentHash);
    idx = parentIndex;
  }
};

/**
 * Get the root hash of the tree.
 * Returns a 64-character hex string.
 */
SparseMerkleTree.prototype.getRoot = function () {
  return this._getNode(TREE_DEPTH, 0);
};

/**
 * Generate a Merkle proof for a username.
 * Returns an array of 32 sibling hashes (one per level, bottom to top).
 */
SparseMerkleTree.prototype.getProof = function (username) {
  var leafIndex = SparseMerkleTree.usernameToIndex(username);
  var siblings = [];
  var idx = leafIndex;

  for (var d = 0; d < TREE_DEPTH; d++) {
    if (idx % 2 === 0) {
      siblings.push(this._getNode(d, idx + 1));
    } else {
      siblings.push(this._getNode(d, idx - 1));
    }
    idx = idx >>> 1;
  }

  return siblings;
};

/**
 * Verify a Merkle proof for a username and account state against a root.
 * Static method — does not require a tree instance.
 *
 * @param {string} root — expected root hash (64-char hex)
 * @param {string} username — account username
 * @param {object} accountState — { balance, staked, delegated, nonce }
 * @param {string[]} proof — array of 32 sibling hashes
 * @returns {boolean}
 */
SparseMerkleTree.verifyProof = function (root, username, accountState, proof) {
  if (!proof || proof.length !== TREE_DEPTH) return false;

  var leafIndex = SparseMerkleTree.usernameToIndex(username);
  var currentHash = SparseMerkleTree.hashState(accountState);

  var idx = leafIndex;
  for (var d = 0; d < TREE_DEPTH; d++) {
    var left, right;
    if (idx % 2 === 0) {
      left = currentHash;
      right = proof[d];
    } else {
      left = proof[d];
      right = currentHash;
    }
    currentHash = crypto.createHash("sha256")
      .update(left + right)
      .digest("hex");
    idx = idx >>> 1;
  }

  return currentHash === root;
};

/**
 * Serialize the tree for storage in a finality block.
 * Returns a JSON-serializable object containing only populated nodes.
 */
SparseMerkleTree.prototype.serialize = function () {
  var nodesObj = {};
  this.nodes.forEach(function (value, key) {
    nodesObj[key] = value;
  });

  var leavesObj = {};
  this.leaves.forEach(function (value, key) {
    leavesObj[key] = value;
  });

  return {
    nodes: nodesObj,
    leaves: leavesObj,
    root: this.getRoot()
  };
};

/**
 * Deserialize a tree from a finality block snapshot.
 * Returns a new SparseMerkleTree instance.
 */
SparseMerkleTree.deserialize = function (data) {
  var tree = new SparseMerkleTree();

  if (data.nodes) {
    var keys = Object.keys(data.nodes);
    for (var i = 0; i < keys.length; i++) {
      tree.nodes.set(keys[i], data.nodes[keys[i]]);
    }
  }

  if (data.leaves) {
    var leafKeys = Object.keys(data.leaves);
    for (var j = 0; j < leafKeys.length; j++) {
      tree.leaves.set(leafKeys[j], data.leaves[leafKeys[j]]);
    }
  }

  return tree;
};

/**
 * Get the number of populated leaves.
 */
SparseMerkleTree.prototype.getLeafCount = function () {
  return this.leaves.size;
};

/**
 * Get all populated leaf usernames.
 */
SparseMerkleTree.prototype.getLeafUsernames = function () {
  return Array.from(this.leaves.keys());
};

// Export constants for testing
SparseMerkleTree.TREE_DEPTH = TREE_DEPTH;
SparseMerkleTree.DEFAULT_HASHES = DEFAULT_HASHES;

module.exports = SparseMerkleTree;
