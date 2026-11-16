import type { Paging, TextLayout, TextPosition } from "../../../src/types/timeline";
import { DEFAULT_TEXT_PX, MAX_TEXT_PX, MIN_TEXT_PX, type PagePreview } from "../../../src/textPages";
import { Field } from "./Inspector";
import type { LayoutPart } from "../textLayout";

// 文字 Text: size, position and paging of the scene text (src/textPages.ts), in the Scene tab
// (with the page list) and the Brand tab (the default for every scene). Each part is optional:
// unset follows the brand (or the built-in 92 px, bottom, no paging), shown as a hint. A slider
// drag is one undo step (App.tsx fieldDispatch); a select or button is one step.

const POSITIONS: { value: TextPosition; label: string }[] = [
  { value: "top", label: "上方 Top" },
  { value: "middle", label: "中间 Middle" },
  { value: "bottom", label: "底部 Bottom" },
];
const PAGINGS: { value: Paging; label: string }[] = [
  { value: "off", label: "不分页 Off" },
  { value: "auto", label: "自动分页（按句子）Auto" },
  { value: "manual", label: "手动分页（空一行换页）Manual" },
];
const label = <T extends string>(list: { value: T; label: string }[], v: T) => list.find((x) => x.value === v)?.label ?? v;

type Props = {
  name: "scene" | "brand";
  layout: TextLayout | null | undefined;
  /** What an unset part falls back to (the brand's layout, for a scene). */
  inherited?: TextLayout | null;
  onChange: <K extends LayoutPart>(part: K, value: TextLayout[K] | undefined) => void;
};

export const TextLayoutFields = ({ name, layout, inherited, onChange }: Props) => {
  const own = layout ?? {};
  const fallbackSize = inherited?.size ?? DEFAULT_TEXT_PX;
  const fallbackPosition = inherited?.position ?? "bottom";
  const fallbackPaging = inherited?.paging ?? "off";
  const from = inherited?.size != null || inherited?.position != null || inherited?.paging != null ? "品牌 brand" : "默认 default";
  const size = own.size ?? fallbackSize;
  return (
    <>
      <Field label="字号 Size" suffix={`${size} px`} hint={own.size == null ? `${from}` : "at 1080p"}>
        <div className="row">
          <input
            type="range"
            min={MIN_TEXT_PX}
            max={MAX_TEXT_PX}
            step={2}
            data-layout-size={name}
            value={size}
            onChange={(e) => onChange("size", Number(e.target.value))}
          />
          {own.size != null && (
            <button type="button" className="btn small" data-layout-size-reset={name} onClick={() => onChange("size", undefined)}>
              重置
            </button>
          )}
        </div>
      </Field>
      <Field label="位置 Position">
        <select
          data-layout-position={name}
          value={own.position ?? ""}
          onChange={(e) => onChange("position", (e.target.value || undefined) as TextPosition | undefined)}
        >
          <option value="">{`${from}: ${label(POSITIONS, fallbackPosition)}`}</option>
          {POSITIONS.map((p) => (
            <option key={p.value} value={p.value}>
              {p.label}
            </option>
          ))}
        </select>
      </Field>
      <Field label="文字分页 Paging" hint={name === "scene" ? "长文字分几页显示，背景和镜头不断" : "每一幕的默认"}>
        <select
          data-layout-paging={name}
          value={own.paging ?? ""}
          onChange={(e) => onChange("paging", (e.target.value || undefined) as Paging | undefined)}
        >
          <option value="">{`${from}: ${label(PAGINGS, fallbackPaging)}`}</option>
          {PAGINGS.map((p) => (
            <option key={p.value} value={p.value}>
              {p.label}
            </option>
          ))}
        </select>
      </Field>
    </>
  );
};

const secs = (ms: number) => (ms / 1000).toFixed(1);

/** The pages of the selected scene with when each shows; a click moves the playhead into that
 * page (past its cross-fade, so the page reads in the preview). */
export const PageList = ({
  pages,
  paging,
  narrated,
  heard,
  onSeek,
  onInsertBreak,
}: {
  pages: PagePreview[];
  paging: Paging;
  /** The scene's narration speaks its text (the render then changes page on the spoken words). */
  narrated: boolean;
  /** Timed over a heard clip (试听). */
  heard: boolean;
  onSeek?: (ms: number) => void;
  onInsertBreak?: () => void;
}) => (
  <div className="page-list" data-page-list>
    {paging === "manual" && (
      <div className="row">
        <button type="button" className="btn small" data-insert-page-break onClick={onInsertBreak}>
          ＋ 在光标处分页 Insert page break
        </button>
        <span className="dim">或在文字里空一行 (a blank line starts a page)</span>
      </div>
    )}
    {pages.length === 0 ? (
      <div className="dim">
        {paging === "manual" ? "还没有分页：在文字里空一行就是新的一页。" : "文字一页就放得下，不用分页。"}
      </div>
    ) : (
      <>
        <div className="dim">
          {pages.length} 页 ·{" "}
          {narrated
            ? heard
              ? "按试听的旁白长度预估；渲染时在念到这一页时换页"
              : "预览按字数估算；渲染时在旁白念到这一页时换页（试听后预览更准）"
            : "按字数分配本场景的时长"}
        </div>
        <ol className="pages">
          {pages.map((p, k) => (
            <li key={k}>
              <button type="button" className="page-item" data-page-item={k} onClick={() => onSeek?.(Math.round(Math.min(p.startMs + 600, (p.startMs + p.endMs) / 2)))}>
                <span className="mono dim">
                  {secs(p.startMs)}–{secs(p.endMs)}s
                </span>{" "}
                {p.text}
              </button>
            </li>
          ))}
        </ol>
      </>
    )}
  </div>
);
