import { AlertTriangle, CheckCircle2, Info, X } from "lucide-react";

import type { ToastItem } from "../../state/store";

const ICONS = {
  info: <Info size={17} />,
  success: <CheckCircle2 size={17} />,
  error: <AlertTriangle size={17} />,
};

interface ToastsProps {
  toasts: ToastItem[];
  onDismiss: (id: number) => void;
}

export function Toasts({ toasts, onDismiss }: ToastsProps) {
  if (toasts.length === 0) return null;
  return (
    <div className="toasts" aria-live="polite" aria-label="Notifications">
      {toasts.map((toast) => (
        <div key={toast.id} className={`toast toast--${toast.level}`}>
          <span className="toast__icon" aria-hidden="true">
            {ICONS[toast.level]}
          </span>
          <div className="toast__body">
            <p className="toast__message">{toast.message}</p>
            {toast.action && (
              <button
                type="button"
                className="toast__action"
                onClick={() => {
                  toast.action?.run();
                  onDismiss(toast.id);
                }}
              >
                {toast.action.label}
              </button>
            )}
          </div>
          <button
            type="button"
            className="toast__close"
            aria-label="Dismiss notification"
            onClick={() => onDismiss(toast.id)}
          >
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
