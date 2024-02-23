use crate::vuf::VUF;
use anyhow::{anyhow, ensure};
use ark_bls12_381::{Bls12_381, Fq12, Fr, G1Affine, G2Affine, G2Projective};
use ark_ec::{
    hashing::HashToCurve, pairing::Pairing, short_weierstrass::Projective, AffineRepr, CurveGroup,
    Group,
};
use ark_ff::Field;
use ark_serialize::{CanonicalDeserialize, CanonicalSerialize};
use ark_std::{
    rand::{CryptoRng, RngCore},
    UniformRand,
};
use std::ops::Mul;

pub struct Scheme {}

pub static DST: &[u8] = b"APTOS_OIDB_VUF_SCHEME0_DST";

impl Scheme {
    fn hash_to_g1(input: &[u8]) -> G1Affine {
        let mapper = ark_ec::hashing::map_to_curve_hasher::MapToCurveBasedHasher::<
            Projective<ark_bls12_381::g1::Config>,
            ark_ff::fields::field_hashers::DefaultFieldHasher<sha2_0_10_6::Sha256, 128>,
            ark_ec::hashing::curve_maps::wb::WBMap<ark_bls12_381::g1::Config>,
        >::new(DST)
        .unwrap();
        mapper.hash(input).unwrap()
    }
}

impl VUF for Scheme {
    fn scheme_name() -> String {
        "Scheme0".to_string()
    }

    fn setup<R: CryptoRng + RngCore>(rng: &mut R) -> (Vec<u8>, Vec<u8>) {
        let sk = Fr::rand(rng);
        let pk = G2Affine::generator() * sk;
        let mut sk_bytes = vec![];
        let mut pk_bytes = vec![];
        sk.serialize_compressed(&mut sk_bytes).unwrap_or_default();
        pk.serialize_compressed(&mut pk_bytes).unwrap_or_default();
        (sk_bytes, pk_bytes)
    }

    fn pk_from_sk(sk: &[u8]) -> anyhow::Result<Vec<u8>> {
        let sk_scalar = Fr::deserialize_compressed(sk).map_err(|e| {
            anyhow!("vuf::scheme0::pk_from_sk failed with sk deserialization error: {e}")
        })?;
        let pk_g2 = G2Projective::generator() * sk_scalar;
        let mut buf = vec![];
        pk_g2
            .into_affine()
            .serialize_compressed(&mut buf)
            .map_err(|e| {
                anyhow!("vuf::scheme0::pk_from_sk failed with pk serialization error: {e}")
            })?;
        Ok(buf)
    }

    fn eval(sk: &[u8], input: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
        let sk_scalar = Fr::deserialize_uncompressed(sk)
            .map_err(|e| anyhow!("vuf::scheme0::eval failed with ok deserialization error: {e}"))?;
        let input_g1 = Self::hash_to_g1(input);
        let output_g1 = input_g1.mul(sk_scalar).into_affine();
        let mut output_bytes = vec![];
        output_g1
            .serialize_compressed(&mut output_bytes)
            .map_err(|e| {
                anyhow!("vuf::scheme0::eval failed with output serialization error: {e}")
            })?;
        Ok((output_bytes, vec![]))
    }

    fn verify(pk: &[u8], input: &[u8], output: &[u8], proof: &[u8]) -> anyhow::Result<()> {
        ensure!(
            proof.is_empty(),
            "vuf::scheme0::verify failed with proof deserialization error"
        );
        let input_g1 = Self::hash_to_g1(input);
        let pk_g2 = G2Affine::deserialize_compressed(pk).map_err(|e| {
            anyhow!("vuf::scheme0::verify failed with pk deserialization error: {e}")
        })?;
        let output_g1 = G1Affine::deserialize_compressed(output).map_err(|e| {
            anyhow!("vuf::scheme0::verify failed with output deserialization error: {e}")
        })?;
        ensure!(
            Fq12::ONE
                == Bls12_381::multi_pairing([-output_g1, input_g1], [G2Affine::generator(), pk_g2])
                    .0,
            "vuf::scheme0::verify failed with final check failure"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::vuf::{scheme0::Scheme, VUF};

    #[test]
    fn gen_eval_verify() {
        let mut rng = ark_std::rand::thread_rng();
        let (sk, pk) = Scheme::setup(&mut rng);
        let pk_another = Scheme::pk_from_sk(&sk).unwrap();
        assert_eq!(pk_another, pk);
        let input: &[u8] = b"hello world again and again and again and again and again and again";
        let (output, proof) = Scheme::eval(&sk, input).unwrap();
        Scheme::verify(&pk, input, &output, &proof).unwrap();
        println!("output={:?}", output);
    }
}