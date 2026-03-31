use crate::ACUITY_NODE_URL;
use subxt::{
    config::RpcConfigFor, rpcs::methods::legacy::LegacyRpcMethods, OnlineClient, PolkadotConfig,
};

pub(crate) type AcuityClient = OnlineClient<PolkadotConfig>;
pub(crate) type AcuityLegacyRpc = LegacyRpcMethods<RpcConfigFor<PolkadotConfig>>;

pub(crate) async fn connect() -> Result<AcuityClient, String> {
    OnlineClient::<PolkadotConfig>::from_insecure_url(ACUITY_NODE_URL)
        .await
        .map_err(|error| format!("Failed to connect to {ACUITY_NODE_URL}: {error}"))
}

pub(crate) async fn connect_legacy_rpc() -> Result<AcuityLegacyRpc, String> {
    let rpc_client = subxt::rpcs::RpcClient::from_insecure_url(ACUITY_NODE_URL)
        .await
        .map_err(|error| format!("Failed to connect RPC client to {ACUITY_NODE_URL}: {error}"))?;
    Ok(LegacyRpcMethods::<RpcConfigFor<PolkadotConfig>>::new(rpc_client))
}
