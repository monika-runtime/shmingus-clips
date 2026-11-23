import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { createPortal } from "react-dom";

import { cx } from "../../lib/cx";
import {
  isMenuHeading,
  isMenuSeparator,
  type MenuItem,
} from "../../state/contextMenu";

const EDGE = 8;

interface ContextMenuProps {
  x: number;
  y: number;
  items: MenuItem[];
  onClose: () => void;
}

export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState({ x, y });

  function enabledItems(): HTMLButtonElement[] {
    const menu = menuRef.current;
    if (!menu) return [];
    return Array.from(
      menu.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)'),
    );
  }

  function focusAt(index: number) {
    const list = enabledItems();
    if (list.length === 0) return;
    const wrapped = ((index % list.length) + list.length) % list.length;
    list[wrapped].focus();
  }

  // Measure once mounted, then keep the menu inside the viewport. A layout
  // effect updates the position before the first paint, so nothing flickers.
  useLayoutEffect(() => {
    const menu = menuRef.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    const left = Math.min(
      Math.max(x, EDGE),
      Math.max(EDGE, window.innerWidth - rect.width - EDGE),
    );
    const top = Math.min(
      Math.max(y, EDGE),
      Math.max(EDGE, window.innerHeight - rect.height - EDGE),
    );
    setPosition((current) =>
      current.x === left && current.y === top ? current : { x: left, y: top },
    );
  }, [x, y]);

  // Move focus into the menu and hand it back to whatever had it before.
  useEffect(() => {
    const previous = document.activeElement;
    const [first] = enabledItems();
    (first ?? menuRef.current)?.focus();

    return () => {
      if (previous instanceof HTMLElement && previous.isConnected) {
        previous.focus();
      }
    };
  }, []);

  // Dismiss on anything that makes the anchor point meaningless.
  useEffect(() => {
    const onPointerDown = (event: MouseEvent) => {
      if (menuRef.current?.contains(event.target as Node)) return;
      onClose();
    };
    const onDetach = () => onClose();

    document.addEventListener("mousedown", onPointerDown, true);
    window.addEventListener("scroll", onDetach, true);
    window.addEventListener("resize", onDetach);
    window.addEventListener("blur", onDetach);
    return () => {
      document.removeEventListener("mousedown", onPointerDown, true);
      window.removeEventListener("scroll", onDetach, true);
      window.removeEventListener("resize", onDetach);
      window.removeEventListener("blur", onDetach);
    };
  }, [onClose]);

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const list = enabledItems();
    const activeIndex = list.indexOf(document.activeElement as HTMLButtonElement);

    switch (event.key) {
      case "Escape":
        event.preventDefault();
        event.stopPropagation();
        onClose();
        break;
      case "Tab":
        // The menu is modal for the keyboard; Tab leaves it rather than
        // wandering into the page behind.
        event.preventDefault();
        onClose();
        break;
      case "ArrowDown":
        event.preventDefault();
        focusAt(activeIndex + 1);
        break;
      case "ArrowUp":
        event.preventDefault();
        focusAt(activeIndex <= 0 ? list.length - 1 : activeIndex - 1);
        break;
      case "Home":
        event.preventDefault();
        focusAt(0);
        break;
      case "End":
        event.preventDefault();
        focusAt(list.length - 1);
        break;
      case "Enter":
      case " ":
        event.preventDefault();
        if (document.activeElement instanceof HTMLButtonElement) {
          document.activeElement.click();
        }
        break;
      default:
        break;
    }
  }

  return createPortal(
    <div
      ref={menuRef}
      className="context-menu"
      role="menu"
      aria-label="Context menu"
      tabIndex={-1}
      style={{ left: position.x, top: position.y }}
      onKeyDown={onKeyDown}
    >
      {items.map((item, index) => {
        if (isMenuSeparator(item)) {
          return (
            <div
              key={`separator-${index}`}
              className="context-menu__separator"
              role="separator"
            />
          );
        }
        if (isMenuHeading(item)) {
          return (
            <div
              key={`heading-${index}`}
              className="context-menu__heading"
              role="presentation"
            >
              {item.heading}
            </div>
          );
        }
        const Icon = item.icon;
        return (
          <button
            key={`${index}-${item.label}`}
            type="button"
            role="menuitem"
            tabIndex={-1}
            className={cx("context-menu__item", item.danger && "is-danger")}
            disabled={item.disabled}
            onMouseEnter={(event) => {
              if (!item.disabled) event.currentTarget.focus();
            }}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            <span className="context-menu__icon" aria-hidden="true">
              {Icon ? <Icon size={16} /> : null}
            </span>
            <span className="context-menu__label">{item.label}</span>
            {item.shortcut && (
              <span className="context-menu__shortcut">{item.shortcut}</span>
            )}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}
