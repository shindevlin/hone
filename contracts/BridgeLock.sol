// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";
import "@openzeppelin/contracts/utils/ReentrancyGuard.sol";

/**
 * @title BridgeLock
 * @notice Source-side lock contract deployed on Base/Ethereum.
 *
 * Users deposit USDT, USDC, DAI, or ETH. The contract emits a
 * `BridgeLockEvent` that the HONE node relayer watches; upon
 * confirmation it mints native HONE to `honeRecipient`.
 *
 * A 4.2 M HONE supply cap is enforced via a dreams counter
 * maintained by the owner/oracle (1 HONE = 100,000,000 dreams).
 *
 * Failed bridges can be refunded by the owner after a 7-day
 * timelock using `unlockFunds`.
 *
 * Deployer / funded wallet: 0xBDe88F2B3a224B242704bD166804E0E12c75e830
 */
contract BridgeLock is Ownable, ReentrancyGuard {
    // ── Constants ────────────────────────────────────────────────────────────

    /// @dev 4.2 M HONE expressed in dreams (1 HONE = 100_000_000 dreams).
    uint256 public constant CAP_DREAMS = 4_200_000 * 100_000_000;

    /// @dev Minimum delay before owner can refund a locked deposit.
    uint256 public constant UNLOCK_TIMELOCK = 7 days;

    // ── State ─────────────────────────────────────────────────────────────────

    /// @notice Running nonce; included in every BridgeLockEvent.
    uint256 public nonce;

    /// @notice Total wHONE-equivalent dreams minted so far (set by oracle).
    uint256 public mintedDreams;

    /// @notice Accepted stablecoins (address => true).
    mapping(address => bool) public acceptedTokens;

    struct PendingDeposit {
        address user;
        address token; // address(0) for ETH
        uint256 amount;
        uint256 lockedAt;
        bool refunded;
    }

    /// @notice nonce => deposit record (populated for refund tracking).
    mapping(uint256 => PendingDeposit) public deposits;

    // ── Events ────────────────────────────────────────────────────────────────

    /**
     * @notice Emitted when a user locks funds for bridging.
     * @param user           EVM sender address.
     * @param token          ERC-20 token address, or address(0) for ETH.
     * @param amount         Raw token amount (respects token decimals).
     * @param honeRecipient HONE chain username to receive minted HONE.
     * @param nonce          Monotonically increasing per-contract nonce.
     */
    event BridgeLockEvent(
        address indexed user,
        address indexed token,
        uint256 amount,
        string honeRecipient,
        uint256 nonce
    );

    /// @notice Emitted when the owner refunds a failed bridge deposit.
    event BridgeRefunded(address indexed user, address indexed token, uint256 amount, uint256 nonce);

    /// @notice Emitted when the oracle updates the minted-dreams counter.
    event MintedDreamsUpdated(uint256 oldValue, uint256 newValue);

    // ── Constructor ───────────────────────────────────────────────────────────

    /**
     * @param initialTokens Array of ERC-20 addresses to whitelist at deploy time.
     */
    constructor(address[] memory initialTokens) Ownable(msg.sender) {
        for (uint256 i = 0; i < initialTokens.length; i++) {
            require(initialTokens[i] != address(0), "Zero token address");
            acceptedTokens[initialTokens[i]] = true;
        }
    }

    // ── User-facing ───────────────────────────────────────────────────────────

    /**
     * @notice Lock ERC-20 tokens (USDT / USDC / DAI) for bridging to HONE.
     * @param token          Whitelisted ERC-20 contract address.
     * @param amount         Amount to lock (token's native decimals).
     * @param honeRecipient HONE chain username that will receive the mint.
     */
    function lockTokens(
        address token,
        uint256 amount,
        string calldata honeRecipient
    ) external nonReentrant {
        require(acceptedTokens[token], "Token not accepted");
        require(amount > 0, "Zero amount");
        require(bytes(honeRecipient).length >= 3, "Invalid HONE recipient");
        require(mintedDreams < CAP_DREAMS, "Bridge cap reached");

        IERC20(token).transferFrom(msg.sender, address(this), amount);

        nonce++;
        deposits[nonce] = PendingDeposit({
            user: msg.sender,
            token: token,
            amount: amount,
            lockedAt: block.timestamp,
            refunded: false
        });

        emit BridgeLockEvent(msg.sender, token, amount, honeRecipient, nonce);
    }

    /**
     * @notice Lock ETH for bridging to HONE.
     * @param honeRecipient HONE chain username that will receive the mint.
     */
    function lockETH(string calldata honeRecipient) external payable nonReentrant {
        require(msg.value > 0, "Zero ETH");
        require(bytes(honeRecipient).length >= 3, "Invalid HONE recipient");
        require(mintedDreams < CAP_DREAMS, "Bridge cap reached");

        nonce++;
        deposits[nonce] = PendingDeposit({
            user: msg.sender,
            token: address(0),
            amount: msg.value,
            lockedAt: block.timestamp,
            refunded: false
        });

        emit BridgeLockEvent(msg.sender, address(0), msg.value, honeRecipient, nonce);
    }

    // ── Owner / Oracle ────────────────────────────────────────────────────────

    /**
     * @notice Called by the HONE relayer oracle to keep the on-chain
     *         dreams counter in sync. Reverts if the new value would
     *         exceed the 4.2 M HONE cap.
     * @param newMintedDreams Cumulative dreams minted so far across all bridges.
     */
    function setMintedDreams(uint256 newMintedDreams) external onlyOwner {
        require(newMintedDreams <= CAP_DREAMS, "Exceeds cap");
        emit MintedDreamsUpdated(mintedDreams, newMintedDreams);
        mintedDreams = newMintedDreams;
    }

    /**
     * @notice Refund a failed bridge deposit back to the original user.
     *         Callable only after the 7-day timelock has elapsed.
     * @param lockNonce The nonce of the deposit to refund.
     */
    function unlockFunds(uint256 lockNonce) external onlyOwner nonReentrant {
        PendingDeposit storage dep = deposits[lockNonce];
        require(dep.user != address(0), "Unknown nonce");
        require(!dep.refunded, "Already refunded");
        require(block.timestamp >= dep.lockedAt + UNLOCK_TIMELOCK, "Timelock active");

        dep.refunded = true;

        if (dep.token == address(0)) {
            (bool ok, ) = dep.user.call{value: dep.amount}("");
            require(ok, "ETH refund failed");
        } else {
            IERC20(dep.token).transfer(dep.user, dep.amount);
        }

        emit BridgeRefunded(dep.user, dep.token, dep.amount, lockNonce);
    }

    /// @notice Add or remove an accepted ERC-20 token.
    function setAcceptedToken(address token, bool accepted) external onlyOwner {
        require(token != address(0), "Zero address");
        acceptedTokens[token] = accepted;
    }

    /// @notice Remaining capacity in dreams before the cap is reached.
    function remainingCapDreams() external view returns (uint256) {
        return CAP_DREAMS - mintedDreams;
    }

    receive() external payable {}
}
