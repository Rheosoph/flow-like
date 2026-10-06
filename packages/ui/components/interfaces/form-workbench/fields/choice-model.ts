/*
 * Which control a choice gets and where ←/→ and a letter lead (spec M2, SURFACE §4). Pure.
 */

const MAX_SEGMENTS = 4;
const MAX_SEGMENT_CHARS = 14;

/** 2 to 4 short options read as a segmented control; anything else is a select (SURFACE §4). */
export function fitsSegments(options: readonly string[]): boolean {
	return (
		options.length >= 2 &&
		options.length <= MAX_SEGMENTS &&
		options.every((option) => option.length <= MAX_SEGMENT_CHARS)
	);
}

/** The next option that starts with the typed letter, after the current one and wrapping (E → EUR). */
export function jumpTo(
	options: readonly string[],
	current: number,
	letter: string,
): number {
	const typed = letter.toLocaleLowerCase();
	for (let step = 1; step <= options.length; step += 1) {
		const at = (current + step + options.length) % options.length;
		if (options[at].toLocaleLowerCase().startsWith(typed)) return at;
	}
	return -1;
}

/** Where ←/→ and a letter lead from `current`; −1 for a key that picks nothing. */
export function targetOf(
	options: readonly string[],
	current: number,
	event: {
		readonly key: string;
		readonly metaKey: boolean;
		readonly ctrlKey: boolean;
		readonly altKey: boolean;
	},
): number {
	const n = options.length;
	if (event.key === "ArrowRight") return (current + 1 + n) % n;
	if (event.key === "ArrowLeft") return (current - 1 + n) % n;
	const plain = !(event.metaKey || event.ctrlKey || event.altKey);
	return plain && event.key.length === 1 && event.key.trim() !== ""
		? jumpTo(options, current, event.key)
		: -1;
}
