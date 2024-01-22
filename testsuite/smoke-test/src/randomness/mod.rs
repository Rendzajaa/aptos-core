// Copyright © Aptos Foundation

use anyhow::{anyhow, bail, ensure};
use anyhow::Result;
use aptos_crypto::{compat::Sha3_256, Uniform};
use aptos_forge::LocalSwarm;
use aptos_rest_client::Client;
use aptos_types::{randomness::{PerBlockRandomness, RandMetadataToSign}, validator_verifier::ValidatorVerifier, ValidatorConsensusInfoMoveStruct};
use digest::Digest;
use move_core_types::{account_address::AccountAddress, language_storage::CORE_CODE_ADDRESS};
use num_traits::Zero;
use rand::{prelude::StdRng, SeedableRng};
use std::{collections::HashMap, time::Duration};
use tokio::time::Instant;
use aptos_logger::info;
use aptos_types::dkg::{DKG, DKGSessionState, DKGState, DKGTrait};
use aptos_types::on_chain_config::OnChainConfig;
use aptos_types::validator_verifier::ValidatorConsensusInfo;

mod dkg_basic;
mod dkg_feature_flag_flips;
mod dkg_with_validator_down;
mod dkg_with_validator_join_leave;
mod e2e_correctness;
mod validator_restart_during_dkg;

async fn get_current_version(rest_client: &Client) -> u64 {
    rest_client
        .get_ledger_information()
        .await
        .unwrap()
        .inner()
        .version
}

async fn get_on_chain_resource<T: OnChainConfig>(rest_client: &Client) -> T {
    let maybe_response = rest_client
        .get_account_resource_bcs::<T>(CORE_CODE_ADDRESS, T::struct_tag().to_string().as_str())
        .await;
    let response = maybe_response.unwrap();
    response.into_inner()
}

async fn get_on_chain_resource_at_version<T: OnChainConfig>(
    rest_client: &Client,
    version: u64,
) -> T {
    let maybe_response = rest_client
        .get_account_resource_at_version_bcs::<T>(
            CORE_CODE_ADDRESS,
            T::struct_tag().to_string().as_str(),
            version,
        )
        .await;
    let response = maybe_response.unwrap();
    response.into_inner()
}

/// Poll the on-chain state until we see a DKG session finishes.
/// Return a `DKGSessionState` of the DKG session seen.
async fn wait_for_dkg_finish(
    client: &Client,
    target_epoch: Option<u64>,
    time_limit_secs: u64,
) -> DKGSessionState {
    let mut dkg_state = get_on_chain_resource::<DKGState>(client).await;
    let timer = Instant::now();
    while timer.elapsed().as_secs() < time_limit_secs
        && !(dkg_state.in_progress.is_none()
            && dkg_state.last_completed.is_some()
            && (target_epoch.is_none()
                || dkg_state
                    .last_completed
                    .as_ref()
                    .map(|session| session.metadata.dealer_epoch + 1)
                    == target_epoch))
    {
        std::thread::sleep(Duration::from_secs(1));
        dkg_state = get_on_chain_resource::<DKGState>(client).await;
    }
    assert!(timer.elapsed().as_secs() < time_limit_secs);
    dkg_state.last_complete().clone()
}

/// Verify that DKG transcript of epoch i (stored in `new_dkg_state`) is correctly generated
/// by the validator set in epoch i-1 (stored in `new_dkg_state`).
fn verify_dkg_transcript(
    dkg_session: &DKGSessionState,
    decrypt_key_map: &HashMap<AccountAddress, <DKG as DKGTrait>::NewValidatorDecryptKey>,
) -> Result<()> {
    info!(
        "Verifying the transcript generated in epoch {}.",
        dkg_session.metadata.dealer_epoch,
    );
    let pub_params = DKG::new_public_params(&dkg_session.metadata);
    let transcript = bcs::from_bytes(dkg_session.transcript.as_slice())
        .map_err(|e|anyhow!("DKG transcript verification failed with transcript deserialization error: {e}"))?;
    println!("transcript={:?}", transcript);
    DKG::verify_transcript(&pub_params, &transcript)?;

    info!("Double-verifying by reconstructing the dealt secret.");
    let dealt_secret_from_shares = dealt_secret_from_shares(
        dkg_session.metadata.target_validator_consensus_infos_cloned(),
        decrypt_key_map,
        &pub_params,
        &transcript,
    );

    println!("dealt_secret_from_shares={:?}", dealt_secret_from_shares);

    let dealt_secret_from_inputs = dealt_secret_from_input(
        &transcript,
        dkg_session.metadata.dealer_validator_set.clone().into_iter().map(|obj| obj.try_into().unwrap()).collect(),
        decrypt_key_map,
    );
    println!("dealt_secret_from_inputs={:?}", dealt_secret_from_inputs);

    ensure!(dealt_secret_from_shares == dealt_secret_from_inputs, "dkg transcript verification failed with final check failure");
    Ok(())
}

fn dealt_secret_from_shares(
    target_validator_set: Vec<ValidatorConsensusInfo>,
    decrypt_key_map: &HashMap<AccountAddress, <DKG as DKGTrait>::NewValidatorDecryptKey>,
    pub_params: &<DKG as DKGTrait>::PublicParams,
    transcript: &<DKG as DKGTrait>::Transcript,
) -> <DKG as DKGTrait>::DealtSecret {
    let player_share_pairs = target_validator_set
        .iter()
        .enumerate()
        .map(|(idx, validator_info)| {
            let dk = decrypt_key_map.get(&validator_info.address).unwrap();
            let secret_key_share =
                DKG::decrypt_secret_share_from_transcript(pub_params, transcript, idx as u64, dk).unwrap();
            (idx as u64, secret_key_share)
        })
        .collect();

    DKG::reconstruct_secret_from_shares(&pub_params, player_share_pairs).unwrap()
}

fn dealt_secret_from_input(
    trx: &<DKG as DKGTrait>::Transcript,
    dealer_validator_set: Vec<ValidatorConsensusInfo>,
    decrypt_key_map: &HashMap<AccountAddress, <DKG as DKGTrait>::DealerPrivateKey>,
) -> <DKG as DKGTrait>::DealtSecret {
    let dealers = DKG::get_dealers(trx);
    println!("dealers={:?}", dealers);
    let input_secrets = dealers.into_iter().map(|dealer_idx|{
        let dealer_sk = decrypt_key_map.get(&dealer_validator_set[dealer_idx as usize].address).unwrap();
        DKG::generate_predictable_input_secret_for_testing(dealer_sk)
    }).collect();

    let aggregated_input_secret = DKG::aggregate_input_secret(input_secrets);
    DKG::dealt_secret_from_input(&aggregated_input_secret)
}

fn num_validators(dkg_state: &DKGSessionState) -> usize {
    dkg_state.metadata.target_validator_set.len()
}

fn decrypt_key_map(swarm: &LocalSwarm) -> HashMap<AccountAddress, <DKG as DKGTrait>::NewValidatorDecryptKey> {
    swarm
        .validators()
        .map(|validator| {
            let dk = validator
                .config()
                .consensus
                .safety_rules
                .initial_safety_rules_config
                .identity_blob()
                .unwrap()
                .try_into_dkg_new_validator_decrypt_key()
                .unwrap();
            (validator.peer_id(), dk)
        })
        .collect::<HashMap<_, _>>()
}
