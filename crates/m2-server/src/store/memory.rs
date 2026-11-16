//! Accounts and characters in memory: lost when the server stops.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};

use m2_server_core::rules::MAX_CHARACTERS;
use m2_server_core::{
    Account, AccountId, Character, CreateRefusal, StoreAnswer, StoreError, StoreRequest,
    StoredFriend,
};

use super::identity_account_name;

#[derive(Debug, Default)]
struct MemoryData {
    accounts: BTreeMap<u64, Account>,
    characters: BTreeMap<u32, (u64, Character)>,
    /// Providers' players' accounts, by issuer and subject.
    identities: BTreeMap<(String, String), u64>,
    /// Friendships, each once: the lower character number first.
    friendships: BTreeSet<(u32, u32)>,
    next_account: u64,
    next_character: u32,
}

impl MemoryData {
    /// Two characters that exist, the lower number first, as a friendship
    /// is kept.
    fn pair(&self, [a, b]: [u32; 2]) -> Result<(u32, u32), StoreError> {
        if a == b {
            return Err(StoreError(format!(
                "character {a} cannot be its own friend"
            )));
        }
        if let Some(missing) = [a, b]
            .into_iter()
            .find(|c| !self.characters.contains_key(c))
        {
            return Err(StoreError(format!("no character {missing}")));
        }
        Ok((a.min(b), a.max(b)))
    }

    /// The friends of character `id`, by name in lowercase.
    fn friends_of(&self, id: u32) -> Vec<StoredFriend> {
        let mut friends: Vec<StoredFriend> = self
            .friendships
            .iter()
            .filter_map(|&(a, b)| {
                if a == id {
                    Some(b)
                } else if b == id {
                    Some(a)
                } else {
                    None
                }
            })
            .filter_map(|friend| self.characters.get(&friend))
            .map(|(owner, character)| StoredFriend {
                id: character.id,
                name: character.name.clone(),
                account: AccountId(*owner),
            })
            .collect();
        friends.sort_by_key(|friend| friend.name.to_ascii_lowercase());
        friends
    }
}

/// Accounts and characters in memory.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    data: Arc<Mutex<MemoryData>>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn data(&self) -> std::sync::MutexGuard<'_, MemoryData> {
        // A panic while holding the lock leaves plain data behind; keep going.
        self.data.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The characters stored, with their accounts' numbers (for tests).
    pub fn characters(&self) -> Vec<(AccountId, Character)> {
        self.data()
            .characters
            .values()
            .map(|(account, character)| (AccountId(*account), character.clone()))
            .collect()
    }

    /// The friendships stored, the lower character number first (for
    /// tests).
    pub fn friendships(&self) -> Vec<(u32, u32)> {
        self.data().friendships.iter().copied().collect()
    }

    pub(super) async fn handle(&self, request: StoreRequest) -> StoreAnswer {
        match request {
            StoreRequest::IdentityLogin {
                provider,
                issuer,
                subject,
            } => StoreAnswer::LoginChecked(Ok(self.identity(&provider, issuer, subject))),
            StoreRequest::LoadCharacters { account } => {
                let data = self.data();
                let mut characters: Vec<Character> = data
                    .characters
                    .values()
                    .filter(|(owner, _)| *owner == account.0)
                    .map(|(_, character)| Character {
                        friends: data.friends_of(character.id),
                        ..character.clone()
                    })
                    .collect();
                characters.sort_by_key(|character| character.slot);
                StoreAnswer::Characters(Ok(characters))
            }
            StoreRequest::CreateCharacter { account, character } => {
                StoreAnswer::CharacterCreated(self.create(account, character))
            }
            StoreRequest::SaveCharacter { account, character } => {
                let mut data = self.data();
                StoreAnswer::Saved(match data.characters.get_mut(&character.id) {
                    Some((owner, stored)) if *owner == account.0 => {
                        // Only a newer save; never its name, class, slot,
                        // empire or friends.
                        if stored.saves < character.saves {
                            *stored = Character {
                                id: stored.id,
                                slot: stored.slot,
                                name: stored.name.clone(),
                                class: stored.class,
                                empire: stored.empire,
                                friends: Vec::new(),
                                ..character
                            };
                        }
                        Ok(())
                    }
                    _ => Err(StoreError(format!(
                        "no character {} of account {}",
                        character.id, account.0
                    ))),
                })
            }
            StoreRequest::AddFriendship { characters } => {
                let mut data = self.data();
                StoreAnswer::FriendshipWritten(data.pair(characters).map(|pair| {
                    data.friendships.insert(pair);
                }))
            }
            StoreRequest::RemoveFriendship { characters: [a, b] } => {
                StoreAnswer::FriendshipWritten(if a == b {
                    Err(StoreError(format!(
                        "character {a} cannot be its own friend"
                    )))
                } else {
                    // No such friendship, or no such character: nothing to end.
                    self.data().friendships.remove(&(a.min(b), a.max(b)));
                    Ok(())
                })
            }
        }
    }

    /// The account of a provider's player, made at their first sign-in.
    fn identity(&self, provider: &str, issuer: String, subject: String) -> Account {
        let name = identity_account_name(provider, &issuer, &subject);
        let mut data = self.data();
        let key = (issuer, subject);
        let id = match data.identities.get(&key) {
            Some(id) => *id,
            None => {
                data.next_account += 1;
                let id = data.next_account;
                let account = Account {
                    id: AccountId(id),
                    name,
                    empire: None,
                };
                data.accounts.insert(id, account);
                data.identities.insert(key, id);
                id
            }
        };
        data.accounts.get(&id).cloned().unwrap_or(Account {
            id: AccountId(id),
            name: String::new(),
            empire: None,
        })
    }

    fn create(
        &self,
        account: AccountId,
        mut character: Character,
    ) -> Result<Character, CreateRefusal> {
        let mut data = self.data();
        if data
            .characters
            .values()
            .any(|(_, c)| c.name.eq_ignore_ascii_case(&character.name))
        {
            return Err(CreateRefusal::NameTaken);
        }
        let owned: Vec<u8> = data
            .characters
            .values()
            .filter(|(owner, _)| *owner == account.0)
            .map(|(_, c)| c.slot)
            .collect();
        if owned.len() >= MAX_CHARACTERS || owned.contains(&character.slot) {
            return Err(CreateRefusal::NoRoom);
        }
        let Some(row) = data.accounts.get_mut(&account.0) else {
            return Err(CreateRefusal::Unavailable);
        };
        row.empire.get_or_insert(character.empire);
        data.next_character += 1;
        character.id = data.next_character;
        character.friends.clear();
        data.characters
            .insert(character.id, (account.0, character.clone()));
        Ok(character)
    }
}
