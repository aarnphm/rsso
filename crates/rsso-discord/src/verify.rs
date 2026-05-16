use thiserror::Error;

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("discord public key is malformed")]
    InvalidPublicKey,
    #[error("discord request signature is missing")]
    MissingSignature,
    #[error("discord request signature is invalid")]
    InvalidSignature,
}

pub fn verify_discord_request(
    public_key_hex: &str,
    signature_hex: Option<&str>,
    timestamp: Option<&str>,
    body: &[u8],
) -> Result<(), VerifyError> {
    let signature = signature_hex.ok_or(VerifyError::MissingSignature)?;
    let timestamp = timestamp.ok_or(VerifyError::MissingSignature)?;
    verify_impl(public_key_hex, signature, timestamp, body)
}

#[cfg(not(target_arch = "wasm32"))]
fn verify_impl(
    public_key_hex: &str,
    signature: &str,
    timestamp: &str,
    body: &[u8],
) -> Result<(), VerifyError> {
    let public_key = decode_fixed::<32>(public_key_hex).ok_or(VerifyError::InvalidPublicKey)?;
    let verifier = serenity::interactions_endpoint::Verifier::try_new(public_key)
        .map_err(|_| VerifyError::InvalidPublicKey)?;
    verifier
        .verify(signature, timestamp, body)
        .map_err(|()| VerifyError::InvalidSignature)
}

#[cfg(target_arch = "wasm32")]
fn verify_impl(
    public_key_hex: &str,
    signature: &str,
    timestamp: &str,
    body: &[u8],
) -> Result<(), VerifyError> {
    let public_key = decode_fixed::<32>(public_key_hex).ok_or(VerifyError::InvalidPublicKey)?;
    let signature = decode_fixed::<64>(signature).ok_or(VerifyError::InvalidSignature)?;
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(&public_key)
        .map_err(|_| VerifyError::InvalidPublicKey)?;
    let signature = ed25519_dalek::Signature::from_bytes(&signature);
    let mut message = Vec::with_capacity(timestamp.len() + body.len());
    message.extend_from_slice(timestamp.as_bytes());
    message.extend_from_slice(body);
    ed25519_dalek::Verifier::verify(&verifying_key, &message, &signature)
        .map_err(|_| VerifyError::InvalidSignature)
}

fn decode_fixed<const N: usize>(value: &str) -> Option<[u8; N]> {
    let decoded = hex::decode(value).ok()?;
    decoded.try_into().ok()
}
