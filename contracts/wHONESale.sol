// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

/**
 * wHONE Sale — Buy wHONE with stablecoins from the treasury pool
 * Shin Devlin
 *
 * NO MINTING. wHONE can only be earned through mining on the HONE chain.
 * This contract holds a pool of wHONE deposited by the treasury (earned tokens).
 * Users buy from this pool with USDC/USDT/DAI at a governance-set price.
 *
 * Flow:
 *   1. Treasury deposits mined wHONE into this contract
 *   2. User approves this contract to spend their USDC
 *   3. User calls buy(stablecoin, amount)
 *   4. Contract transfers USDC from user to treasury
 *   5. Contract transfers wHONE from pool to user
 *
 * When the pool is empty, nobody can buy. More wHONE must be mined and deposited.
 */
contract wHONESale is Ownable {
    /// @notice The wHONE token contract
    IERC20 public whone;

    /// @notice Treasury address — receives stablecoin payments
    address public treasury;

    /// @notice Price per wHONE in stablecoin smallest units (6 decimals for USDC/USDT)
    /// e.g. 100000 = $0.10, 1000000 = $1.00, 10000 = $0.01
    uint256 public pricePerToken;

    /// @notice Accepted stablecoins
    mapping(address => bool) public acceptedStablecoins;

    /// @notice Stablecoin decimals (6 for USDC/USDT, 18 for DAI)
    mapping(address => uint8) public stablecoinDecimals;

    /// @notice Whether the sale is active
    bool public saleActive;

    /// @notice Total wHONE sold through this contract
    uint256 public totalSold;

    event TokensPurchased(
        address indexed buyer,
        address indexed stablecoin,
        uint256 stablecoinAmount,
        uint256 whoneAmount
    );

    event PoolDeposited(address indexed depositor, uint256 amount);
    event PoolWithdrawn(address indexed to, uint256 amount);
    event PriceUpdated(uint256 oldPrice, uint256 newPrice);
    event StablecoinAdded(address indexed token, uint8 decimals);
    event SaleToggled(bool active);

    constructor(
        address _whone,
        address _treasury,
        uint256 _pricePerToken
    ) Ownable(msg.sender) {
        require(_whone != address(0), "wHONE: zero address");
        require(_treasury != address(0), "Treasury: zero address");
        require(_pricePerToken > 0, "Price must be > 0");

        whone = IERC20(_whone);
        treasury = _treasury;
        pricePerToken = _pricePerToken;
        saleActive = true;
    }

    /**
     * @notice Available wHONE in the sale pool.
     */
    function available() public view returns (uint256) {
        return whone.balanceOf(address(this));
    }

    /**
     * @notice Buy wHONE with a stablecoin. Tokens come from the pool, not minted.
     * @param stablecoin The stablecoin to pay with (must be accepted)
     * @param stablecoinAmount Amount of stablecoin to spend
     */
    function buy(address stablecoin, uint256 stablecoinAmount) external {
        require(saleActive, "Sale not active");
        require(acceptedStablecoins[stablecoin], "Stablecoin not accepted");
        require(stablecoinAmount > 0, "Amount must be > 0");

        // Calculate wHONE amount
        uint8 scDecimals = stablecoinDecimals[stablecoin];
        uint256 normalizedAmount;
        if (scDecimals > 6) {
            normalizedAmount = stablecoinAmount / (10 ** (scDecimals - 6));
        } else if (scDecimals < 6) {
            normalizedAmount = stablecoinAmount * (10 ** (6 - scDecimals));
        } else {
            normalizedAmount = stablecoinAmount;
        }

        // wHONE has 10 decimals. price is in 6-decimal units per 1 wHONE
        uint256 whoneAmount = (normalizedAmount * (10 ** 10)) / pricePerToken;
        require(whoneAmount > 0, "Amount too small");

        // Check pool has enough
        require(whone.balanceOf(address(this)) >= whoneAmount, "Insufficient pool - more wHONE must be mined");

        // Transfer stablecoin from buyer to treasury
        require(
            IERC20(stablecoin).transferFrom(msg.sender, treasury, stablecoinAmount),
            "Stablecoin transfer failed"
        );

        // Transfer wHONE from pool to buyer
        require(
            whone.transfer(msg.sender, whoneAmount),
            "wHONE transfer failed"
        );

        totalSold += whoneAmount;
        emit TokensPurchased(msg.sender, stablecoin, stablecoinAmount, whoneAmount);
    }

    /**
     * @notice Calculate how much wHONE you get for a stablecoin amount.
     */
    function quote(address stablecoin, uint256 stablecoinAmount) external view returns (uint256) {
        if (!acceptedStablecoins[stablecoin]) return 0;
        uint8 scDecimals = stablecoinDecimals[stablecoin];
        uint256 normalizedAmount;
        if (scDecimals > 6) {
            normalizedAmount = stablecoinAmount / (10 ** (scDecimals - 6));
        } else if (scDecimals < 6) {
            normalizedAmount = stablecoinAmount * (10 ** (6 - scDecimals));
        } else {
            normalizedAmount = stablecoinAmount;
        }
        return (normalizedAmount * (10 ** 10)) / pricePerToken;
    }

    // ─── Pool Management ────────────────────────────────────────────

    /**
     * @notice Deposit wHONE into the sale pool. Anyone can deposit (typically treasury).
     * Caller must approve this contract first.
     */
    function deposit(uint256 amount) external {
        require(
            whone.transferFrom(msg.sender, address(this), amount),
            "Deposit transfer failed"
        );
        emit PoolDeposited(msg.sender, amount);
    }

    /**
     * @notice Withdraw unsold wHONE from the pool. Owner only.
     */
    function withdraw(uint256 amount) external onlyOwner {
        require(whone.balanceOf(address(this)) >= amount, "Insufficient pool");
        require(whone.transfer(treasury, amount), "Withdraw transfer failed");
        emit PoolWithdrawn(treasury, amount);
    }

    // ─── Admin ──────────────────────────────────────────────────────

    function setPrice(uint256 newPrice) external onlyOwner {
        require(newPrice > 0, "Price must be > 0");
        emit PriceUpdated(pricePerToken, newPrice);
        pricePerToken = newPrice;
    }

    function addStablecoin(address token, uint8 decimals) external onlyOwner {
        acceptedStablecoins[token] = true;
        stablecoinDecimals[token] = decimals;
        emit StablecoinAdded(token, decimals);
    }

    function removeStablecoin(address token) external onlyOwner {
        acceptedStablecoins[token] = false;
    }

    function toggleSale(bool active) external onlyOwner {
        saleActive = active;
        emit SaleToggled(active);
    }

    function setTreasury(address newTreasury) external onlyOwner {
        require(newTreasury != address(0), "Zero address");
        treasury = newTreasury;
    }
}
