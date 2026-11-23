//! Requests to Authentik's token and userinfo endpoints.
//!
//! Everything here is the authorization-code exchange and its refresh
//! counterpart. The client is public, so no client secret is ever sent; PKCE's
//! verifier is what proves this app started the login. Response bodies are
//! never logged: the success body contains the tokens.

use std::time::Duration;

use reqwest::StatusCode;
use serde::Deserialize;

use super::discovery::Discovery;
use super::{CLIENT_ID, REDIRECT_URI};
use crate::error::{AppError, Result};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: Option<u64>,
}

/// The account details Authentik returns for the access token.
#[derive(Debug, Deserialize)]
pub struct UserInfo {
    pub sub: String,
    pub name: Option<String>,
    pub preferred_username: Option<String>,
    pub email: Option<String>,
}

/// The token endpoint's OAuth error body.
#[derive(Debug, Deserialize)]
struct OAuthError {
    error: String,
    error_description: Option<String>,
}

pub enum TokenFailure {
    /// `invalid_grant`: the code or refresh token has expired or was already
    /// rotated. The caller must clear the stored session and ask to sign in.
    InvalidGrant,
    Failed(AppError),
}

/// Exchange the loopback code for tokens.
pub async fn exchange_code(
    discovery: &Discovery,
    code: &str,
    verifier: &str,
) -> std::result::Result<TokenResponse, TokenFailure> {
    post_token(
        discovery,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ],
    )
    .await
}

/// Trade a refresh token for a new access token (and, with Authentik, a new
/// refresh token that invalidates the old one).
pub async fn refresh(
    discovery: &Discovery,
    refresh_token: &str,
) -> std::result::Result<TokenResponse, TokenFailure> {
    post_token(
        discovery,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ],
    )
    .await
}

async fn post_token(
    discovery: &Discovery,
    params: &[(&str, &str)],
) -> std::result::Result<TokenResponse, TokenFailure> {
    let client = client().map_err(TokenFailure::Failed)?;
    let response = client
        .post(&discovery.token_endpoint)
        .form(params)
        .send()
        .await
        .map_err(|err| {
            TokenFailure::Failed(AppError::msg(format!(
                "Could not reach Authentik to complete the sign-in: {err}"
            )))
        })?;

    let status = response.status();
    let body = response.text().await.map_err(|err| {
        TokenFailure::Failed(AppError::msg(format!(
            "Could not read Authentik's sign-in response: {err}"
        )))
    })?;

    if !status.is_success() {
        return Err(failure(status, &body));
    }

    let token: TokenResponse = serde_json::from_str(&body).map_err(|err| {
        TokenFailure::Failed(AppError::msg(format!(
            "Authentik returned an unexpected token response: {err}"
        )))
    })?;
    if token.access_token.trim().is_empty() {
        return Err(TokenFailure::Failed(AppError::msg(
            "Authentik's token response did not include an access token.",
        )));
    }
    Ok(token)
}

/// Turn a non-2xx token response into an error, preserving `invalid_grant` so
/// the caller can clear the dead session.
fn failure(status: StatusCode, body: &str) -> TokenFailure {
    let parsed: Option<OAuthError> = serde_json::from_str(body).ok();
    match parsed {
        Some(error) if error.error == "invalid_grant" => TokenFailure::InvalidGrant,
        Some(error) => {
            let message = match error
                .error_description
                .filter(|description| !description.trim().is_empty())
            {
                Some(description) => {
                    format!(
                        "Authentik refused the sign-in ({}: {description}).",
                        error.error
                    )
                }
                None => format!("Authentik refused the sign-in ({}).", error.error),
            };
            TokenFailure::Failed(AppError::msg(message))
        }
        // Do not echo the body: an error page could still carry credentials.
        None => TokenFailure::Failed(AppError::msg(format!(
            "Authentik refused the sign-in (HTTP {}).",
            status.as_u16()
        ))),
    }
}

/// Ask Authentik who the access token belongs to.
pub async fn fetch_userinfo(discovery: &Discovery, access_token: &str) -> Result<UserInfo> {
    let client = client()?;
    let response = client
        .get(&discovery.userinfo_endpoint)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|err| {
            AppError::msg(format!(
                "Could not ask Authentik which account signed in: {err}"
            ))
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(AppError::msg(format!(
            "Authentik could not confirm the new sign-in (HTTP {}). Try again.",
            status.as_u16()
        )));
    }

    let body = response.text().await.map_err(|err| {
        AppError::msg(format!("Could not read Authentik's account details: {err}"))
    })?;
    let user: UserInfo = serde_json::from_str(&body).map_err(|err| {
        AppError::msg(format!(
            "Authentik described the account in an unexpected format: {err}"
        ))
    })?;
    if user.sub.trim().is_empty() {
        return Err(AppError::msg(
            "Authentik did not say which account signed in. Try again.",
        ));
    }
    Ok(user)
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|err| {
            AppError::msg(format!(
                "Could not start the HTTP client for Authentik: {err}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_response_carries_the_rotating_refresh_token() {
        let token: TokenResponse = serde_json::from_str(
            r#"{
                "access_token": "access-1",
                "refresh_token": "refresh-1",
                "expires_in": 3600,
                "token_type": "Bearer",
                "scope": "openid profile email offline_access"
            }"#,
        )
        .expect("token response");
        assert_eq!(token.access_token, "access-1");
        assert_eq!(token.refresh_token.as_deref(), Some("refresh-1"));
        assert_eq!(token.expires_in, Some(3600));
    }

    #[test]
    fn userinfo_offers_a_fallback_name() {
        let user: UserInfo = serde_json::from_str(
            r#"{"sub": "mafio", "email": "maf@example.com", "preferred_username": "mafio"}"#,
        )
        .expect("userinfo");
        assert_eq!(user.sub, "mafio");
        assert!(user.name.is_none());
        assert_eq!(user.preferred_username.as_deref(), Some("mafio"));
    }

    #[test]
    fn invalid_grant_is_classified_so_the_session_can_be_cleared() {
        let failure = failure(
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid_grant","error_description":"Refresh token invalid"}"#,
        );
        assert!(matches!(failure, TokenFailure::InvalidGrant));
    }

    #[test]
    fn other_oauth_errors_keep_their_description() {
        match failure(
            StatusCode::BAD_REQUEST,
            r#"{"error":"invalid_request","error_description":"Missing code"}"#,
        ) {
            TokenFailure::Failed(err) => {
                assert!(err.to_string().contains("invalid_request"), "{err}");
                assert!(err.to_string().contains("Missing code"), "{err}");
            }
            _ => panic!("expected a plain failure"),
        }
    }

    #[test]
    fn a_non_json_error_body_does_not_leak_into_the_message() {
        match failure(StatusCode::BAD_GATEWAY, "upstream proxy said no") {
            TokenFailure::Failed(err) => {
                let message = err.to_string();
                assert!(message.contains("HTTP 502"), "{message}");
                assert!(!message.contains("upstream proxy said no"), "{message}");
            }
            _ => panic!("expected a plain failure"),
        }
    }
}
