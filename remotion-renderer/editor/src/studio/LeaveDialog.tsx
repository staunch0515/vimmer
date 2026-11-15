import { Modal } from "./Modal";

type Props = {
  /** The project's name, or the file's. */
  name: string;
  /** What leaving means, for the title: "Close", "Go to projects", "Open another". */
  action: string;
  onSave: () => void;
  onDiscard: () => void;
  onCancel: () => void;
  saving?: boolean;
};

/** Asked before leaving a document with unsaved changes: save, drop the changes, or stay. */
export const LeaveDialog = ({ name, action, onSave, onDiscard, onCancel, saving }: Props) => (
  <Modal title={`${action}: unsaved changes`} onClose={onCancel} dismissible={!saving}>
    <div className="st-form">
      <div>
        “{name}” has changes that are not saved. Save them first?
      </div>
      <div className="st-modal-foot">
        <button className="btn ghost danger" disabled={saving} onClick={onDiscard}>
          Don't save
        </button>
        <span className="spacer" />
        <button className="btn ghost" disabled={saving} onClick={onCancel}>
          Cancel
        </button>
        <button className="btn primary" data-autofocus disabled={saving} onClick={onSave}>
          {saving ? "Saving…" : "Save"}
        </button>
      </div>
    </div>
  </Modal>
);
