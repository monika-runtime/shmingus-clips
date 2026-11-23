import { Film, Play } from "lucide-react";
import { useEffect, useState } from "react";

import { mediaUrl } from "../lib/bridge";
import { cx } from "../lib/cx";
import type { Clip } from "../lib/types";

interface ClipThumbProps {
  clip: Clip;
  className?: string;
}

export function ClipThumb({ clip, className }: ClipThumbProps) {
  const source = clip.thumbnail ? mediaUrl(clip.thumbnail) : null;
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    setFailed(false);
  }, [source]);

  return (
    <div className={cx("thumb", className)}>
      {source && !failed ? (
        <img
          className="thumb__image"
          src={source}
          alt=""
          loading="lazy"
          draggable={false}
          onError={() => setFailed(true)}
        />
      ) : (
        <div className="thumb__fallback" aria-hidden="true">
          <Film size={22} />
        </div>
      )}
      <span className="thumb__veil" aria-hidden="true" />
      <span className="thumb__play" aria-hidden="true">
        <Play size={20} fill="currentColor" />
      </span>
    </div>
  );
}
