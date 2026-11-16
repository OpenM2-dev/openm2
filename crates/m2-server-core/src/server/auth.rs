//! Providers' sign-ins on the auth server, and tickets on the game server
//! (`docs/specs/server/session.md`, "Signing in" and "Tickets").

use std::net::IpAddr;

use m2_protocol::profile::openm2::{
    LoginRefused, Provider, ProviderLogin, ServerMessage, SignInOptions, SignedIn, TicketLogin,
};
use secrecy::{ExposeSecret as _, SecretBox, SecretString};

use super::{Event, IssuedTicket, Listener, Server, Stage, TICKET_LEN, Waiter};
use crate::sign_in::{Identity, SignIn, SignInRefusal};
use crate::store::{Account, RequestId, StoreError, StoreRequest};
use crate::time::{ConnId, Millis};

/// The longest provider id: the most SignInOptions carries.
const MAX_PROVIDER_ID: usize = 16;
/// The most providers SignInOptions carries.
const MAX_PROVIDERS: usize = 8;
/// The longest authorization code: the most ProviderLogin carries.
const MAX_CODE: usize = 4096;
/// A sign-in's redirect: the client's own listener on the loopback address
/// (RFC 8252), `http://127.0.0.1:<port>/callback`.
const REDIRECT_START: &str = "http://127.0.0.1:";
const REDIRECT_END: &str = "/callback";

/// LoginRefused's reasons (`session.md`, "Logins"). 1 is reserved and never
/// sent.
const IN_GAME: u8 = 2;
const TOO_MANY: u8 = 4;
const BAD_TICKET: u8 = 5;
const PROVIDER_REFUSED: u8 = 6;
const PROVIDER_UNAVAILABLE: u8 = 7;

/// True for a provider id: 1 to 16 lowercase ASCII letters, digits and
/// hyphens.
pub fn valid_provider_id(id: &str) -> bool {
    (1..=MAX_PROVIDER_ID).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// True for `http://127.0.0.1:<port>/callback`, the port 1 to 65535 with no
/// leading zero.
fn valid_redirect(redirect: &str) -> bool {
    let Some(port) = redirect
        .strip_prefix(REDIRECT_START)
        .and_then(|rest| rest.strip_suffix(REDIRECT_END))
    else {
        return false;
    };
    !port.starts_with('0')
        && port.bytes().all(|b| b.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port != 0)
}

/// True for a PKCE code verifier (RFC 7636): 43 to 128 of `A-Z a-z 0-9 - .
/// _ ~`.
fn valid_verifier(verifier: &[u8]) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(b))
}

/// True for an authorization code (RFC 6749): 1 to 4096 printable ASCII
/// characters.
fn valid_code(code: &[u8]) -> bool {
    (1..=MAX_CODE).contains(&code.len()) && code.iter().all(|b| (0x20..=0x7E).contains(b))
}

/// Whether `given` is `ticket`, in a time that does not depend on where
/// they differ.
fn same_ticket(ticket: &[u8; TICKET_LEN], given: &[u8]) -> bool {
    if given.len() != TICKET_LEN {
        return false;
    }
    let diff = ticket
        .iter()
        .zip(given)
        .fold(0u8, |diff, (a, b)| diff | (a ^ b));
    std::hint::black_box(diff) == 0
}

impl Server {
    /// Storage's answer to a sign-in: the account of the identity, and a
    /// ticket for it.
    pub(super) fn login_checked(
        &mut self,
        now: Millis,
        conn: ConnId,
        result: Result<Account, StoreError>,
    ) {
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        let provider = state.provider.take().unwrap_or_default();
        match result {
            Ok(account) if self.online.contains_key(&account.id) => {
                self.refuse_sign_in(now, conn, IN_GAME, "the account is in game");
            }
            Ok(account) => {
                let id = account.id;
                let account_name = account.name.clone();
                let ticket = self.issue_ticket(now, account).to_vec();
                self.set_stage(conn, Stage::LoggedIn);
                self.event(Event::SignedIn {
                    conn,
                    account: id,
                    provider,
                });
                let signed_in = SignedIn {
                    account: account_name,
                    ticket,
                };
                self.send(conn, ServerMessage::SignedIn(signed_in));
            }
            Err(StoreError(error)) => {
                self.event(Event::StoreFailed {
                    what: "find a sign-in's account",
                    error,
                });
                self.close(now, conn, "storage could not find the account");
            }
        }
    }

    /// StartSignIn: the providers and, when there are any, a nonce for this
    /// connection, which replaces any earlier one. From then on the
    /// connection is pinged, and it must sign in within
    /// [`crate::Config::sign_in_time`] of its first StartSignIn.
    pub(super) fn start_sign_in(&mut self, now: Millis, conn: ConnId) {
        let Some(state) = self.conns.get(&conn) else {
            return;
        };
        if state.listener != Listener::Auth || state.stage != Stage::Login {
            self.close(now, conn, "a sign-in out of turn");
            return;
        }
        let providers: Vec<Provider> = self
            .config
            .providers
            .iter()
            .take(MAX_PROVIDERS)
            .map(|offer| Provider {
                id: offer.id.clone(),
                name: offer.name.clone(),
                authorize: offer.authorize.clone(),
                client_id: offer.client_id.clone(),
            })
            .collect();
        let nonce = if providers.is_empty() {
            String::new()
        } else {
            self.random_nonce()
        };
        let (ping_every, sign_in_time) = (self.config.ping_every, self.config.sign_in_time);
        if let Some(state) = self.conns.get_mut(&conn)
            && !providers.is_empty()
        {
            state.nonce = Some(nonce.clone());
            state.next_ping.get_or_insert(now.plus(ping_every));
            state.sign_in_by.get_or_insert(now.plus(sign_in_time));
        }
        self.send(
            conn,
            ServerMessage::SignInOptions(SignInOptions { providers, nonce }),
        );
    }

    /// ProviderLogin: refused at once when the address failed too often,
    /// when no nonce was given out, or when the provider, the redirect, the
    /// verifier or the code cannot be right; otherwise handed to the edge to
    /// check. The nonce is used up either way.
    pub(super) fn provider_login(&mut self, now: Millis, conn: ConnId, login: ProviderLogin) {
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        if state.listener != Listener::Auth || state.stage != Stage::Login {
            self.close(now, conn, "a sign-in out of turn");
            return;
        }
        let peer = state.peer;
        let nonce = state.nonce.take();
        if self.failed_too_often(now, peer) {
            self.refuse_sign_in(now, conn, TOO_MANY, "too many failed logins");
            return;
        }
        let ProviderLogin {
            provider,
            code,
            verifier,
            redirect,
        } = login;
        let code = SecretString::from(code);
        let verifier = SecretString::from(verifier);
        let known = self.config.providers.iter().any(|p| p.id == provider);
        let Some(nonce) = nonce.filter(|_| {
            known
                && valid_redirect(&redirect)
                && valid_verifier(verifier.expose_secret().as_bytes())
                && valid_code(code.expose_secret().as_bytes())
        }) else {
            self.count_failure(now, peer);
            self.refuse_sign_in(
                now,
                conn,
                PROVIDER_REFUSED,
                "a sign-in the server cannot check",
            );
            return;
        };
        if let Some(state) = self.conns.get_mut(&conn) {
            state.stage = Stage::Checking;
            state.provider = Some(provider.clone());
        }
        self.check_sign_in(
            conn,
            SignIn {
                provider,
                code,
                verifier,
                redirect,
                nonce,
            },
        );
    }

    /// The edge's answer to a sign-in: storage finds or makes the account,
    /// or the sign-in is refused.
    pub(super) fn sign_in_checked(
        &mut self,
        now: Millis,
        request: RequestId,
        result: Result<Identity, SignInRefusal>,
    ) {
        let Some(&Waiter::Conn(conn)) = self.requests.get(&request) else {
            return;
        };
        self.requests.remove(&request);
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        if state.waiting != Some(request) {
            return;
        }
        state.waiting = None;
        let peer = state.peer;
        let provider = state.provider.clone().unwrap_or_default();
        match result {
            Ok(Identity { issuer, subject }) => {
                let what = StoreRequest::IdentityLogin {
                    provider,
                    issuer,
                    subject,
                };
                self.request(conn, what);
            }
            Err(SignInRefusal::Refused) => {
                self.count_failure(now, peer);
                let why = "the provider did not confirm the sign-in";
                self.refuse_sign_in(now, conn, PROVIDER_REFUSED, why);
            }
            Err(SignInRefusal::Unavailable) => {
                let why = "the provider could not be reached";
                self.refuse_sign_in(now, conn, PROVIDER_UNAVAILABLE, why);
            }
        }
    }

    /// Refuses a sign-in and closes the connection: its nonce is used up.
    fn refuse_sign_in(&mut self, now: Millis, conn: ConnId, reason: u8, why: &'static str) {
        self.event(Event::LoginRefused { conn, why });
        self.send(conn, ServerMessage::LoginRefused(LoginRefused { reason }));
        self.close(now, conn, why);
    }

    /// A new ticket for the account: 32 random bytes, used once, valid for
    /// [`crate::Config::key_lifetime`]. A ticket given out to the account
    /// before stops working.
    fn issue_ticket(&mut self, now: Millis, account: Account) -> [u8; TICKET_LEN] {
        let ticket = self.random_ticket();
        let expires = now.plus(self.config.key_lifetime);
        self.tickets.insert(
            account.name.to_ascii_lowercase(),
            IssuedTicket {
                account,
                ticket: SecretBox::new(Box::new(ticket)),
                expires,
            },
        );
        ticket
    }

    /// TicketLogin: the ticket must have been given out to that account
    /// name, be unused and not expired. Anything else gets LoginRefused and
    /// the connection is closed.
    pub(super) fn ticket_login(&mut self, now: Millis, conn: ConnId, login: TicketLogin) {
        let Some(state) = self.conns.get(&conn) else {
            return;
        };
        if state.listener != Listener::Game || state.stage != Stage::Ticket {
            self.close(now, conn, "a ticket out of turn");
            return;
        }
        let peer = state.peer;
        if self.failed_too_often(now, peer) {
            self.refuse_ticket(now, conn, TOO_MANY, "too many wrong tickets");
            return;
        }
        let name = login.username.to_ascii_lowercase();
        let valid = self.tickets.get(&name).is_some_and(|issued| {
            issued.expires > now && same_ticket(issued.ticket.expose_secret(), &login.ticket)
        });
        let Some(issued) = valid.then(|| self.tickets.remove(&name)).flatten() else {
            self.count_failure(now, peer);
            self.refuse_ticket(now, conn, BAD_TICKET, "a wrong ticket");
            return;
        };
        let account = issued.account;
        if self.online.contains_key(&account.id) {
            self.refuse_ticket(now, conn, IN_GAME, "the account is in game already");
            return;
        }
        self.online.insert(account.id, conn);
        let id = account.id;
        if let Some(state) = self.conns.get_mut(&conn) {
            state.stage = Stage::Joining;
            state.account = Some(account);
        }
        self.event(Event::Joined { conn, account: id });
        self.load_characters(conn, id);
    }

    /// Refuses a ticket login and closes the connection.
    fn refuse_ticket(&mut self, now: Millis, conn: ConnId, reason: u8, why: &'static str) {
        self.event(Event::LoginRefused { conn, why });
        self.send(conn, ServerMessage::LoginRefused(LoginRefused { reason }));
        self.close(now, conn, why);
    }

    fn failed_too_often(&self, now: Millis, peer: IpAddr) -> bool {
        self.failures.get(&peer).is_some_and(|times| {
            times
                .iter()
                .filter(|at| now.since(**at) < self.config.failure_window)
                .count()
                >= self.config.max_failures
        })
    }

    fn count_failure(&mut self, now: Millis, peer: IpAddr) {
        let limit = self.config.max_failures.max(1);
        let times = self.failures.entry(peer).or_default();
        times.push(now);
        if times.len() > limit {
            times.remove(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tickets_match_only_whole() {
        let ticket = [7u8; TICKET_LEN];
        assert!(same_ticket(&ticket, &[7; TICKET_LEN]));
        let mut last = [7u8; TICKET_LEN];
        last[TICKET_LEN - 1] = 8;
        assert!(!same_ticket(&ticket, &last));
        assert!(!same_ticket(&ticket, &[7; 31]));
        assert!(!same_ticket(&ticket, &[]));
    }

    #[test]
    fn provider_ids_are_short_and_lowercase() {
        assert!(valid_provider_id("google"));
        assert!(valid_provider_id("my-idp-2"));
        assert!(valid_provider_id(&"a".repeat(16)));
        assert!(!valid_provider_id(&"a".repeat(17)));
        assert!(!valid_provider_id(""));
        assert!(!valid_provider_id("Google"));
        assert!(!valid_provider_id("my_idp"));
        assert!(!valid_provider_id("a:b"));
    }

    #[test]
    fn redirects_are_the_clients_loopback_listener() {
        assert!(valid_redirect("http://127.0.0.1:52817/callback"));
        assert!(valid_redirect("http://127.0.0.1:1/callback"));
        assert!(valid_redirect("http://127.0.0.1:65535/callback"));
        for bad in [
            "http://127.0.0.1:0/callback",
            "http://127.0.0.1:65536/callback",
            "http://127.0.0.1:080/callback",
            "http://127.0.0.1:/callback",
            "http://127.0.0.1:80/callback/",
            "http://127.0.0.1:80/callback?x=1",
            "http://127.0.0.1:+80/callback",
            "https://127.0.0.1:80/callback",
            "http://localhost:80/callback",
            "http://127.0.0.1@evil.example:80/callback",
            "http://127.0.0.1:80@evil.example/callback",
        ] {
            assert!(!valid_redirect(bad), "{bad}");
        }
    }

    #[test]
    fn verifiers_and_codes_follow_their_rfcs() {
        assert!(valid_verifier(&[b'a'; 43]));
        assert!(valid_verifier(
            b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        ));
        assert!(valid_verifier(&[b'~'; 128]));
        assert!(!valid_verifier(&[b'a'; 42]));
        assert!(!valid_verifier(&[b'a'; 129]));
        assert!(!valid_verifier(&[b'+'; 43]));
        assert!(valid_code(b"4/0Ab-x_y.z"));
        assert!(valid_code(&[b'a'; 4096]));
        assert!(!valid_code(b""));
        assert!(!valid_code(b"tab\there"));
        assert!(!valid_code("caf\u{e9}".as_bytes()));
    }
}
