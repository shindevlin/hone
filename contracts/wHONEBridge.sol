// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

/**
 * wHONE Bridge - Lock-and-Release bridge between EVM chains and native HONE
 * Shin Devlin
 *
 * NO BURNING. Locked tokens stay in the pool for reverse direction.
 *
 * EVM -> HONE native:
 *   1. User calls lockForBridge(amount, honeUsername)
 *   2. wHONE locked in this contract
 *   3. HONE chain watcher sees the Lock event
 *   4. Native HONE credited to user's account
 *
 * HONE native -> EVM:
 *   1. User requests bridge on HONE chain (locks native HONE)
 *   2. Bridge authority signs a release proof
 *   3. User calls releaseFromBridge(amount, proof)
 *   4. wHONE released from this contract's pool to user
 *
 * Liquidity stays on both sides. Nothing is destroyed.
 */
contract wHONEBridge is Ownable {
    IERC20 public whone;

    /// @notice Bridge authority that signs release proofs
    address public bridgeAuthority;

    /// @notice Track processed release nonces (prevent double-release)
    mapping(uint256 => bool) public processedNonces;

    /// @notice Running nonce for locks
    uint256 public lockNonce;

    /// @notice Permanent binding: HONE username <-> EVM address
    /// Set once during first bridge interaction, immutable after
    mapping(string => address) public usernameToAddress;
    mapping(address => string) public addressToUsername;

    event Locked(
        address indexed sender,
        uint256 amount,
        string honeUsername,
        uint256 nonce
    );

    event AccountBound(
        address indexed evmAddress,
        string honeUsername
    );

    event Released(
        address indexed recipient,
        uint256 amount,
        uint256 nonce
    );

    constructor(address _whone, address _bridgeAuthority) Ownable(msg.sender) {
        require(_whone != address(0), "Zero wHONE address");
        require(_bridgeAuthority != address(0), "Zero authority");
        whone = IERC20(_whone);
        bridgeAuthority = _bridgeAuthority;
    }

    /**
     * @notice Lock wHONE to bridge to native HONE chain.
     * Tokens stay in this contract (not burned). HONE chain credits the user.
     * @param amount Amount of wHONE to lock (10 decimals)
     * @param honeUsername The user's HONE chain username to credit
     */
    /**
     * @notice Register the permanent binding between a HONE username and EVM address.
     * Called by the bridge authority after verifying the account exists on HONE chain.
     * Once bound, this address is the ONLY address that can bridge for this username.
     */
    function bindAccount(string calldata honeUsername, address evmAddress) external {
        require(msg.sender == bridgeAuthority, "Only bridge authority can bind");
        require(bytes(honeUsername).length >= 3, "Invalid username");
        require(evmAddress != address(0), "Zero address");
        require(usernameToAddress[honeUsername] == address(0), "Username already bound");
        require(bytes(addressToUsername[evmAddress]).length == 0, "Address already bound");

        usernameToAddress[honeUsername] = evmAddress;
        addressToUsername[evmAddress] = honeUsername;

        emit AccountBound(evmAddress, honeUsername);
    }

    function lockForBridge(uint256 amount, string calldata honeUsername) external {
        require(amount > 0, "Zero amount");
        require(bytes(honeUsername).length > 0, "Empty username");

        // Username must be bound to an address
        address boundAddress = usernameToAddress[honeUsername];
        require(boundAddress != address(0), "Username not registered - create account first via /create");

        // Only the bound address can bridge for this username
        require(msg.sender == boundAddress, "Wallet not bound to this username");

        require(
            whone.transferFrom(msg.sender, address(this), amount),
            "Transfer failed"
        );

        lockNonce++;
        emit Locked(msg.sender, amount, honeUsername, lockNonce);
    }

    /**
     * @notice Release wHONE from the pool (HONE native -> EVM direction).
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
        require(whone.balanceOf(address(this)) >= amount, "Insufficient bridge pool");
        require(bytes(addressToUsername[msg.sender]).length > 0, "Wallet not bound to any HONE account");

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

        require(whone.transfer(msg.sender, amount), "Release transfer failed");

        emit Released(msg.sender, amount, nonce);
    }

    /**
     * @notice Available wHONE in the bridge pool.
     */
    function poolBalance() external view returns (uint256) {
        return whone.balanceOf(address(this));
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
