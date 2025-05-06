pragma solidity 0.8.20;

import "forge-std/Test.sol";
import "forge-std/console.sol";

import "../src/Sandooo.sol";

/// @title IERC20
/// @notice ERC20 interface for testing purposes.
interface IERC20 {
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(
        address indexed owner,
        address indexed spender,
        uint256 value
    );

    /// @notice Returns the name of the token.
    function name() external view returns (string memory);

    /// @notice Returns the symbol of the token.
    function symbol() external view returns (string memory);

    /// @notice Returns the number of decimals used to get its user representation.
    function decimals() external view returns (uint8);

    /// @notice Returns the total supply of the token.
    function totalSupply() external view returns (uint256);

    /// @notice Returns the amount of tokens owned by `account`.
    function balanceOf(address account) external view returns (uint256);

    /// @notice Moves `amount` tokens from the caller's account to `to`.
    function transfer(address to, uint256 value) external returns (bool);

    /// @notice Returns the remaining number of tokens that `spender` will be allowed to spend on behalf of `owner` through {transferFrom}.
    function allowance(
        address owner,
        address spender
    ) external view returns (uint256);

    /// @notice Sets `amount` as the allowance of `spender` over the caller's tokens.
    function approve(address spender, uint256 value) external returns (bool);

    /// @notice Moves `amount` tokens from `from` to `to` using the allowance mechanism.
    function transferFrom(
        address from,
        address to,
        uint256 value
    ) external returns (bool);
}

/// @title IWETH
/// @notice WETH interface for deposit/withdraw testing.
interface IWETH is IERC20 {
    /// @notice Deposits ETH and mints WETH.
    function deposit() external payable;

    /// @notice Burns WETH and withdraws ETH.
    function withdraw(uint amount) external;
}

/// @title IUniswapV2Pair
/// @notice Uniswap V2 pair interface for reserve queries in tests.
interface IUniswapV2Pair {
    /// @notice Returns the first token in the pair.
    function token0() external returns (address);

    /// @notice Returns the second token in the pair.
    function token1() external returns (address);

    /// @notice Returns the reserves of the pair.
    function getReserves()
        external
        view
        returns (uint112 reserve0, uint112 reserve1, uint32 blockTimestampLast);
}

/// @title SandoooTest
/// @notice Foundry test contract for Sandooo sandwich bot contract.
/// @dev Contains integration tests for deposit, withdrawal, and recovery of tokens and ETH.
// anvil --fork-url http://localhost:8545 --port 2000
// forge test --fork-url http://localhost:2000 --match-contract SandoooTest -vv
contract SandoooTest is Test {
    Sandooo bot;
    IWETH weth = IWETH(0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2);

    receive() external payable {}

    function test() public {
        console.log("Sandooo bot test starting");

        // Create Sandooo instance
        bot = new Sandooo();

        uint256 amountIn = 100000000000000000; // 0.1 ETH

        // Wrap 0.1 ETH to 0.1 WETH and send to Sandooo contract
        weth.deposit{value: amountIn}();
        weth.transfer(address(bot), amountIn);

        // Check if WETH is properly sent
        uint256 botBalance = weth.balanceOf(address(bot));
        console.log("Bot WETH balance: %s", botBalance);

        // Check if we can recover WETH
        bot.recoverToken(address(weth), botBalance);
        uint256 botBalanceAfterRecover = weth.balanceOf(address(bot));
        console.log(
            "Bot WETH balance after recover: %s",
            botBalanceAfterRecover
        ); // should be 0

        // Check if we can recover ETH
        (bool s, ) = address(bot).call{value: amountIn}("");
        console.log("ETH transfer: %s", s);
        uint256 testEthBal = address(this).balance;
        uint256 botEthBal = address(bot).balance;
        console.log("Curr ETH balance: %s", testEthBal);
        console.log("Bot ETH balance: %s", botEthBal);

        // Send zero address to retrieve ETH
        bot.recoverToken(address(0), botEthBal);

        uint256 testEthBalAfterRecover = address(this).balance;
        uint256 botEthBalAfterRecover = address(bot).balance;
        console.log("ETH balance after recover: %s", testEthBalAfterRecover);
        console.log("Bot ETH balance after recover: %s", botEthBalAfterRecover);

        console.log("============================");

        // Transfer WETH to contract again
        weth.transfer(address(bot), amountIn);
        uint256 startingWethBalance = weth.balanceOf(address(bot));
        console.log("Starting WETH balance: %s", startingWethBalance);

        address usdt = 0xdAC17F958D2ee523a2206206994597C13D831ec7;
        address wethUsdtV2 = 0x0d4a11d5EEaaC28EC3F61d100daF4d40471f1852;

        IUniswapV2Pair pair = IUniswapV2Pair(wethUsdtV2);
        address token0 = pair.token0();
        address token1 = pair.token1();

        // We will be testing WETH --> USDT
        // So it's zeroForOne if WETH is token0
        uint8 zeroForOne = address(weth) == token0 ? 1 : 0;

        // Calculate the amountOut using reserves
        (uint112 reserve0, uint112 reserve1, ) = IUniswapV2Pair(address(pair))
            .getReserves();

        uint256 reserveIn;
        uint256 reserveOut;

        if (zeroForOne == 1) {
            reserveIn = reserve0;
            reserveOut = reserve1;
        } else {
            reserveIn = reserve1;
            reserveOut = reserve0;
        }

        uint256 amountInWithFee = amountIn * 997;
        uint256 numerator = amountInWithFee * reserveOut;
        uint256 denominator = reserveIn * 1000 + amountInWithFee;
        uint256 targetAmountOut = numerator / denominator;

        console.log("Amount in: %s", amountIn);
        console.log("Target amount out: %s", targetAmountOut);

        bytes memory data = abi.encodePacked(
            uint64(block.number), // blockNumber
            uint8(zeroForOne), // zeroForOne
            address(pair), // pair
            address(weth), // tokenIn
            uint256(amountIn), // amountIn
            uint256(targetAmountOut) // amountOut
        );
        console.log("Calldata:");
        console.logBytes(data);

        uint gasBefore = gasleft();
        (bool success, ) = address(bot).call(data);
        uint gasAfter = gasleft();
        uint gasUsed = gasBefore - gasAfter;
        console.log("Swap success: %s", success);
        console.log("Gas used: %s", gasUsed);

        uint256 usdtBalance = IERC20(usdt).balanceOf(address(bot));
        console.log("Bot USDT balance: %s", usdtBalance);

        require(success, "FAILED");
    }
}
