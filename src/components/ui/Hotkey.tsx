import { Fragment } from "react";

import { cx } from "../../lib/cx";
import { formatAccelerator } from "../../lib/format";

interface HotkeyProps {
  accelerator: string;
  className?: string;
}

export function Hotkey({ accelerator, className }: HotkeyProps) {
  const keys = formatAccelerator(accelerator);
  return (
    <span
      className={cx("hotkey", className)}
      title={keys.join(" + ")}
      aria-label={`Hotkey: ${keys.join(" plus ")}`}
    >
      {keys.map((key, index) => (
        <Fragment key={`${key}-${index}`}>
          {index > 0 && (
            <span className="hotkey__plus" aria-hidden="true">
              +
            </span>
          )}
          <kbd className="hotkey__key">{key}</kbd>
        </Fragment>
      ))}
    </span>
  );
}
