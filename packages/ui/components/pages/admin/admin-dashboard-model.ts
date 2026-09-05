import type { RegistryStats } from "../../../lib/schema/wasm";

export type DashboardRegistryStats = Omit<RegistryStats, "verifiedPackages">;

/** Missing or invalid counts must reach the query error state instead of looking empty. */
export function readDashboardCount(
	value: unknown,
	fieldName = "count",
): number {
	if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
		throw new Error(`Invalid dashboard count: ${fieldName}`);
	}
	return value;
}

/** The admin registry endpoint uses snake_case; some clients already normalize it. */
export function normalizeRegistryStats(raw: unknown): DashboardRegistryStats {
	if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
		throw new Error("Invalid registry statistics response");
	}

	const record = raw as Record<string, unknown>;
	const count = (camelCase: string, snakeCase: string): number => {
		const hasCamelCase = Object.prototype.hasOwnProperty.call(
			record,
			camelCase,
		);
		const hasSnakeCase = Object.prototype.hasOwnProperty.call(
			record,
			snakeCase,
		);
		if (!hasCamelCase && !hasSnakeCase) {
			throw new Error(`Missing registry statistic: ${camelCase}`);
		}
		const camelValue = hasCamelCase
			? readDashboardCount(record[camelCase], camelCase)
			: undefined;
		const snakeValue = hasSnakeCase
			? readDashboardCount(record[snakeCase], snakeCase)
			: undefined;
		if (
			camelValue !== undefined &&
			snakeValue !== undefined &&
			camelValue !== snakeValue
		) {
			throw new Error(`Conflicting registry statistic: ${camelCase}`);
		}
		return (camelValue ?? snakeValue) as number;
	};

	return {
		totalPackages: count("totalPackages", "total_packages"),
		totalVersions: count("totalVersions", "total_versions"),
		totalDownloads: count("totalDownloads", "total_downloads"),
		pendingReview: count("pendingReview", "pending_review"),
		activePackages: count("activePackages", "active_packages"),
		rejectedPackages: count("rejectedPackages", "rejected_packages"),
	};
}

export interface DashboardQueuePriority {
	/** Null means the count is unavailable or still loading. */
	count: number | null;
	/** Lower values identify more urgent work. */
	priority: number;
}

/** Put known work first, unavailable queues next, and confirmed empty queues last. */
export function prioritizeDashboardQueues<T extends DashboardQueuePriority>(
	queues: readonly T[],
): T[] {
	return queues
		.map((queue, index) => ({
			queue,
			index,
			rank:
				queue.count === null ? 1 : readDashboardCount(queue.count) > 0 ? 0 : 2,
		}))
		.sort(
			(a, b) =>
				a.rank - b.rank ||
				a.queue.priority - b.queue.priority ||
				a.index - b.index,
		)
		.map(({ queue }) => queue);
}
