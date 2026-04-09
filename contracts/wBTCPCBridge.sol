// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

/**
 * wBTCPC Bridge - Lock-and-Release bridge between EVM chains and native BTCPC
 * Shin Devlin
 *
 * NO BURNING. Locked tokens stay in the pool for reverse direction.
 *
 * EVM -> BTCPC native:
 *   1. User calls lockForBridge(amount, btcpcUsername)
 *   2. wBTCPC locked in this contract
 *   3. BTCPC chain watcher sees the Lock event
 *   4. Native BTCPC credited to user's account
 *
 * BTCPC native -> EVM:
 *   1. User requests bridge on BTCPC chain (locks native BTCPC)
 *   2. Bridge authority signs a release proof
 *   3. User calls releaseFromBridge(amount, proof)
 *   4. wBTCPC released from this contract's pool to user
 *
 * Liquidity stays on both sides. Nothing is destroyed.
 */
contract wBTCPCBridge is Ownable {
    IERC20 public wbtcpc;

    /// @notice Bridge authority that signs release proofs
    address public bridgeAuthority;

    /// @notice Track processed release nonces (prevent double-release)
    mapping(uint256 => bool) public processedNonces;

    /// @notice Running nonce for locks
    uint256 public lockNonce;

    event Locked(
        address indexed sender,
        uint256 amount,
        string btcpcUsername,
        uint256 nonce
    );

    event Released(
        address indexed recipient,
        uint256 amount,
        uint256 nonce
    );

    constructor(address _wbtcpc, address _bridgeAuthority) Ownable(msg.sender) {
        require(_wbtcpc != address(0), "Zero wBTCPC address");
        require(_bridgeAuthority != address(0), "Zero authority");
        wbtcpc = IERC20(_wbtcpc);
        bridgeAuthority = _bridgeAuthority;
    }

    /**
     * @notice Lock wBTCPC to bridge to native BTCPC chain.
     * Tokens stay in this contract (not burned). BTCPC chain credits the user.
     * @param amount Amount of wBTCPC to lock (10 decimals)
     * @param btcpcUsername The user's BTCPC chain username to credit
     */
    function lockForBridge(uint256 amount, string calldata btcpcUsername) external {
        require(amount > 0, "Zero amount");
        require(bytes(btcpcUsername).length > 0, "Empty username");

        require(
            wbtcpc.transferFrom(msg.sender, address(this), amount),
            "Transfer failed"
        );

        lockNonce++;
        emit Locked(msg.sender, amount, btcpcUsername, lockNonce);
    }

    /**
     * @notice Release wBTCPC from the pool (BTCPC native -> EVM direction).
     * Requires a signed proof from the bridge authority.
     * @param amount Amount to release
     * @param nonce Unique nonce for this release (prevents replay)
     * @param signature Bridge authority signature over (chainId, recipient, amount, nonce)
     */
    function releaseFromBridge(
        uint256 amount,
        uint256 nonce,
        bytes calldata signature
    ) external {
        require(amount > 0, "Zero amount");
        require(!processedNonces[nonce], "Nonce already processed");
        require(wbtcpc.balanceOf(address(this)) >= amount, "Insufficient bridge pool");

        // Verify signature from bridge authority
        bytes32 messageHash = keccak256(
            abi.encodePacked(block.chainid, msg.sender, amount, nonce)
        );
        bytes32 ethSignedHash = keccak256(
            abi.encodePacked("\x19Ethereum Signed Message:\n32", messageHash)
        );

        address recovered = _recoverSigner(ethSignedHash, signature);
        require(recovered == bridgeAuthority, "Invalid bridge signature");

        processedNonces[nonce] = true;

        require(wbtcpc.transfer(msg.sender, amount), "Release transfer failed");

        emit Released(msg.sender, amount, nonce);
    }

    /**
     * @notice Available wBTCPC in the bridge pool.
     */
    function poolBalance() external view returns (uint256) {
        return wbtcpc.balanceOf(address(this));
    }

    /**
     * @notice Update bridge authority (key rotation).
     */
    function updateBridgeAuthority(address newAuthority) external onlyOwner {
        require(newAuthority != address(0), "Zero address");
        bridgeAuthority = newAuthority;
    }

    function _recoverSigner(bytes32 hash, bytes memory sig) internal pure returns (address) {
        require(sig.length == 65, "Invalid sig length");
        bytes32 r; bytes32 s; uint8 v;
        assembly {
            r := mload(add(sig, 32))
            s := mload(add(sig, 64))
            v := byte(0, mload(add(sig, 96)))
        }
        if (v < 27) v += 27;
        require(v == 27 || v == 28, "Invalid v");
        return ecrecover(hash, v, r, s);
    }
}
