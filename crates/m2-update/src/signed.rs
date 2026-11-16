//! The signed file: a line with the signature, then the manifest's bytes.

use crate::{Manifest, PublicKey, SigningKey, UpdateError};

/// What the signature line starts with.
pub const SIGNATURE_LINE: &str = "openm2-manifest ed25519 ";

/// The largest manifest, in bytes: 8 MiB.
pub const MAX_MANIFEST: usize = 8 << 20;

/// The signature line's length: its start, 128 hex digits and `\n`.
const LINE_LEN: usize = SIGNATURE_LINE.len() + 128 + 1;

/// The largest signed file, in bytes.
pub const MAX_SIGNED: usize = LINE_LEN + MAX_MANIFEST;

/// Signs a manifest's JSON with `key`: the signed file's bytes. The JSON
/// must be a valid manifest; it is kept byte for byte.
pub fn sign(json: &[u8], key: &SigningKey) -> Result<Vec<u8>, UpdateError> {
    Manifest::from_json(json)?;
    let signature = key.sign(json)?;
    let mut signed = Vec::with_capacity(LINE_LEN + json.len());
    signed.extend_from_slice(SIGNATURE_LINE.as_bytes());
    signed.extend_from_slice(crate::hex::encode(&signature).as_bytes());
    signed.push(b'\n');
    signed.extend_from_slice(json);
    Ok(signed)
}

/// Checks a signed file's signature with `key`, then reads its manifest.
pub fn open(signed: &[u8], key: &PublicKey) -> Result<Manifest, UpdateError> {
    if signed.len() > MAX_SIGNED {
        return Err(UpdateError::TooLarge {
            limit: crate::MAX_MANIFEST,
        });
    }
    let (line, json) = signed
        .split_at_checked(LINE_LEN)
        .ok_or(UpdateError::SignatureLine)?;
    let digits = line
        .strip_prefix(SIGNATURE_LINE.as_bytes())
        .and_then(|rest| rest.strip_suffix(b"\n"))
        .and_then(|digits| std::str::from_utf8(digits).ok())
        .ok_or(UpdateError::SignatureLine)?;
    let signature: [u8; 64] = crate::hex::decode(digits, true).ok_or(UpdateError::SignatureLine)?;
    if !key.verifies(json, &signature) {
        return Err(UpdateError::BadSignature);
    }
    Manifest::from_json(json)
}
