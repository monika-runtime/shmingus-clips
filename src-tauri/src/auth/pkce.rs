//! PKCE (RFC 7636) and the authorization request.
//!
//! The verifier never leaves the process except in the token request; the
//! challenge, state and nonce go into the browser URL. `state` is what proves
//! the loopback callback belongs to this login attempt.

use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

use super::discovery::Discovery;
use super::{CLIENT_ID, REDIRECT_URI, SCOPES};
use crate::error::{AppError, Result};

/// A verifier and its S256 challenge, both base64url without padding.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        Self::from_verifier(base64_url(&random_bytes(32)))
    }

    pub fn from_verifier(verifier: impl Into<String>) -> Self {
        let verifier = verifier.into();
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = base64_url(&digest);
        Self {
            verifier,
            challenge,
        }
    }
}

/// An unguessable URL-safe value for `state` or `nonce`.
pub fn random_url_token() -> String {
    base64_url(&random_bytes(32))
}

/// Build the browser URL for the authorization request.
///
/// Email identity is carried by the `profile email` scopes; `offline_access`
/// asks Authentik for the refresh token that keeps the session alive.
pub fn authorize_url(
    discovery: &Discovery,
    pkce: &Pkce,
    state: &str,
    nonce: &str,
) -> Result<String> {
    let mut url = reqwest::Url::parse(&discovery.authorization_endpoint).map_err(|err| {
        AppError::msg(format!(
            "Authentik's authorization endpoint is not a valid URL: {err}"
        ))
    })?;

    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPES)
        .append_pair("state", state)
        .append_pair("nonce", nonce)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256");

    Ok(url.to_string())
}

fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}

fn base64_url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn discovery() -> Discovery {
        Discovery {
            issuer: super::super::ISSUER.to_string(),
            authorization_endpoint: "https://auth.shmingus.net/application/o/authorize/"
                .to_string(),
            token_endpoint: "https://auth.shmingus.net/application/o/token/".to_string(),
            userinfo_endpoint: "https://auth.shmingus.net/application/o/userinfo/".to_string(),
        }
    }

    #[test]
    fn pkce_matches_the_rfc_7636_s256_example() {
        // Appendix B of RFC 7636.
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_verifiers_and_tokens_are_url_safe_and_unguessable() {
        let pkce = Pkce::generate();
        assert!(
            (43..=128).contains(&pkce.verifier.len()),
            "{}",
            pkce.verifier
        );
        assert_eq!(pkce.challenge.len(), 43);
        for value in [&pkce.verifier, &pkce.challenge] {
            assert!(
                value
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
                "{value}"
            );
        }

        assert_ne!(random_url_token(), random_url_token());
        assert_eq!(random_url_token().len(), 43);
    }

    #[test]
    fn the_authorize_url_carries_every_parameter_the_flow_needs() {
        let pkce = Pkce::from_verifier("a-test-verifier");
        let url = reqwest::Url::parse(
            &authorize_url(&discovery(), &pkce, "state-1", "nonce-1").expect("url"),
        )
        .expect("parse");
        let params: HashMap<String, String> = url.query_pairs().into_owned().collect();

        assert_eq!(url.path(), "/application/o/authorize/");
        assert_eq!(params["response_type"], "code");
        assert_eq!(params["client_id"], CLIENT_ID);
        assert_eq!(params["redirect_uri"], REDIRECT_URI);
        assert_eq!(params["scope"], "openid profile email offline_access");
        assert_eq!(params["state"], "state-1");
        assert_eq!(params["nonce"], "nonce-1");
        assert_eq!(params["code_challenge"], pkce.challenge);
        assert_eq!(params["code_challenge_method"], "S256");

        // A public client proves itself with PKCE, never a secret.
        assert!(!params.contains_key("client_secret"));
    }
}
