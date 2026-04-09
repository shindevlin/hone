// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

/**
 * wBTCPC Bonding Curve Sale — Price increases with demand
 * Shin Devlin
 *
 * NO MINTING. Sells from a pool of earned wBTCPC.
 * Price is determined by a linear bonding curve:
 *   price = basePrice + (totalSold * priceIncrement)
 *
 * More bought = higher price. Pool empty = cannot buy.
 * Owner can adjust basePrice and priceIncrement (governance later).
 */
contract wBTCPCBondingCurve is Ownable {
    IERC20 public wbtcpc;
    address public treasury;

    /// @notice Base price in stablecoin units (6 decimals). e.g. 50000 = $0.05
    uint256 public basePrice;

    /// @notice Price increment per wBTCPC sold (6 decimals). e.g. 1 = $0.000001 per token sold
    uint256 public priceIncrement;

    /// @notice Total wBTCPC sold through this curve (10 decimals)
    uint256 public totalSold;

    /// @notice Accepted stablecoins
    mapping(address => bool) public acceptedStablecoins;
    mapping(address => uint8) public stablecoinDecimals;

    bool public saleActive;

    event TokensPurchased(address indexed buyer, address indexed stablecoin, uint256 stablecoinAmount, uint256 wbtcpcAmount, uint256 priceAtPurchase);
    event PriceParametersUpdated(uint256 basePrice, uint256 priceIncrement);

    constructor(address _wbtcpc, address _treasury, uint256 _basePrice, uint256 _priceIncrement) Ownable(msg.sender) {
        require(_wbtcpc != address(0) && _treasury != address(0), "Zero address");
        wbtcpc = IERC20(_wbtcpc);
        treasury = _treasury;
        basePrice = _basePrice;
        priceIncrement = _priceIncrement;
        saleActive = true;
    }

    /// @notice Current price per wBTCPC in 6-decimal stablecoin units
    function currentPrice() public view returns (uint256) {
        // totalSold is in 10 decimals. Convert to whole tokens for price calc.
        uint256 tokensSoldWhole = totalSold / (10 ** 10);
        return basePrice + (tokensSoldWhole * priceIncrement);
    }

    /// @notice Available wBTCPC in pool
    function available() public view returns (uint256) {
        return wbtcpc.balanceOf(address(this));
    }

    /// @notice Get a quote: how much wBTCPC for a stablecoin amount
    function quote(address stablecoin, uint256 stablecoinAmount) external view returns (uint256 wbtcpcAmount, uint256 avgPrice) {
        if (!acceptedStablecoins[stablecoin]) return (0, 0);
        uint256 normalized = _normalizeStablecoin(stablecoin, stablecoinAmount);
        uint256 price = currentPrice();
        if (price == 0) return (0, 0);
        wbtcpcAmount = (normalized * (10 ** 10)) / price;
        avgPrice = price;
    }

    /// @notice Buy wBTCPC with stablecoin at the current bonding curve price
    function buy(address stablecoin, uint256 stablecoinAmount) external {
        require(saleActive, "Sale not active");
        require(acceptedStablecoins[stablecoin], "Stablecoin not accepted");
        require(stablecoinAmount > 0, "Zero amount");

        uint256 normalized = _normalizeStablecoin(stablecoin, stablecoinAmount);
        uint256 price = currentPrice();
        require(price > 0, "Price is zero");

        uint256 wbtcpcAmount = (normalized * (10 ** 10)) / price;
        require(wbtcpcAmount > 0, "Amount too small");
        require(wbtcpc.balanceOf(address(this)) >= wbtcpcAmount, "Insufficient pool");

        require(IERC20(stablecoin).transferFrom(msg.sender, treasury, stablecoinAmount), "Payment failed");
        require(wbtcpc.transfer(msg.sender, wbtcpcAmount), "Transfer failed");

        totalSold += wbtcpcAmount;
        emit TokensPurchased(msg.sender, stablecoin, stablecoinAmount, wbtcpcAmount, price);
    }

    /// @notice Deposit wBTCPC into the sale pool
    function deposit(uint256 amount) external {
        require(wbtcpc.transferFrom(msg.sender, address(this), amount), "Deposit failed");
    }

    function _normalizeStablecoin(address stablecoin, uint256 amount) internal view returns (uint256) {
        uint8 dec = stablecoinDecimals[stablecoin];
        if (dec > 6) return amount / (10 ** (dec - 6));
        if (dec < 6) return amount * (10 ** (6 - dec));
        return amount;
    }

    // ─── Admin ──────────────────────────────────────────────────
    function setPriceParameters(uint256 _basePrice, uint256 _priceIncrement) external onlyOwner {
        basePrice = _basePrice;
        priceIncrement = _priceIncrement;
        emit PriceParametersUpdated(_basePrice, _priceIncrement);
    }

    function addStablecoin(address token, uint8 decimals) external onlyOwner {
        acceptedStablecoins[token] = true;
        stablecoinDecimals[token] = decimals;
    }

    function removeStablecoin(address token) external onlyOwner {
        acceptedStablecoins[token] = false;
    }

    function toggleSale(bool active) external onlyOwner {
        saleActive = active;
    }

    function setTreasury(address newTreasury) external onlyOwner {
        require(newTreasury != address(0), "Zero address");
        treasury = newTreasury;
    }

    function withdraw(uint256 amount) external onlyOwner {
        require(wbtcpc.transfer(treasury, amount), "Withdraw failed");
    }
}
