// Copyright (c) 2024 RISC Zero, Inc.
//
// All rights reserved.

use std::{env, str::FromStr};

use alloy::{
    network::Ethereum,
    primitives::{Address, Bytes, U256},
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            WalletFiller,
        },
        network::EthereumWallet,
        Identity, Provider, ProviderBuilder, RootProvider,
    },
    signers::{
        k256::ecdsa::SigningKey,
        local::{LocalSigner, PrivateKeySigner},
    },
    transports::{http::Http, Transport},
};
use anyhow::{anyhow, Context, Result};
use reqwest::Client as HttpClient;
use url::Url;

use crate::{
    contracts::{
        boundless_market::{BoundlessMarketService, MarketError},
        set_verifier::SetVerifierService,
        ProofRequest, RequestError,
    },
    order_stream_client::Client as OrderStreamClient,
    storage::{
        storage_provider_from_env, BuiltinStorageProvider, BuiltinStorageProviderError,
        StorageProvider,
    },
};

type ProviderWallet = FillProvider<
    JoinFill<
        JoinFill<
            Identity,
            JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider<Http<HttpClient>>,
    Http<HttpClient>,
    Ethereum,
>;

#[derive(thiserror::Error, Debug)]
#[non_exhaustive]
/// Client error
pub enum ClientError {
    #[error("Storage provider error {0}")]
    StorageProviderError(#[from] BuiltinStorageProviderError),
    #[error("Market error {0}")]
    MarketError(#[from] MarketError),
    #[error("RequestError {0}")]
    RequestError(#[from] RequestError),
    #[error("Error {0}")]
    Error(#[from] anyhow::Error),
}

#[derive(Clone)]
/// Client for interacting with the boundless market
pub struct Client<T, P, S> {
    pub boundless_market: BoundlessMarketService<T, P>,
    pub set_verifier: SetVerifierService<T, P>,
    pub signer: LocalSigner<SigningKey>,
    pub storage_provider: S,
    pub offchain_client: OrderStreamClient,
}

impl<T, P, S> Client<T, P, S>
where
    T: Transport + Clone,
    P: Provider<T, Ethereum> + 'static + Clone,
    S: StorageProvider + Clone,
{
    /// Create a new client
    pub fn new(
        boundless_market: BoundlessMarketService<T, P>,
        set_verifier: SetVerifierService<T, P>,
        signer: LocalSigner<SigningKey>,
        storage_provider: S,
        offchain_client: OrderStreamClient,
        tx_timeout: Option<std::time::Duration>,
    ) -> Self {
        let mut boundless_market = boundless_market.clone();
        let mut set_verifier = set_verifier.clone();
        if let Some(timeout) = tx_timeout {
            boundless_market = boundless_market.with_timeout(timeout);
            set_verifier = set_verifier.with_timeout(timeout);
        }
        Self { boundless_market, set_verifier, signer, storage_provider, offchain_client }
    }

    /// Get the provider
    pub fn provider(&self) -> P {
        self.boundless_market.instance().provider().clone()
    }

    /// Get the caller address
    pub fn caller(&self) -> Address {
        self.signer.address()
    }

    pub fn with_timeout(self, tx_timeout: std::time::Duration) -> Self {
        Self::new(
            self.boundless_market,
            self.set_verifier,
            self.signer,
            self.storage_provider,
            self.offchain_client,
            Some(tx_timeout),
        )
    }

    /// Upload an image to the storage provider
    pub async fn upload_image(&self, elf: &[u8]) -> Result<String, ClientError> {
        Ok(self
            .storage_provider
            .upload_image(elf)
            .await
            .map_err(|_| anyhow!("Failed to upload image"))?)
    }

    /// Upload input to the storage provider
    pub async fn upload_input(&self, input: &[u8]) -> Result<String, ClientError> {
        Ok(self
            .storage_provider
            .upload_input(input)
            .await
            .map_err(|_| anyhow!("Failed to upload input"))?)
    }

    /// Submit a proof request.
    ///
    /// If the request ID is not set, a random ID will be generated.
    /// If the bidding start is not set, the current block number will be used.
    pub async fn submit_request(&self, request: &ProofRequest) -> Result<U256, ClientError>
    where
        <S as StorageProvider>::Error: std::fmt::Debug,
    {
        let mut request = request.clone();

        if request.id == U256::ZERO {
            request.id = self.boundless_market.request_id_from_nonce().await?;
        };
        if request.offer.biddingStart == 0 {
            request.offer.biddingStart = self
                .provider()
                .get_block_number()
                .await
                .context("Failed to get current block number")?
        };

        request.validate()?;

        Ok(self.boundless_market.submit_request(&request, &self.signer.clone()).await?)
    }

    /// Submit a proof request offchain via the order stream service.
    ///
    /// If the request ID is not set, a random ID will be generated.
    /// If the bidding start is not set, the current block number will be used.
    pub async fn submit_request_offchain(&self, request: &ProofRequest) -> Result<U256, ClientError>
    where
        <S as StorageProvider>::Error: std::fmt::Debug,
    {
        let mut request = request.clone();

        if request.id == U256::ZERO {
            request.id = self.boundless_market.request_id_from_rand().await?;
        };
        if request.offer.biddingStart == 0 {
            request.offer.biddingStart = self
                .provider()
                .get_block_number()
                .await
                .context("Failed to get current block number")?
        };
        // Ensure address' balance is sufficient to cover the request
        let balance = self.boundless_market.balance_of(request.client_address()?).await?;
        if balance < U256::from(request.offer.maxPrice) {
            return Err(ClientError::Error(anyhow!(
                "Insufficient balance to cover request: {} < {}",
                balance,
                request.offer.maxPrice
            )));
        }

        let order = self.offchain_client.submit_request(&request).await?;

        Ok(U256::from(order.request.id))
    }

    /// Wait for a request to be fulfilled.
    ///
    /// The check interval is the time between each check for fulfillment.
    /// The timeout is the maximum time to wait for the request to be fulfilled.
    pub async fn wait_for_request_fulfillment(
        &self,
        request_id: U256,
        check_interval: std::time::Duration,
        expires_at: u64,
    ) -> Result<(Bytes, Bytes), ClientError> {
        Ok(self
            .boundless_market
            .wait_for_request_fulfillment(request_id, check_interval, expires_at)
            .await?)
    }
}

impl Client<Http<HttpClient>, ProviderWallet, BuiltinStorageProvider> {
    /// Create a new client from environment variables
    ///
    /// The following environment variables are required:
    /// - PRIVATE_KEY: The private key of the wallet
    /// - RPC_URL: The URL of the RPC server
    /// - ORDER_STREAM_URL: The URL of the order stream server
    /// - BOUNDLESS_MARKET_ADDRESS: The address of the market contract
    /// - SET_VERIFIER_ADDRESS: The address of the set verifier contract
    pub async fn from_env() -> Result<Self, ClientError> {
        let private_key_str = env::var("private_key").context("private_key not set")?;
        let private_key =
            PrivateKeySigner::from_str(&private_key_str).context("Invalid private_key")?;
        let rpc_url_str = env::var("RPC_URL").context("RPC_URL not set")?;
        let rpc_url = Url::parse(&rpc_url_str).context("Invalid RPC_URL")?;
        let boundless_market_address_str =
            env::var("BOUNDLESS_MARKET_ADDRESS").context("BOUNDLESS_MARKET_ADDRESS not set")?;
        let boundless_market_address = Address::from_str(&boundless_market_address_str)
            .context("Invalid BOUNDLESS_MARKET_ADDRESS")?;
        let set_verifier_address_str =
            env::var("SET_VERIFIER_ADDRESS").context("SET_VERIFIER_ADDRESS not set")?;
        let set_verifier_address =
            Address::from_str(&set_verifier_address_str).context("Invalid SET_VERIFIER_ADDRESS")?;

        let caller = private_key.address();
        let signer = private_key.clone();
        let wallet = EthereumWallet::from(private_key.clone());
        let provider =
            ProviderBuilder::new().with_recommended_fillers().wallet(wallet).on_http(rpc_url);

        let boundless_market =
            BoundlessMarketService::new(boundless_market_address, provider.clone(), caller);
        let set_verifier = SetVerifierService::new(set_verifier_address, provider.clone(), caller);

        let storage_provider = storage_provider_from_env().await?;

        let order_stream_url = env::var("ORDER_STREAM_URL").context("ORDER_STREAM_URL not set")?;
        let chain_id = provider.get_chain_id().await.context("Failed to get chain ID")?;
        let offchain_client = OrderStreamClient::new(
            Url::parse(&order_stream_url).context("Invalid ORDER_STREAM_URL")?,
            signer.clone(),
            boundless_market_address,
            chain_id,
        );

        Ok(Self { boundless_market, set_verifier, signer, storage_provider, offchain_client })
    }

    /// Create a new client from parts
    pub async fn from_parts(
        private_key: PrivateKeySigner,
        rpc_url: Url,
        boundless_market_address: Address,
        set_verifier_address: Address,
        order_stream_url: Url,
        storage_provider: BuiltinStorageProvider,
    ) -> Result<Self, ClientError> {
        let caller = private_key.address();
        let signer = private_key.clone();
        let wallet = EthereumWallet::from(private_key.clone());
        let provider =
            ProviderBuilder::new().with_recommended_fillers().wallet(wallet).on_http(rpc_url);

        let boundless_market =
            BoundlessMarketService::new(boundless_market_address, provider.clone(), caller);
        let set_verifier = SetVerifierService::new(set_verifier_address, provider.clone(), caller);

        let chain_id = provider.get_chain_id().await.context("Failed to get chain ID")?;
        let offchain_client = OrderStreamClient::new(
            order_stream_url,
            signer.clone(),
            boundless_market_address,
            chain_id,
        );

        Ok(Self { boundless_market, set_verifier, signer, storage_provider, offchain_client })
    }
}
