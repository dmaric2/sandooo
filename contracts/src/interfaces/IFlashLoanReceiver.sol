// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/**
 * @title IFlashLoanReceiver
 * @author Aave
 * @notice Defines the interface for the flash loan receiver contract.
 * @dev Implement this interface to develop a flashloan-enabled contract.
 */
interface IFlashLoanReceiver {
    /**
     * @notice Executes an operation after receiving the flash-borrowed asset(s).
     * @dev Ensure that by the end of this function, the flash loan must be repaid with the `amounts + premiums`.
     * @param assets The addresses of the flash-borrowed assets
     * @param amounts The amounts of the flash-borrowed assets
     * @param premiums The fees of the flash-borrowed assets (typically 0.09%)
     * @param initiator The address initiating the flash loan
     * @param params Arbitrary bytes passed from the flash loan request
     * @return boolean indicating whether the operation was successful
     */
    function executeOperation(
        address[] calldata assets,
        uint256[] calldata amounts,
        uint256[] calldata premiums,
        address initiator,
        bytes calldata params
    ) external returns (bool);

    /**
     * @notice Executes an operation after receiving a single flash-borrowed asset.
     * @dev Used with flashLoanSimple; ensure the loan is repaid by the end of this function.
     * @param asset The address of the flash-borrowed asset
     * @param amount The amount of the flash-borrowed asset
     * @param premium The fee of the flash-borrowed asset (typically 0.09%)
     * @param initiator The address initiating the flash loan
     * @param params Arbitrary bytes passed from the flash loan request
     * @return boolean indicating whether the operation was successful
     */
    function executeOperation(
        address asset,
        uint256 amount,
        uint256 premium,
        address initiator,
        bytes calldata params
    ) external returns (bool);
}
