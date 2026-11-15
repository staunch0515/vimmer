import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { errorText, useStudioApi } from "./api";
import { cardHue, filterProjects, formatDuration, formatWhen } from "./format";
import { GenerateDialog } from "./GenerateDialog";
import { Modal } from "./Modal";
import type { Lang, OpenedProject, Orientation, ProjectSummary, StudioSettings } from "./types";

type Props = {
  /** A project was opened (clicked, created, generated or imported): show it in the editor. */
  onOpen: (opened: OpenedProject) => void | Promise<void>;
  /** The project open in the editor: it cannot be renamed or deleted from here. */
  currentDir?: string | null;
  /** Opens a plain timeline file outside any project, as the editor always could. */
  onOpenFile?: () => void;
  /** Goes back to the editor without opening anything (when one is already open). */
  onBack?: () => void;
};

const ORIGIN_LABEL: Record<ProjectSummary["origin"], string | null> = {
  blank: null,
  generated: "Generated",
  imported: "Imported",
};

const Thumb = ({ project }: { project: ProjectSummary }) => {
  const api = useStudioApi();
  const [broken, setBroken] = useState(false);
  if (project.thumb && !broken) {
    return (
      <img
        className="st-thumb-img"
        src={api.fileUrl(project.thumb, project.thumbVersion)}
        alt=""
        draggable={false}
        onError={() => setBroken(true)}
      />
    );
  }
  // No render and no scene image yet: a title card in the project's own colour.
  const hue = cardHue(project.name);
  return (
    <div
      className={`st-thumb-card ${project.orientation}`}
      style={{ background: `linear-gradient(135deg, hsl(${hue} 45% 22%), hsl(${(hue + 40) % 360} 50% 12%))` }}
    >
      <span>{project.name}</span>
    </div>
  );
};

type MenuProps = {
  /** The project is open in the editor: no rename, no delete. */
  locked: boolean;
  onOpen: () => void;
  onRename: () => void;
  onDuplicate: () => void;
  onReveal: () => void;
  onDelete: () => void;
  onClose: () => void;
};

const CardMenu = (p: MenuProps) => {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const away = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) p.onClose();
    };
    const key = (e: KeyboardEvent) => e.key === "Escape" && p.onClose();
    window.addEventListener("mousedown", away);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("mousedown", away);
      window.removeEventListener("keydown", key);
    };
  }, [p]);
  const item = (label: string, run: () => void, danger = false) => (
    <button
      className={`st-menu-item${danger ? " danger" : ""}`}
      role="menuitem"
      onClick={(e) => {
        e.stopPropagation();
        p.onClose();
        run();
      }}
    >
      {label}
    </button>
  );
  return (
    <div ref={ref} className="st-menu" role="menu">
      {item("Open", p.onOpen)}
      {!p.locked && item("Rename…", p.onRename)}
      {item("Duplicate", p.onDuplicate)}
      {item("Show in Finder", p.onReveal)}
      {!p.locked && <div className="st-menu-sep" />}
      {!p.locked && item("Move to Trash…", p.onDelete, true)}
    </div>
  );
};

const NewProjectDialog = ({ onClose, onCreate }: { onClose: () => void; onCreate: (name: string, lang: Lang | null, o: Orientation) => Promise<void> }) => {
  const [name, setName] = useState("");
  const [orientation, setOrientation] = useState<Orientation>("landscape");
  const [lang, setLang] = useState<Lang | "">("");
  const [busy, setBusy] = useState(false);
  const submit = async () => {
    setBusy(true);
    await onCreate(name, lang || null, orientation);
    setBusy(false);
  };
  return (
    <Modal title="New project" onClose={onClose}>
      <form
        className="st-form"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <label className="field">
          <span className="field-label">Name</span>
          <input data-autofocus value={name} placeholder="Untitled" onChange={(e) => setName(e.target.value)} />
        </label>
        <div className="st-row">
          <div className="field">
            <span className="field-label">Format</span>
            <div className="segmented">
              <button type="button" className={`seg${orientation === "landscape" ? " active" : ""}`} onClick={() => setOrientation("landscape")}>
                Landscape 16:9
              </button>
              <button type="button" className={`seg${orientation === "portrait" ? " active" : ""}`} onClick={() => setOrientation("portrait")}>
                Portrait 9:16
              </button>
            </div>
          </div>
          <label className="field">
            <span className="field-label">Language</span>
            <select value={lang} onChange={(e) => setLang(e.target.value as Lang | "")}>
              <option value="">Not set</option>
              <option value="zh">中文</option>
              <option value="ja">日本語</option>
              <option value="en">English</option>
            </select>
          </label>
        </div>
        <div className="st-modal-foot">
          <span className="spacer" />
          <button type="button" className="btn ghost" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn primary" disabled={busy}>
            Create project
          </button>
        </div>
      </form>
    </Modal>
  );
};

export const Home = ({ onOpen, onOpenFile, onBack, currentDir }: Props) => {
  const api = useStudioApi();
  const [projects, setProjects] = useState<ProjectSummary[] | null>(null);
  const [settings, setSettings] = useState<StudioSettings | null>(null);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [menu, setMenu] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<{ dir: string; name: string } | null>(null);
  const [dialog, setDialog] = useState<"new" | "generate" | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const now = useMemo(() => Date.now(), [projects]);

  const refresh = useCallback(async () => {
    try {
      const [list, s] = await Promise.all([api.listProjects(), api.settings()]);
      setProjects(list);
      setSettings(s);
    } catch (e) {
      setError(errorText(e));
      setProjects((p) => p ?? []);
    }
  }, [api]);
  useEffect(() => void refresh(), [refresh]);

  /** Runs one action on a project, showing its error and refreshing the list after. */
  const act = async (dir: string | null, work: () => Promise<void>) => {
    setError(null);
    setNotice(null);
    setBusy(dir ?? "new");
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    }
    setBusy(null);
    await refresh();
  };

  const open = (dir: string) => act(dir, async () => void (await onOpen(await api.openProject(dir))));
  const create = (name: string, lang: Lang | null, orientation: Orientation) =>
    act(null, async () => {
      const made = await api.createProject(name, lang, orientation);
      setDialog(null);
      await onOpen(await api.openProject(made.dir));
    });
  const importTimeline = () =>
    act(null, async () => {
      const path = await api.pickTimelineFile();
      if (!path) return;
      const got = await api.importTimeline(path);
      const parts = [`Imported “${got.project.name}” with ${got.copied.length} file${got.copied.length === 1 ? "" : "s"}.`];
      if (got.missing.length) parts.push(`Not found: ${got.missing.join(", ")}.`);
      setNotice(parts.join(" "));
    });
  const rename = (dir: string, name: string) =>
    act(dir, async () => {
      setRenaming(null);
      await api.renameProject(dir, name);
    });
  const remove = (p: ProjectSummary) =>
    act(p.dir, async () => {
      const yes = await api.confirm(
        `Move “${p.name}” to the Trash?\n\nIts timeline, assets and ${p.renders} render${p.renders === 1 ? "" : "s"} go with it. You can put it back from the Trash.`,
        "Move to Trash",
      );
      if (yes) await api.trashProject(p.dir);
    });
  const changeRoot = () =>
    act(null, async () => {
      const folder = await api.pickFolder();
      if (folder) setSettings(await api.setProjectsRoot(folder));
    });
  const defaultRoot = () =>
    act(null, async () => {
      if (settings) setSettings(await api.setProjectsRoot(settings.defaultRoot));
    });

  const shown = filterProjects(projects ?? [], query);
  const recent = !query && settings ? settings.recent.slice(0, 4) : [];

  return (
    <div className="st-home">
      <header className="st-home-bar glass">
        <div className="brand">
          <span className="logo" aria-hidden />
          <span>Vimmer</span>
          <span className="brand-sub">Studio</span>
        </div>
        {onBack && (
          <button className="btn ghost" onClick={onBack} title="Back to the open project">
            ← Back to editor
          </button>
        )}
        <input
          className="st-search"
          type="search"
          placeholder="Search projects"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Search projects"
        />
        <span className="spacer" />
        {onOpenFile && (
          <button className="btn ghost" onClick={onOpenFile} title="Open a timeline file without making a project">
            Open file…
          </button>
        )}
        <button className="btn ghost" onClick={() => void importTimeline()} title="Make a project from a timeline file and the files it uses">
          Import timeline…
        </button>
        <button className="btn" onClick={() => setDialog("new")}>
          New project
        </button>
        <button className="btn primary" onClick={() => setDialog("generate")}>
          Generate from text
        </button>
      </header>

      <main className="st-home-body">
        {error && (
          <div className="st-banner bad" role="alert">
            <span>{error}</span>
            <button className="btn ghost small" onClick={() => setError(null)}>
              Dismiss
            </button>
          </div>
        )}
        {notice && (
          <div className="st-banner" role="status">
            <span>{notice}</span>
            <button className="btn ghost small" onClick={() => setNotice(null)}>
              Dismiss
            </button>
          </div>
        )}

        {recent.length > 0 && (
          <section>
            <h2 className="st-section">Recently opened</h2>
            <div className="st-recent">
              {recent.map((p) => (
                <button key={p.dir} className="st-recent-item" onClick={() => void open(p.dir)} title={p.dir}>
                  <span className="st-recent-thumb">
                    <Thumb project={p} />
                  </span>
                  <span className="st-recent-text">
                    <span className="st-recent-name">{p.name}</span>
                    <span className="dim">{formatWhen(p.updatedMs, now)}</span>
                  </span>
                </button>
              ))}
            </div>
          </section>
        )}

        <section>
          <h2 className="st-section">
            {query ? `Projects matching “${query}”` : "All projects"}
            {projects && <span className="dim"> · {shown.length}</span>}
          </h2>
          {projects === null ? (
            <p className="dim">Loading projects…</p>
          ) : projects.length === 0 ? (
            <div className="st-empty">
              <p className="st-empty-title">No projects yet</p>
              <p className="dim">Paste a text and let Claude turn it into a video, or start from an empty project.</p>
              <div className="st-empty-actions">
                <button className="btn primary" onClick={() => setDialog("generate")}>
                  Generate from text
                </button>
                <button className="btn" onClick={() => setDialog("new")}>
                  New project
                </button>
              </div>
            </div>
          ) : shown.length === 0 ? (
            <p className="dim">No project has “{query}” in its name.</p>
          ) : (
            <div className="st-grid">
              {shown.map((p) => (
                <article
                  key={p.dir}
                  className={`st-card${busy === p.dir ? " busy" : ""}`}
                  tabIndex={0}
                  onClick={() => renaming?.dir !== p.dir && void open(p.dir)}
                  onKeyDown={(e) => e.key === "Enter" && e.target === e.currentTarget && void open(p.dir)}
                  title={p.dir}
                >
                  <div className="st-thumb">
                    <Thumb project={p} />
                    <span className="st-duration mono">{formatDuration(p.durationMs)}</span>
                  </div>
                  <div className="st-card-body">
                    {renaming?.dir === p.dir ? (
                      <input
                        className="st-rename"
                        autoFocus
                        value={renaming.name}
                        onClick={(e) => e.stopPropagation()}
                        onChange={(e) => setRenaming({ dir: p.dir, name: e.target.value })}
                        onFocus={(e) => e.target.select()}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Enter") void rename(p.dir, renaming.name);
                          if (e.key === "Escape") setRenaming(null);
                        }}
                        onBlur={() => (renaming.name.trim() && renaming.name !== p.name ? void rename(p.dir, renaming.name) : setRenaming(null))}
                        aria-label="Project name"
                      />
                    ) : (
                      <div className="st-card-name">{p.name}</div>
                    )}
                    <div className="st-card-meta dim">
                      <span>{formatWhen(p.updatedMs, now)}</span>
                      {p.dir === currentDir && <span className="st-tag ok">Open in editor</span>}
                      {ORIGIN_LABEL[p.origin] && <span className="st-tag">{ORIGIN_LABEL[p.origin]}</span>}
                      {p.renders > 0 && (
                        <span className="st-tag ok">
                          {p.renders} render{p.renders === 1 ? "" : "s"}
                        </span>
                      )}
                    </div>
                  </div>
                  <button
                    className="st-more"
                    aria-label={`Actions for ${p.name}`}
                    aria-haspopup="menu"
                    onClick={(e) => {
                      e.stopPropagation();
                      setMenu(menu === p.dir ? null : p.dir);
                    }}
                  >
                    ⋯
                  </button>
                  {menu === p.dir && (
                    <CardMenu
                      locked={p.dir === currentDir}
                      onClose={() => setMenu(null)}
                      onOpen={() => void open(p.dir)}
                      onRename={() => setRenaming({ dir: p.dir, name: p.name })}
                      onDuplicate={() => void act(p.dir, async () => void (await api.duplicateProject(p.dir)))}
                      onReveal={() => void act(p.dir, () => api.reveal(p.dir))}
                      onDelete={() => void remove(p)}
                    />
                  )}
                </article>
              ))}
            </div>
          )}
        </section>
      </main>

      {settings && (
        <footer className="st-home-foot">
          <span className="dim">Projects folder</span>
          <span className="mono st-root" title={settings.root}>
            {settings.root}
          </span>
          <button className="btn ghost small" onClick={() => void changeRoot()}>
            Change…
          </button>
          {!settings.isDefault && (
            <button className="btn ghost small" onClick={() => void defaultRoot()} title={settings.defaultRoot}>
              Use default
            </button>
          )}
          <button className="btn ghost small" onClick={() => void act(null, () => api.reveal(settings.root))}>
            Show in Finder
          </button>
        </footer>
      )}

      {dialog === "new" && <NewProjectDialog onClose={() => setDialog(null)} onCreate={create} />}
      {dialog === "generate" && (
        <GenerateDialog
          onClose={() => {
            setDialog(null);
            void refresh();
          }}
          onOpenProject={(dir) => {
            setDialog(null);
            void open(dir);
          }}
        />
      )}
    </div>
  );
};
