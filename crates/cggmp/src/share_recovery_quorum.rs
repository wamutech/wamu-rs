//! Share recovery with quorum implementation.
//!
//! Ref: <https://wamu.tech/specification#share-recovery-quorum>.

use curv::elliptic::curves::Secp256k1;
use multi_party_ecdsa::protocols::multi_party_ecdsa::gg_2020::state_machine::keygen::LocalKey;
use round_based::{Msg, StateMachine};
use std::collections::HashMap;
use std::time::Duration;
use wamu_core::crypto::VerifyingKey;
use wamu_core::{IdentityProvider, SigningShare, SubShare};

use crate::authorized_key_refresh::{AuthorizedKeyRefresh, AuthorizedKeyRefreshMessage, Error};
use crate::identity_auth;
use crate::identity_auth::IdentityAuthentication;
use crate::key_refresh::AugmentedKeyRefresh;

const SHARE_RECOVERY_QUORUM: &str = "share-recovery-quorum";

/// A [StateMachine](StateMachine) that implements [share recovery with a surviving quorum of honest parties as described by the Wamu protocol](https://wamu.tech/specification#share-recovery-quorum).
pub struct ShareRecoveryQuorum<'a, I: IdentityProvider> {
    // Identity authentication.
    /// The decentralized identity provider of the party.
    identity_provider: &'a I,
    /// Verifying keys for other the parties.
    verified_parties: &'a [VerifyingKey],
    /// Party index.
    idx: u16,
    /// Total number of parties.
    n_parties: u16,

    // Key refresh.
    /// The "signing share" of the party
    /// (only `None` for the recovering party, `Some` for all other parties).
    signing_share_option: Option<&'a SigningShare>,
    /// The "sub-share" of the party
    /// (only `None` for the recovering party, `Some` for all other parties).
    sub_share_option: Option<&'a SubShare>,
    /// Local key of the party (with secret share cleared/zerorized).
    local_key_option: Option<LocalKey<Secp256k1>>,
    /// Maps existing indices to new ones for refreshing parties.
    old_to_new_map: &'a HashMap<u16, u16>,
    /// The threshold.
    // NOTE: Quorum size = threshold + 1
    threshold: u16,

    // State machine management.
    /// Outgoing message queue.
    message_queue: Vec<Msg<AuthorizedKeyRefreshMessage<'a, I, identity_auth::Message>>>,
    /// Identity authentication state machine (must succeed before key refresh is performed).
    init_state_machine: IdentityAuthentication<'a, I>,
    /// Key refresh state machine (activated after successful identity authentication).
    refresh_state_machine: Option<AugmentedKeyRefresh<'a, I>>,
}

impl<'a, I: IdentityProvider> ShareRecoveryQuorum<'a, I> {
    /// Initializes party for the share recovery with quorum protocol.
    pub fn new(
        signing_share_option: Option<&'a SigningShare>,
        sub_share_option: Option<&'a SubShare>,
        identity_provider: &'a I,
        verified_parties: &'a [VerifyingKey],
        // `LocalKey<Secp256k1>` with secret share set to zero.
        local_key_option: Option<LocalKey<Secp256k1>>,
        party_index_option: Option<u16>,
        n_parties: u16,
        old_to_new_map: &'a HashMap<u16, u16>,
        // NOTE: Quorum size = threshold + 1
        current_threshold_option: Option<u16>,
    ) -> Result<
        ShareRecoveryQuorum<'a, I>,
        Error<'a, I, <IdentityAuthentication<'a, I> as StateMachine>::Err>,
    > {
        // Initializes identity authentication state machine.
        let idx = local_key_option
            .as_ref()
            .map(|it| it.i)
            .or(party_index_option)
            .ok_or(Error::InvalidInput)?;
        let init_state_machine = IdentityAuthentication::new(
            SHARE_RECOVERY_QUORUM,
            identity_provider,
            verified_parties,
            idx,
            n_parties,
            local_key_option.is_none(),
        );

        // Initializes share recovery state machine.
        let threshold = local_key_option
            .as_ref()
            .map(|it| it.t)
            .or(current_threshold_option)
            .ok_or(Error::InvalidInput)?;
        let mut share_recovery_quorum = Self {
            // Identity authentication.
            identity_provider,
            verified_parties,
            idx,
            n_parties,
            // Key refresh.
            signing_share_option,
            sub_share_option,
            local_key_option,
            old_to_new_map,
            threshold,
            // State machine management.
            message_queue: Vec::new(),
            init_state_machine,
            refresh_state_machine: None,
        };

        // Retrieves messages from immediate state transitions (if any) and wraps them.
        share_recovery_quorum.update_composite_message_queue()?;

        // Returns share recovery machine.
        Ok(share_recovery_quorum)
    }
}

impl<'a, I: IdentityProvider> AuthorizedKeyRefresh<'a, I> for ShareRecoveryQuorum<'a, I> {
    type InitStateMachineType = IdentityAuthentication<'a, I>;

    impl_required_authorized_key_refresh_getters!(
        init_state_machine,
        refresh_state_machine,
        message_queue
    );

    /// Initializes party for the key refresh protocol (if necessary).
    fn init_key_refresh(&mut self) -> Result<(), <Self as StateMachine>::Err> {
        if self.refresh_state_machine.is_none() {
            // Initializes key refresh state machine.
            let is_initiator = self.local_key_option.is_none();
            let key_refresh = AugmentedKeyRefresh::new(
                self.signing_share_option,
                self.sub_share_option,
                self.identity_provider,
                self.verified_parties,
                self.local_key_option.take(),
                is_initiator.then_some(self.idx),
                self.old_to_new_map,
                self.threshold,
                self.n_parties,
                is_initiator.then_some(self.threshold),
            )?;

            // Sets key refresh as the active state machine.
            self.refresh_state_machine = Some(key_refresh);

            // Retrieves messages from immediate state transitions (if any) and wraps them.
            self.update_composite_message_queue()?;
        }

        Ok(())
    }
}

impl_state_machine_for_authorized_key_refresh!(ShareRecoveryQuorum, idx, n_parties);

// Implement `Debug` trait for `ShareRecoveryQuorum` for test simulations.
#[cfg(test)]
impl<'a, I: IdentityProvider> std::fmt::Debug for ShareRecoveryQuorum<'a, I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Share Recovery Quorum")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm::{AugmentedType, SubShareOutput};
    use crate::keygen::tests::simulate_key_gen;
    use curv::elliptic::curves::Scalar;
    use round_based::dev::Simulation;

    pub fn simulate_share_recovery_quorum(
        // Party key configs including the "signing share", "sub-share", identity provider and
        // `LocalKey<Secp256k1>` from `multi-party-ecdsa` with the secret share cleared/zerorized.
        party_key_configs: Vec<(
            Option<&SigningShare>,
            Option<&SubShare>,
            &impl IdentityProvider,
            Option<LocalKey<Secp256k1>>,
            Option<u16>, // recovering party index,
            Option<u16>, // current threshold (needed by recovering party),
        )>,
        current_to_new_idx_map: &HashMap<u16, u16>,
        n_parties: u16,
    ) -> Vec<AugmentedType<LocalKey<Secp256k1>, SubShareOutput>> {
        // Creates simulation.
        let mut simulation = Simulation::new();

        // Creates a list of verifying keys for all parties.
        let verifying_keys: Vec<VerifyingKey> = party_key_configs
            .iter()
            .map(|(_, _, identity_provider, ..)| identity_provider.verifying_key())
            .collect();

        // Adds parties to simulation.
        for (
            signing_share,
            sub_share,
            identity_provider,
            local_key,
            recovering_party_index,
            current_threshold_option,
        ) in party_key_configs
        {
            simulation.add_party(
                ShareRecoveryQuorum::new(
                    signing_share,
                    sub_share,
                    identity_provider,
                    &verifying_keys,
                    local_key,
                    recovering_party_index,
                    n_parties,
                    current_to_new_idx_map,
                    current_threshold_option,
                )
                .unwrap(),
            );
        }

        // Runs simulation and returns output.
        simulation.run().unwrap()
    }

    #[test]
    fn share_recovery_quorum_works() {
        let threshold = 2;
        let n_parties = 4;
        let recovering_party_idx = 2u16;

        // Runs key gen simulation for test parameters.
        let (aug_keys, identity_providers) = simulate_key_gen(threshold, n_parties);
        // Verifies that we got enough keys and identities for "existing" parties from keygen.
        assert_eq!(aug_keys.len(), identity_providers.len());
        assert_eq!(aug_keys.len(), n_parties as usize);

        // Keep copy of current public key for later verification.
        let pub_key_init = aug_keys[0].base.public_key();

        // Creates key configs and party indices for all parties.
        let mut party_key_configs = Vec::new();
        let mut current_to_new_idx_map = HashMap::new();
        for (i, key) in aug_keys.iter().enumerate() {
            // Create party key config and index entry.
            let idx = i as u16 + 1;
            let (signing_share, sub_share) = key.extra.as_ref().unwrap();
            let local_key = key.base.clone();
            if idx == recovering_party_idx {
                party_key_configs.push((
                    None,
                    None,
                    &identity_providers[i],
                    None,
                    Some(local_key.i),
                    Some(threshold),
                ));
            } else {
                current_to_new_idx_map.insert(local_key.i, idx);
                party_key_configs.push((
                    Some(signing_share),
                    Some(sub_share),
                    &identity_providers[i],
                    Some(local_key),
                    None,
                    None,
                ));
            }
        }

        // Runs share recovery with quorum simulation for test parameters.
        let new_keys =
            simulate_share_recovery_quorum(party_key_configs, &current_to_new_idx_map, n_parties);

        // Verifies the refreshed/generated keys and configuration for all parties.
        assert_eq!(new_keys.len(), n_parties as usize);
        for (i, new_key) in new_keys.iter().enumerate() {
            // Verifies threshold and number of parties.
            assert_eq!(new_key.base.t, threshold);
            assert_eq!(new_key.base.n, n_parties);
            // Verifies that the secret share was cleared/zerorized.
            assert_eq!(new_key.base.keys_linear.x_i, Scalar::<Secp256k1>::zero());
            // Verifies that the public key hasn't changed.
            assert_eq!(new_key.base.public_key(), pub_key_init);
            // Verifies that the "signing share" and "sub-share" have changed.
            let (prev_signing_share, prev_sub_share) = aug_keys[i].extra.as_ref().unwrap();
            let (new_signing_share, new_sub_share) = new_key.extra.as_ref().unwrap();
            assert_ne!(
                new_signing_share.to_be_bytes(),
                prev_signing_share.to_be_bytes()
            );
            assert_ne!(new_sub_share.as_tuple(), prev_sub_share.as_tuple());
        }
    }
}
