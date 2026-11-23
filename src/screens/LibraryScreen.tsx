import {
  Clapperboard,
  Film,
  FolderOpen,
  Gamepad2,
  Link2,
  Play,
  Search,
  Star,
} from "lucide-react";
import { useState, type MouseEvent as ReactMouseEvent } from "react";

import { ClipCard } from "../components/ClipCard";
import { Alert } from "../components/ui/Alert";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/ui/EmptyState";
import { Input } from "../components/ui/Input";
import { PageHeader } from "../components/ui/PageHeader";
import { Select } from "../components/ui/Select";
import { Spinner } from "../components/ui/Spinner";
import { backend } from "../lib/bridge";
import { cx } from "../lib/cx";
import { errorMessage } from "../lib/errors";
import { formatBytes, isToday, pluralize } from "../lib/format";
import {
  isNativeContextTarget,
  useContextMenu,
} from "../state/contextMenu";
import { useApp } from "../state/store";

type Sort = "newest" | "oldest" | "longest";
type Filter = "all" | "favorites" | "shared" | `game:${string}`;

const SORT_OPTIONS = [
  { value: "newest", label: "Newest first" },
  { value: "oldest", label: "Oldest first" },
  { value: "longest", label: "Longest first" },
];

export function LibraryScreen() {
  const {
    clips,
    clipsLoading,
    clipsError,
    refreshClips,
    openPlayer,
    startBuffer,
    saveClipNow,
    status,
    busy,
    shares,
    pushToast,
  } = useApp();
  const { openContextMenu, defaultMenuItems } = useContextMenu();
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<Sort>("newest");
  const [filter, setFilter] = useState<Filter>("all");
  const [folderBusy, setFolderBusy] = useState(false);

  const games = Array.from(
    new Set(
      clips
        .map((clip) => clip.game)
        .filter((game): game is string => Boolean(game)),
    ),
  ).sort((a, b) => a.localeCompare(b));

  const favoriteCount = clips.filter((clip) => clip.favorite).length;
  const sharedCount = clips.filter((clip) => Boolean(shares[clip.id])).length;
  const gameCounts = new Map<string, number>();
  for (const clip of clips) {
    if (clip.game) gameCounts.set(clip.game, (gameCounts.get(clip.game) ?? 0) + 1);
  }
  const totalBytes = clips.reduce((total, clip) => total + clip.sizeBytes, 0);
  const clipsToday = clips.filter((clip) => isToday(clip.createdAt)).length;
  const search = query.trim().toLowerCase();
  const buffering = status.state === "buffering";

  const filtered = clips.filter((clip) => {
    if (search && !`${clip.title} ${clip.game ?? ""}`.toLowerCase().includes(search)) {
      return false;
    }
    if (filter === "favorites") return clip.favorite;
    if (filter === "shared") return Boolean(shares[clip.id]);
    if (filter.startsWith("game:")) return clip.game === filter.slice(5);
    return true;
  });

  const sorted = [...filtered].sort((a, b) => {
    if (sort === "longest") return b.durationMs - a.durationMs;
    const aTime = Date.parse(a.createdAt);
    const bTime = Date.parse(b.createdAt);
    return sort === "oldest" ? aTime - bTime : bTime - aTime;
  });

  async function openFolder() {
    setFolderBusy(true);
    try {
      await backend.openClipsFolder();
    } catch (error) {
      pushToast("error", errorMessage(error));
    } finally {
      setFolderBusy(false);
    }
  }

  function openBackgroundMenu(event: ReactMouseEvent<HTMLDivElement>) {
    // Text fields keep the native editing menu; cards bring their own.
    if (isNativeContextTarget(event.target)) return;
    event.preventDefault();
    event.stopPropagation();
    openContextMenu(event, defaultMenuItems);
  }

  return (
    <div className="page" onContextMenu={openBackgroundMenu}>
      <PageHeader
        eyebrow="Library"
        title="Your clips"
        description={
          clips.length > 0
            ? `${pluralize(clips.length, "clip")} · ${formatBytes(totalBytes)} · ${clipsToday} today`
            : "Everything you save lands here, ready to trim or export."
        }
        actions={
          <Button
            variant="secondary"
            size="sm"
            icon={folderBusy ? <Spinner /> : <FolderOpen size={16} />}
            disabled={folderBusy}
            onClick={() => void openFolder()}
          >
            Open folder
          </Button>
        }
      />

      {clipsError && (
        <Alert
          tone="warn"
          title="Could not refresh the library"
          action={
            <Button size="sm" variant="secondary" onClick={() => void refreshClips()}>
              Retry
            </Button>
          }
        >
          {clipsError}
        </Alert>
      )}

      <div className="library-toolbar">
        <Input
          icon={<Search size={16} />}
          placeholder="Search clips and games…"
          aria-label="Search clips"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          className="library-toolbar__search"
        />
        <Select
          label="Sort clips"
          value={sort}
          options={SORT_OPTIONS}
          onChange={(value) => setSort(value as Sort)}
        />
        <span
          className="library-toolbar__count"
          aria-label={`Showing ${pluralize(sorted.length, "clip")} of ${clips.length}`}
        >
          {sorted.length} of {clips.length}
        </span>
      </div>

      {clips.length > 0 && (
        <div className="filter-chips" role="group" aria-label="Filter clips">
          <button
            type="button"
            className={cx("filter-chip", filter === "all" && "is-active")}
            aria-pressed={filter === "all"}
            aria-label={`All - ${pluralize(clips.length, "clip")}`}
            title={`All - ${pluralize(clips.length, "clip")}`}
            onClick={() => setFilter("all")}
          >
            All
            <span className="filter-chip__count">{clips.length}</span>
          </button>
          <button
            type="button"
            className={cx("filter-chip", filter === "favorites" && "is-active")}
            aria-pressed={filter === "favorites"}
            aria-label={`Favorites - ${pluralize(favoriteCount, "favorite")}`}
            title={`Favorites - ${pluralize(favoriteCount, "favorite")}`}
            onClick={() => setFilter("favorites")}
          >
            <Star size={14} aria-hidden="true" />
            Favorites
            <span className="filter-chip__count">{favoriteCount}</span>
          </button>
          <button
            type="button"
            className={cx("filter-chip", filter === "shared" && "is-active")}
            aria-pressed={filter === "shared"}
            aria-label={`Shared - ${pluralize(sharedCount, "shared clip")}`}
            title={`Shared - ${pluralize(sharedCount, "shared clip")}`}
            onClick={() => setFilter("shared")}
          >
            <Link2 size={14} aria-hidden="true" />
            Shared
            <span className="filter-chip__count">{sharedCount}</span>
          </button>
          {games.map((game) => (
            <button
              key={game}
              type="button"
              className={cx(
                "filter-chip",
                filter === `game:${game}` && "is-active",
              )}
              aria-pressed={filter === `game:${game}`}
              aria-label={`${game} - ${pluralize(gameCounts.get(game) ?? 0, "clip")}`}
              title={`${game} - ${pluralize(gameCounts.get(game) ?? 0, "clip")}`}
              onClick={() => setFilter(`game:${game}`)}
            >
              <Gamepad2 size={14} aria-hidden="true" />
              {game}
            </button>
          ))}
        </div>
      )}

      {clipsLoading && clips.length === 0 ? (
        <div className="clip-grid">
          {Array.from({ length: 6 }, (_, index) => (
            <div key={index} className="clip-card clip-card--skeleton skeleton" />
          ))}
        </div>
      ) : clips.length === 0 ? (
        <EmptyState
          icon={<Film size={30} />}
          title="No clips yet"
          message={
            buffering
              ? "The replay buffer is running. Save the last few seconds from here or with your hotkey and the clip lands in this library."
              : "Start the replay buffer, then hit Save clip after a play you want to keep. The last few seconds become an MP4 here."
          }
          actions={
            <>
              <Button
                variant="primary"
                icon={
                  busy.save ? (
                    <Spinner />
                  ) : buffering ? (
                    <Clapperboard size={16} />
                  ) : (
                    <Play size={16} />
                  )
                }
                disabled={
                  busy.save || status.state === "starting" || status.state === "saving"
                }
                onClick={() => void (buffering ? saveClipNow() : startBuffer())}
              >
                {buffering
                  ? busy.save
                    ? "Saving…"
                    : "Save clip now"
                  : "Start replay buffer"}
              </Button>
              <Button
                variant="secondary"
                icon={<FolderOpen size={16} />}
                disabled={folderBusy}
                onClick={() => void openFolder()}
              >
                Open clips folder
              </Button>
            </>
          }
        />
      ) : sorted.length === 0 ? (
        <EmptyState
          compact
          icon={filter === "shared" && !search ? <Link2 size={26} /> : <Search size={26} />}
          title={
            filter === "shared" && !search ? "No shared clips yet" : "No clips match"
          }
          message={
            filter === "shared" && !search
              ? "Share a clip from the player or its card menu and it will show up here."
              : "Try a different search or clear the active filter."
          }
          actions={
            <Button
              variant="secondary"
              size="sm"
              onClick={() => {
                setQuery("");
                setFilter("all");
              }}
            >
              Clear filters
            </Button>
          }
        />
      ) : (
        <div className="clip-grid">
          {sorted.map((clip) => (
            <ClipCard key={clip.id} clip={clip} onOpen={openPlayer} />
          ))}
        </div>
      )}
    </div>
  );
}
