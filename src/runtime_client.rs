use crate::{acuity_runtime::api, ACUITY_NODE_URL};
use parity_scale_codec::{Compact, Decode, Encode};
use subxt::{OnlineClient, PolkadotConfig};
use subxt_signer::sr25519::Keypair as SignerKeypair;

pub(crate) type AcuityClient = OnlineClient<PolkadotConfig>;

pub(crate) async fn connect() -> Result<AcuityClient, String> {
    OnlineClient::<PolkadotConfig>::from_insecure_url(ACUITY_NODE_URL)
        .await
        .map_err(|error| format!("Failed to connect to {ACUITY_NODE_URL}: {error}"))
}

/// Queries the free balance (in planck) for `address` using an already-open
/// client.  Returns `0` if the account does not exist yet.
pub(crate) async fn fetch_account_balance(
    client: &AcuityClient,
    address: &str,
) -> Result<u128, String> {
    use sp_core::{crypto::Ss58Codec, sr25519::Public};
    use subxt::utils::AccountId32;

    let public = Public::from_ss58check(address)
        .map_err(|e| format!("Invalid SS58 address: {e:?}"))?;
    let account_id = AccountId32(public.0);

    let at_block = client
        .at_current_block()
        .await
        .map_err(|e| format!("Failed to get latest block for balance query: {e}"))?;

    let storage_addr = api::storage().system().account();
    let maybe_value = at_block
        .storage()
        .try_fetch(storage_addr, (account_id,))
        .await
        .map_err(|e| format!("Balance storage query failed: {e}"))?;

    let Some(value_thunk) = maybe_value else {
        return Ok(0);
    };

    let account_info = value_thunk
        .decode()
        .map_err(|e| format!("Failed to decode account info: {e}"))?;

    Ok(account_info.data.free.into())
}

/// Estimates the partial fee (in planck) for submitting `call` signed by
/// `signer`.  Opens a fresh connection for the estimate.
pub(crate) async fn estimate_fee<Call>(
    call: &Call,
    signer: &SignerKeypair,
) -> Result<u128, String>
where
    Call: subxt::tx::Payload,
{
    let client = connect().await?;
    let at = client
        .at_current_block()
        .await
        .map_err(|e| format!("Failed to get latest block for fee estimation: {e}"))?;

    // Use offline signing (nonce defaults to 0) — the nonce has no effect on
    // fee calculation, and skipping the nonce RPC avoids a failure for new
    // accounts that have never submitted a transaction.
    let mut signable = at
        .tx()
        .create_signable_offline(call, Default::default())
        .map_err(|e| format!("Failed to build signable transaction for fee estimation: {e}"))?;

    let signed = signable
        .sign(signer)
        .map_err(|e| format!("Failed to sign transaction for fee estimation: {e}"))?;

    // Call TransactionPaymentApi_query_info via call_raw, decoding the response
    // as (Compact<u64>, Compact<u64>, u8, u64) — Acuity's RuntimeDispatchInfo
    // uses u64 for partial_fee, not u128 as assumed by partial_fee_estimate().
    let encoded_tx = signed.encoded();
    let mut params = encoded_tx.to_vec();
    (encoded_tx.len() as u32).encode_to(&mut params);

    let response = at
        .runtime_apis()
        .call_raw("TransactionPaymentApi_query_info", Some(&params))
        .await
        .map_err(|e| {
            let msg = format!("Fee estimation failed: {e}");
            tracing::warn!("{msg}");
            msg
        })?;

    // data layout: { weight: { ref_time: Compact<u64>, proof_size: Compact<u64> },
    //                class: u8, partial_fee: u64 }
    let (_, _, _, partial_fee) =
        <(Compact<u64>, Compact<u64>, u8, u64)>::decode(&mut response.as_ref())
            .map_err(|e| format!("Failed to decode fee info: {e}"))?;

    Ok(partial_fee as u128)
}
