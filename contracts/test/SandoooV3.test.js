// Unit tests for SandoooV3 contract
const { ethers } = require("hardhat");
const { expect } = require("chai");
const { parseEther } = ethers.utils;

// Test constants
const AAVE_V3_POOL_ADDRESS = "0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2";
const WETH_ADDRESS = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";

describe("SandoooV3", function() {
  let sandoooV3;
  let owner;
  let otherAccount;
  
  before(async function() {
    [owner, otherAccount] = await ethers.getSigners();
    
    // Deploy the SandoooV3 contract
    const SandoooV3Factory = await ethers.getContractFactory("SandoooV3");
    sandoooV3 = await SandoooV3Factory.deploy(AAVE_V3_POOL_ADDRESS);
    await sandoooV3.deployed();
  });
  
  describe("Deployment", function() {
    it("Should set the right owner", async function() {
      expect(await sandoooV3.owner()).to.equal(owner.address);
    });
    
    it("Should set the correct Aave V3 Pool address", async function() {
      expect(await sandoooV3.AAVE_POOL()).to.equal(AAVE_V3_POOL_ADDRESS);
    });
  });
  
  describe("Access Control", function() {
    it("Should allow only owner to call executeSandwichWithFlashloan", async function() {
      // Create minimal valid sandwich data (just header)
      const blockNumber = ethers.BigNumber.from(String(await ethers.provider.getBlockNumber()));
      const blockHeader = ethers.utils.hexZeroPad(blockNumber.toHexString(), 8);
      
      // Try to call with non-owner account
      await expect(
        sandoooV3.connect(otherAccount).executeSandwichWithFlashloan(
          WETH_ADDRESS,
          parseEther("1"),
          blockHeader
        )
      ).to.be.revertedWithCustomError(sandoooV3, "NotOwner");
    });
    
    it("Should allow only owner to call recover functions", async function() {
      await expect(
        sandoooV3.connect(otherAccount).recoverToken(WETH_ADDRESS, parseEther("1"))
      ).to.be.revertedWithCustomError(sandoooV3, "NotOwner");
      
      await expect(
        sandoooV3.connect(otherAccount).recoverETH()
      ).to.be.revertedWithCustomError(sandoooV3, "NotOwner");
    });
  });
  
  describe("Input Validation", function() {
    it("Should revert if sandwich data is too short", async function() {
      // Create data shorter than 8 bytes
      const invalidData = "0x1234";
      
      await expect(
        sandoooV3.executeSandwichWithFlashloan(
          WETH_ADDRESS,
          parseEther("1"),
          invalidData
        )
      ).to.be.revertedWithCustomError(sandoooV3, "InvalidData");
    });
    
    it("Should revert if sandwich data length is invalid", async function() {
      // Create data with 8 bytes header + invalid trade length (not a multiple of 105)
      const blockNumber = ethers.BigNumber.from(String(await ethers.provider.getBlockNumber()));
      const blockHeader = ethers.utils.hexZeroPad(blockNumber.toHexString(), 8);
      const invalidData = ethers.utils.hexConcat([blockHeader, "0x1234"]); // 8 + 2 bytes
      
      await expect(
        sandoooV3.executeSandwichWithFlashloan(
          WETH_ADDRESS,
          parseEther("1"),
          invalidData
        )
      ).to.be.revertedWithCustomError(sandoooV3, "InvalidData");
    });
    
    it("Should revert if block number doesn't match", async function() {
      // Create data with wrong block number
      const wrongBlockNumber = ethers.BigNumber.from(String(await ethers.provider.getBlockNumber())).add(100);
      const blockHeader = ethers.utils.hexZeroPad(wrongBlockNumber.toHexString(), 8);
      
      await expect(
        sandoooV3.executeSandwichWithFlashloan(
          WETH_ADDRESS,
          parseEther("1"),
          blockHeader
        )
      ).to.be.revertedWithCustomError(sandoooV3, "WrongBlock");
    });
  });
  
  // Note: For flashloan execution and sandwich trade tests, we would need:
  // 1. Fork mainnet for Aave V3 Pool and other dependencies
  // 2. Set up mock tokens, pairs, and liquidity
  // 3. Create valid sandwich trade data
  // This is better tested in an integration test environment
});
