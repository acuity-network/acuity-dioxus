use crate::ACUITY_NODE_URL;
use subxt::{OnlineClient, PolkadotConfig};

pub(crate) type AcuityClient = OnlineClient<PolkadotConfig>;

pub(crate) async fn connect() -> Result<AcuityClient, String> {
    OnlineClient::<PolkadotConfig>::from_insecure_url(ACUITY_NODE_URL)
        .await
        .map_err(|error| format!("Failed to connect to {ACUITY_NODE_URL}: {error}"))
}
