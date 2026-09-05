import type {
	IExecutionUsageRecord,
	IPaginatedResponse,
} from "../../lib/schema/usage/tracking";

const DAY_MS = 24 * 60 * 60 * 1000;

export function homeActivityDays(value: unknown): 1 | 7 | 30 {
	return value === 1 || value === 30 ? value : 7;
}

export function hasAttentionSeverity(status: string): boolean {
	return ["error", "fatal"].includes(status.toLowerCase());
}

export interface HomeActivityBucket {
	day: string;
	count: number;
	attentionCount: number;
}

export interface HomeAppActivity {
	appId: string | null;
	count: number;
	attentionCount: number;
}

/** The history endpoint returns newest records, so every aggregate retains its sample coverage. */
export function summarizeHomeExecutions(
	response: IPaginatedResponse<IExecutionUsageRecord>,
	configuredDays: unknown,
	now = Date.now(),
) {
	const days = homeActivityDays(configuredDays);
	const today = new Date(now);
	const todayStart = Date.UTC(
		today.getUTCFullYear(),
		today.getUTCMonth(),
		today.getUTCDate(),
	);
	const from = todayStart - (days - 1) * DAY_MS;
	const buckets: HomeActivityBucket[] = Array.from(
		{ length: days },
		(_, i) => ({
			day: new Date(from + i * DAY_MS).toISOString().slice(0, 10),
			count: 0,
			attentionCount: 0,
		}),
	);
	const apps = new Map<string | null, HomeAppActivity>();
	const seen = new Set<string>();
	const rows: IExecutionUsageRecord[] = [];
	let invalidDates = 0;
	for (const row of response.items) {
		if (seen.has(row.id)) continue;
		seen.add(row.id);
		const timestamp = Date.parse(row.created_at);
		if (!Number.isFinite(timestamp)) {
			invalidDates++;
			continue;
		}
		if (timestamp < from || timestamp > now) continue;
		rows.push(row);
		const bucket = buckets[Math.floor((timestamp - from) / DAY_MS)];
		const attention = hasAttentionSeverity(row.status) ? 1 : 0;
		bucket.count++;
		bucket.attentionCount += attention;
		const app = apps.get(row.app_id) ?? {
			appId: row.app_id,
			count: 0,
			attentionCount: 0,
		};
		app.count++;
		app.attentionCount += attention;
		apps.set(row.app_id, app);
	}
	rows.sort((a, b) => Date.parse(b.created_at) - Date.parse(a.created_at));
	return {
		days,
		from,
		to: now,
		rows,
		buckets,
		apps: [...apps.values()].sort(
			(a, b) =>
				b.count - a.count || (a.appId ?? "").localeCompare(b.appId ?? ""),
		),
		scanned: seen.size,
		total: response.total,
		partial: seen.size < response.total,
		invalidDates,
	};
}

export function homeActivityCoverage(
	statistics: ReturnType<typeof summarizeHomeExecutions>,
): string {
	const period =
		statistics.days === 1
			? "Today (UTC)"
			: `Last ${statistics.days} days (UTC)`;
	return `${period}: ${statistics.rows.length.toLocaleString()} records in the latest ${statistics.scanned.toLocaleString()} of ${statistics.total.toLocaleString()} available records. ${statistics.partial ? "Sample counts may omit earlier activity in this period." : "All available execution records were checked."}${statistics.invalidDates ? ` ${statistics.invalidDates} records with invalid dates were excluded.` : ""}`;
}

export function homeUsageDollars(microDollars: number): string {
	return new Intl.NumberFormat(undefined, {
		style: "currency",
		currency: "USD",
		minimumFractionDigits: 2,
		maximumFractionDigits: 4,
	}).format(microDollars / 1_000_000);
}
