// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import "@openzeppelin/contracts/access/Ownable.sol";

interface AggregatorV3Interface {
  function decimals() external view returns (uint8);
  function description() external view returns (string memory);
  function version() external view returns (uint256);
  function getRoundData(uint80 _roundId) external view returns (
      uint80 roundId,
      int256 answer,
      uint256 startedAt,
      uint256 updatedAt,
      uint80 answeredInRound
    );
  function latestRoundData() external view returns (
      uint80 roundId,
      int256 answer,
      uint256 startedAt,
      uint256 updatedAt,
      uint80 answeredInRound
    );
}

/**
 * @title HoneHybridExchange
 * @dev This contract implements a hybrid exchange mechanism for wHONE tokens,
 * transitioning from an initial bonding curve pricing model to an AMM-aligned price
 * via an oracle. It features a user-funded, FIFO (First-In, First-Out) selling queue
 * and distributes sale proceeds between the original wHONE depositor and a designated
 * fee wallet.
 */
contract HoneHybridExchange is Ownable {
    // --- Token Interfaces ---
    IERC20 public immutable whone;
    IERC20 public immutable usdtToken;
    IERC20 public immutable usdcToken;
    IERC20 public immutable daiToken;

    // --- Fee Configuration ---
    address public immutable shindevlinWallet;
    uint256 public shindevlinFeeNumerator;
    uint256 public shindevlinFeeDenominator;

    // --- Bonding Curve Parameters ---
    uint256 public initialPrice6Decimals;
    uint256 public priceIncrement6Decimals;
    uint256 public totalWhoneSoldByContract;

    // --- FIFO Queue for wHONE Depositors ---
    struct SellerDeposit {
        address seller;
        uint256 amount;
    }
    SellerDeposit[] public whoneQueue;
    uint256 public head;

    // --- Pricing State Management ---
    enum PricingState {
        BondingCurve,
        AmmAligned
    }
    PricingState public currentPricingState;

    // --- Oracle for AMM-aligned Pricing ---
    AggregatorV3Interface public whoneUsdPriceFeed;

    // --- Events ---
    event WhoneDeposited(address indexed seller, uint256 amount, uint256 index);
    event WhoneWithdrawn(address indexed seller, uint256 amount, uint256 index);
    event WhoneSold(address indexed buyer, address indexed seller, uint256 whoneAmount, uint256 stablecoinAmount, address indexed stablecoinUsed);
    event PricingStateTransitioned(PricingState oldState, PricingState newState);
    event PriceParametersUpdated(uint256 newInitialPrice, uint256 newPriceIncrement);
    event FeeParametersUpdated(uint256 newNumerator, uint256 newDenominator);

    // --- Constants for Decimal Handling ---
    uint256 private constant WHONE_DECIMALS_FACTOR = 10**10;

    // Mapping to store actual decimals of each stablecoin token
    mapping(address => uint8) public stablecoinDecimals;

    constructor(
        address _whone,
        address _usdtToken,
        address _usdcToken,
        address _daiToken,
        address _shindevlinWallet,
        uint256 _shindevlinFeeNumerator,
        uint256 _shindevlinFeeDenominator,
        uint256 _initialPrice6Decimals,
        uint256 _priceIncrement6Decimals,
        address _whoneUsdPriceFeedAddress
    ) Ownable(msg.sender) {
        require(_whone != address(0), "WHONE address cannot be zero.");
        require(_shindevlinWallet != address(0), "Shindevlin wallet address cannot be zero.");
        require(_shindevlinFeeDenominator > 0, "Fee denominator cannot be zero.");
        require(_whoneUsdPriceFeedAddress != address(0), "WHONE/USD Price Feed address cannot be zero.");

        whone = IERC20(_whone);
        usdtToken = IERC20(_usdtToken);
        usdcToken = IERC20(_usdcToken);
        daiToken = IERC20(_daiToken);
        shindevlinWallet = _shindevlinWallet;
        shindevlinFeeNumerator = _shindevlinFeeNumerator;
        shindevlinFeeDenominator = _shindevlinFeeDenominator;
        initialPrice6Decimals = _initialPrice6Decimals;
        priceIncrement6Decimals = _priceIncrement6Decimals;
        whoneUsdPriceFeed = AggregatorV3Interface(_whoneUsdPriceFeedAddress);

        if (_usdtToken != address(0)) stablecoinDecimals[_usdtToken] = 6;
        if (_usdcToken != address(0)) stablecoinDecimals[_usdcToken] = 6;
        if (_daiToken != address(0)) stablecoinDecimals[_daiToken] = 18;

        currentPricingState = PricingState.BondingCurve;
    }

    function transitionToAmmAligned() external onlyOwner {
        require(currentPricingState == PricingState.BondingCurve, "Already in AMM-aligned state.");
        currentPricingState = PricingState.AmmAligned;
        emit PricingStateTransitioned(PricingState.BondingCurve, PricingState.AmmAligned);
    }

    function setBondingCurveParameters(uint256 _initialPrice6Decimals_, uint256 _priceIncrement6Decimals_) external onlyOwner {
        require(currentPricingState == PricingState.BondingCurve, "Only in BondingCurve state.");
        initialPrice6Decimals = _initialPrice6Decimals_;
        priceIncrement6Decimals = _priceIncrement6Decimals_;
        emit PriceParametersUpdated(_initialPrice6Decimals_, _priceIncrement6Decimals_);
    }

    function setFeeParameters(uint256 _numerator, uint256 _denominator) external onlyOwner {
        require(_denominator > 0, "Denominator cannot be zero.");
        shindevlinFeeNumerator = _numerator;
        shindevlinFeeDenominator = _denominator;
        emit FeeParametersUpdated(_numerator, _denominator);
    }

    function addOrUpdateStablecoin(address _tokenAddress, uint8 _decimals) external onlyOwner {
        require(_tokenAddress != address(0), "Zero address.");
        stablecoinDecimals[_tokenAddress] = _decimals;
    }

    function removeStablecoin(address _tokenAddress) external onlyOwner {
        stablecoinDecimals[_tokenAddress] = 0;
    }

    function _normalizeStablecoinTo6Decimals(address _stablecoin, uint256 _amount) internal view returns (uint256) {
        uint8 decimals = stablecoinDecimals[_stablecoin];
        require(decimals > 0, "Not supported.");
        if (decimals == 6) return _amount;
        if (decimals > 6) return _amount / (10**(decimals - 6));
        return _amount * (10**(6 - decimals));
    }

    function _convert6DecimalsToStablecoinDecimals(address _stablecoin, uint256 _amount6Decimals) internal view returns (uint256) {
        uint8 decimals = stablecoinDecimals[_stablecoin];
        require(decimals > 0, "Not supported.");
        if (decimals == 6) return _amount6Decimals;
        if (decimals > 6) return _amount6Decimals * (10**(decimals - 6));
        return _amount6Decimals / (10**(6 - decimals));
    }

    function getWhonePrice6Decimals() public view returns (uint256 price6Decimals) {
        if (currentPricingState == PricingState.BondingCurve) {
            uint256 totalWhoneSoldWholeTokens = totalWhoneSoldByContract / WHONE_DECIMALS_FACTOR;
            price6Decimals = initialPrice6Decimals + (totalWhoneSoldWholeTokens * priceIncrement6Decimals);
        } else {
            (, int256 priceAnswer, , , ) = whoneUsdPriceFeed.latestRoundData();
            require(priceAnswer > 0, "Invalid price.");
            uint8 oracleDecimals = whoneUsdPriceFeed.decimals();
            price6Decimals = uint256(priceAnswer) / (10**(oracleDecimals - 6));
        }
        return price6Decimals;
    }

    function availableWhoneInQueue() public view returns (uint256) {
        uint256 total = 0;
        for (uint256 i = head; i < whoneQueue.length; i++) {
            total += whoneQueue[i].amount;
        }
        return total;
    }

    function depositWhoneForSale(uint256 _amount) external {
        require(_amount > 0, "Amount > 0.");
        whone.transferFrom(msg.sender, address(this), _amount);
        whoneQueue.push(SellerDeposit(msg.sender, _amount));
        emit WhoneDeposited(msg.sender, _amount, whoneQueue.length - 1);
    }

    function buyWhoneWithStablecoin(address _stablecoinAddress, uint256 _stablecoinAmount, uint256 _minWhoneAmount) external {
        require(_stablecoinAmount > 0, "Amount > 0.");
        require(head < whoneQueue.length, "Queue empty.");
        require(stablecoinDecimals[_stablecoinAddress] > 0, "Not supported.");

        IERC20(_stablecoinAddress).transferFrom(msg.sender, address(this), _stablecoinAmount);
        uint256 stablecoinAmount6Decimals = _normalizeStablecoinTo6Decimals(_stablecoinAddress, _stablecoinAmount);
        uint256 currentPrice6Decimals = getWhonePrice6Decimals();
        require(currentPrice6Decimals > 0, "Invalid price.");

        uint256 whoneAmountToBuyer = (stablecoinAmount6Decimals * WHONE_DECIMALS_FACTOR) / currentPrice6Decimals;
        require(whoneAmountToBuyer >= _minWhoneAmount, "Slippage.");
        require(whoneAmountToBuyer <= availableWhoneInQueue(), "Insufficient pool.");

        uint256 whoneRemaining = whoneAmountToBuyer;
        while (whoneRemaining > 0 && head < whoneQueue.length) {
            SellerDeposit storage deposit = whoneQueue[head];
            uint256 take = whoneRemaining;
            if (take >= deposit.amount) {
                take = deposit.amount;
                head++;
            } else {
                deposit.amount -= take;
            }

            uint256 val6 = (take * currentPrice6Decimals) / WHONE_DECIMALS_FACTOR;
            uint256 fee6 = (val6 * shindevlinFeeNumerator) / shindevlinFeeDenominator;
            uint256 share6 = val6 - fee6;

            uint256 feeNative = _convert6DecimalsToStablecoinDecimals(_stablecoinAddress, fee6);
            uint256 shareNative = _convert6DecimalsToStablecoinDecimals(_stablecoinAddress, share6);

            IERC20(_stablecoinAddress).transfer(deposit.seller, shareNative);
            IERC20(_stablecoinAddress).transfer(shindevlinWallet, feeNative);
            whoneRemaining -= take;
        }

        whone.transfer(msg.sender, whoneAmountToBuyer);
        totalWhoneSoldByContract += whoneAmountToBuyer;
        emit WhoneSold(msg.sender, msg.sender, whoneAmountToBuyer, _stablecoinAmount, _stablecoinAddress);
    }

    function withdrawOwnWhone(uint256 _index, uint256 _amount) external {
        require(_index >= head && _index < whoneQueue.length, "Invalid index.");
        SellerDeposit storage deposit = whoneQueue[_index];
        require(deposit.seller == msg.sender, "Not owner.");
        require(_amount > 0 && _amount <= deposit.amount, "Invalid amount.");

        deposit.amount -= _amount;
        whone.transfer(msg.sender, _amount);
        if (deposit.amount == 0 && _index == head) {
            while (head < whoneQueue.length && whoneQueue[head].amount == 0) head++;
        }
        emit WhoneWithdrawn(msg.sender, _amount, _index);
    }
}
