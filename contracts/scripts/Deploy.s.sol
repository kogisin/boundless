// Copyright (c) 2024 RISC Zero, Inc.
//
// All rights reserved.

pragma solidity ^0.8.20;

import {Script, console2} from "forge-std/Script.sol";
import "forge-std/Test.sol";
import {IRiscZeroVerifier} from "risc0/IRiscZeroVerifier.sol";
import {ControlID, RiscZeroGroth16Verifier} from "risc0/groth16/RiscZeroGroth16Verifier.sol";
import {RiscZeroCheats} from "risc0/test/RiscZeroCheats.sol";
import {UnsafeUpgrades, Upgrades} from "openzeppelin-foundry-upgrades/Upgrades.sol";
import {ConfigLoader, DeploymentConfig} from "./Config.s.sol";

import {ProofMarket} from "../src/ProofMarket.sol";
import {RiscZeroSetVerifier} from "../src/RiscZeroSetVerifier.sol";

// For local testing:
// TODO: Uncommenting these lines currently creates a chicken-egg problem in that
// `cargo build -Ftest-utils` cannot complete without first running `forge build` and `forge build`
// cannot run without building the guests.
//import {ImageID as AssesorImgId} from "../src/AssessorImageID.sol";
//import {ImageID as SetBuidlerId} from "../src/SetBuilderImageID.sol";

contract Deploy is Script, RiscZeroCheats {
    // Path to deployment config file, relative to the project root.
    string constant CONFIG_FILE = "contracts/deployment.toml";

    IRiscZeroVerifier verifier;
    RiscZeroSetVerifier setVerifier;
    address proofMarketAddress;
    bytes32 setBuilderImageId;
    bytes32 assessorImageId;

    function run() external {
        string memory setBuilderGuestUrl = "";
        string memory assessorGuestUrl = "";

        // load ENV variables first
        uint256 deployerKey = vm.envOr("DEPLOYER_PRIVATE_KEY", uint256(0));
        require(deployerKey != 0, "No deployer key provided. Please set the env var DEPLOYER_PRIVATE_KEY.");
        vm.rememberKey(deployerKey);

        address proofMarketOwner = vm.envAddress("PROOF_MARKET_OWNER");
        console2.log("ProofMarket Owner:", proofMarketOwner);

        // Read and log the chainID
        uint256 chainId = block.chainid;
        console2.log("You are deploying on ChainID %d", chainId);

        // Load the deployment config
        DeploymentConfig memory deploymentConfig =
            ConfigLoader.loadDeploymentConfig(string.concat(vm.projectRoot(), "/", CONFIG_FILE));

        // Assign parsed config values to the variables
        verifier = IRiscZeroVerifier(deploymentConfig.router);
        setVerifier = RiscZeroSetVerifier(deploymentConfig.setVerifier);
        setBuilderImageId = deploymentConfig.setBuilderImageId;
        setBuilderGuestUrl = deploymentConfig.setBuilderGuestUrl;
        assessorImageId = deploymentConfig.assessorImageId;
        assessorGuestUrl = deploymentConfig.assessorGuestUrl;

        vm.startBroadcast(deployerKey);

        // Deploy the verifier, if not already deployed.
        if (address(verifier) == address(0)) {
            verifier = deployRiscZeroVerifier();
        } else {
            console2.log("Using IRiscZeroVerifier contract deployed at", address(verifier));
        }

        // Set the setBuilderImageId and assessorImageId if not set.
        if (setBuilderImageId == bytes32(0)) {
            // TODO: Currently cannot work. See note in imports.
            //setBuilderImageId = SetBuidlerId.SET_BUILDER_GUEST_ID;
            revert("set builder image ID must be set in deployment.toml");
        }
        if (assessorImageId == bytes32(0)) {
            // TODO: Currently cannot work. See note in imports.
            //assessorImageId = AssesorImgId.ASSESSOR_GUEST_ID;
            revert("assessor image ID must be set in deployment.toml");
        }

        if (bytes(vm.envOr("RISC0_DEV_MODE", string(""))).length > 0) {
            // TODO: Create a more robust way of getting a URI for guests, and ensure that it is
            // in-sync with the configured image ID.
            string memory cwd = vm.envString("PWD");
            setBuilderGuestUrl =
                string.concat("file://", cwd, "/target/riscv-guest/riscv32im-risc0-zkvm-elf/release/set-builder-guest");
            console2.log("Set builder URI", setBuilderGuestUrl);
            assessorGuestUrl =
                string.concat("file://", cwd, "/target/riscv-guest/riscv32im-risc0-zkvm-elf/release/assessor-guest");
            console2.log("Assessor URI", assessorGuestUrl);
        }

        // Deploy the setVerifier, if not already deployed.
        if (address(setVerifier) == address(0)) {
            setVerifier = new RiscZeroSetVerifier(verifier, setBuilderImageId, setBuilderGuestUrl);
            console2.log("Deployed RiscZeroSetVerifier to", address(setVerifier));
        } else {
            console2.log("Using RiscZeroSetVerifier contract deployed at", address(setVerifier));
        }

        // Deploy the proof market
        address newImplementation = address(new ProofMarket(setVerifier, assessorImageId));
        console2.log("Deployed new ProofMarket implementation at", newImplementation);
        proofMarketAddress = UnsafeUpgrades.deployUUPSProxy(
            newImplementation, abi.encodeCall(ProofMarket.initialize, (proofMarketOwner, assessorGuestUrl))
        );
        console2.log("Deployed ProofMarket (proxy) to", proofMarketAddress);

        vm.stopBroadcast();
    }
}
