//! Augmented signing implementation.
//!
//! Ref: <https://wamu.tech/specification#signing>.

use cggmp_threshold_ecdsa::presign::{PresigningOutput, PresigningTranscript, SSID};
use cggmp_threshold_ecdsa::sign::state_machine::{Signing, M};
use curv::arithmetic::Converter;
use curv::elliptic::curves::{Scalar, Secp256k1};
use curv::BigInt;
use round_based::{Msg, StateMachine};
use std::collections::HashMap;
use std::time::Duration;
use wamu_core::crypto::VerifyingKey;
use wamu_core::{IdentityProvider, SigningShare, SubShare};

use crate::asm::{AugmentedStateMachine, AugmentedType, IdentityAuthParams};
use crate::errors::Error;

/// A wrapper around the [`cggmp-threshold-ecdsa` Signing StateMachine](https://github.com/webb-tools/cggmp-threshold-ecdsa/blob/main/src/sign/state_machine.rs) that [augments signing as described by the Wamu protocol](https://wamu.tech/specification#signing).
pub struct AugmentedSigning<'a, I: IdentityProvider> {
    /// Wrapped `cggmp-threshold-ecdsa` Signing `StateMachine`.
    state_machine: Signing,
    /// An augmented message queue.
    message_queue:
        Vec<Msg<AugmentedType<<Signing as StateMachine>::MessageBody, IdentityAuthParams>>>,
    /// The decentralized identity provider of the party.
    identity_provider: &'a I,
    /// Verifying keys for other the parties.
    verified_parties: &'a [VerifyingKey],
    /// A byte representation of the message to be signed.
    message: &'a [u8],
}

impl<'a, I: IdentityProvider> AugmentedSigning<'a, I> {
    /// Initializes party for the augmented signing protocol.
    pub fn new(
        signing_share: &SigningShare,
        sub_share: &SubShare,
        identity_provider: &'a I,
        verified_parties: &'a [VerifyingKey],
        message: &'a [u8],
        mut ssid: SSID<Secp256k1>,
        presigning_data: HashMap<
            u16,
            (PresigningOutput<Secp256k1>, PresigningTranscript<Secp256k1>),
        >,
        // l in the CGGMP20 paper.
        pre_signing_output_idx: usize,
    ) -> Result<Self, Error<<Signing as StateMachine>::Err>> {
        // Reconstructs secret share.
        let secret_share = wamu_core::share_split_reconstruct::reconstruct(
            signing_share,
            sub_share,
            identity_provider,
        )?;
        // Sets the reconstructed secret share.
        ssid.X.keys_linear.x_i = Scalar::<Secp256k1>::from_bytes(&secret_share.to_be_bytes())
            .map_err(|_| Error::Core(wamu_core::Error::Encoding))?;

        // Creates a SHA256 message digest.
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(message);
        let message_digest = hasher.finalize();

        // Initializes state machine.
        let mut aug_signing = Self {
            state_machine: Signing::new(
                ssid,
                pre_signing_output_idx,
                BigInt::from_bytes(&message_digest),
                presigning_data,
            )?,
            message_queue: Vec::new(),
            identity_provider,
            verified_parties,
            message,
        };

        // Retrieves messages from immediate state transitions (if any) and augments them.
        aug_signing.update_augmented_message_queue()?;

        // Returns augmented state machine.
        Ok(aug_signing)
    }
}

impl<'a, I: IdentityProvider> AugmentedStateMachine for AugmentedSigning<'a, I> {
    type StateMachineType = Signing;
    type AdditionalParams = IdentityAuthParams;
    type AdditionalOutput = ();

    // Implements all required `AugmentedStateMachine` methods.
    impl_required_augmented_state_machine_methods!(state_machine, message_queue);

    fn pre_handle_incoming(
        &mut self,
        msg: &Msg<
            AugmentedType<
                <Self::StateMachineType as StateMachine>::MessageBody,
                Self::AdditionalParams,
            >,
        >,
    ) -> Result<(), Error<<Self::StateMachineType as StateMachine>::Err>> {
        match msg.body.base.0 {
            // Verifies the expected additional parameters from Round 1.
            // Round 2 of `cggmp-threshold-ecdsa` Signing is the Output phase,
            M::Round1(_) => match msg.body.extra.as_ref() {
                Some(params) => {
                    // Verifies that signer is a verified party.
                    if !self.verified_parties.contains(&params.verifying_key) {
                        return Err(Error::Core(wamu_core::Error::UnauthorizedParty));
                    }
                    // Verifies that the signature is valid.
                    wamu_core::crypto::verify_signature(
                        &params.verifying_key,
                        &wamu_core::utils::prefix_message_bytes(self.message),
                        &params.verifying_signature,
                    )?;
                    Ok(())
                }
                // Returns an error if expected additional parameters are missing.
                None => Err(Error::MissingParams {
                    bad_actors: vec![msg.sender as usize],
                }),
            },
            // No modifications for other rounds.
            _ => Ok(()),
        }
    }

    fn augment_outgoing_message(
        &self,
        _: u16,
        msg_body: &<Self::StateMachineType as StateMachine>::MessageBody,
    ) -> Result<Option<Self::AdditionalParams>, Error<<Self::StateMachineType as StateMachine>::Err>>
    {
        match msg_body.0 {
            // Adds additional parameters to Round 1 messages.
            M::Round1(_) => Ok(Some(IdentityAuthParams {
                verifying_key: self.identity_provider.verifying_key(),
                verifying_signature: self
                    .identity_provider
                    .sign(&wamu_core::utils::prefix_message_bytes(self.message)),
            })),
            // No modifications for other rounds.
            _ => Ok(None),
        }
    }
}

// No additional output.
type AdditionalOutput = ();

// Implements `StateMachine` trait for `AugmentedSigning`.
impl_state_machine_for_augmented_state_machine!(
    AugmentedSigning,
    Signing,
    IdentityAuthParams,
    AdditionalOutput
);

// Implement `Debug` trait for `AugmentedSigning` for test simulations.
#[cfg(test)]
impl<'a, I: IdentityProvider> std::fmt::Debug for AugmentedSigning<'a, I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Augmented Signing")
    }
}

#[cfg(test)]
pub mod tests {
    use crate::asm::SubShareOutput;
    use cggmp_threshold_ecdsa::presign::state_machine::PreSigning;
    use cggmp_threshold_ecdsa::presign::PreSigningSecrets;
    use cggmp_threshold_ecdsa::sign::SigningOutput;
    use cggmp_threshold_ecdsa::utilities::sha2::Sha256;
    use curv::arithmetic::traits::{Modulo, One, Samplable};
    use curv::arithmetic::Integer;
    use curv::cryptographic_primitives::secret_sharing::feldman_vss::VerifiableSS;
    use curv::elliptic::curves::{Point, Scalar};
    use fs_dkr::ring_pedersen_proof::RingPedersenStatement;
    use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2020::state_machine::keygen::LocalKey;
    use round_based::dev::Simulation;
    use wamu_core::test_utils::MockECDSAIdentityProvider;

    use super::*;
    use crate::keygen::tests::simulate_key_gen;

    fn simulate_sign(
        keys_and_pre_signing_output: Vec<(
            &SigningShare,
            &SubShare,
            &MockECDSAIdentityProvider,
            SSID<Secp256k1>,
            HashMap<u16, (PresigningOutput<Secp256k1>, PresigningTranscript<Secp256k1>)>,
        )>,
        message: &[u8],
        pre_signing_output_idx: usize,
    ) -> Vec<AugmentedType<Option<SigningOutput<Secp256k1>>, AdditionalOutput>> {
        // Creates simulation.
        let mut simulation = Simulation::new();

        // Creates a list of verifying keys for all parties.
        let verifying_keys: Vec<VerifyingKey> = keys_and_pre_signing_output
            .iter()
            .map(|(_, _, identity_provider, ..)| identity_provider.verifying_key())
            .collect();

        // Adds parties to simulation.
        for (signing_share, sub_share, identity_provider, ssid, pre_signing_data) in
            keys_and_pre_signing_output.into_iter()
        {
            // Add party to simulation.
            simulation.add_party(
                AugmentedSigning::new(
                    signing_share,
                    sub_share,
                    identity_provider,
                    &verifying_keys,
                    message,
                    ssid.clone(),
                    pre_signing_data.clone(),
                    pre_signing_output_idx,
                )
                .unwrap(),
            );
        }

        // Runs simulation and returns output.
        simulation.run().unwrap()
    }

    fn simulate_pre_sign(
        inputs: Vec<(
            SSID<Secp256k1>,
            PreSigningSecrets,
            HashMap<u16, BigInt>,
            HashMap<u16, BigInt>,
            HashMap<u16, BigInt>,
        )>,
        pre_signing_output_idx: usize,
    ) -> Vec<Option<(PresigningOutput<Secp256k1>, PresigningTranscript<Secp256k1>)>> {
        // Creates simulation.
        let mut simulation = Simulation::new();

        // Adds parties to simulation.
        for (
            ssid,
            secrets,
            aux_ring_pedersen_n_hat_values,
            aux_ring_pedersen_s_values,
            aux_ring_pedersen_t_values,
        ) in inputs.into_iter()
        {
            // Add party to simulation.
            simulation.add_party(
                PreSigning::new(
                    ssid,
                    secrets,
                    aux_ring_pedersen_s_values,
                    aux_ring_pedersen_t_values,
                    aux_ring_pedersen_n_hat_values,
                    pre_signing_output_idx,
                )
                .unwrap(),
            );
        }

        // Runs simulation and returns output.
        simulation.run().unwrap()
    }

    fn generate_pre_sign_input(
        aug_keys: &[AugmentedType<LocalKey<Secp256k1>, SubShareOutput>],
        identity_providers: &[MockECDSAIdentityProvider],
    ) -> Vec<(
        SSID<Secp256k1>,
        PreSigningSecrets,
        HashMap<u16, BigInt>,
        HashMap<u16, BigInt>,
        HashMap<u16, BigInt>,
    )> {
        // Generates auxiliary "ring" Pedersen parameters for all parties.
        let mut aux_ring_pedersen_n_hat_values = HashMap::with_capacity(aug_keys.len());
        let mut aux_ring_pedersen_s_values = HashMap::with_capacity(aug_keys.len());
        let mut aux_ring_pedersen_t_values = HashMap::with_capacity(aug_keys.len());
        for idx in 1..=aug_keys.len() as u16 {
            let (ring_pedersen_params, _) = RingPedersenStatement::<Secp256k1, Sha256>::generate();
            aux_ring_pedersen_n_hat_values.insert(idx, ring_pedersen_params.N);
            aux_ring_pedersen_s_values.insert(idx, ring_pedersen_params.S);
            aux_ring_pedersen_t_values.insert(idx, ring_pedersen_params.T);
        }
        // Reconstructs secret shares, creates pre-signing inputs and auxiliary parameters for ZK proofs.
        let generator = Point::<Secp256k1>::generator().to_point();
        let group_order = Scalar::<Secp256k1>::group_order();
        let party_indices: Vec<u16> = (1..=aug_keys.len() as u16).collect();
        aug_keys
            .iter()
            .enumerate()
            .map(|(i, aug_key)| {
                // Reconstructs secret share and update local key.
                let (signing_share, sub_share) = aug_key.extra.as_ref().unwrap();
                let secret_share = wamu_core::share_split_reconstruct::reconstruct(
                    signing_share,
                    sub_share,
                    &identity_providers[i],
                )
                .unwrap();
                let mut local_key = aug_key.base.clone();
                local_key.keys_linear.x_i =
                    Scalar::<Secp256k1>::from_bytes(&secret_share.to_be_bytes()).unwrap();

                // Creates SSID and pre-signing secrets.
                // We already have Paillier keys from GG20 key gen or FS-DKR so we just reuse them.
                let paillier_ek = local_key.paillier_key_vec[local_key.i as usize - 1].clone();
                let paillier_dk = local_key.paillier_dk.clone();
                // See Figure 6, Round 1.
                // Ref: <https://eprint.iacr.org/2021/060.pdf>.
                let phi = (&paillier_dk.p - BigInt::one()) * (&paillier_dk.q - BigInt::one());
                let r = BigInt::sample_below(&paillier_ek.n);
                let lambda = BigInt::sample_below(&phi);
                let t = BigInt::mod_pow(&r, &BigInt::from(2), &paillier_ek.n);
                let s = BigInt::mod_pow(&t, &lambda, &paillier_ek.n);
                // Composes SSID.
                let ssid = SSID {
                    g: generator.clone(),
                    q: group_order.clone(),
                    P: party_indices.clone(),
                    rid: wamu_core::crypto::RandomBytes::generate().to_be_bytes(),
                    X: local_key,
                    Y: None, // Y is not needed for 4-round signing.
                    N: paillier_ek.n.clone(),
                    S: s,
                    T: t,
                };
                // Composes pre-signing secrets.
                let pre_sign_secrets = PreSigningSecrets {
                    x_i: BigInt::from_bytes(&secret_share.to_be_bytes()),
                    y_i: None, // Y is not needed for 4-round signing.
                    ek: paillier_ek,
                    dk: paillier_dk,
                };

                (
                    ssid,
                    pre_sign_secrets,
                    aux_ring_pedersen_n_hat_values.clone(),
                    aux_ring_pedersen_s_values.clone(),
                    aux_ring_pedersen_t_values.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn sign_works() {
        // Iterates over parameters for creating test cases with different thresholds and number of parties.
        // NOTE: Quorum size = threshold + 1
        for (threshold, n_parties) in [
            // 2/2 signing keys.
            (1, 2),
            // 3/4 signing keys.
            (2, 4),
        ] {
            // Runs key gen simulation for test parameters.
            let (aug_keys, identity_providers) = simulate_key_gen(threshold, n_parties);
            // Verifies that we got enough keys and identities for "existing" parties from keygen.
            assert_eq!(aug_keys.len(), identity_providers.len());
            assert_eq!(aug_keys.len(), n_parties as usize);

            // Extracts and verifies the shared secret key.
            let secret_shares: Vec<Scalar<Secp256k1>> = aug_keys
                .iter()
                .enumerate()
                .map(|(idx, it)| {
                    let (signing_share, sub_share) = it.extra.as_ref().unwrap();
                    Scalar::<Secp256k1>::from_bytes(
                        &wamu_core::share_split_reconstruct::reconstruct(
                            signing_share,
                            sub_share,
                            &identity_providers[idx],
                        )
                        .unwrap()
                        .to_be_bytes(),
                    )
                    .unwrap()
                })
                .collect();
            let sec_key = aug_keys[0].base.vss_scheme.reconstruct(
                &(0..n_parties).collect::<Vec<u16>>(),
                &secret_shares.clone(),
            );
            let pub_key = aug_keys[0].base.public_key();
            assert_eq!(Point::<Secp256k1>::generator() * &sec_key, pub_key);

            // Verifies that transforming of x_i, which is a (t,n) share of x, into a (t,t+1) share omega_i using
            // an appropriate lagrangian coefficient lambda_{i,S} as defined by GG18 and GG20 works.
            // Ref: https://eprint.iacr.org/2021/060.pdf (Section 1.2.8)
            // Ref: https://eprint.iacr.org/2019/114.pdf (Section 4.2)
            // Ref: https://eprint.iacr.org/2020/540.pdf (Section 3.2)
            let omega_shares: Vec<Scalar<Secp256k1>> = aug_keys
                .iter()
                .enumerate()
                .map(|(idx, it)| {
                    let x_i = secret_shares[idx].clone();
                    let lambda_i_s = VerifiableSS::<Secp256k1, Sha256>::map_share_to_new_params(
                        &it.base.vss_scheme.parameters,
                        it.base.i - 1,
                        &(0..it.base.n).collect::<Vec<u16>>(),
                    );
                    lambda_i_s * x_i
                })
                .collect();
            let omega_sec_key = omega_shares
                .iter()
                .fold(Scalar::<Secp256k1>::zero(), |acc, x| acc + x);
            assert_eq!(omega_sec_key, sec_key);

            // Runs pre-signing simulation for test parameters and verifies the results.
            let pre_signing_output_idx = 1; // l in the CGGMP20 paper.
            let pre_sign_inputs = generate_pre_sign_input(&aug_keys, &identity_providers);
            let ssids: Vec<SSID<Secp256k1>> = pre_sign_inputs
                .iter()
                .map(|(ssid, ..)| ssid.clone())
                .collect();
            let pre_sign_results = simulate_pre_sign(pre_sign_inputs, pre_signing_output_idx);
            // Verifies that r, the x projection of R = g^k-1 is computed correctly.
            let q = Scalar::<Secp256k1>::group_order();
            let r_dist = pre_sign_results[0].as_ref().unwrap().0.R.x_coord().unwrap();
            let k = Scalar::<Secp256k1>::from_bigint(
                &pre_sign_results
                    .iter()
                    .filter_map(|it| it.as_ref().map(|(output, _)| output.k_i.clone()))
                    .fold(BigInt::from(0), |acc, x| BigInt::mod_add(&acc, &x, q)),
            );
            let r_direct = (Point::<Secp256k1>::generator() * k.invert().unwrap())
                .x_coord()
                .unwrap();
            assert_eq!(r_dist, r_direct);
            // Verifies that chi_i are additive shares of kx.
            let k_x = &k * &sec_key;
            let chi_i_sum = Scalar::<Secp256k1>::from_bigint(
                &pre_sign_results
                    .iter()
                    .filter_map(|it| it.as_ref().map(|(output, _)| output.chi_i.clone()))
                    .fold(BigInt::from(0), |acc, x| BigInt::mod_add(&acc, &x, q)),
            );
            assert_eq!(k_x, chi_i_sum);

            // Creates inputs for signing simulation based on test parameters and pre-signing outputs.
            let message = b"Hello, world!";
            // Creates signing parameters.
            let signing_keys_and_pre_signing_output: Vec<(
                &SigningShare,
                &SubShare,
                &MockECDSAIdentityProvider,
                SSID<Secp256k1>,
                HashMap<u16, (PresigningOutput<Secp256k1>, PresigningTranscript<Secp256k1>)>,
            )> = pre_sign_results
                .into_iter()
                .filter_map(|it| {
                    it.map(|(output, transcript)| {
                        let idx = output.i as usize - 1;
                        let aug_key = &aug_keys[idx];
                        let (signing_share, sub_share) = aug_key.extra.as_ref().unwrap();
                        (
                            signing_share,
                            sub_share,
                            &identity_providers[idx],
                            ssids[idx].clone(),
                            HashMap::from([(pre_signing_output_idx as u16, (output, transcript))]),
                        )
                    })
                })
                .collect();

            // Runs signing simulation for test parameters and verifies the output signature.
            let results = simulate_sign(signing_keys_and_pre_signing_output, message, pre_signing_output_idx);
            // Extracts signature from results.
            let signature = results[0]
                .base
                .as_ref()
                .map(|it| (it.r.clone(), it.sigma.clone()))
                .unwrap();
            // Create SHA256 message digest.
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(message);
            let message_digest = BigInt::from_bytes(&hasher.finalize());
            let s_direct = (k.to_bigint() * (message_digest + (&r_direct * &sec_key.to_bigint())))
                .mod_floor(q);
            let expected_signature = (r_direct, s_direct);
            // Compares expected signature
            assert_eq!(signature, expected_signature);
        }
    }
}
