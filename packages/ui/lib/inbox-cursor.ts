function cursorParts(cursor: string): readonly [number, string] {
	const split = cursor.indexOf(":");
	const micros = split < 0 ? Number.NaN : Number(cursor.slice(0, split));
	return [
		Number.isFinite(micros) ? micros : Number.NEGATIVE_INFINITY,
		cursor.slice(split + 1),
	];
}

/** Server order of `<createdAt epoch micros>:<entry id>` cursors: micros numerically, then ids by code unit. */
export function compareInboxCursors(a: string, b: string): number {
	const [microsA, idA] = cursorParts(a);
	const [microsB, idB] = cursorParts(b);
	if (microsA !== microsB) return microsA < microsB ? -1 : 1;
	if (idA === idB) return 0;
	return idA < idB ? -1 : 1;
}
