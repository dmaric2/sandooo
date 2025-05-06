const hardhat = require("hardhat");
const fs = require("fs");
const path = require("path");
require("dotenv").config();

async function main() {
  console.log("Deploying SandoooV3 contract...");
  
  // Get the contract factory
  const SandoooV3 = await hardhat.ethers.getContractFactory("SandoooV3");
  
  // Aave V3 Pool address from the .env file
  const aavePoolAddress = process.env.AAVE_POOL_ADDRESS;
  
  if (!aavePoolAddress) {
    throw new Error("AAVE_POOL_ADDRESS is not defined in .env file");
  }
  
  console.log(`Using Aave V3 Pool address: ${aavePoolAddress}`);
  
  // Deploy the contract
  const sandooo = await SandoooV3.deploy(aavePoolAddress);
  
  // Wait for the contract to be deployed
  await sandooo.deployed();
  
  console.log(`SandoooV3 deployed to: ${sandooo.address}`);
  
  // Save the deployment information to a file
  const deploymentInfo = {
    SandoooV3Address: sandooo.address,
    AavePoolAddress: aavePoolAddress,
    Network: hardhat.network.name,
    DeploymentTime: new Date().toISOString(),
  };
  
  // Save to a JSON file
  const deploymentPath = path.join(__dirname, "../deployment-info.json");
  fs.writeFileSync(
    deploymentPath,
    JSON.stringify(deploymentInfo, null, 2)
  );
  
  console.log(`Deployment information saved to ${deploymentPath}`);
  
  // Output reminder to update the constants file
  console.log("\n-----------------------------------------------------");
  console.log("IMPORTANT: Remember to update the SANDOOO_V3_ADDRESS in");
  console.log("src/common/constants.rs with the deployed contract address:");
  console.log(`pub const SANDOOO_V3_ADDRESS: &str = "${sandooo.address}";`);
  console.log("-----------------------------------------------------\n");
  
  // Verification command
  console.log("\nVerification command:");
  console.log(`npx hardhat verify --network ${hardhat.network.name} ${sandooo.address} ${aavePoolAddress}`);
  
  // Get deployer account
  const [deployer] = await hardhat.ethers.getSigners();
  console.log(`Deploying with account: ${deployer.address}`);
  console.log(`Owner: ${await sandooo.owner()}`);
  console.log(`Aave Pool: ${await sandooo.AAVE_POOL()}`);
}

main()
  .then(() => process.exit(0))
  .catch((error) => {
    console.error(error);
    process.exit(1);
  });
