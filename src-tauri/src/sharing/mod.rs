//! Client for the self-hosted clip share service.
//!
//! Uploads are manual: the desktop app posts the MP4 (and its thumbnail, when
//! one exists) to `<share_base_url>/api/upload`, remembers the server-issued
//! delete token on disk through [`store`], and can later list or delete those
//! shares. The delete token never crosses the Tauri command boundary - the
//! frontend only ever sees [`ShareInfo`].

mod store;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use reqwest::StatusCode;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

use crate::auth;
use crate::error::{AppError, Result};
use crate::media;
use crate::model::{
    Clip, ServerRequirement, ServerShare, Settings, ShareInfo, ShareServerStatus,
};

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

/// Share preset: chat clients (Discord above all) will not fetch a large
/// `og:video`, so anything over this is re-encoded on this machine before it is
/// uploaded. Clips already inside the cap are sent untouched.
const SHARE_PRESET_BYTES: u64 = 10 * 1024 * 1024;
/// Aim slightly under the cap so container overhead cannot push it over.
const SHARE_PRESET_TARGET: u64 = 9_500_000;
const SHARE_PRESET_AUDIO_KBPS: f64 = 64.0;
/// Below this the result would be unwatchable; long clips keep their original
/// and the server advertises only the thumbnail card.
const SHARE_PRESET_MIN_VIDEO_KBPS: f64 = 200.0;

struct TempCopy(PathBuf);

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct ShareCopy {
    path: PathBuf,
    _guard: Option<TempCopy>,
}

/// The file to upload: the clip itself, or a compact preset copy for large
/// clips. Building the copy is best effort; on failure the original is used.
async fn share_source(clip: &Clip) -> ShareCopy {
    let original = PathBuf::from(&clip.path);
    let size = std::fs::metadata(&original)
        .map(|meta| meta.len())
        .unwrap_or(0);
    if size <= SHARE_PRESET_BYTES {
        return ShareCopy {
            path: original,
            _guard: None,
        };
    }

    let out = crate::settings::cache_dir()
        .join("share")
        .join(format!("{}.mp4", uuid::Uuid::new_v4()));
    if let Some(parent) = out.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return ShareCopy { path: original, _guard: None };
        }
    }
    let input = original.clone();
    let job_out = out.clone();
    let job = tokio::task::spawn_blocking(move || transcode_for_share(&input, &job_out)).await;
    match job {
        Ok(Ok(())) => {
            let size = std::fs::metadata(&out).map(|meta| meta.len()).unwrap_or(u64::MAX);
            if size <= SHARE_PRESET_BYTES {
                log::info!("share preset: compact copy is {} bytes", size);
                ShareCopy {
                    path: out.clone(),
                    _guard: Some(TempCopy(out)),
                }
            } else {
                let _ = std::fs::remove_file(&out);
                log::warn!("share preset stayed over 10 MB; uploading the original");
                ShareCopy { path: original, _guard: None }
            }
        }
        Ok(Err(err)) => {
            log::warn!("share preset failed ({err}); uploading the original");
            ShareCopy { path: original, _guard: None }
        }
        Err(err) => {
            log::warn!("share preset task failed ({err}); uploading the original");
            ShareCopy { path: original, _guard: None }
        }
    }
}

/// Video bitrate and width that should land under the preset cap.
fn preset_bitrate(duration_ms: u64) -> Option<(u32, u32)> {
    let seconds = (duration_ms as f64 / 1000.0).max(0.5);
    let total_kbps = SHARE_PRESET_TARGET as f64 * 8.0 / seconds / 1000.0;
    let video = total_kbps - SHARE_PRESET_AUDIO_KBPS;
    if video < SHARE_PRESET_MIN_VIDEO_KBPS {
        return None;
    }
    Some(if video >= 600.0 {
        (video as u32, 1280)
    } else if video >= 300.0 {
        (video as u32, 960)
    } else {
        (video as u32, 854)
    })
}

fn transcode_for_share(input: &Path, out: &Path) -> Result<()> {
    let probe = media::probe(input)?;
    let Some((mut video_kbps, mut width)) = preset_bitrate(probe.duration_ms) else {
        return Err(AppError::msg(
            "the clip is too long to fit the 10 MB share preset at a watchable quality",
        ));
    };

    for attempt in 0..2 {
        let scale = format!(
            "scale='if(gt(iw,ih),min({width},iw),-2)':'if(gt(iw,ih),-2,min({width},ih))'"
        );
        let bitrate = format!("{video_kbps}k");
        let maxrate = format!("{}k", video_kbps * 2);
        let bufsize = format!("{}k", video_kbps * 4);
        media::run(
            "ffmpeg",
            &[
                "-y".as_ref(),
                "-v".as_ref(),
                "error".as_ref(),
                "-i".as_ref(),
                input.as_os_str(),
                "-map".as_ref(),
                "0:v:0".as_ref(),
                "-map".as_ref(),
                "0:a:0?".as_ref(),
                "-vf".as_ref(),
                scale.as_ref(),
                "-c:v".as_ref(),
                "libx264".as_ref(),
                "-preset".as_ref(),
                "veryfast".as_ref(),
                "-b:v".as_ref(),
                bitrate.as_ref(),
                "-maxrate".as_ref(),
                maxrate.as_ref(),
                "-bufsize".as_ref(),
                bufsize.as_ref(),
                "-pix_fmt".as_ref(),
                "yuv420p".as_ref(),
                "-c:a".as_ref(),
                "aac".as_ref(),
                "-b:a".as_ref(),
                "64k".as_ref(),
                "-ac".as_ref(),
                "2".as_ref(),
                "-movflags".as_ref(),
                "+faststart".as_ref(),
                out.as_os_str(),
            ],
        )?;

        let size = std::fs::metadata(out).map(|meta| meta.len()).unwrap_or(u64::MAX);
        if size <= SHARE_PRESET_BYTES {
            return Ok(());
        }
        if attempt == 0 {
            let ratio = SHARE_PRESET_TARGET as f64 / size as f64 * 0.85;
            video_kbps = ((video_kbps as f64 * ratio).max(120.0)) as u32;
            width = width.min(854);
            let _ = std::fs::remove_file(out);
        }
    }
    Ok(())
}

/// One upload per clip at a time, even across UI surfaces.
static SHARING_IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn sharing_slot() -> &'static Mutex<HashSet<String>> {
    SHARING_IN_FLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

struct SharingGuard(String);

impl Drop for SharingGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = sharing_slot().lock() {
            set.remove(&self.0);
        }
    }
}

/// Upload `clip` to the configured share server and remember the result.
///
/// The returned share never contains the delete token.
pub async fn share_clip(clip: &Clip, settings: &Settings) -> Result<ShareInfo> {
    // A clip that already has a share is never uploaded twice; the stored
    // record is authoritative even when the interface lost track of it.
    if let Some(existing) = store().record(&clip.id) {
        return Ok(existing.share);
    }
    let _guard = {
        let mut in_flight = sharing_slot()
            .lock()
            .map_err(|_| AppError::msg("the share queue is unavailable"))?;
        if !in_flight.insert(clip.id.clone()) {
            return Err(AppError::msg("This clip is already being shared."));
        }
        SharingGuard(clip.id.clone())
    };

    let base = normalize_base_url(&settings.share_base_url)?;
    // Catch a server that has moved on before any upload is attempted.
    if let Ok(requirement) = server_requirement(settings).await {
        if requirement.required {
            let minimum = requirement.min_version.unwrap_or_default();
            return Err(AppError::msg(format!(
                "This share server needs Shmingus Clips {minimum} or newer (this build is {}). Update Shmingus Clips to share clips.",
                requirement.current_version
            )));
        }
    }
    let mut auth = upload_auth(settings).await?;
    let client = client(UPLOAD_TIMEOUT)?;
    let source = share_source(clip).await;

    let mut response = send_upload(&client, &base, clip, &source.path, &auth).await?;

    // An Authentik access token can be retired between two uploads (the
    // 60-second expiry margin does not cover a token the server has already
    // rejected). Refresh once and retry; the multipart body is rebuilt because
    // a Form cannot be sent twice.
    if response.status() == StatusCode::UNAUTHORIZED {
        if let UploadAuth::Authentik(stale) = &auth {
            let stale = stale.clone();
            let fresh = auth::refreshed_access_token(&stale).await?;
            auth = UploadAuth::Authentik(fresh);
            response = send_upload(&client, &base, clip, &source.path, &auth).await?;
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

/// Which credential an upload carries. Uploads are account-only.
enum UploadAuth {
    /// A fresh Authentik access token, obtained (and refreshed) locally.
    Authentik(String),
}

impl UploadAuth {
    fn token(&self) -> &str {
        match self {
            Self::Authentik(token) => token,
        }
    }
}

/// Pick the credential for this upload: Authentik when signed in, otherwise the
/// static token. With neither, the upload cannot work - say what to do.
async fn upload_auth(_settings: &Settings) -> Result<UploadAuth> {
    if auth::is_signed_in() {
        let token = auth::access_token().await?;
        return Ok(UploadAuth::Authentik(token));
    }
    Err(AppError::msg(
        "Sign in with your Shmingus account to upload clips.",
    ))
}

async fn send_upload(
    client: &reqwest::Client,
    base: &str,
    clip: &Clip,
    video: &Path,
    auth: &UploadAuth,
) -> Result<reqwest::Response> {
    let form = upload_form(clip, video).await?;
    client
        .post(format!("{base}/api/upload"))
        .bearer_auth(auth.token())
        .multipart(form)
        .send()
        .await
        .map_err(|err| AppError::msg(format!("Could not reach the share server at {base}: {err}")))
}

/// Build the multipart body. The video is mandatory; the thumbnail is not.
async fn upload_form(clip: &Clip, video_path: &Path) -> Result<Form> {
    let video = file_part(video_path, "video/mp4").await?;
    let mut form = Form::new().part("file", video);

    if let Some(thumbnail) = &clip.thumbnail {
        if Path::new(thumbnail).is_file() {
            match file_part(Path::new(thumbnail), "image/jpeg").await {
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

/// Compare dotted versions numerically; pre-release suffixes are ignored.
fn version_at_least(current: &str, minimum: &str) -> bool {
    let parse = |value: &str| -> Vec<u64> {
        value
            .split('.')
            .map(|part| {
                part.chars()
                    .take_while(|ch| ch.is_ascii_digit())
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0)
            })
            .collect()
    };
    let mut a = parse(current);
    let mut b = parse(minimum);
    a.resize(3, 0);
    b.resize(3, 0);
    a >= b
}

/// Ask the server which client it still supports.
pub async fn server_requirement(settings: &Settings) -> Result<ServerRequirement> {
    let base = normalize_base_url(&settings.share_base_url)?;
    let client = client(HEALTH_TIMEOUT)?;
    let response = client
        .get(format!("{base}/healthz"))
        .send()
        .await
        .map_err(|err| {
            AppError::msg(format!("Could not reach the share server at {base}: {err}"))
        })?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|err| AppError::msg(format!("The share server health reply was unreadable: {err}")))?;
    if !status.is_success() {
        return Err(AppError::msg(format!(
            "The share server health check failed (HTTP {})",
            status.as_u16()
        )));
    }
    let min_version = body
        .get("minClientVersion")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let server_version = body
        .get("version")
        .and_then(|value| value.as_str())
        .map(str::to_string);
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let required = min_version
        .as_deref()
        .map(|minimum| !version_at_least(&current_version, minimum))
        .unwrap_or(false);
    Ok(ServerRequirement {
        required,
        current_version,
        min_version,
        server_version,
    })
}

/// Every share the signed-in account owns, as the server sees it.
pub async fn list_account_shares(settings: &Settings) -> Result<Vec<ServerShare>> {
    let base = normalize_base_url(&settings.share_base_url)?;
    let credential = upload_auth(settings).await?;
    let client = client(Duration::from_secs(12))?;
    let response = client
        .get(format!("{base}/api/shares"))
        .bearer_auth(credential.token())
        .send()
        .await
        .map_err(|err| {
            AppError::msg(format!("Could not reach the share server at {base}: {err}"))
        })?;
    let status = response.status();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(AppError::msg(
            "Sign in with your Shmingus account to see your shares.",
        ));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(with_snippet(
            format!("Could not list your shares (HTTP {})", status.as_u16()),
            &body,
        ));
    }
    response
        .json::<Vec<ServerShare>>()
        .await
        .map_err(|err| AppError::msg(format!("The share server sent an unexpected list: {err}")))
}

/// Delete a share by its server id using the account's own session.
pub async fn delete_server_share(settings: &Settings, id: &str) -> Result<()> {
    let base = normalize_base_url(&settings.share_base_url)?;
    let credential = upload_auth(settings).await?;
    let client = client(Duration::from_secs(12))?;
    let response = client
        .delete(format!("{base}/api/shares/{id}"))
        .bearer_auth(credential.token())
        .send()
        .await
        .map_err(|err| {
            AppError::msg(format!("Could not reach the share server at {base}: {err}"))
        })?;
    let status = response.status();
    if status.is_success() || status == StatusCode::NOT_FOUND {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    Err(delete_error(status, &body))
}

/// Keep the public page in step with a rename. Best effort: the caller has
/// already renamed the clip locally and only needs to warn when this fails.
pub async fn update_share_title(clip_id: &str, title: &str, settings: &Settings) -> Result<()> {
    let Some(record) = store().record(clip_id) else {
        return Ok(());
    };
    let credential = upload_auth(settings).await?;
    let client = client(Duration::from_secs(8))?;
    let url = format!("{}/api/shares/{}", record.base_url, record.share.id);
    let response = client
        .patch(&url)
        .bearer_auth(credential.token())
        .json(&serde_json::json!({ "title": title }))
        .send()
        .await
        .map_err(|err| {
            AppError::msg(format!("Could not reach the share server at {url}: {err}"))
        })?;

    if response.status().is_success() {
        return Ok(());
    }
    let status = response.status();
    let detail = response.text().await.unwrap_or_default();
    let detail = detail.trim();
    Err(AppError::msg(if detail.is_empty() {
        format!("The share server refused the rename ({status}).")
    } else {
        format!("The share server refused the rename ({status}): {detail}")
    }))
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

    let mut request = client
        .delete(&url)
        .header("X-Delete-Token", record.delete_token.as_str());
    // The uploader's account may delete its own share even without the token,
    // which is what makes the manager work after a reinstall.
    if auth::is_signed_in() {
        match auth::access_token().await {
            Ok(token) => request = request.bearer_auth(token),
            Err(err) => log::warn!("could not attach the account to the share delete: {err}"),
        }
    }
    let response = request
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

    let request = client.get(format!("{base}/healthz"));

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
async fn file_part(path: &Path, mime: &str) -> Result<Part> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|err| AppError::msg(format!("Could not read {}: {err}", path.display())))?;
    let name = path
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

fn upload_error(status: StatusCode, body: &str, auth: &UploadAuth) -> AppError {
    match (status.as_u16(), auth) {
        (401 | 403, UploadAuth::Authentik(_)) => AppError::msg(
            "The share server rejected your Authentik sign-in. Sign in again, then retry the upload.",
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
    fn version_compare_handles_short_and_suffixed_versions() {
        assert!(version_at_least("0.5.10", "0.5.9"));
        assert!(!version_at_least("0.5.9", "0.5.10"));
        assert!(version_at_least("1.0", "1.0.0"));
        assert!(version_at_least("0.6.0", "0.5.9"));
        assert!(!version_at_least("0.5.9-beta", "0.5.10"));
    }

    #[test]
    fn preset_bitrate_stays_inside_the_cap() {
        for duration_ms in [5_000u64, 30_000, 120_000, 600_000] {
            let Some((video_kbps, width)) = super::preset_bitrate(duration_ms) else {
                continue;
            };
            let seconds = duration_ms as f64 / 1000.0;
            let projected =
                (video_kbps as f64 + super::SHARE_PRESET_AUDIO_KBPS) * 1000.0 * seconds / 8.0;
            assert!(
                projected <= super::SHARE_PRESET_TARGET as f64 * 1.05,
                "{duration_ms} ms projects {projected} bytes"
            );
            assert!(video_kbps >= 120);
            assert!(width <= 1280);
        }
    }

    #[tokio::test]
    async fn a_large_clip_is_compressed_for_sharing() {
        if !crate::media::ffmpeg_available() {
            eprintln!("skipping: ffmpeg unavailable");
            return;
        }
        let dir = std::env::temp_dir().join(format!("share-preset-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("big.mp4");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-v", "error", "-y",
                "-f", "lavfi", "-i",
                "color=c=gray:s=640x360:r=30:d=2,noise=alls=100:allf=t",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
                "-map", "0:v", "-map", "1:a",
                "-c:v", "libx264", "-preset", "ultrafast", "-crf", "0",
                "-c:a", "aac", "-shortest",
            ])
            .arg(&input)
            .status()
            .expect("run ffmpeg");
        assert!(status.success());
        let size = std::fs::metadata(&input).unwrap().len();
        assert!(size > super::SHARE_PRESET_BYTES, "fixture is only {size} bytes");
        let probe = crate::media::probe(&input).unwrap();

        let clip = Clip {
            id: "preset-test".into(),
            title: "Preset test".into(),
            game: None,
            path: input.to_string_lossy().into_owned(),
            thumbnail: None,
            created_at: String::new(),
            duration_ms: probe.duration_ms,
            size_bytes: size,
            width: probe.width,
            height: probe.height,
            favorite: false,
        };

        let copy = super::share_source(&clip).await;
        assert!(copy._guard.is_some(), "expected a temporary preset copy");
        let out_size = std::fs::metadata(&copy.path).unwrap().len();
        assert!(
            out_size <= super::SHARE_PRESET_BYTES,
            "preset copy is {out_size} bytes"
        );
        let out_probe = crate::media::probe(&copy.path).unwrap();
        assert!(out_probe.audio_streams >= 1);
        assert!(out_probe.duration_ms > 1500);
        drop(copy);
        let _ = std::fs::remove_dir_all(dir);
    }

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

}
