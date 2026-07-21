// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/ERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";
import "@openzeppelin/contracts/utils/Pausable.sol";

/**
 * wHONE — Wrapped Bitcoin Proof of Compute
 * Shin Devlin
 *
 * ERC-20 on each supported EVM chain (ethereum, base, arbitrum, optimism, bsc, polygon).
 * Miners accumulate cross-chain credits on the HONE chain (0.1 HONE per chain per
 * HONE mined), then claim in bulk at any time using a signed proof from the HONE oracle.
 *
 * Fee model:
 *   - claimFee paid in native gas token (ETH on mainnet, etc.) — anti-spam
 *   - 50% → treasury, 50% → staker pool
 *   - Gas paid by claimer on top
 *
 * Claim proof replay prevention: each proof includes a unique nonce issued by
 * the HONE chain when recordCrossChainClaim is called. Used nonces are stored
 * permanently — one nonce, one mint.
 */
contract wHONE is ERC20, Ownable, Pausable {

    uint8 private constant _DECIMALS = 10;

    /// @notice Oracle address whose secp256k1 signatures authorize claims
    address public signingAuthority;

    /// @notice Claim fee in native gas token (wei)
    uint256 public claimFee = 0.0001 ether;

    address public treasury;
    address public stakerPool;
    uint256 public totalFeesCollected;

    /// @notice Nonces issued by the HONE chain — each usable exactly once
    mapping(bytes32 => bool) public usedNonces;

    event Claimed(address indexed claimer, string honeAccount, uint256 amount, bytes32 nonce, uint256 feePaid);
    event SigningAuthorityUpdated(address indexed previous, address indexed next);
    event ClaimFeeUpdated(uint256 oldFee, uint256 newFee);

    constructor(address _signingAuthority) ERC20("Wrapped HONE", "wHONE") Ownable(msg.sender) {
        require(_signingAuthority != address(0), "zero authority");
        signingAuthority = _signingAuthority;
        treasury   = msg.sender;
        stakerPool = msg.sender;
    }

    function decimals() public pure override returns (uint8) { return _DECIMALS; }

    /**
     * @notice Claim accumulated wHONE rewards.
     *
     * @param honeAccount  The HONE account name (e.g. "shindevlin"), packed as bytes32
     * @param amount        Raw token units (10 decimal fixed-point)
     * @param nonce         One-time nonce issued by the HONE chain for this claim
     * @param signature     ECDSA (EIP-191) signature from signingAuthority over
     *                      keccak256(chainId, msg.sender, honeAccount, amount, nonce)
     *
     * Proof message binds to: this specific chain, this wallet, this account,
     * this amount, and this unique nonce — cannot be replayed on any other chain
     * or for any other address.
     */
    function claim(
        bytes32 honeAccount,
        uint256 amount,
        bytes32 nonce,
        bytes calldata signature
    ) external payable whenNotPaused {
        require(!usedNonces[nonce],  "wHONE: nonce already used");
        require(amount > 0,          "wHONE: zero amount");
        require(msg.value >= claimFee, "wHONE: insufficient claim fee");

        bytes32 msgHash = keccak256(abi.encodePacked(
            block.chainid,
            msg.sender,
            honeAccount,
            amount,
            nonce
        ));
        bytes32 ethHash = keccak256(abi.encodePacked("\x19Ethereum Signed Message:\n32", msgHash));

        require(_recover(ethHash, signature) == signingAuthority, "wHONE: invalid signature");

        usedNonces[nonce] = true;
        _mint(msg.sender, amount);

        // Split fee 50/50 treasury / staker pool
        if (msg.value > 0) {
            uint256 half = msg.value / 2;
            (bool t,) = treasury.call{value: half}("");
            require(t, "treasury transfer failed");
            (bool s,) = stakerPool.call{value: msg.value - half}("");
            require(s, "stakerPool transfer failed");
            totalFeesCollected += msg.value;
        }

        emit Claimed(msg.sender, "", amount, nonce, msg.value);
    }

    // ── Admin ──────────────────────────────────────────────────────────────────

    function setSigningAuthority(address a) external onlyOwner {
        require(a != address(0), "zero address");
        emit SigningAuthorityUpdated(signingAuthority, a);
        signingAuthority = a;
    }

    function setClaimFee(uint256 fee) external onlyOwner {
        emit ClaimFeeUpdated(claimFee, fee);
        claimFee = fee;
    }

    function setTreasury(address a)   external onlyOwner { require(a != address(0), "zero"); treasury   = a; }
    function setStakerPool(address a) external onlyOwner { require(a != address(0), "zero"); stakerPool = a; }

    function pause()   external onlyOwner { _pause(); }
    function unpause() external onlyOwner { _unpause(); }

    // ── Internal ───────────────────────────────────────────────────────────────

    function _recover(bytes32 hash, bytes calldata sig) internal pure returns (address) {
        require(sig.length == 65, "bad sig length");
        bytes32 r; bytes32 s; uint8 v;
        assembly {
            r := calldataload(sig.offset)
            s := calldataload(add(sig.offset, 32))
            v := byte(0, calldataload(add(sig.offset, 64)))
        }
        if (v < 27) v += 27;
        require(v == 27 || v == 28, "bad v");
        return ecrecover(hash, v, r, s);
    }
}
