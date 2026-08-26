/**
 * Which files have a tab.
 *
 * The strip lists what is *open*; the explorer lists what *exists*. `main` is the
 * board itself and is never in this list — it always has a tab and cannot be
 * closed, because the canvas is always showing something.
 */

/** Opening a file that is already open is a no-op, so the tab order stays put. */
export function withFileOpen(
	openFileIds: readonly string[],
	moduleId: string,
): string[] {
	return openFileIds.includes(moduleId)
		? [...openFileIds]
		: [...openFileIds, moduleId];
}

export function withFileClosed(
	openFileIds: readonly string[],
	moduleId: string,
): string[] {
	return openFileIds.filter((id) => id !== moduleId);
}

/**
 * What to show after closing `moduleId`, given it was the file on screen.
 *
 * The tab that slid into its place, else the one before it, else `main` — the
 * same rule an editor uses, so closing a run of tabs walks left rather than
 * throwing you back to the root each time.
 */
export function fileAfterClose(
	openFileIds: readonly string[],
	moduleId: string,
): string | null {
	const index = openFileIds.indexOf(moduleId);
	if (index === -1) return null;
	const remaining = withFileClosed(openFileIds, moduleId);
	return remaining[index] ?? remaining[index - 1] ?? null;
}

/** Tabs for modules the board no longer has cannot survive a delete elsewhere. */
export function withMissingFilesDropped(
	openFileIds: readonly string[],
	exists: (moduleId: string) => boolean,
): string[] {
	return openFileIds.filter(exists);
}
