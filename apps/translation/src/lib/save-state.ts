export interface PendingSave {
	entry: string;
	version: number;
	tree: Record<string, unknown>;
}

/** Keep dirty entries that changed after the request captured its snapshot. */
export function dirtyAfterSave(
	current: ReadonlySet<string>,
	pending: readonly PendingSave[],
	currentVersions: Readonly<Record<string, number>>,
): Set<string> {
	const next = new Set(current);
	for (const { entry, version } of pending) {
		if ((currentVersions[entry] ?? 0) === version) next.delete(entry);
	}
	return next;
}
