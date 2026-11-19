//! Checking a device's key (`docs/specs/server/session.md`, "Signing in
//! with a device key"): the signature must be the key's over the nonce the
//! core gave out and the connection's own TLS channel binding, so a
//! signature taken from one connection is worth nothing on another.

use aws_lc_rs::signature::{ED25519, UnparsedPublicKey};
use m2_protocol::key_sign_in::{BINDING_LEN, ISSUER, PublicKey, signed_message};
use m2_server_core::{Identity, KeySignIn, SignInRefusal};

/// The identity of a device's key whose signature verifies: the key is the
/// subject at [`ISSUER`]. Refused when the connection has no channel
/// binding, or the signature is not the key's over the nonce and the
/// binding.
pub fn check(
    sign_in: &KeySignIn,
    binding: Option<&[u8; BINDING_LEN]>,
) -> Result<Identity, SignInRefusal> {
    let binding = binding.ok_or(SignInRefusal::Refused)?;
    let message = signed_message(&sign_in.nonce, binding).ok_or(SignInRefusal::Refused)?;
    UnparsedPublicKey::new(&ED25519, &sign_in.public_key)
        .verify(&message, &sign_in.signature)
        .map_err(|_| SignInRefusal::Refused)?;
    Ok(Identity {
        issuer: ISSUER.to_owned(),
        subject: PublicKey::from_bytes(sign_in.public_key).to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_server_core::ConnId;

    /// RFC 8032, 7.1, TEST 1: its public key.
    const PUBLIC: [u8; 32] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];

    /// That key's signature over the nonce `n0nce` and the binding
    /// `[0xAA; 32]`, as `m2-net`'s `a_sign_in_signature_is_pinned` makes it.
    const SIGNATURE: &str = "c75342f410bde1bafd1cfa5d9491a6f58efdc8ad39fc9b54fb439ce1f3075d1c\
                             7b3725ef6fb85bcfa6149356a370b3a7f9533cdaa51a3fcd90144c64c2f4760b";

    fn signature() -> [u8; 64] {
        let mut bytes = [0u8; 64];
        for (byte, pair) in bytes.iter_mut().zip(SIGNATURE.as_bytes().chunks(2)) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap();
        }
        bytes
    }

    fn sign_in() -> KeySignIn {
        KeySignIn {
            conn: ConnId(1),
            public_key: PUBLIC,
            signature: signature(),
            nonce: "n0nce".to_owned(),
        }
    }

    #[test]
    fn the_clients_signature_verifies() {
        let identity = check(&sign_in(), Some(&[0xAA; 32])).unwrap();
        assert_eq!(identity.issuer, "openm2:key");
        assert_eq!(
            identity.subject,
            "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
    }

    #[test]
    fn anything_else_is_refused() {
        let refused = Err(SignInRefusal::Refused);
        assert_eq!(check(&sign_in(), None), refused, "no binding");
        assert_eq!(
            check(&sign_in(), Some(&[0xAB; 32])),
            refused,
            "another connection's binding"
        );
        let other_nonce = KeySignIn {
            nonce: "n0ncf".to_owned(),
            ..sign_in()
        };
        assert_eq!(
            check(&other_nonce, Some(&[0xAA; 32])),
            refused,
            "another nonce"
        );
        let mut flipped = sign_in();
        flipped.signature[10] ^= 1;
        assert_eq!(check(&flipped, Some(&[0xAA; 32])), refused, "a flipped bit");
        let mut other_key = sign_in();
        other_key.public_key[0] ^= 1;
        assert_eq!(check(&other_key, Some(&[0xAA; 32])), refused, "another key");
        let not_a_key = KeySignIn {
            public_key: [0; 32],
            signature: [0; 64],
            ..sign_in()
        };
        assert_eq!(check(&not_a_key, Some(&[0xAA; 32])), refused, "zeros");
    }
}
