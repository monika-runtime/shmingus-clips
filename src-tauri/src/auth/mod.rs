//! Authentik sign-in for Shmingus Clips.
//!
//! The desktop app is a public OAuth2/OIDC client: it runs the authorization
//! code flow with PKCE (S256) in the user's system browser, receives the code
//! on a loopback redirect (`http://127.0.0.1:47823/callback`), and keeps the
//! resulting tokens in `~/.config/clipforge/auth.json` (mode 0600). Only the
//! access token matters to the rest of the app - [`access_token`] hands it to
//! uploads and refreshes it in the background when it is close to expiry.
//!
//! The endpoints come from Authentik's discovery document at runtime; they are
//! never assembled from the issuer by hand. Tokens are never logged and never
//! leave this module except as a bearer header.

mod discovery;
mod loopback;
mod pkce;
mod store;
mod tokens;

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::net::TcpListener;

use crate::error::{AppError, Result};
use crate::model::AuthStatus;

use pkce::{Pkce, authorize_url, random_url_token};
use store::{AuthStore, StoredTokens};

/// The Authentik application ("Clips").
pub const ISSUER: &str = "https://auth.shmingus.net/application/o/clips/";
pub const CLIENT_ID: &str = "3b3qlXdl8ZXav4OGMITMnApAzoU8hNNBeQqSqrCZ";
/// Registered with Authentik; the port is fixed and must not fall back to a
/// random one (Authentik matches redirect URIs strictly).
pub const REDIRECT_URI: &str = "http://127.0.0.1:47823/callback";
pub const CALLBACK_HOST: &str = "127.0.0.1";
pub const CALLBACK_PORT: u16 = 47823;
pub const CALLBACK_PATH: &str = "/callback";
/// `offline_access` is what earns a refresh token.
pub const SCOPES: &str = "openid profile email offline_access";

/// Waiting longer than this means the user gave up in the browser.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Used when the token endpoint does not state `expires_in`.
const DEFAULT_ACCESS_TTL_SECS: u64 = 3600;

const SIGN_IN_REQUIRED: &str = "Sign in with Authentik to continue.";
const SIGN_IN_AGAIN: &str = "Your Authentik session has expired. Sign in again.";

static STORE: OnceLock<Arc<AuthStore>> = OnceLock::new();

fn store() -> Arc<AuthStore> {
    STORE.get_or_init(AuthStore::load).clone()
}

/// Only one loopback listener (and one browser window) may exist at a time.
static LOGIN_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

struct LoginGuard;

impl LoginGuard {
    fn acquire() -> Result<Self> {
        if LOGIN_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(AppError::msg("A sign-in is already in progress."));
        }
        Ok(Self)
    }
}

impl Drop for LoginGuard {
    fn drop(&mut self) {
        LOGIN_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

/// Unix seconds, the clock used for token expiry.
pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

/// Whether this computer has a stored Authentik sign-in.
///
/// A stored but expired session still counts: the refresh token can usually
/// revive it, and [`access_token`] says so when it cannot.
pub fn is_signed_in() -> bool {
    store().is_signed_in()
}

/// The public sign-in state. Never contains tokens.
pub fn status() -> AuthStatus {
    store().status()
}

/// Sign in through the system browser and wait for Authentik's redirect.
pub async fn login() -> Result<AuthStatus> {
    let _guard = LoginGuard::acquire()?;

    let discovery = discovery::get().await?;
    let pkce = Pkce::generate();
    let state = random_url_token();
    let nonce = random_url_token();
    let url = authorize_url(&discovery, &pkce, &state, &nonce)?;

    // Bind before opening the browser so an occupied port fails loudly instead
    // of after the user has already approved the login.
    let listener = bind_callback().await?;
    open_browser(&url)?;
    log::info!("opened the system browser for Authentik sign-in");

    let code = loopback::wait_for_callback(listener, &state, CALLBACK_TIMEOUT).await?;
    let token = tokens::exchange_code(&discovery, &code, &pkce.verifier)
        .await
        .map_err(|failure| match failure {
            tokens::TokenFailure::InvalidGrant => {
                AppError::msg("The sign-in code was not accepted (it may have expired). Try again.")
            }
            tokens::TokenFailure::Failed(err) => err,
        })?;

    let user = tokens::fetch_userinfo(&discovery, &token.access_token).await?;
    let stored = StoredTokens {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: now_secs().saturating_add(
            token
                .expires_in
                .unwrap_or(DEFAULT_ACCESS_TTL_SECS)
                .min(i64::MAX as u64) as i64,
        ),
        sub: Some(user.sub),
        name: user.name.or(user.preferred_username),
        email: user.email,
    };

    store().put(stored).map_err(|err| {
        AppError::msg(format!(
            "Signed in, but the tokens could not be stored on this computer: {err}"
        ))
    })?;
    log::info!("signed in to Authentik");
    Ok(status())
}

/// Forget the stored sign-in. No server call is needed: the local tokens are
/// what uploads use.
pub fn logout() -> Result<()> {
    store()
        .clear()
        .map_err(|err| AppError::msg(format!("Could not clear the Authentik sign-in: {err}")))?;
    log::info!("signed out of Authentik");
    Ok(())
}

/// A usable access token, refreshed first when it expires within a minute.
///
/// The error is actionable when the session is gone: the store has already been
/// cleared by then, so the next attempt starts from a clean sign-in.
pub async fn access_token() -> Result<String> {
    let store = store();
    let Some(tokens) = store.get() else {
        return Err(AppError::msg(SIGN_IN_REQUIRED));
    };
    if !store::needs_refresh(now_secs(), tokens.expires_at) {
        return Ok(tokens.access_token);
    }

    // One refresh at a time: refresh tokens rotate, so two concurrent refreshes
    // with the same token would make the loser fail (and clear) the session.
    let _guard = store.refresh_lock.lock().await;
    let Some(tokens) = store.get() else {
        return Err(AppError::msg(SIGN_IN_REQUIRED));
    };
    if !store::needs_refresh(now_secs(), tokens.expires_at) {
        return Ok(tokens.access_token);
    }
    refresh_now(&store, tokens)
        .await
        .map(|tokens| tokens.access_token)
}

/// Force one refresh after the server rejected `stale`.
///
/// Callers pass the token that got a 401. If another request already refreshed
/// it in the meantime, the replacement is returned without a second refresh.
pub async fn refreshed_access_token(stale: &str) -> Result<String> {
    let store = store();
    let _guard = store.refresh_lock.lock().await;
    let Some(tokens) = store.get() else {
        return Err(AppError::msg(SIGN_IN_REQUIRED));
    };
    if tokens.access_token != stale && !store::needs_refresh(now_secs(), tokens.expires_at) {
        return Ok(tokens.access_token);
    }
    refresh_now(&store, tokens)
        .await
        .map(|tokens| tokens.access_token)
}

async fn refresh_now(store: &AuthStore, current: StoredTokens) -> Result<StoredTokens> {
    let Some(refresh_token) = current
        .refresh_token
        .clone()
        .filter(|token| !token.trim().is_empty())
    else {
        clear_stale(store);
        return Err(AppError::msg(SIGN_IN_AGAIN));
    };

    let discovery = discovery::get().await?;
    let next = match tokens::refresh(&discovery, &refresh_token).await {
        Ok(next) => next,
        Err(tokens::TokenFailure::InvalidGrant) => {
            clear_stale(store);
            return Err(AppError::msg(SIGN_IN_AGAIN));
        }
        Err(tokens::TokenFailure::Failed(err)) => return Err(err),
    };

    let stored = StoredTokens {
        access_token: next.access_token,
        // Authentik rotates refresh tokens: every refresh kills the old one.
        // Keeping the replacement is what makes the next refresh work.
        refresh_token: next.refresh_token.or(Some(refresh_token)),
        expires_at: now_secs().saturating_add(
            next.expires_in
                .unwrap_or(DEFAULT_ACCESS_TTL_SECS)
                .min(i64::MAX as u64) as i64,
        ),
        sub: current.sub,
        name: current.name,
        email: current.email,
    };

    // Persist before returning: if the rotated token is lost here, the server
    // will reject the old one on the next refresh.
    store.put(stored.clone()).map_err(|err| {
        AppError::msg(format!(
            "Could not save the refreshed Authentik session: {err}"
        ))
    })?;
    log::info!("refreshed the Authentik access token");
    Ok(stored)
}

fn clear_stale(store: &AuthStore) {
    if let Err(err) = store.clear() {
        log::warn!("could not clear the expired Authentik sign-in: {err}");
    }
}

async fn bind_callback() -> Result<TcpListener> {
    let address = std::net::SocketAddr::from(([127, 0, 0, 1], CALLBACK_PORT));
    TcpListener::bind(address).await.map_err(|err| {
        AppError::msg(format!(
            "Could not listen on {REDIRECT_URI} for the Authentik sign-in response: {err}. \
             Close any other copy of Shmingus Clips and try again."
        ))
    })
}

fn open_browser(url: &str) -> Result<()> {
    Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|err| AppError::msg(format!("Could not open your web browser to sign in: {err}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_redirect_uri_matches_the_listener_constants() {
        let url = reqwest::Url::parse(REDIRECT_URI).expect("redirect uri");
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some(CALLBACK_HOST));
        assert_eq!(url.port(), Some(CALLBACK_PORT));
        assert_eq!(url.path(), CALLBACK_PATH);
        assert_eq!(url.query(), None);
    }

    #[test]
    fn the_share_error_names_authentik() {
        // `sharing` uses this exact wording when no sign-in and no static token
        // are available.
        assert!(SIGN_IN_REQUIRED.contains("Authentik"));
    }
}
