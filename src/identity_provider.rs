//! Types and abstractions for [decentralized identities](https://ethereum.org/en/decentralized-identity/).

use crate::crypto::{Signature, VerifyingKey};

/// Interface for an identity provider.
///
/// **NOTE:** For interoperability with existing wallet solutions,
/// the only requirement for decentralized identity providers is
/// the ability to compute cryptographic signatures for any arbitrary message in such a way that
/// the output signature can be verified in a non-interactive manner.
pub trait IdentityProvider {
    /// Returns the verifying key (i.e public key or address) for the identity.
    fn verifying_key(&self) -> VerifyingKey;

    /// Computes signature for a message.
    fn sign(&self, msg: &[u8]) -> Signature;

    /// Computes signature for a message and returns (`r`, `s`) as (`[u8; 32]`, `[u8; 32]`).
    fn sign_message_share(&self, msg: &[u8]) -> ([u8; 32], [u8; 32]);
}
