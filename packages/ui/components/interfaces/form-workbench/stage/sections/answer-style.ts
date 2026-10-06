/*
 * The answer's look inside the workbench, scoped to its [data-fl-chat-prose] column so the chat keeps its
 * own: the canvas's block rhythm and SURFACE's answer tables. Static class strings, so Tailwind sees them.
 */

/** The streaming caret: a soft-pulsing bar after the last block of the answer, still while the reader's motion setting allows it. */
export const CARET = [
	"[&_[data-slate-editor]>:last-child]:after:ml-0.5 [&_[data-slate-editor]>:last-child]:after:inline-block",
	"[&_[data-slate-editor]>:last-child]:after:h-[1em] [&_[data-slate-editor]>:last-child]:after:w-2",
	"[&_[data-slate-editor]>:last-child]:after:bg-foreground [&_[data-slate-editor]>:last-child]:after:align-[-2px]",
	"[&_[data-slate-editor]>:last-child]:after:content-[''] [&_[data-slate-editor]>:last-child]:after:animate-pulse-soft",
	"motion-reduce:[&_[data-slate-editor]>:last-child]:after:animate-none",
].join(" ");

/**
 * The follow marker of a live answer. It sits below the last block's 14 px margin (RHYTHM) and the caret ends
 * 7 px above that block's bottom edge, so lifting it by those 21 px puts it on the caret's line: the body then
 * keeps the caret CARET_GAP above its bottom edge, as the canvas does.
 */
export const CARET_MARKER = "relative -top-[21px] h-0";

/**
 * The canvas's block rhythm: 14 px after a paragraph or a list, 8 px between list items, room above
 * headings. Heading rules name the tag so they outrank the static editor's `h3:first-of-type` reset; the
 * answer's first block never has room above it. Its type: the h2 at 26/32 with -0.01em and the h3 with
 * no tracking (both need `!`: the chat's unlayered heading rule would win), bold at 600, muted list
 * markers and bullets as a small dot. Paragraphs keep the default wrap: `text-wrap: pretty` re-breaks
 * Plate's nested-span paragraphs away from the canvas's lines, whose breaks `answer-ties.ts` gives.
 */
export const RHYTHM = [
	"[&_.slate-p]:py-0 [&_.slate-p]:mb-3.5 [&_.slate-p[data-slate-list-style-type]]:mb-2",
	"[&_[data-slate-list-style-type]+.slate-p:not([data-slate-list-style-type])]:mt-3.5",
	"[&_[data-slate-list-style-type]+.slate-table]:mt-3.5 [&_.slate-blockquote]:mb-3.5",
	"[&_.slate-editor_h1.slate-h1]:mt-8 [&_.slate-editor_h1.slate-h1]:mb-3",
	"[&_.slate-editor_h2.slate-h2]:mt-8 [&_.slate-editor_h2.slate-h2]:mb-3",
	"[&_.slate-editor_h2.slate-h2]:text-[26px]/[32px] [&_.slate-editor_h2.slate-h2]:tracking-[-0.01em]!",
	"[&_.slate-editor_h3.slate-h3]:mt-6.5 [&_.slate-editor_h3.slate-h3]:mb-2 [&_.slate-editor_h3.slate-h3]:tracking-normal!",
	"[&_.slate-editor_h4.slate-h4]:mt-5 [&_.slate-editor_h4.slate-h4]:mb-1.5",
	"[&_.slate-editor>:first-child]:mt-0!",
	"[&_strong]:font-semibold",
	"[&_li]:marker:text-muted-foreground [&_[data-slate-list-style-type=disc]_li]:marker:text-[0.8em]",
].join(" ");

/**
 * SURFACE §Tables in the answer: no viewer toolbar, handles or sort arrows; 13 px Inter on the card, the
 * header muted at weight 500 on --surface-sunken, horizontal hairlines only, text cells wrap, numeric
 * columns right-aligned in mono on one line (cells marked by `markNumericColumns`).
 */
export const TABLE = [
	"[&_.slate-table]:mt-1 [&_.slate-table]:mb-4 [&_.slate-table]:py-0",
	"[&_.slate-table>div>div:first-child]:hidden [&_.slate-table_.overflow-auto]:bg-card",
	"[&_.slate-table_table]:m-0 [&_.slate-table_table]:leading-4.5",
	"[&_.slate-table_thead]:border-b-0 [&_.slate-table_thead]:bg-surface-sunken [&_.slate-table_thead]:backdrop-blur-none",
	"[&_.slate-table_th]:border-0 [&_.slate-table_th]:bg-surface-sunken [&_.slate-table_th]:py-2",
	"[&_.slate-table_th]:font-medium [&_.slate-table_th]:text-muted-foreground",
	"[&_.slate-table_th_svg]:hidden [&_.slate-table_th_span]:pointer-events-none",
	"[&_.slate-table_tbody_tr]:border-0 [&_.slate-table_tbody_tr]:bg-transparent",
	"[&_.slate-table_td]:border-x-0 [&_.slate-table_td]:border-b-0 [&_.slate-table_td]:border-t [&_.slate-table_td]:border-hairline",
	"[&_.slate-table_td]:bg-transparent [&_.slate-table_td]:min-w-0 [&_.slate-table_td]:py-2",
	"[&_.slate-table_td_*]:text-[13px]/[18px] [&_.slate-table_td_*]:whitespace-normal [&_.slate-table_td_*]:wrap-break-word",
	"[&_.slate-table_[data-fw-num]]:text-right [&_.slate-table_[data-fw-num]_*]:text-right",
	"[&_.slate-table_th[data-fw-num]>div]:justify-end [&_.slate-table_td[data-fw-num]_*]:whitespace-nowrap",
	"[&_.slate-table_td[data-fw-num]_*]:font-mono [&_.slate-table_td[data-fw-num]_*]:tabular-nums",
].join(" ");

/** Cell padding: 12 px at the sides, 8 px on a touch screen (canvas `ctl.cell`). */
export const CELL_SIDES = {
	fine: "[&_.slate-table_th]:px-3 [&_.slate-table_td]:px-3",
	touch: "[&_.slate-table_th]:px-2 [&_.slate-table_td]:px-2",
} as const;
