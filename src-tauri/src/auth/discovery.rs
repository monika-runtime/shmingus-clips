//! Runtime discovery of Authentik's OIDC endpoints.
//!
//! The issuer is fixed, but the endpoint paths are not: Authentik serves
//! `authorize` and `token` under global paths while `jwks` and `end-session`
//! are per-application. Reading the discovery document is the only safe way to
//! find them, so the app fetches it once and caches it for the process.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use tokio::sync::Mutex;

use super::ISSUER;
use crate::error::{AppError, Result};

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: String,
}

static CACHE: OnceLock<Mutex<Option<Discovery>>> = OnceLock::new();

fn cache() -> &'static Mutex<Option<Discovery>> {
    CACHE.get_or_init(|| Mutex::new(None))
}

/// The endpoints for this process, fetched on first use.
pub async fn get() -> Result<Discovery> {
    let mut cached = cache().lock().await;
    if let Some(discovery) = cached.as_ref() {
        return Ok(discovery.clone());
    }
    let discovery = fetch().await?;
    *cached = Some(discovery.clone());
    Ok(discovery)
}

/// `<issuer>/.well-known/openid-configuration`, without a doubled slash when
/// the issuer already ends in one.
pub fn discovery_url(issuer: &str) -> String {
    format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    )
}

async fn fetch() -> Result<Discovery> {
    let url = discovery_url(ISSUER);
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|err| {
            AppError::msg(format!(
                "Could not start the HTTP client for Authentik: {err}"
            ))
        })?;

    let response = client.get(&url).send().await.map_err(|err| {
        AppError::msg(format!(
            "Could not reach Authentik to read its sign-in configuration at {url}: {err}"
        ))
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(AppError::msg(format!(
            "Authentik's sign-in configuration could not be read (HTTP {} at {url}).",
            status.as_u16()
        )));
    }

    let body = response.text().await.map_err(|err| {
        AppError::msg(format!(
            "Could not read Authentik's sign-in configuration: {err}"
        ))
    })?;
    let discovery: Discovery = serde_json::from_str(&body).map_err(|err| {
        AppError::msg(format!(
            "Authentik's sign-in configuration was not valid JSON: {err}"
        ))
    })?;

    validate(&discovery, ISSUER)?;
    Ok(discovery)
}

/// The document must describe the issuer we asked about and use https
/// endpoints; anything else could send the code or tokens somewhere unintended.
fn validate(discovery: &Discovery, expected_issuer: &str) -> Result<()> {
    if normalize(&discovery.issuer) != normalize(expected_issuer) {
        return Err(AppError::msg(format!(
            "Authentik's discovery document belongs to {} instead of {expected_issuer}.",
            discovery.issuer
        )));
    }

    for (name, endpoint) in [
        ("authorization", &discovery.authorization_endpoint),
        ("token", &discovery.token_endpoint),
        ("userinfo", &discovery.userinfo_endpoint),
    ] {
        let parsed = reqwest::Url::parse(endpoint).map_err(|err| {
            AppError::msg(format!(
                "Authentik's {name} endpoint is not a valid URL: {err}"
            ))
        })?;
        if parsed.scheme() != "https" || parsed.host_str().is_none() {
            return Err(AppError::msg(format!(
                "Authentik's {name} endpoint is not an https URL."
            )));
        }
    }

    Ok(())
}

fn normalize(issuer: &str) -> &str {
    issuer.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(issuer: &str) -> Discovery {
        Discovery {
            issuer: issuer.to_string(),
            authorization_endpoint: "https://auth.shmingus.net/application/o/authorize/".into(),
            token_endpoint: "https://auth.shmingus.net/application/o/token/".into(),
            userinfo_endpoint: "https://auth.shmingus.net/application/o/userinfo/".into(),
        }
    }

    #[test]
    fn the_discovery_url_is_built_from_the_issuer_without_doubling_the_slash() {
        assert_eq!(
            discovery_url(ISSUER),
            "https://auth.shmingus.net/application/o/clips/.well-known/openid-configuration"
        );
        assert_eq!(
            discovery_url("https://example.com/"),
            "https://example.com/.well-known/openid-configuration"
        );
    }

    #[test]
    fn a_document_for_the_configured_issuer_is_accepted_with_or_without_a_trailing_slash() {
        validate(&document(ISSUER), ISSUER).expect("same issuer");
        validate(
            &document("https://auth.shmingus.net/application/o/clips"),
            ISSUER,
        )
        .expect("same issuer without slash");
    }

    #[test]
    fn a_document_for_another_issuer_is_refused() {
        let err = validate(&document("https://evil.example/"), ISSUER).expect_err("foreign issuer");
        assert!(err.to_string().contains("instead of"), "{err}");
    }

    #[test]
    fn an_endpoint_that_is_not_https_is_refused() {
        let mut discovery = document(ISSUER);
        discovery.token_endpoint = "http://auth.shmingus.net/token".into();
        assert!(validate(&discovery, ISSUER).is_err());
    }
}
