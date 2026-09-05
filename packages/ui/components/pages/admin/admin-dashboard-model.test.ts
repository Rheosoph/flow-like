import { describe, expect, test } from "bun:test";
import {
	normalizeRegistryStats,
	prioritizeDashboardQueues,
	readDashboardCount,
} from "./admin-dashboard-model";

const apiStats = {
	total_packages: 24,
	total_versions: 63,
	total_downloads: 8145,
	pending_review: 4,
	active_packages: 18,
	rejected_packages: 2,
};

const dashboardStats = {
	totalPackages: 24,
	totalVersions: 63,
	totalDownloads: 8145,
	pendingReview: 4,
	activePackages: 18,
	rejectedPackages: 2,
};

describe("registry statistics contract", () => {
	test("renders the counts returned by the snake_case admin API", () => {
		expect(normalizeRegistryStats(apiStats)).toEqual(dashboardStats);
	});

	test("accepts an already normalized response", () => {
		expect(normalizeRegistryStats(dashboardStats)).toEqual(dashboardStats);
	});

	test("preserves confirmed zero counts", () => {
		expect(
			normalizeRegistryStats({ ...apiStats, pending_review: 0 }).pendingReview,
		).toBe(0);
	});

	test("does not turn a missing pending count into an empty review queue", () => {
		const { pending_review, ...incomplete } = apiStats;
		expect(() => normalizeRegistryStats(incomplete)).toThrow(
			"Missing registry statistic: pendingReview",
		);
	});

	test.each([{ raw: null }, { raw: [] }, { raw: "unavailable" }, { raw: 0 }])(
		"rejects invalid response %p",
		({ raw }) => {
			expect(() => normalizeRegistryStats(raw)).toThrow(
				"Invalid registry statistics response",
			);
		},
	);

	test.each([
		undefined,
		null,
		"4",
		Number.NaN,
		Number.POSITIVE_INFINITY,
		-1,
		1.5,
	])("rejects a malformed API count %p", (value) => {
		expect(() =>
			normalizeRegistryStats({ ...apiStats, pending_review: value }),
		).toThrow("Invalid dashboard count: pending_review");
	});

	test("accepts matching aliases and rejects conflicting aliases", () => {
		expect(normalizeRegistryStats({ ...apiStats, ...dashboardStats })).toEqual(
			dashboardStats,
		);
		expect(() =>
			normalizeRegistryStats({ ...apiStats, pendingReview: 0 }),
		).toThrow("Conflicting registry statistic: pendingReview");
	});
});

describe("dashboard queue counts", () => {
	test("retains safe integer counts and rejects values that lose precision", () => {
		expect(readDashboardCount(0)).toBe(0);
		expect(readDashboardCount(Number.MAX_SAFE_INTEGER)).toBe(
			Number.MAX_SAFE_INTEGER,
		);
		expect(() => readDashboardCount(Number.MAX_SAFE_INTEGER + 1)).toThrow();
	});

	test("puts urgent work before large backlogs, then unknown and empty queues", () => {
		const queues = [
			{ id: "empty", count: 0, priority: 0 },
			{ id: "unavailable", count: null, priority: 0 },
			{ id: "reviews", count: 80, priority: 2 },
			{ id: "critical", count: 1, priority: 0 },
			{ id: "alerts", count: 5, priority: 1 },
		] as const;
		expect(prioritizeDashboardQueues(queues).map((queue) => queue.id)).toEqual([
			"critical",
			"alerts",
			"reviews",
			"unavailable",
			"empty",
		]);
		expect(queues[0].id).toBe("empty");
	});

	test("keeps equal-priority queues stable across count refreshes", () => {
		const queues = [
			{ id: "apps", count: 1, priority: 2 },
			{ id: "suites", count: 7, priority: 2 },
		];
		expect(prioritizeDashboardQueues(queues).map((queue) => queue.id)).toEqual([
			"apps",
			"suites",
		]);
	});
});
