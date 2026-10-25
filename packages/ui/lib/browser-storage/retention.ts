export const RETENTION_KINDS = [
	"undoHistory",
	"boardSnapshots",
	"chats",
	"globalChat",
	"flowpilot",
	"discardedSync",
	"temporaryFiles",
	"viewports",
] as const;

export type RetentionKind = (typeof RETENTION_KINDS)[number];

/** Days to keep each kind of local history; `null` keeps it until it is deleted by hand. */
export type StorageRetentionPolicy = Readonly<
	Record<RetentionKind, number | null>
>;

export const DEFAULT_RETENTION_POLICY: StorageRetentionPolicy = {
	undoHistory: 30,
	boardSnapshots: 90,
	chats: 30,
	globalChat: null,
	flowpilot: 30,
	discardedSync: 30,
	temporaryFiles: 7,
	viewports: 30,
};

export const RETENTION_DAY_OPTIONS = [1, 7, 14, 30, 60, 90, 180, 365] as const;

export const DAY_MS = 24 * 60 * 60 * 1000;

const POLICY_KEY = "flow-like:storage-retention";
const LAST_RUN_KEY = "flow-like:storage-retention:last-run";

function isRetentionDays(value: unknown): value is number | null {
	return (
		value === null ||
		(typeof value === "number" &&
			Number.isInteger(value) &&
			value >= 1 &&
			value <= 3650)
	);
}

export function parseRetentionPolicy(
	raw: string | null,
): StorageRetentionPolicy {
	if (!raw) return DEFAULT_RETENTION_POLICY;
	let stored: unknown;
	try {
		stored = JSON.parse(raw);
	} catch {
		return DEFAULT_RETENTION_POLICY;
	}
	if (!stored || typeof stored !== "object") return DEFAULT_RETENTION_POLICY;
	const policy: Record<RetentionKind, number | null> = {
		...DEFAULT_RETENTION_POLICY,
	};
	for (const kind of RETENTION_KINDS) {
		const value = (stored as Record<string, unknown>)[kind];
		if (isRetentionDays(value)) policy[kind] = value;
	}
	return policy;
}

/** Per browser, like every store it governs. Unreadable storage falls back to the defaults. */
export function readRetentionPolicy(): StorageRetentionPolicy {
	try {
		return parseRetentionPolicy(globalThis.localStorage?.getItem(POLICY_KEY));
	} catch {
		return DEFAULT_RETENTION_POLICY;
	}
}

export function writeRetentionPolicy(policy: StorageRetentionPolicy): void {
	try {
		globalThis.localStorage?.setItem(POLICY_KEY, JSON.stringify(policy));
	} catch {
		// Without localStorage the defaults keep applying.
	}
}

export function readLastRetentionRun(): number | undefined {
	try {
		const value = Number(globalThis.localStorage?.getItem(LAST_RUN_KEY));
		return Number.isFinite(value) && value > 0 ? value : undefined;
	} catch {
		return undefined;
	}
}

export function writeLastRetentionRun(at: number): void {
	try {
		globalThis.localStorage?.setItem(LAST_RUN_KEY, String(at));
	} catch {
		// Only feeds the "last cleanup" label.
	}
}

export function retentionCutoff(days: number, now = Date.now()): number {
	return now - days * DAY_MS;
}
