import { useCallback, useEffect, useRef, useState } from "react";
import { errorText, useStudioApi } from "./api";
import { ASSET_FILTERS, assetFacts, countAssets, filterAssets, usedByLabel, type AssetFilter } from "./format";
import type { AssetInfo, AssetKind } from "./types";

/** The drag type of an asset dragged out of the panel; the data is `{path, kind}` as JSON. */
export const ASSET_DRAG_TYPE = "application/x-vimmer-asset";

/** What "use" does with each kind in the current scene; null: nothing to do from here. */
export const USE_LABEL: Record<AssetKind, string | null> = {
  image: "Use as scene image",
  video: "Use as scene video",
  audio: "Use as scene audio",
  icon: "Add as icon layer",
  font: null,
};

type Props = {
  /** The project folder. */
  dir: string;
  /** The editor's current document (saved or not): decides what counts as used. */
  timelineJson: string;
  /** The scene "use" applies to, for the button's tooltip. */
  sceneLabel?: string | null;
  /** Puts the asset into the current scene. Absent: the panel only manages files. */
  onUse?: (asset: AssetInfo) => void;
  /** Bumped by the owner when files changed outside the panel (an import from the inspector). */
  refreshKey?: number;
};

const Tile = ({ asset, url }: { asset: AssetInfo; url: string }) => {
  switch (asset.kind) {
    case "image":
    case "icon":
      return <img className={`st-asset-img ${asset.kind}`} src={url} alt="" draggable={false} loading="lazy" />;
    case "video":
      // The first frame, without downloading the clip.
      return <video className="st-asset-img" src={`${url}#t=0.1`} preload="metadata" muted playsInline />;
    case "audio":
      return (
        <span className="st-asset-glyph" aria-hidden>
          ♪
        </span>
      );
    case "font":
      return (
        <span className="st-asset-glyph" aria-hidden>
          Aa
        </span>
      );
  }
};

const Preview = ({ asset, url }: { asset: AssetInfo; url: string }) => {
  switch (asset.kind) {
    case "image":
    case "icon":
      return <img className={`st-preview-media ${asset.kind}`} src={url} alt={asset.name} />;
    case "video":
      return <video className="st-preview-media" src={url} controls preload="metadata" />;
    case "audio":
      return <audio className="st-preview-audio" src={url} controls preload="metadata" />;
    case "font":
      return <div className="st-preview-font">Aa Bb Cc 123</div>;
  }
};

export const AssetPanel = ({ dir, timelineJson, sceneLabel, onUse, refreshKey = 0 }: Props) => {
  const api = useStudioApi();
  const [assets, setAssets] = useState<AssetInfo[] | null>(null);
  const [filter, setFilter] = useState<AssetFilter>("all");
  const [selected, setSelected] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const latest = useRef({ dir, timelineJson });
  latest.current = { dir, timelineJson };

  const load = useCallback(async () => {
    const asked = latest.current;
    try {
      const list = await api.listAssets(asked.dir, asked.timelineJson);
      // An answer for an older document or another project is dropped.
      if (latest.current === asked || (latest.current.dir === asked.dir && latest.current.timelineJson === asked.timelineJson)) setAssets(list);
    } catch (e) {
      setError(errorText(e));
      setAssets((a) => a ?? []);
    }
  }, [api]);

  // Usage follows the document; wait for a pause in typing before asking again.
  useEffect(() => {
    const handle = window.setTimeout(() => void load(), assets === null ? 0 : 300);
    return () => window.clearTimeout(handle);
    // `assets` only decides the first delay.
  }, [load, dir, timelineJson, refreshKey]);
  useEffect(() => {
    setAssets(null);
    setSelected(null);
    setError(null);
    setNote(null);
  }, [dir]);

  const importFiles = useCallback(
    async (sources: string[]) => {
      if (sources.length === 0) return;
      setBusy(true);
      setError(null);
      try {
        const got = await api.importAssets(latest.current.dir, sources);
        const added = got.filter((g) => !g.reused).length;
        const reused = got.length - added;
        setNote(
          [added ? `Added ${added} file${added === 1 ? "" : "s"}.` : "", reused ? `${reused} already in the project.` : ""].filter(Boolean).join(" "),
        );
        if (got.length) setSelected(got[got.length - 1].path);
        setFilter("all");
      } catch (e) {
        setError(errorText(e));
      }
      setBusy(false);
      await load();
    },
    [api, load],
  );

  // Files dropped from Finder anywhere on the window go into the project.
  useEffect(() => {
    let off: (() => void) | undefined;
    let closed = false;
    void api.onFilesDropped((paths) => void importFiles(paths)).then((u) => (closed ? u() : (off = u)));
    return () => {
      closed = true;
      off?.();
    };
  }, [api, importFiles]);

  const run = async (work: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    }
    setBusy(false);
    await load();
  };

  const counts = countAssets(assets ?? []);
  const shown = filterAssets(assets ?? [], filter);
  const current = assets?.find((a) => a.path === selected) ?? null;

  const removeUnused = () =>
    run(async () => {
      const yes = await api.confirm(
        `Move ${counts.unused} unused file${counts.unused === 1 ? "" : "s"} to the Trash?\n\nOnly files no scene, music or brand setting uses are moved. You can put them back from the Trash.`,
        "Move to Trash",
      );
      if (!yes) return;
      const removed = await api.trashUnusedAssets(dir, timelineJson);
      setNote(`Moved ${removed.length} file${removed.length === 1 ? "" : "s"} to the Trash.`);
      setSelected(null);
    });
  const removeOne = (asset: AssetInfo) =>
    run(async () => {
      const yes = await api.confirm(`Move “${asset.name}” to the Trash?`, "Move to Trash");
      if (!yes) return;
      await api.trashAsset(dir, asset.path, timelineJson);
      setSelected(null);
    });

  return (
    <div className="st-assets">
      <div className="st-panel-head">
        <span className="section-label">Assets</span>
        <span className="dim">{assets ? counts.all : ""}</span>
        <span className="spacer" />
        <button className="btn small" disabled={busy} onClick={() => void api.pickMediaFiles().then(importFiles, (e) => setError(errorText(e)))}>
          Import…
        </button>
      </div>

      {assets && assets.length > 0 && (
        <div className="st-filters" role="tablist" aria-label="Filter assets">
          {ASSET_FILTERS.filter((f) => f.value === "all" || counts[f.value] > 0).map((f) => (
            <button
              key={f.value}
              role="tab"
              aria-selected={filter === f.value}
              className={`st-chip${filter === f.value ? " active" : ""}`}
              onClick={() => setFilter(f.value)}
            >
              {f.label}
              <span className="dim"> {counts[f.value]}</span>
            </button>
          ))}
        </div>
      )}

      {error && (
        <div className="st-inline bad" role="alert">
          {error}
        </div>
      )}
      {note && !error && <div className="st-inline">{note}</div>}

      <div className="st-asset-scroll">
        {assets === null ? (
          <p className="dim st-pad">Loading…</p>
        ) : assets.length === 0 ? (
          <div className="st-drop-hint">
            <p>No files yet</p>
            <p className="dim">Drop images, video, audio, SVG icons or fonts here, or use Import.</p>
          </div>
        ) : (
          <div className="st-asset-grid">
            {shown.map((a) => (
              <button
                key={a.path}
                className={`st-asset${selected === a.path ? " selected" : ""}`}
                onClick={() => setSelected(selected === a.path ? null : a.path)}
                draggable
                onDragStart={(e) => {
                  e.dataTransfer.setData(ASSET_DRAG_TYPE, JSON.stringify({ path: a.path, kind: a.kind }));
                  e.dataTransfer.setData("text/plain", a.path);
                  e.dataTransfer.effectAllowed = "copy";
                }}
                title={`${a.path}\n${assetFacts(a)}\n${usedByLabel(a.usedBy)}`}
              >
                <span className="st-asset-thumb">
                  <Tile asset={a} url={api.fileUrl(a.file, a.modifiedMs)} />
                  <span className={`st-used${a.usedBy.length ? " on" : ""}`}>{a.usedBy.length ? `Used ×${a.usedBy.length}` : "Unused"}</span>
                </span>
                <span className="st-asset-name">{a.name}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      {current && (
        <div className="st-asset-detail">
          <Preview asset={current} url={api.fileUrl(current.file, current.modifiedMs)} />
          <div className="st-asset-detail-name" title={current.path}>
            {current.name}
          </div>
          <div className="dim">{assetFacts(current)}</div>
          <div className={current.usedBy.length ? "" : "dim"}>{usedByLabel(current.usedBy)}</div>
          <div className="st-actions">
            {onUse && USE_LABEL[current.kind] && (
              <button
                className="btn small primary"
                onClick={() => onUse(current)}
                title={sceneLabel ? `In scene ${sceneLabel}` : "In the current scene"}
              >
                {USE_LABEL[current.kind]}
              </button>
            )}
            <button className="btn small" onClick={() => void run(() => api.reveal(current.file))}>
              Show in Finder
            </button>
            <button
              className="btn small danger"
              disabled={busy || current.usedBy.length > 0}
              title={current.usedBy.length ? "In use: remove it from the timeline first" : "Move to the Trash"}
              onClick={() => void removeOne(current)}
            >
              Remove
            </button>
          </div>
        </div>
      )}

      {assets && counts.unused > 0 && (
        <div className="st-panel-foot">
          <button className="btn ghost small" disabled={busy} onClick={() => void removeUnused()}>
            Remove {counts.unused} unused…
          </button>
        </div>
      )}
    </div>
  );
};
