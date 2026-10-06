import { cx } from "../../../settings/devices/primitives/tone";

/*
 * The control recipe of the workbench rail (SURFACE §3): 36 px controls, `--card` fill, 1 px `--input` border,
 * 6 px radius, hover `--border-strong`, a 2 px `--ring` outline on focus, a critical border while invalid.
 * Touch (`layout.touch`) switches to 44 px controls and 16 px text. Containers are measured, never the
 * viewport, so every metric here is a function of that flag.
 */

const SURFACE =
	"block w-full min-w-0 rounded-lg border border-input bg-card text-foreground placeholder:text-muted-foreground hover:border-border-strong aria-invalid:border-critical-line aria-invalid:hover:border-critical focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-solid focus-visible:outline-ring disabled:cursor-not-allowed disabled:opacity-60";

/** The 2 px edge of a field that differs from the compared run (`markers.differs`). */
export const DIFFERS_EDGE = "shadow-[inset_2px_0_0_var(--foreground)]";

/** The focus ring of a box that contains its own input (chips, the masked input). */
export const BOX_FOCUS =
	"focus-within:outline-2 focus-within:outline-offset-1 focus-within:outline-solid focus-within:outline-ring";

const LINE = "text-[13.5px]/5";
const TOUCH_LINE = "text-base/5";

export interface StyleOptions {
	readonly touch: boolean;
	readonly differs?: boolean;
	readonly className?: string;
}

function edge(options: StyleOptions) {
	return options.differs ? DIFFERS_EDGE : undefined;
}

/** A one-line control: text, date, masked, pairs inputs. */
export function lineControl(options: StyleOptions) {
	return cx(
		SURFACE,
		options.touch ? `h-11 px-3 ${TOUCH_LINE}` : `h-9 px-2.75 ${LINE}`,
		edge(options),
		options.className,
	);
}

/** The growing text box: one line until it holds more. */
export function textBox(options: StyleOptions) {
	return cx(
		SURFACE,
		"resize-none overflow-hidden",
		options.touch
			? `min-h-11 px-3 py-2.75 ${TOUCH_LINE}`
			: `min-h-9 px-2.75 py-1.75 ${LINE}`,
		edge(options),
		options.className,
	);
}

/** Numbers: mono, tabular, right-aligned. */
export function numberControl(options: StyleOptions) {
	return cx(
		SURFACE,
		"text-right font-mono tabular-nums",
		options.touch ? "h-11 px-3 text-base" : "h-9 px-2.75 text-[13px]",
		edge(options),
		options.className,
	);
}

/** A button that looks like a control (switch row, drop row, file row). */
export function rowControl(options: StyleOptions) {
	return cx(
		SURFACE,
		"flex items-center text-left",
		options.touch ? "min-h-11" : "h-9",
		edge(options),
		options.className,
	);
}

/** The label text size. */
export const labelSize = (touch: boolean) =>
	touch ? "text-sm/4.5" : "text-[13px]/4.5";

/** A square ghost icon button (× on a file row, the calendar button, the reset of a pair row). */
export const ghostIcon = (touch: boolean, size?: string) =>
	cx(
		"inline-flex shrink-0 items-center justify-center rounded-md p-0 text-muted-foreground hover:bg-row-hover hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-solid focus-visible:outline-ring",
		size ?? (touch ? "size-11" : "size-7"),
	);

/** A 12 px text button: "Show", "Hide", "Try again", "Add it", "Add row". 16 px high with a 24 px hit area. */
export const textButton = (touch: boolean) =>
	cx(
		"relative shrink-0 rounded-sm bg-transparent px-0.5 text-xs/4 font-medium text-ink-2 hover:underline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-solid focus-visible:outline-ring",
		touch ? "min-h-11" : "h-4 after:absolute after:-inset-1 after:content-['']",
	);
