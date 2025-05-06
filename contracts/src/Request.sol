// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title Request
/// @notice Utility contract for querying ERC20 token metadata from a target token address.
/// @dev Provides a function to fetch name, symbol, decimals, and total supply for any ERC20-compliant token.
interface IERC20 {
    /// @notice Returns the name of the token.
    function name() external view returns (string memory);

    /// @notice Returns the symbol of the token.
    function symbol() external view returns (string memory);

    /// @notice Returns the decimals of the token.
    function decimals() external view returns (uint8);

    /// @notice Returns the total supply of the token.
    function totalSupply() external view returns (uint256);
}

/// @title Request
/// @notice Provides a method to fetch ERC20 token metadata in a single call.
contract Request {
    /// @notice Fetches the metadata of an ERC20 token.
    /// @param targetToken The address of the token contract to query.
    /// @return name The name of the token.
    /// @return symbol The symbol of the token.
    /// @return decimals The decimals of the token.
    /// @return totalSupply The total supply of the token.
    function getTokenInfo(
        address targetToken
    )
        external
        view
        returns (
            string memory name,
            string memory symbol,
            uint8 decimals,
            uint256 totalSupply
        )
    {
        IERC20 t = IERC20(targetToken);

        name = t.name();
        symbol = t.symbol();
        decimals = t.decimals();
        totalSupply = t.totalSupply();
    }
}
