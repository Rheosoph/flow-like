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
