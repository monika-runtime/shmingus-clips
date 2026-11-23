//! Client for the self-hosted clip share service.
//!
//! Uploads are manual: the desktop app posts the MP4 (and its thumbnail, when
//! one exists) to `<share_base_url>/api/upload`, remembers the server-issued
//! delete token on disk through [`store`], and can later list or delete those
//! shares. The delete token never crosses the Tauri command boundary - the
//! frontend only ever sees [`ShareInfo`].

mod store;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

use crate::auth;
use crate::error::{AppError, Result};
use crate::model::{Clip, Settings, ShareInfo, ShareServerStatus};

use store::{ShareRecord, ShareStore};

/// Uploading a large clip can take minutes on a slow upstream.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Deleting is a tiny request; don't let a stuck server hang the UI.
const DELETE_TIMEOUT: Duration = Duration::from_secs(60);
/// The health check must fail fast so Settings can show a result immediately.
const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
/// How much of an error body is shown back to the user.
const SNIPPET_CHARS: usize = 200;

static STORE: OnceLock<Arc<ShareStore>> = OnceLock::new();

fn store() -> Arc<ShareStore> {
    STORE.get_or_init(ShareStore::load).clone()
}

/// The upload endpoint's JSON body. `deleteToken` is kept here and in the local
/// store; it is deliberately absent from [`ShareInfo`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    id: String,
    url: String,
    embed_url: String,
    video_url: String,
    delete_token: String,
    created_at: String,
}

/// Upload `clip` to the configured share server and remember the result.
///
/// The returned share never contains the delete token.
pub async fn share_clip(clip: &Clip, settings: &Settings) -> Result<ShareInfo> {
    let base = normalize_base_url(&settings.share_base_url)?;
    let mut auth = upload_auth(settings).await?;
    let client = client(UPLOAD_TIMEOUT)?;

    let mut response = send_upload(&client, &base, clip, &auth).await?;

    // An Authentik access token can be retired between two uploads (the
    // 60-second expiry margin does not cover a token the server has already
    // rejected). Refresh once and retry; the multipart body is rebuilt because
    // a Form cannot be sent twice.
    if response.status() == StatusCode::UNAUTHORIZED {
        if let UploadAuth::Authentik(stale) = &auth {
            let stale = stale.clone();
            let fresh = auth::refreshed_access_token(&stale).await?;
            auth = UploadAuth::Authentik(fresh);
            response = send_upload(&client, &base, clip, &auth).await?;
        }
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(upload_error(status, &body, &auth));
    }

    let upload: UploadResponse = serde_json::from_str(&body).map_err(|err| {
        AppError::msg(format!(
            "The share server replied with something unexpected (HTTP {}): {err}",
            status.as_u16()
        ))
    })?;

    let info = ShareInfo {
        id: upload.id,
        url: upload.url,
        embed_url: upload.embed_url,
        video_url: upload.video_url,
        created_at: upload.created_at,
    };

    let record = ShareRecord {
        share: info.clone(),
        delete_token: upload.delete_token,
        base_url: base,
    };

    if let Err(err) = store().put(&clip.id, record) {
        // The upload itself succeeded; say so rather than pretending nothing
        // reached the server (and give the user the link they can still use).
        return Err(AppError::msg(format!(
            "The clip was uploaded to {} but the share could not be saved locally: {err}",
            info.url
        )));
    }

    Ok(info)
}

/// Which credential an upload carries.
enum UploadAuth {
    /// A fresh Authentik access token, obtained (and refreshed) locally.
    Authentik(String),
    /// The legacy static token from Settings, for signed-out installs.
    Static(String),
}

impl UploadAuth {
    fn token(&self) -> &str {
        match self {
            Self::Authentik(token) | Self::Static(token) => token,
        }
    }
}

/// Pick the credential for this upload: Authentik when signed in, otherwise the
/// static token. With neither, the upload cannot work - say what to do.
async fn upload_auth(settings: &Settings) -> Result<UploadAuth> {
    if auth::is_signed_in() {
        let token = auth::access_token().await?;
        return Ok(UploadAuth::Authentik(token));
    }

    match bearer_token(settings.share_token.as_deref()) {
        Some(token) => Ok(UploadAuth::Static(token.to_string())),
        None => Err(AppError::msg("Sign in with Authentik to upload clips.")),
    }
}

async fn send_upload(
    client: &reqwest::Client,
    base: &str,
    clip: &Clip,
    auth: &UploadAuth,
) -> Result<reqwest::Response> {
    let form = upload_form(clip).await?;
    client
        .post(format!("{base}/api/upload"))
        .bearer_auth(auth.token())
        .multipart(form)
        .send()
        .await
        .map_err(|err| AppError::msg(format!("Could not reach the share server at {base}: {err}")))
}

/// Build the multipart body. The video is mandatory; the thumbnail is not.
async fn upload_form(clip: &Clip) -> Result<Form> {
    let video = file_part(&clip.path, "video/mp4").await?;
    let mut form = Form::new().part("file", video);

    if let Some(thumbnail) = &clip.thumbnail {
        if Path::new(thumbnail).is_file() {
            match file_part(thumbnail, "image/jpeg").await {
                Ok(part) => form = form.part("thumbnail", part),
                // The thumbnail is a nicety; a share without one is still useful.
                Err(err) => log::warn!("sharing without a thumbnail: {err}"),
            }
        }
    }

    Ok(form
        .text("title", clip.title.clone())
        .text("game", clip.game.clone().unwrap_or_default()))
}

/// The remembered share of `clip_id`, if it was uploaded from this machine.
pub fn get_share(clip_id: &str) -> Option<ShareInfo> {
    store().get(clip_id)
}

/// Every remembered share, clip id -> share.
pub fn list_shares() -> HashMap<String, ShareInfo> {
    store().all()
}

/// Delete the share of `clip_id` on the server it was uploaded to.
///
/// The local record is dropped when the server confirms the deletion (2xx) or
/// says it is already gone (404). Any other outcome keeps the record so the
/// user can retry.
pub async fn delete_share(clip_id: &str) -> Result<()> {
    let store = store();
    let Some(record) = store.record(clip_id) else {
        // Nothing to delete: deleting an unshared clip is a success.
        return Ok(());
    };

    let url = format!(
        "{}/api/shares/{}",
        record.base_url.trim_end_matches('/'),
        record.share.id
    );
    let client = client(DELETE_TIMEOUT)?;

    let response = client
        .delete(&url)
        .header("X-Delete-Token", record.delete_token.as_str())
        .send()
        .await
        .map_err(|err| {
            AppError::msg(format!(
                "Could not reach the share server to delete this share: {err}"
            ))
        })?;

    let status = response.status();
    if status.is_success() || status == StatusCode::NOT_FOUND {
        store.remove(clip_id).map_err(|err| {
            AppError::msg(format!(
                "The share was deleted on the server, but the local record could not be removed: {err}"
            ))
        })?;
        return Ok(());
    }

    let body = response.text().await.unwrap_or_default();
    Err(delete_error(status, &body))
}

/// Ask the configured server how it is doing.
///
/// A missing or malformed base URL is an error with a fix in it; an unreachable
/// or unhealthy server is reported as `ok: false` so Settings can show the
/// detail instead of an opaque failure.
pub async fn test_share_server(settings: &Settings) -> Result<ShareServerStatus> {
    let base = normalize_base_url(&settings.share_base_url)?;
    let client = client(HEALTH_TIMEOUT)?;

    let mut request = client.get(format!("{base}/healthz"));
    if let Some(token) = bearer_token(settings.share_token.as_deref()) {
        request = request.bearer_auth(token);
    }

    match request.send().await {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if status.is_success() {
                Ok(ShareServerStatus {
                    ok: true,
                    detail: format!("Connected (HTTP {})", status.as_u16()),
                })
            } else {
                Ok(ShareServerStatus {
                    ok: false,
                    detail: describe_failure(status, &body),
                })
            }
        }
        Err(err) => Ok(ShareServerStatus {
            ok: false,
            detail: format!("Could not reach {base}: {err}"),
        }),
    }
}

/// Validate and normalise a user-entered share server URL.
///
/// Returns the base without a trailing slash, ready to append `/api/...` to.
/// Empty input is by far the common case (sharing was never configured), so it
/// gets its own message pointing at the settings screen.
pub fn normalize_base_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::msg(
            "Set the share server URL in Settings → Sharing.",
        ));
    }

    let Ok(url) = reqwest::Url::parse(trimmed) else {
        return Err(invalid_base_url());
    };
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid_base_url());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(AppError::msg(
            "The share server URL must not contain a query or fragment (Settings → Sharing).",
        ));
    }

    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn invalid_base_url() -> AppError {
    AppError::msg(
        "The share server URL must be a full http:// or https:// address (Settings → Sharing).",
    )
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

/// Build a multipart file part.
///
/// reqwest can stream a file with `Part::file`, but that needs the crate's
/// `stream` feature, which this app does not enable (and `Cargo.toml` is not
/// ours to change). Reading the file into memory is the supported fallback and
/// is fine for clips up to 512 MB.
async fn file_part(path: &str, mime: &str) -> Result<Part> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|err| AppError::msg(format!("Could not read {path}: {err}")))?;
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "upload.bin".to_string());

    Part::bytes(bytes)
        .file_name(name)
        .mime_str(mime)
        .map_err(|err| AppError::msg(format!("invalid content type {mime}: {err}")))
}

fn client(timeout: Duration) -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|err| AppError::msg(format!("could not start the HTTP client: {err}")))
}

/// An upload token is optional; blank or whitespace-only is the same as unset.
fn bearer_token(token: Option<&str>) -> Option<&str> {
    token.map(str::trim).filter(|token| !token.is_empty())
}

fn upload_error(status: StatusCode, body: &str, auth: &UploadAuth) -> AppError {
    match (status.as_u16(), auth) {
        (401 | 403, UploadAuth::Authentik(_)) => AppError::msg(
            "The share server rejected your Authentik sign-in. Sign in again, then retry the upload.",
        ),
        (401 | 403, UploadAuth::Static(_)) => AppError::msg(
            "The share server rejected the upload token. Check the upload token in Settings → Sharing.",
        ),
        (413, _) => AppError::msg("The share server refused the clip because it is too large."),
        (code, _) => with_snippet(format!("Upload failed (HTTP {code})"), body),
    }
}

fn delete_error(status: StatusCode, body: &str) -> AppError {
    match status.as_u16() {
        401 | 403 => AppError::msg(
            "The share server refused to delete this share (its delete token was not accepted).",
        ),
        code => with_snippet(format!("Could not delete the share (HTTP {code})"), body),
    }
}

fn describe_failure(status: StatusCode, body: &str) -> String {
    let reason = status.canonical_reason().unwrap_or("error");
    let snippet = snippet(body);
    if snippet.is_empty() {
        format!("HTTP {} {reason}", status.as_u16())
    } else {
        format!("HTTP {} {reason}: {snippet}", status.as_u16())
    }
}

fn with_snippet(prefix: String, body: &str) -> AppError {
    let snippet = snippet(body);
    if snippet.is_empty() {
        AppError::msg(prefix)
    } else {
        AppError::msg(format!("{prefix}: {snippet}"))
    }
}

/// A one-line, length-limited excerpt of an HTTP error body.
fn snippet(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= SNIPPET_CHARS {
        return collapsed;
    }
    let mut cut = collapsed.chars().take(SNIPPET_CHARS).collect::<String>();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_base_url_points_at_the_settings_screen() {
        let err = normalize_base_url("   ").unwrap_err().to_string();
        assert!(err.contains("Settings → Sharing"), "{err}");
    }

    #[test]
    fn a_base_url_must_be_http_or_https_with_a_host() {
        for bad in [
            "example.com",
            "ftp://clips.example.com",
            "file:///tmp/clips",
            "http://",
            "https://",
        ] {
            assert!(
                normalize_base_url(bad).is_err(),
                "{bad} should have been rejected"
            );
        }
    }

    #[test]
    fn a_base_url_must_not_carry_a_query_or_fragment() {
        assert!(normalize_base_url("https://clips.example.com/?debug=1").is_err());
        assert!(normalize_base_url("https://clips.example.com/#top").is_err());
    }

    #[test]
    fn a_base_url_is_trimmed_and_loses_trailing_slashes() {
        assert_eq!(
            normalize_base_url(" https://clips.example.com/ ").unwrap(),
            "https://clips.example.com"
        );
        assert_eq!(
            normalize_base_url("http://localhost:21020").unwrap(),
            "http://localhost:21020"
        );
        assert_eq!(
            normalize_base_url("https://example.com/clips/").unwrap(),
            "https://example.com/clips"
        );
    }

    #[test]
    fn blank_upload_tokens_are_treated_as_unset() {
        assert_eq!(bearer_token(Some("  secret ")), Some("secret"));
        assert_eq!(bearer_token(Some("   ")), None);
        assert_eq!(bearer_token(None), None);
    }

    #[test]
    fn error_bodies_are_collapsed_and_shortened() {
        assert_eq!(snippet("  bad\n\nrequest  "), "bad request");
        let long = "x".repeat(SNIPPET_CHARS + 50);
        let short = snippet(&long);
        assert_eq!(short.chars().count(), SNIPPET_CHARS + 1);
        assert!(short.ends_with('…'));
    }

    #[test]
    fn a_rejected_authentik_upload_asks_to_sign_in_again() {
        let err = upload_error(
            StatusCode::UNAUTHORIZED,
            "",
            &UploadAuth::Authentik("test-access-token".into()),
        )
        .to_string();
        assert!(err.contains("rejected your Authentik sign-in"), "{err}");
    }

    #[test]
    fn a_rejected_static_token_still_points_at_the_settings_screen() {
        let err = upload_error(
            StatusCode::UNAUTHORIZED,
            "",
            &UploadAuth::Static("static-token".into()),
        )
        .to_string();
        assert!(err.contains("upload token in Settings"), "{err}");
    }
}
