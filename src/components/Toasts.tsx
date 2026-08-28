export interface Toast {
  readonly id: number;
  readonly tone: "error" | "ok";
  readonly title: string;
  readonly body: string;
}

interface Props {
  readonly toasts: readonly Toast[];
  readonly onDismiss: (id: number) => void;
}

export function Toasts({ toasts, onDismiss }: Props) {
  if (toasts.length === 0) return null;
  return (
    <div className="toasts">
      {toasts.map((t) => (
        <div key={t.id} className={`toast toast--${t.tone}`} role="status">
          <div className="toast__body">
            <div className="toast__title">{t.title}</div>
            {t.body}
          </div>
          <button
            className="toast__close"
            onClick={() => onDismiss(t.id)}
            aria-label="Dismiss"
            title="Dismiss"
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
