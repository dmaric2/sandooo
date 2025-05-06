// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "./interfaces/IPool.sol";
import "./interfaces/IFlashLoanReceiver.sol";
import "./interfaces/IERC20.sol";

/// @title SandoooV3 (refactored)
/// @notice Executes single-asset sandwich attacks with Aave V3 flash-loans
contract SandoooV3 is IFlashLoanReceiver {
    /*───────────────────  Constants / immutables  ───────────────────*/

    /// @notice Contract owner (can trigger flash-loan executions & recover funds)
    address public immutable owner;

    /// @notice Aave V3 Pool used for flash-loans (settable once at deploy)
    address public immutable AAVE_POOL;

    bytes4 private constant TOKEN_TRANSFER_ID = 0xa9059cbb;      // transfer(address,uint256)
    bytes4 private constant TOKEN_APPROVE_ID  = 0x095ea7b3;      // approve(address,uint256)
    bytes4 private constant V2_SWAP_ID        = 0x022c0d9f;      // swap(uint,uint,address,bytes)

    /*────────────────────────  State  ───────────────────────────────*/

    uint256 private _unlocked = 1;                        // re-entrancy mutex

    /*────────────────────────  Errors  ──────────────────────────────*/

    error NotOwner();
    error InvalidData();
    error WrongBlock();
    error Unauthorized();
    error InvalidInitiator();
    error InsufficientProfit();
    error EthTransferFailed();

    /*─────────────────────  Constructor  ────────────────────────────*/

    constructor(address _aavePool) {
        owner      = msg.sender;
        AAVE_POOL  = _aavePool;
    }

    /*─────────────────────  Modifiers  ──────────────────────────────*/

    modifier onlyOwner() {
        if (msg.sender != owner) revert NotOwner();
        _;
    }

    modifier nonReentrant() {
        require(_unlocked == 1, "LOCKED");
        _unlocked = 0;
        _;
        _unlocked = 1;
    }

    /*─────────────────────  External API  ───────────────────────────*/

    /// @notice Initiates an Aave V3 flash-loan and executes the sandwich bundle
    /// @param asset         Token to borrow (e.g. WETH)
    /// @param amount        Amount to borrow
    /// @param sandwichData  Encoded bundle: 8-byte block number + N×(105-byte trade)
    function executeSandwichWithFlashloan(
        address asset,
        uint256 amount,
        bytes calldata sandwichData
    ) external onlyOwner nonReentrant {
        /*── pre-checks ─────────────────────────────────────────────*/

        // 8-byte header + N×105 bytes body
        if (sandwichData.length <= 8 ||
            (sandwichData.length - 8) % 105 != 0
        ) revert InvalidData();

        // block-number pinning (first 8 bytes)
        uint64 blk;
        assembly { blk := shr(192, calldataload(sandwichData.offset)) }
        if (blk != block.number) revert WrongBlock();

        /*── request flash-loan ─────────────────────────────────────*/

        IPool(AAVE_POOL).flashLoanSimple(
            address(this),
            asset,
            amount,
            sandwichData,
            0                                    // referralCode
        );

        /*── sweep profit ───────────────────────────────────────────*/

        uint256 bal = IERC20(asset).balanceOf(address(this));
        if (bal != 0) _safeTransfer(asset, owner, bal);
    }

    /*─────────────────  Aave flash-loan callback  ───────────────────*/

    function executeOperation(
        address asset,
        uint256 amount,
        uint256 premium,
        address initiator,
        bytes calldata params
    ) external override nonReentrant returns (bool) {
        if (msg.sender != AAVE_POOL)         revert Unauthorized();
        if (initiator != address(this))      revert InvalidInitiator();

        uint256 repay = amount + premium;

        _executeSandwich(asset, params);                     // main bundle

        // verify profit & approve repayment (safe approve pattern)
        uint256 bal = IERC20(asset).balanceOf(address(this));
        if (bal < repay) revert InsufficientProfit();

        _safeApprove(asset, AAVE_POOL, 0);       // reset to satisfy USDT-style tokens
        _safeApprove(asset, AAVE_POOL, repay);

        return true;                                           // Aave pulls funds
    }

    /*────────────────────  Owner rescue hooks  ─────────────────────*/

    function recoverToken(address token, uint256 amount) external onlyOwner {
        _safeTransfer(token, owner, amount);
    }

    function recoverETH() external onlyOwner {
        (bool ok, ) = owner.call{value: address(this).balance}("");
        if (!ok) revert EthTransferFailed();
    }

    /*─────────────────  Unused multi-asset callback  ───────────────*/

    function executeOperation(
        address[] calldata,
        uint256[] calldata,
        uint256[] calldata,
        address,
        bytes calldata
    ) external pure override returns (bool) {
        revert("MULTI_ASSET_NOT_SUPPORTED");
    }

    /*────────────────────  Internal: sandwich loop  ────────────────*/

    function _executeSandwich(address /*asset*/, bytes calldata data) internal {
        uint256 offset = 8;                                  // skip block #
        uint256 end    = data.length;

        while (offset < end) {
            bool zeroForOne  = uint8(data[offset]) == 1;
            offset += 1;

            address pair;
            address tokenIn;
            uint256 amountIn;
            uint256 amountOut;

            assembly {
                // pair (20 bytes)
                pair := shr(96, calldataload(add(data.offset, offset)))
                offset := add(offset, 20)

                // tokenIn (20 bytes)
                tokenIn := shr(96, calldataload(add(data.offset, offset)))
                offset := add(offset, 20)

                // amountIn (32 bytes)
                amountIn := calldataload(add(data.offset, offset))
                offset   := add(offset, 32)

                // amountOut (32 bytes)
                amountOut := calldataload(add(data.offset, offset))
                offset     := add(offset, 32)
            }

            _safeTransfer(tokenIn, pair, amountIn);          // pay in

            _executeSwap(pair, zeroForOne, amountOut);       // pull out
        }
    }

    /*────────────────────  Internal: swap  ─────────────────────────*/

    /// @dev Correct ABI-encoded call for Uniswap V2 swap
    function _executeSwap(address pair, bool zeroForOne, uint256 amountOut) internal {
        assembly {
            let ptr := mload(0x40)

            // selector
            mstore(ptr, V2_SWAP_ID)

            // amounts
            switch zeroForOne
            case 0 {
                mstore(add(ptr, 0x04), amountOut)           // amount0Out
                mstore(add(ptr, 0x24), 0)                  // amount1Out
            }
            default {
                mstore(add(ptr, 0x04), 0)
                mstore(add(ptr, 0x24), amountOut)
            }

            // to
            mstore(add(ptr, 0x44), shl(96, address()))      // address(this)

            // offset to bytes data (0xa0 = 160)
            mstore(add(ptr, 0x64), 0xa0)

            // bytes length = 0
            mstore(add(ptr, 0x84), 0)

            if iszero(
                call(gas(), pair, 0, ptr, 0xa4, 0, 0)       // 0xa4 = 164 bytes
            ) { revert(0, 0) }
        }
    }

    /*────────────────  Internal: safe ERC-20 helpers  ──────────────*/

    function _safeTransfer(address token, address to, uint256 amount) private {
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, TOKEN_TRANSFER_ID)
            mstore(add(ptr, 0x04), to)
            mstore(add(ptr, 0x24), amount)

            if iszero(call(gas(), token, 0, ptr, 0x44, 0, 0)) { revert(0, 0) }

            switch returndatasize()
            case 0 { }                                 // non-standard ERC-20
            case 0x20 {
                returndatacopy(ptr, 0, 0x20)
                if iszero(mload(ptr)) { revert(0, 0) } // false ⇒ revert
            }
            default { revert(0, 0) }
        }
    }

    function _safeApprove(address token, address to, uint256 amount) private {
        assembly {
            let ptr := mload(0x40)
            mstore(ptr, TOKEN_APPROVE_ID)
            mstore(add(ptr, 0x04), to)
            mstore(add(ptr, 0x24), amount)

            if iszero(call(gas(), token, 0, ptr, 0x44, 0, 0)) { revert(0, 0) }

            switch returndatasize()
            case 0 { }
            case 0x20 {
                returndatacopy(ptr, 0, 0x20)
                if iszero(mload(ptr)) { revert(0, 0) }
            }
            default { revert(0, 0) }
        }
    }

    /*────────────────────  ETH receive / fallback  ─────────────────*/

    receive() external payable {}
    fallback() external payable { revert("DIRECT_CALLS_NOT_ALLOWED"); }
}
