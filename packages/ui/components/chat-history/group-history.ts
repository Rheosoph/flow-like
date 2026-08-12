import type { IHistoryEntry, IHistoryGroup } from "./chat-history-types";

const DAY_MS = 24 * 60 * 60 * 1000;

/**
 * Bucket conversations into pinned + calendar sections.
 *
 * Returns an ordered array rather than a keyed object: the render order is part of the contract
 * (Pinned always first, then newest bucket down), and an object would leave that resting on
 * `Object.entries` insertion order.
 *
 * `now` is injectable so the buckets are testable without freezing the clock.
 */
export function groupHistoryByDate(
	entries: readonly IHistoryEntry[],
	now: number = Date.now(),
): IHistoryGroup[] {
	const reference = new Date(now);
	const todayStart = new Date(
		reference.getFullYear(),
		reference.getMonth(),
		reference.getDate(),
	).getTime();
	const yesterdayStart = todayStart - DAY_MS;
	const weekStart = todayStart - 7 * DAY_MS;
	const monthStart = todayStart - 30 * DAY_MS;

	const pinned: IHistoryEntry[] = [];
	const buckets: Record<string, IHistoryEntry[]> = {
		today: [],
		yesterday: [],
		week: [],
		month: [],
		older: [],
	};

	for (const entry of entries) {
		if (entry.pinnedAt) {
			pinned.push(entry);
			continue;
		}
		const at = entry.updatedAt;
		if (at >= todayStart) buckets.today.push(entry);
		else if (at >= yesterdayStart) buckets.yesterday.push(entry);
		else if (at >= weekStart) buckets.week.push(entry);
		else if (at >= monthStart) buckets.month.push(entry);
		else buckets.older.push(entry);
	}

	// Most recently pinned first, so a fresh pin lands where the user is looking.
	pinned.sort((a, b) => (b.pinnedAt ?? 0) - (a.pinnedAt ?? 0));

	const groups: IHistoryGroup[] = [];
	if (pinned.length > 0)
		groups.push({
			key: "pinned",
			label: "Pinned",
			entries: pinned,
			pinned: true,
		});

	const dated: [string, string][] = [
		["today", "Today"],
		["yesterday", "Yesterday"],
		["week", "Previous 7 days"],
		["month", "Previous 30 days"],
		["older", "Older"],
	];
	for (const [key, label] of dated) {
		const bucket = buckets[key];
		if (bucket.length > 0) groups.push({ key, label, entries: bucket });
	}

	return groups;
}
