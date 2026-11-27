import { backend } from "../lib/bridge";
import { errorMessage } from "../lib/errors";
import type { AuthStatus, Clip, ShareInfo } from "../lib/types";
import { useApp } from "./store";

/** Upload errors that mean the share server wants an Authentik session. */
const AUTH_ERROR = /sign[- ]?in|401|unauthori[sz]ed|authentik/i;

function isAuthError(error: unknown): boolean {
  return AUTH_ERROR.test(errorMessage(error));
}

/**
 * Opening a clip's share: reuse the link we already have, otherwise upload it
 * and open the dialog once the server answers.
 *
 * Uploads need an Authentik account unless the server still uses a static
 * upload token. When the user is signed out the browser opens for sign-in
 * first - they already clicked Share, so this is the action they asked for.
 */
export function useShareActions() {
  const {
    settings,
    shares,
    sharingClipId,
    setSharingClip,
    setShare,
    openShareDialog,
    openSettings,
    pushToast,
  } = useApp();


  function offerSignIn(message: string) {
    pushToast("error", message, {
      label: "Open sharing settings",
      run: () => openSettings("sharing"),
    });
    openSettings("sharing");
  }

  async function share(clip: Clip): Promise<void> {
    // An existing link can still be opened even when sharing has since been
    // switched off; only a new upload needs a server.
    const known = shares[clip.id];
    if (known) {
      openShareDialog(clip, known);
      return;
    }

    if (!settings?.shareBaseUrl.trim()) {
      pushToast(
        "error",
        "Add your share server URL in Settings → Sharing before sharing a clip.",
        {
          label: "Open sharing settings",
          run: () => openSettings("sharing"),
        },
      );
      return;
    }

    // The in-memory list can lag behind a share made before this window
    // opened. Ask the backend before doing any work, so an existing link is
    // reused instead of uploading a second copy (and no sign-in is needed).
    let found: ShareInfo | null = null;
    try {
      found = await backend.getShare(clip.id);
    } catch {
      // Ignored on purpose: share_clip below gives the actionable error.
    }
    if (found) {
      setShare(clip.id, found);
      openShareDialog(clip, found);
      return;
    }

    setSharingClip(clip.id);
    pushToast("info", `Preparing and uploading “${clip.title}”…`);
    try {
      // Uploads are account-only. Sign in first when there is no session.
      let auth: AuthStatus | null = null;
      try {
        auth = await backend.authStatus();
      } catch {
        // An older build without auth commands cannot answer. Let the upload
        // report the real problem.
        auth = null;
      }

      if (auth && !auth.signedIn) {
        pushToast(
          "info",
          "Opening your browser to sign in to your Shmingus account…",
        );
        try {
          auth = await backend.authLogin();
        } catch (error) {
          offerSignIn(`Could not sign in: ${errorMessage(error)}`);
          return;
        }
        if (!auth.signedIn) {
          offerSignIn("Sign-in did not finish. Sign in, then share the clip again.");
          return;
        }
        pushToast(
          "success",
          `Signed in as ${auth.name ?? auth.sub ?? "your Shmingus account"}.`,
        );
      }

      const uploaded = await backend.shareClip(clip.id);
      setShare(clip.id, uploaded);
      openShareDialog(clip, uploaded);
      pushToast("success", `Uploaded “${clip.title}”.`);
    } catch (error) {
      const detail = `Could not share “${clip.title}”: ${errorMessage(error)}`;
      if (isAuthError(error)) {
        offerSignIn(detail);
      } else {
        pushToast("error", detail);
      }
    } finally {
      setSharingClip(null);
    }
  }

  return {
    share,
    uploadingId: sharingClipId,
    isUploading: (clipId: string) => sharingClipId === clipId,
  };
}
