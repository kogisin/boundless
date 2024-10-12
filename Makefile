.POSIX:
.SILENT:

.PHONY: devnet-up devnet-down check-deps clean all

# Variables
ANVIL_PORT = 8545
ANVIL_BLOCK_TIME = 2
RISC0_DEV_MODE = 1
RUST_LOG = info,broker=debug,boundless_market=debug
PRIVATE_KEY = 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
DEPOSIT_AMOUNT = 10

LOGS_DIR = logs
PID_FILE = $(LOGS_DIR)/devnet.pid

all: devnet-up

# Check that required dependencies are installed
check-deps:
	for cmd in forge cargo anvil jq; do \
		command -v $$cmd >/dev/null 2>&1 || { echo "Error: $$cmd is not installed."; exit 1; }; \
	done

devnet-up: check-deps
	mkdir -p $(LOGS_DIR)
	echo "Building contracts..."
	forge build || { echo "Failed to build contracts"; $(MAKE) devnet-down; exit 1; }
	echo "Building Rust project..."
	cargo build --bin broker || { echo "Failed to build broker binary"; $(MAKE) devnet-down; exit 1; }
	echo "Starting Anvil..."
	anvil -b $(ANVIL_BLOCK_TIME) > $(LOGS_DIR)/anvil.txt 2>&1 & echo $$! >> $(PID_FILE)
	sleep 5
	echo "Deploying contracts..."
	RISC0_DEV_MODE=$(RISC0_DEV_MODE) forge script contracts/scripts/Deploy.s.sol --rpc-url http://localhost:$(ANVIL_PORT) --broadcast -vv || { echo "Failed to deploy contracts"; $(MAKE) devnet-down; exit 1; }
	echo "Fetching contract addresses..."
	{ \
		SET_VERIFIER_ADDRESS=$$(jq -re '.transactions[] | select(.contractName == "RiscZeroSetVerifier") | .contractAddress' ./broadcast/Deploy.s.sol/31337/run-latest.json); \
		PROOF_MARKET_ADDRESS=$$(jq -re '.transactions[] | select(.contractName == "ProofMarket") | .contractAddress' ./broadcast/Deploy.s.sol/31337/run-latest.json); \
		echo "Contract deployed at addresses:"; \
		echo "SET_VERIFIER_ADDRESS=$$SET_VERIFIER_ADDRESS"; \
		echo "PROOF_MARKET_ADDRESS=$$PROOF_MARKET_ADDRESS"; \
		echo "Updating .env file..."; \
		sed -i.bak "s/^SET_VERIFIER_ADDRESS=.*/SET_VERIFIER_ADDRESS=$$SET_VERIFIER_ADDRESS/" .env && \
		sed -i.bak "s/^PROOF_MARKET_ADDRESS=.*/PROOF_MARKET_ADDRESS=$$PROOF_MARKET_ADDRESS/" .env && \
		rm .env.bak; \
		echo ".env file updated successfully."; \
		echo "Starting Broker service..."; \
		RISC0_DEV_MODE=$(RISC0_DEV_MODE) RUST_LOG=$(RUST_LOG) ./target/debug/broker \
			--private-key $(PRIVATE_KEY) \
			--proof-market-addr $$PROOF_MARKET_ADDRESS \
			--set-verifier-addr $$SET_VERIFIER_ADDRESS \
			--deposit-amount $(DEPOSIT_AMOUNT) > $(LOGS_DIR)/broker.txt 2>&1 & echo $$! >> $(PID_FILE); \
	} || { echo "Failed to fetch addresses or start broker"; $(MAKE) devnet-down; exit 1; }
	echo "Devnet is up and running!"
	echo "Make sure to run 'source .env' to load the environment variables."

devnet-down:
	echo "Bringing down all services..."
	if [ -f $(PID_FILE) ]; then \
		while read pid; do \
			kill $$pid 2>/dev/null || true; \
		done < $(PID_FILE); \
		rm $(PID_FILE); \
	fi
	echo "Devnet stopped."

clean: devnet-down
	echo "Cleaning up..."
	rm -rf $(LOGS_DIR) ./broadcast
	cargo clean
	forge clean
	echo "Cleanup complete."
