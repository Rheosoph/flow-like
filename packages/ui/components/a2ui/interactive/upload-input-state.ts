export function limitUploadBatch<T>(
	files: readonly T[],
	currentCount: number,
	multiple: boolean,
	maxFiles: number,
): T[] {
	const available = multiple ? Math.max(0, maxFiles - currentCount) : 1;
	return files.slice(0, available);
}

export function mergeSuccessfulUploadBatch<T>(
	current: readonly T[],
	results: readonly T[],
	multiple: boolean,
	maxFiles: number,
	isSuccessful: (value: T) => boolean,
): T[] {
	const successful = results.filter(isSuccessful);
	if (!multiple) return successful[0] ? [successful[0]] : current.slice(0, 1);
	return [...current, ...successful].slice(0, maxFiles);
}

export interface SettledUploadBatch<T> {
	/** The input's value: what it held before plus the batch's successes. */
	readonly committed: T[];
	/** What the input shows: the committed files, then each failure with its error. */
	readonly display: T[];
}

/**
 * A failed upload stays on screen until it is removed or replaced — in single mode
 * too, where dropping it made a file vanish a moment after it was picked.
 */
export function settleUploadBatch<T>(
	current: readonly T[],
	results: readonly T[],
	multiple: boolean,
	maxFiles: number,
	isSuccessful: (value: T) => boolean,
): SettledUploadBatch<T> {
	const committed = mergeSuccessfulUploadBatch(
		current,
		results,
		multiple,
		maxFiles,
		isSuccessful,
	);
	const failed = results.filter((value) => !isSuccessful(value));
	return { committed, display: [...committed, ...failed] };
}
