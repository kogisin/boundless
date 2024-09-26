// Copyright (c) 2024 RISC Zero, Inc.
//
// All rights reserved.

use std::borrow::Cow;
#[cfg(not(target_os = "zkvm"))]
use std::str::FromStr;

#[cfg(not(target_os = "zkvm"))]
use alloy::{
    contract::Error as ContractErr,
    primitives::Bytes,
    signers::{Error as SignerErr, Signature, SignerSync},
    sol_types::{Error as DecoderErr, SolInterface, SolStruct},
    transports::TransportError,
};

use alloy_sol_types::{eip712_domain, Eip712Domain};

use alloy_primitives::{
    aliases::{U160, U192},
    Address, U256,
};
use serde::{Deserialize, Serialize};

#[cfg(not(target_os = "zkvm"))]
use thiserror::Error;

// proof_market.rs is a copy of IProofMarket.sol with alloy derive statements added.
// See the build.rs script in this crate for more details.
include!(concat!(env!("OUT_DIR"), "/proof_market.rs"));

/// Status of a proving request
#[derive(Debug, PartialEq)]
pub enum ProofStatus {
    /// The request has expired.
    Expired,
    /// The request is locked in and waiting for fulfillment.
    Locked,
    /// The request has been fulfilled.
    Fulfilled,
    /// The request has an unknown status.
    ///
    /// This is used to represent the status of a request
    /// with no evidence in the state. The request may be
    /// open for bidding or it may not exist.
    Unknown,
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct EIP721DomainSaltless {
    pub name: Cow<'static, str>,
    pub version: Cow<'static, str>,
    pub chain_id: u64,
    pub verifying_contract: Address,
}

impl EIP721DomainSaltless {
    pub fn alloy_struct(&self) -> Eip712Domain {
        eip712_domain! {
            name: self.name.clone(),
            version: self.version.clone(),
            chain_id: self.chain_id,
            verifying_contract: self.verifying_contract,
        }
    }
}

pub(crate) fn request_id(addr: &Address, id: u32) -> U256 {
    let addr = U160::try_from(*addr).unwrap();
    let id = U256::from((U192::from(addr) << 32) | U192::from(id));
    id
}

impl ProvingRequest {
    pub fn new(
        id: u32,
        addr: &Address,
        requirements: Requirements,
        image_url: &str,
        input: Input,
        offer: Offer,
    ) -> Self {
        Self {
            id: request_id(addr, id),
            requirements,
            imageUrl: image_url.to_string(),
            input,
            offer,
        }
    }

    pub fn client_address(&self) -> Address {
        let shifted_id: U256 = self.id >> 32;
        let shifted_bytes: [u8; 32] = shifted_id.to_be_bytes();
        let addr_bytes: [u8; 20] =
            shifted_bytes[12..32].try_into().expect("Failed to extract address bytes");
        let lower_160_bits = U160::from_be_bytes(addr_bytes);

        Address::from(lower_160_bits)
    }

    #[cfg(not(target_os = "zkvm"))]
    pub fn sign_request(
        &self,
        signer: &impl SignerSync,
        contract_addr: Address,
        chain_id: u64,
    ) -> Result<Signature, SignerErr> {
        let domain = eip712_domain(contract_addr, chain_id);
        let hash = self.eip712_signing_hash(&domain.alloy_struct());
        signer.sign_hash_sync(&hash)
    }
}

#[cfg(not(target_os = "zkvm"))]
alloy::sol!(
    #![sol(rpc, all_derives)]
    "../../contracts/src/IRiscZeroSetVerifier.sol"
);

use sha2::{Digest as _, Sha256};
#[cfg(not(target_os = "zkvm"))]
use IProofMarket::IProofMarketErrors;
#[cfg(not(target_os = "zkvm"))]
use IRiscZeroSetVerifier::IRiscZeroSetVerifierErrors;

impl Predicate {
    /// Evaluates the predicate against the given journal.
    #[inline]
    pub fn eval(&self, journal: impl AsRef<[u8]>) -> bool {
        match self.predicateType {
            PredicateType::DigestMatch => self.data.as_ref() == Sha256::digest(journal).as_slice(),
            PredicateType::PrefixMatch => journal.as_ref().starts_with(&self.data),
            PredicateType::__Invalid => panic!("invalid PredicateType"),
        }
    }
}

#[cfg(not(target_os = "zkvm"))]
pub mod proof_market;
#[cfg(not(target_os = "zkvm"))]
pub mod set_verifier;

#[cfg(not(target_os = "zkvm"))]
#[derive(Error, Debug)]
pub enum TxnErr {
    #[error("SetVerifier error: {0:?}")]
    SetVerifierErr(IRiscZeroSetVerifierErrors),

    #[error("ProofMarket Err: {0:?}")]
    ProofMarketErr(IProofMarket::IProofMarketErrors),

    #[error("decoding err: missing data")]
    MissingData,

    #[error("decoding err: bytes decoding")]
    BytesDecode,

    #[error("contract error: {0}")]
    ContractErr(#[from] ContractErr),

    #[error("abi decoder error: {0} - {1}")]
    DecodeErr(DecoderErr, Bytes),
}

#[cfg(not(target_os = "zkvm"))]
fn decode_contract_err<T: SolInterface>(err: ContractErr) -> Result<T, TxnErr> {
    match err {
        ContractErr::TransportError(TransportError::ErrorResp(ts_err)) => {
            let Some(data) = ts_err.data else {
                return Err(TxnErr::MissingData);
            };

            let data = data.get().trim_matches('"');

            let Ok(data) = Bytes::from_str(data) else {
                return Err(TxnErr::BytesDecode);
            };

            let decoded_error = match T::abi_decode(&data, true) {
                Ok(res) => res,
                Err(err) => {
                    return Err(TxnErr::DecodeErr(err, data));
                }
            };

            Ok(decoded_error)
        }
        _ => Err(TxnErr::ContractErr(err)),
    }
}

#[cfg(not(target_os = "zkvm"))]
impl IRiscZeroSetVerifierErrors {
    pub fn decode_error(err: ContractErr) -> TxnErr {
        match decode_contract_err(err) {
            Ok(res) => TxnErr::SetVerifierErr(res),
            Err(decode_err) => decode_err,
        }
    }
}
#[cfg(not(target_os = "zkvm"))]
impl IProofMarketErrors {
    pub fn decode_error(err: ContractErr) -> TxnErr {
        match decode_contract_err(err) {
            Ok(res) => TxnErr::ProofMarketErr(res),
            Err(decode_err) => decode_err,
        }
    }
}

#[cfg(not(target_os = "zkvm"))]
pub fn eip712_domain(addr: Address, chain_id: u64) -> EIP721DomainSaltless {
    EIP721DomainSaltless {
        name: "IProofMarket".into(),
        version: "1".into(),
        chain_id,
        verifying_contract: addr,
    }
}

// TODO: when upgrading to risc0-ethereum-contracts 1.1.0 this function will be removed.
pub fn encode_seal(receipt: &risc0_zkvm::Receipt) -> anyhow::Result<Vec<u8>> {
    use risc0_zkvm::sha::Digestible;

    let seal = match receipt.inner.clone() {
        risc0_zkvm::InnerReceipt::Fake(receipt) => {
            let seal = receipt.claim.digest().as_bytes().to_vec();
            let selector = &[0u8; 4];
            // Create a new vector with the capacity to hold both selector and seal
            let mut selector_seal = Vec::with_capacity(selector.len() + seal.len());
            selector_seal.extend_from_slice(selector);
            selector_seal.extend_from_slice(&seal);
            selector_seal
        }
        risc0_zkvm::InnerReceipt::Groth16(receipt) => {
            let selector = &receipt.verifier_parameters.as_bytes()[..4];
            // Create a new vector with the capacity to hold both selector and seal
            let mut selector_seal = Vec::with_capacity(selector.len() + receipt.seal.len());
            selector_seal.extend_from_slice(selector);
            selector_seal.extend_from_slice(receipt.seal.as_ref());
            selector_seal
        }
        _ => anyhow::bail!("Unsupported receipt type"),
    };
    Ok(seal)
}

#[cfg(not(target_os = "zkvm"))]
pub mod test_utils {
    use aggregation_set::AGGREGATION_SET_GUEST_ID;
    use alloy::{
        network::{Ethereum, EthereumWallet},
        node_bindings::AnvilInstance,
        primitives::{Address, FixedBytes, U256},
        providers::{
            ext::AnvilApi,
            fillers::{
                ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller, WalletFiller,
            },
            Identity, ProviderBuilder, RootProvider,
        },
        signers::local::PrivateKeySigner,
        transports::BoxTransport,
    };
    use anyhow::Result;
    use guest_assessor::ASSESSOR_GUEST_ID;
    use risc0_zkvm::sha::Digest;

    use crate::contracts::{proof_market::ProofMarketService, set_verifier::SetVerifierService};

    alloy::sol!(
        #![sol(rpc)]
        MockVerifier,
        "../../contracts/out/RiscZeroMockVerifier.sol/RiscZeroMockVerifier.json"
    );

    alloy::sol!(
        #![sol(rpc)]
        SetVerifier,
        "../../contracts/out/RiscZeroSetVerifier.sol/RiscZeroSetVerifier.json"
    );

    alloy::sol!(
        #![sol(rpc)]
        ProofMarket,
        "../../contracts/out/ProofMarket.sol/ProofMarket.json"
    );

    // Note: I was completely unable to solve this with generics or trait objects
    type ProviderWallet = FillProvider<
        JoinFill<
            JoinFill<JoinFill<JoinFill<Identity, GasFiller>, NonceFiller>, ChainIdFiller>,
            WalletFiller<EthereumWallet>,
        >,
        RootProvider<BoxTransport>,
        BoxTransport,
        Ethereum,
    >;

    pub struct TestCtx {
        pub verifier_addr: Address,
        pub set_verifier_addr: Address,
        pub proof_market_addr: Address,
        pub prover_signer: PrivateKeySigner,
        pub customer_signer: PrivateKeySigner,
        pub prover_provider: ProviderWallet,
        pub prover_market: ProofMarketService<BoxTransport, ProviderWallet>,
        pub customer_provider: ProviderWallet,
        pub customer_market: ProofMarketService<BoxTransport, ProviderWallet>,
        pub set_verifier: SetVerifierService<BoxTransport, ProviderWallet>,
    }

    impl TestCtx {
        async fn deploy_contracts(anvil: &AnvilInstance) -> Result<(Address, Address, Address)> {
            let deployer_signer: PrivateKeySigner = anvil.keys()[0].clone().into();
            let deployer_provider = ProviderBuilder::new()
                .with_recommended_fillers()
                .wallet(EthereumWallet::from(deployer_signer.clone()))
                .on_builtin(&anvil.endpoint())
                .await
                .unwrap();

            let verifier =
                MockVerifier::deploy(&deployer_provider, FixedBytes::ZERO).await.unwrap();

            let set_verifier = SetVerifier::deploy(
                &deployer_provider,
                *verifier.address(),
                <[u8; 32]>::from(Digest::from(AGGREGATION_SET_GUEST_ID)).into(),
                String::new(),
            )
            .await
            .unwrap();

            let proof_market = ProofMarket::deploy(
                &deployer_provider,
                *set_verifier.address(),
                <[u8; 32]>::from(Digest::from(ASSESSOR_GUEST_ID)).into(),
            )
            .await
            .unwrap();

            // Mine forward some blocks
            deployer_provider.anvil_mine(Some(U256::from(10)), Some(U256::from(2))).await.unwrap();
            deployer_provider.anvil_set_interval_mining(2).await.unwrap();

            Ok((*verifier.address(), *set_verifier.address(), *proof_market.address()))
        }

        pub async fn new(anvil: &AnvilInstance) -> Result<Self> {
            let (verifier_addr, set_verifier_addr, proof_market_addr) =
                TestCtx::deploy_contracts(&anvil).await.unwrap();

            let prover_signer: PrivateKeySigner = anvil.keys()[1].clone().into();
            let customer_signer: PrivateKeySigner = anvil.keys()[2].clone().into();
            let verifier_signer: PrivateKeySigner = anvil.keys()[0].clone().into();

            let prover_provider = ProviderBuilder::new()
                .with_recommended_fillers()
                .wallet(EthereumWallet::from(prover_signer.clone()))
                .on_builtin(&anvil.endpoint())
                .await
                .unwrap();
            let customer_provider = ProviderBuilder::new()
                .with_recommended_fillers()
                .wallet(EthereumWallet::from(customer_signer.clone()))
                .on_builtin(&anvil.endpoint())
                .await
                .unwrap();
            let verifier_provider = ProviderBuilder::new()
                .with_recommended_fillers()
                .wallet(EthereumWallet::from(verifier_signer.clone()))
                .on_builtin(&anvil.endpoint())
                .await
                .unwrap();

            let prover_market = ProofMarketService::new(
                proof_market_addr,
                prover_provider.clone(),
                prover_signer.address(),
            );

            let customer_market = ProofMarketService::new(
                proof_market_addr,
                customer_provider.clone(),
                customer_signer.address(),
            );

            let set_verifier = SetVerifierService::new(
                set_verifier_addr,
                verifier_provider,
                verifier_signer.address(),
            );

            Ok(TestCtx {
                verifier_addr,
                set_verifier_addr,
                proof_market_addr,
                prover_signer,
                customer_signer,
                prover_provider,
                prover_market,
                customer_provider,
                customer_market,
                set_verifier,
            })
        }
    }
}
