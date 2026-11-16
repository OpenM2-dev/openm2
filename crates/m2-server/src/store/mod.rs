//! Storage: the server core's requests carried out
//! (`docs/specs/server/storage.md`).
//!
//! [`Store::handle`] answers one [`StoreRequest`]. Two kinds of storage
//! answer the same requests by the same rules: [`MemoryStore`] (tests and
//! quick runs) and [`PgStore`] (Postgres). Players sign in only with a
//! provider: each gets an account of their own at their
//! first sign-in, found again by the issuer and subject
//! ([`identity_account_name`] names it). No password is kept.

mod memory;
mod postgres;

use m2_server_core::{StoreAnswer, StoreRequest};

pub use memory::MemoryStore;
pub use postgres::{OpenError, PgStore, WorldLock};

/// Where accounts and characters are kept.
#[derive(Debug, Clone)]
pub enum Store {
    /// In memory: lost when the server stops. For tests and quick runs.
    Memory(MemoryStore),
    /// In Postgres.
    Postgres(PgStore),
}

impl Store {
    /// A store kept in memory.
    pub fn memory() -> Self {
        Store::Memory(MemoryStore::new())
    }

    /// Closes the store's connections, once the last saves are done.
    pub async fn close(&self) {
        if let Store::Postgres(postgres) = self {
            postgres.close().await;
        }
    }

    /// Carries out one request.
    pub async fn handle(&self, request: StoreRequest) -> StoreAnswer {
        match self {
            Store::Memory(memory) => memory.handle(request).await,
            Store::Postgres(postgres) => postgres.handle(request).await,
        }
    }
}

/// The name of the account made for a provider's player at their first
/// sign-in: the provider's id, a colon, and the first 13 characters of the base32 (RFC 4648, lowercase)
/// of the SHA-256 of the issuer, a zero byte and the subject; 30 characters
/// at most.
pub fn identity_account_name(provider: &str, issuer: &str, subject: &str) -> String {
    const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut input = Vec::with_capacity(issuer.len() + subject.len() + 1);
    input.extend_from_slice(issuer.as_bytes());
    input.push(0);
    input.extend_from_slice(subject.as_bytes());
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, &input);
    // The first 65 bits, five at a time.
    let bits = digest
        .as_ref()
        .iter()
        .take(9)
        .fold(0u128, |bits, byte| (bits << 8) | u128::from(*byte));
    let suffix: String = (0..13)
        .map(|i| {
            let index = (bits >> (72 - 5 * (i + 1))) & 31;
            char::from(BASE32[index as usize])
        })
        .collect();
    format!("{provider}:{suffix}")
}

/// The most accounts [`game_masters`] takes.
pub const MAX_GAME_MASTERS: usize = 64;

/// The game masters' accounts from a list of account names separated by
/// commas (`OPENM2_GAME_MASTERS`), in lowercase: each a name
/// [`identity_account_name`] can give (a provider's id, a colon and 13 of
/// `a`–`z` and `2`–`7`), compared without case; spaces around them and
/// empty places are ignored, and an empty list is nobody. The error names
/// the first name that is not one, or says there are too many.
pub fn game_masters(list: &str) -> Result<std::collections::BTreeSet<String>, String> {
    let mut masters = std::collections::BTreeSet::new();
    for name in list
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let name = name.to_ascii_lowercase();
        let valid = name.split_once(':').is_some_and(|(provider, suffix)| {
            m2_server_core::rules::valid_provider_id(provider)
                && suffix.len() == 13
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
        });
        if !valid {
            return Err(format!(
                "{name:?} is not an account name (a provider's id, a colon and 13 letters and digits, as /account shows it)"
            ));
        }
        masters.insert(name);
    }
    if masters.len() > MAX_GAME_MASTERS {
        return Err(format!(
            "{} accounts, at most {MAX_GAME_MASTERS}",
            masters.len()
        ));
    }
    Ok(masters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_masters_list_takes_account_names_only() {
        let name = identity_account_name("clerk", "https://clerk.example", "user_1");
        let list = format!(" {}, ,CLERK:ABCDEFGHIJ234 ", name.to_ascii_uppercase());
        let masters = game_masters(&list).unwrap();
        assert_eq!(masters.len(), 2);
        assert!(masters.contains(&name));
        assert!(masters.contains("clerk:abcdefghij234"));
        assert!(game_masters("").unwrap().is_empty());
        assert!(game_masters(" , ").unwrap().is_empty());
        for wrong in [
            "clerk",
            "clerk:abc",
            "clerk:abcdefghij2345",
            "clerk:abcdefghij018",
            ":abcdefghij234",
            "Big_Provider:abcdefghij234",
            "seventeen-letters:abcdefghij234",
        ] {
            let error = game_masters(wrong).unwrap_err();
            assert!(error.contains("not an account name"), "{wrong}: {error}");
        }
        let many: Vec<String> = (0..=MAX_GAME_MASTERS)
            .map(|i| identity_account_name("clerk", "https://clerk.example", &i.to_string()))
            .collect();
        assert!(
            game_masters(&many.join(","))
                .unwrap_err()
                .contains("at most")
        );
        assert_eq!(
            game_masters(&many[1..].join(",")).unwrap().len(),
            MAX_GAME_MASTERS
        );
    }
}
