import { useEffect, useRef, type ReactNode } from "react";

type Props = {
  title: string;
  onClose: () => void;
  /** False while something runs that Escape or a click outside must not dismiss. */
  dismissible?: boolean;
  wide?: boolean;
  children: ReactNode;
};

/** A centered dialog over a dimmed backdrop. Escape and a click outside close it. */
export const Modal = ({ title, onClose, dismissible = true, wide, children }: Props) => {
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && dismissible) {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [dismissible, onClose]);
  useEffect(() => {
    // Focus the first field so typing works at once.
    const first = box.current?.querySelector<HTMLElement>("[data-autofocus], textarea, input, select, button");
    first?.focus();
  }, []);
  return (
    <div
      className="st-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget && dismissible) onClose();
      }}
    >
      <div ref={box} className={`st-modal${wide ? " wide" : ""}`} role="dialog" aria-modal="true" aria-label={title}>
        <div className="st-modal-head">
          <span className="st-modal-title">{title}</span>
          <span className="spacer" />
          {dismissible && (
            <button className="btn ghost small" onClick={onClose} aria-label="Close">
              ✕
            </button>
          )}
        </div>
        {children}
      </div>
    </div>
  );
};
