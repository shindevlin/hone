// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";
import "@openzeppelin/contracts/utils/ReentrancyGuard.sol";

/**
 * @title BridgeReserve
 * @notice Destination-side reserve contract deployed on Base/Ethereum.
 *
 * Holds USDT, USDC, DAI, and ETH that back outbound BTCPC → EVM
 * bridge payouts (BridgeUnwrap flow).
 *
 * Flow:
 *   1. User submits a BridgeUnwrap entry on the BTCPC chain.
 *   2. BTCPC node burns the user's wBTCPC and queues an UnlockRequest.
 *   3. The trusted relayer calls `unlock()` here with the BTCPC tx hash.
 *   4. Funds are released to `recipient`; the tx hash is marked spent.
 *
 * Replay protection: each `btcpcTxHash` can only be used once.
 *
 * Deployer / funded wallet: 0xBDe88F2B3a224B242704bD166804E0E12c75e830
 */
contract BridgeReserve is Ownable, ReentrancyGuard {
    // ── State ─────────────────────────────────────────────────────────────────

    /// @notice Trusted relayer address authorised to call `unlock`.
    address public relayer;

    /// @notice btcpcTxHash => already processed (replay protection).
    mapping(bytes32 => bool) public processedTxHashes;

    // ── Events ────────────────────────────────────────────────────────────────

    /**
     * @notice Emitted when the reserve pays out an unwrap request.
     * @param recipient    EVM address that receives the funds.
     * @param token        ERC-20 address, or address(0) for ETH.
     * @param amount       Raw amount transferred.
     * @param btcpcTxHash  BTCPC chain transaction hash that authorised this payout.
     */
    event BridgeUnlockEvent(
        address indexed recipient,
        address indexed token,
        uint256 amount,
        bytes32 indexed btcpcTxHash
    );

    /// @notice Emitted when the owner funds the reserve with ERC-20 tokens.
    event ReserveFunded(address indexed token, uint256 amount);

    /// @notice Emitted when the owner funds the reserve with ETH.
    event ReserveFundedETH(uint256 amount);

    /// @notice Emitted when the relayer address is updated.
    event RelayerUpdated(address indexed oldRelayer, address indexed newRelayer);

    // ── Constructor ───────────────────────────────────────────────────────────

    /**
     * @param _relayer Initial trusted relayer address.
     */
    constructor(address _relayer) Ownable(msg.sender) {
        require(_relayer != address(0), "Zero relayer address");
        relayer = _relayer;
    }

    // ── Modifiers ─────────────────────────────────────────────────────────────

    modifier onlyRelayer() {
        require(msg.sender == relayer, "Caller is not the relayer");
        _;
    }

    // ── Relayer-facing ────────────────────────────────────────────────────────

    /**
     * @notice Release funds to a recipient after the BTCPC node confirms a
     *         BridgeUnwrap entry. Each `btcpcTxHash` is single-use.
     * @param recipient    EVM address to pay out.
     * @param token        ERC-20 token to send, or address(0) for ETH.
     * @param amount       Amount to transfer (token's native decimals for ERC-20; wei for ETH).
     * @param btcpcTxHash  Sha-256 hash of the BTCPC chain BridgeUnwrap transaction.
     */
    function unlock(
        address recipient,
        address token,
        uint256 amount,
        bytes32 btcpcTxHash
    ) external nonReentrant onlyRelayer {
        require(recipient != address(0), "Zero recipient");
        require(amount > 0, "Zero amount");
        require(!processedTxHashes[btcpcTxHash], "TX hash already processed");

        processedTxHashes[btcpcTxHash] = true;

        if (token == address(0)) {
            require(address(this).balance >= amount, "Insufficient ETH reserve");
            (bool ok, ) = recipient.call{value: amount}("");
            require(ok, "ETH transfer failed");
        } else {
            require(
                IERC20(token).balanceOf(address(this)) >= amount,
                "Insufficient token reserve"
            );
            IERC20(token).transfer(recipient, amount);
        }

        emit BridgeUnlockEvent(recipient, token, amount, btcpcTxHash);
    }

    // ── Owner / Funding ───────────────────────────────────────────────────────

    /**
     * @notice Fund the reserve with an ERC-20 token (USDT / USDC / DAI).
     *         Owner must approve this contract before calling.
     * @param token  ERC-20 token address.
     * @param amount Amount to deposit.
     */
    function fundToken(address token, uint256 amount) external onlyOwner nonReentrant {
        require(token != address(0), "Zero token address");
        require(amount > 0, "Zero amount");
        IERC20(token).transferFrom(msg.sender, address(this), amount);
        emit ReserveFunded(token, amount);
    }

    /**
     * @notice Fund the reserve with ETH. Send ETH with this call.
     */
    function fundETH() external payable onlyOwner {
        require(msg.value > 0, "Zero ETH");
        emit ReserveFundedETH(msg.value);
    }

    /**
     * @notice Update the trusted relayer (key rotation / multisig change).
     * @param newRelayer New relayer address.
     */
    function setRelayer(address newRelayer) external onlyOwner {
        require(newRelayer != address(0), "Zero address");
        emit RelayerUpdated(relayer, newRelayer);
        relayer = newRelayer;
    }

    /**
     * @notice Emergency withdrawal of ERC-20 tokens to owner.
     * @param token  Token address.
     * @param amount Amount to withdraw.
     */
    function emergencyWithdrawToken(address token, uint256 amount) external onlyOwner nonReentrant {
        require(token != address(0), "Zero address");
        IERC20(token).transfer(owner(), amount);
    }

    /**
     * @notice Emergency withdrawal of ETH to owner.
     * @param amount Wei to withdraw.
     */
    function emergencyWithdrawETH(uint256 amount) external onlyOwner nonReentrant {
        require(address(this).balance >= amount, "Insufficient balance");
        (bool ok, ) = owner().call{value: amount}("");
        require(ok, "ETH withdraw failed");
    }

    /// @notice Reserve ETH balance (wei).
    function ethBalance() external view returns (uint256) {
        return address(this).balance;
    }

    /// @notice Reserve balance for any ERC-20 token.
    function tokenBalance(address token) external view returns (uint256) {
        return IERC20(token).balanceOf(address(this));
    }

    receive() external payable {}
}
