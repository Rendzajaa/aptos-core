// Copyright © Aptos Foundation
// Parts of the project are originally copyright © Meta Platforms, Inc.
// SPDX-License-Identifier: Apache-2.0

use aptos_crypto::{
    ed25519::{Ed25519PrivateKey, Ed25519PublicKey},
    hash::{CryptoHasher as _, TestOnlyHasher},
    multi_ed25519::{MultiEd25519PublicKey, MultiEd25519Signature},
    secp256k1_ecdsa, secp256r1_ecdsa,
    traits::{SigningKey, Uniform},
};
use aptos_crypto_derive::{BCSCryptoHash, CryptoHasher};
use aptos_types::{
    contract_event, event,
    state_store::{
        state_key::StateKey,
        state_value::{PersistedStateValueMetadata, StateValueMetadata},
    },
    transaction,
    validator_txn::ValidatorTransaction,
    write_set,
};
use move_core_types::language_storage;
use rand::{rngs::StdRng, SeedableRng};
use serde::{Deserialize, Serialize};
use serde_reflection::{Registry, Result, Samples, Tracer, TracerConfig};

/// Default output file.
pub fn output_file() -> Option<&'static str> {
    Some("tests/staged/aptos.yaml")
}

/// This aims at signing canonically serializable BCS data
#[derive(CryptoHasher, BCSCryptoHash, Serialize, Deserialize)]
struct TestAptosCrypto(String);

/// Record sample values for crypto types used by transactions.
fn trace_crypto_values(tracer: &mut Tracer, samples: &mut Samples) -> Result<()> {
    let mut hasher = TestOnlyHasher::default();
    hasher.update(b"Test message");
    let hashed_message = hasher.finish();

    let message = TestAptosCrypto("Hello, World".to_string());

    let mut rng: StdRng = SeedableRng::from_seed([0; 32]);
    let private_key = Ed25519PrivateKey::generate(&mut rng);
    let public_key: Ed25519PublicKey = (&private_key).into();
    let signature = private_key.sign(&message).unwrap();

    tracer.trace_value(samples, &hashed_message)?;
    tracer.trace_value(samples, &public_key)?;
    tracer.trace_value::<MultiEd25519PublicKey>(samples, &public_key.into())?;
    tracer.trace_value(samples, &signature)?;
    tracer.trace_value::<MultiEd25519Signature>(samples, &signature.into())?;

    let secp256k1_private_key = secp256k1_ecdsa::PrivateKey::generate(&mut rng);
    let secp256k1_public_key = aptos_crypto::PrivateKey::public_key(&secp256k1_private_key);
    let secp256k1_signature = secp256k1_private_key.sign(&message).unwrap();
    tracer.trace_value(samples, &secp256k1_private_key)?;
    tracer.trace_value(samples, &secp256k1_public_key)?;
    tracer.trace_value(samples, &secp256k1_signature)?;

    let secp256r1_ecdsa_private_key = secp256r1_ecdsa::PrivateKey::generate(&mut rng);
    let secp256r1_ecdsa_public_key =
        aptos_crypto::PrivateKey::public_key(&secp256r1_ecdsa_private_key);
    let secp256r1_ecdsa_signature = secp256r1_ecdsa_private_key.sign(&message).unwrap();
    tracer.trace_value(samples, &secp256r1_ecdsa_private_key)?;
    tracer.trace_value(samples, &secp256r1_ecdsa_public_key)?;
    tracer.trace_value(samples, &secp256r1_ecdsa_signature)?;

    Ok(())
}

pub fn get_registry() -> Result<Registry> {
    let mut tracer =
        Tracer::new(TracerConfig::default().is_human_readable(bcs::is_human_readable()));
    let mut samples = Samples::new();
    // 1. Record samples for types with custom deserializers.
    trace_crypto_values(&mut tracer, &mut samples)?;
    tracer.trace_value(&mut samples, &event::EventKey::random())?;
    tracer.trace_value(&mut samples, &write_set::WriteOp::Deletion {
        metadata: StateValueMetadata::none(),
    })?;

    // 2. Trace the main entry point(s) + every enum separately.
    tracer.trace_type::<contract_event::ContractEvent>(&samples)?;
    tracer.trace_type::<language_storage::TypeTag>(&samples)?;
    tracer.trace_type::<ValidatorTransaction>(&samples)?;
    tracer.trace_type::<transaction::Transaction>(&samples)?;
    tracer.trace_type::<transaction::TransactionArgument>(&samples)?;
    tracer.trace_type::<transaction::TransactionPayload>(&samples)?;
    tracer.trace_type::<transaction::WriteSetPayload>(&samples)?;
    tracer.trace_type::<StateKey>(&samples)?;
    tracer.trace_type::<PersistedStateValueMetadata>(&samples)?;

    tracer.trace_type::<transaction::authenticator::AccountAuthenticator>(&samples)?;
    tracer.trace_type::<transaction::authenticator::TransactionAuthenticator>(&samples)?;
    tracer.trace_type::<transaction::authenticator::AnyPublicKey>(&samples)?;
    tracer.trace_type::<transaction::authenticator::AnySignature>(&samples)?;
    tracer.trace_type::<write_set::WriteOp>(&samples)?;
    tracer.registry()
}