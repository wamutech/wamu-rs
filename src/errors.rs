//! Types and abstractions for protocol errors.

/// A protocol error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// A signature from an unauthorized party.
    UnauthorizedParty,
    /// A cryptography error.
    Crypto(CryptoError),
}

impl From<CryptoError> for Error {
    fn from(error: CryptoError) -> Self {
        Self::Crypto(error)
    }
}

/// A cryptography error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CryptoError {
    /// An invalid signature for the message.
    InvalidSignature,
    /// An invalid verifying key.
    InvalidVerifyingKey,
    /// An unsupported signature algorithm.
    UnsupportedSignatureAlgorithm,
    /// An unsupported hash function.
    UnsupportedHashFunction,
    /// An unsupported elliptic curve.
    UnsupportedEllipticCurve,
    /// An unsupported key encoding standard.
    UnsupportedKeyEncoding,
    /// An unsupported signature encoding standard.
    UnsupportedSignatureEncoding,
    /// A signature algorithm mismatch (e.g between the verifying key and signature).
    SignatureAlgorithmMismatch,
    /// An elliptic curve mismatch (e.g between the verifying key and signature).
    EllipticCurveMismatch,
}

/// An identity authenticated request verification error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityAuthedRequestError {
    /// Not the expected command.
    CommandMismatch,
    /// An expired request i.e initiated too far in the past.
    Expired,
    /// A request with an invalid timestamp i.e a timestamp too far in the future.
    InvalidTimestamp,
    /// A request with either an invalid signature or an unauthorized signer.
    Unauthorized(Error),
}

impl From<Error> for IdentityAuthedRequestError {
    fn from(error: Error) -> Self {
        Self::Unauthorized(error)
    }
}

impl From<CryptoError> for IdentityAuthedRequestError {
    fn from(error: CryptoError) -> Self {
        Self::Unauthorized(Error::Crypto(error))
    }
}

/// An identity authenticated request verification error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuorumApprovedRequestError {
    /// Not enough approvals to form a quorum.
    InsufficientApprovals,
    /// A request with either an invalid signature or an unauthorized signer.
    Unauthorized(Error),
}

impl From<Error> for QuorumApprovedRequestError {
    fn from(error: Error) -> Self {
        Self::Unauthorized(error)
    }
}

impl From<CryptoError> for QuorumApprovedRequestError {
    fn from(error: CryptoError) -> Self {
        Self::Unauthorized(Error::Crypto(error))
    }
}

/// A share backup or recovery error.
#[derive(Debug)]
pub enum ShareBackupRecoveryError {
    /// Encrypted data can't be converted into a valid signing share e.g decrypted output that's not 32 bytes long.
    InvalidSigningShare,
    /// Encrypted data can't be converted into a valid sub share e.g decrypted output that's not 32 bytes long.
    InvalidSubShare,
    /// An encryption/decryption error.
    EncryptionError(aes_gcm::Error),
}

impl From<aes_gcm::Error> for ShareBackupRecoveryError {
    fn from(error: aes_gcm::Error) -> Self {
        ShareBackupRecoveryError::EncryptionError(error)
    }
}
