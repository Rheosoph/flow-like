import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	advance,
	byRole,
	click,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
// Device-lib modules load after the DOM exists (a static import breaks combined runs, W2-FAKES).
const { SAMPLE_APPS, SAMPLE_IDS, SAMPLE_NOW } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { AppMetricsBlock } = await import("./app-metrics");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const WAREHOUSE = SAMPLE_IDS.warehouse;
const APP = SAMPLE_APPS.supportPortal;
const SLOW = 20_000;
const MIB = 1024 ** 2;

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;

interface OpenOptions {
	arrange?(fake: Fake): void | Promise<void>;
	backend?: MountDevicesOptions["backend"];
	agentFeatures?: Record<string, never>;
	hubVersion?: "old";
}

async function open(appId: string, options: OpenOptions = {}) {
	const { arrange, backend, ...rest } = options;
	const fake = await createFakeWorkspace(undefined, rest);
	await arrange?.(fake);
	const view = await mountDevices(<AppMetricsBlock appId={appId} />, {
		fake,
		host: "app",
		search: `id=${appId}`,
		...(backend ? { backend } : {}),
	});
	await view.settle();
	return view;
}

type View = Awaited<ReturnType<typeof open>>;

const text = (root: ParentNode = document.body) =>
	(root.textContent ?? "").replace(/\s+/g, " ");
const sent = (view: View) => view.fake.api.commands.map(([, type]) => type);
const block = (view: View) =>
	view.container.querySelector<HTMLElement>(
		"#observe-app-metrics",
	) as HTMLElement;
const stamp = (view: View) =>
	block(view).querySelector<HTMLElement>("[data-stamp]") as HTMLElement;

async function until(check: () => boolean, ms = 6_000) {
	const end = performance.now() + ms;
	while (!check() && performance.now() < end) await advance(50);
	expect(check()).toBe(true);
}

const MACHINE =
	/placement|replica|cgroup|invocations|metric_coverage|project_metrics|observed_placement_workers/;

const sample = (sampled: number) => ({
	project_id: APP,
	desired_replicas: 2,
	ready_replicas: 2,
	cpu_percent: sampled ? 39.2 : null,
	cpu_basis: "one_logical_cpu",
	memory_bytes: sampled ? 628 * MIB : null,
	running_replicas: 2,
	processes_observed: 2,
	io: {
		basis: "observed_placement_workers",
		read_bytes_per_second: 48 * 1024,
		written_bytes_per_second: 212 * 1024,
	},
	metric_coverage: {
		sampled_placements: sampled,
		missing_or_stale_placements: 1 - sampled,
		oldest_sample_at: SAMPLE_NOW - 4,
		freshness_seconds: 15,
	},
	usage: {
		reported_replicas: 2,
		expected_replicas: 2,
		invocations_started: 1843,
		invocations_succeeded: 1812,
		invocations_failed: 19,
		in_flight: 3,
	},
	usage_retained: {
		since: 1_788_220_800,
		counters: {
			invocations_started: 48_211,
			invocations_succeeded: 47_650,
			invocations_failed: 402,
		},
		tail_loss_possible: false,
	},
});

describe("app metrics", () => {
	test(
		"sampled live per device and totalled; the coverage line is the freshness",
		async () => {
			const view = await open(APP, {
				arrange: (fake) =>
					fake.agent(EDGE).record(`metrics:project:${APP}`, sample(1)),
			});
			await until(() => /39\.2/.test(text(block(view))));
			expect(text(block(view))).toContain("App metrics");
			expect(text(stamp(view))).toContain("1 of 1 running service sampled");
			expect(stamp(view).dataset.age).toBe("live");
			const shown = text(block(view));
			expect(shown).toContain("% of one CPU");
			expect(shown).toContain("Sum of the app's services");
			expect(shown).toContain("628.0");
			expect(shown).toContain("212.0");
			expect(shown).toContain("1,843");
			expect(shown).toContain(
				"from 2 of 2 running instances · Not a billing record",
			);
			expect(shown).toContain("48,211");
			expect(shown).toContain("kept across restarts · Not a billing record");
			const row = block(view).querySelector(
				"[data-app-row=live]",
			) as HTMLElement;
			expect(text(row)).toContain("edge-berlin-01");
			expect(text(row)).toContain("1 of 1");
			expect(row.querySelector("a")?.getAttribute("href")).toContain(
				`device=${EDGE}`,
			);
			expect(row.querySelector("a")?.getAttribute("href")).toContain(
				"tab=metrics",
			);
			expect(row.querySelector("a")?.getAttribute("href")).toContain(
				`id=${APP}`,
			);
			// One device: no total row.
			expect(block(view).querySelector("[data-app-row=total]")).toBeNull();
			expect(shown).not.toMatch(MACHINE);
			expect(document.querySelectorAll("[data-dv-primary]").length).toBe(0);
			expect(
				view.fake.api.commands.find(
					([, type]) => type === "project_metrics",
				)?.[2].project_id,
			).toBe(APP);
		},
		SLOW,
	);

	test(
		"a running service without a fresh sample: values are unavailable, never zero, and the line names what is missing",
		async () => {
			const view = await open(APP, {
				arrange: (fake) =>
					fake.agent(EDGE).record(`metrics:project:${APP}`, sample(0)),
			});
			await until(() => /Not sampled/.test(text(block(view))));
			expect(text(stamp(view))).toContain("0 of 1 running service sampled");
			expect(stamp(view).dataset.age).toBe("notloaded");
			expect(
				text(block(view).querySelector("[data-app-coverage]") as Element),
			).toContain(
				"Not sampled: 1 service on edge-berlin-01 (no fresh sample). CPU and memory stay unavailable until every running service has a fresh sample.",
			);
			const cells = Array.from(
				block(view).querySelectorAll<HTMLElement>("[data-metric]"),
			).map((cell) => text(cell));
			expect(
				cells.filter((cell) => cell.includes("Unavailable right now")).length,
			).toBe(4);
			expect(cells.some((cell) => /^CPU0/.test(cell))).toBe(false);
			expect(document.querySelector("[data-kind=error]")).toBeNull();
		},
		SLOW,
	);

	test("an offline device isn't asked: its row says why and offers Diagnose", async () => {
		const view = await open(SAMPLE_APPS.warehouseScanner);
		const row = block(view).querySelector(
			"[data-app-row=nolive]",
		) as HTMLElement;
		expect(text(row)).toContain("warehouse-pi");
		expect(text(row)).toContain(
			"warehouse-pi is offline. App metrics need a live connection.",
		);
		expect(text(stamp(view))).toContain("0 of 1 running service sampled");
		expect(text(block(view))).toContain(
			"Not sampled: scanner-ingest on warehouse-pi (offline).",
		);
		await click(byRole("button", "Diagnose", row));
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "diagnose",
			deviceId: WAREHOUSE,
		});
		expect(sent(view)).not.toContain("project_metrics");
		expect(view.fake.agent(WAREHOUSE).sessions).toBe(0);
	});

	test("every service stopped: nothing to sample, said as such", async () => {
		const view = await open(SAMPLE_APPS.crmSync);
		expect(text(block(view))).toContain("No running services to sample");
		expect(text(block(view))).toContain(
			"No running services here: nightly-sync is stopped.",
		);
		expect(text(stamp(view))).toContain("no running services");
		expect(block(view).querySelector("[data-metric]")).toBeNull();
		expect(sent(view)).not.toContain("project_metrics");
	});

	test("locked devices: the block asks to unlock and reads nothing", async () => {
		const view = await open(APP);
		view.fake.workspace.keys.lockAll();
		await view.settle();
		await until(() => /Metrics need unlocked devices/.test(text(block(view))));
		expect(block(view).querySelector("[data-kind=locked]")).not.toBeNull();
		expect(block(view).querySelector("[data-kind=empty]")).toBeNull();
		expect(stamp(view).dataset.age).toBe("locked");
		await click(byRole("button", "Unlock several…", block(view)));
		expect(useOverlayStore.getState().overlay.kind).toBe("unlock_several");
	});

	test("without permission to read the app's flows the block isn't rendered at all", async () => {
		const view = await open(APP, {
			backend: {
				roleState: {
					getOwnRole: async () => ({
						role_id: "viewer",
						role_name: "Viewer",
						permissions: 0,
						is_owner: false,
					}),
				} as never,
			},
		});
		await view.settle();
		await until(() => block(view) === null);
		expect(queryByRole("table")).toBeNull();
		expect(sent(view)).not.toContain("project_metrics");
	});

	test(
		"older hub and older agent: the same block, no new command, no error",
		async () => {
			const view = await open(APP, {
				agentFeatures: {},
				hubVersion: "old",
				arrange: (fake) =>
					fake.agent(EDGE).record(`metrics:project:${APP}`, sample(1)),
			});
			await until(() => /39\.2/.test(text(block(view))));
			expect(text(block(view))).toContain("this block shows no trends");
			for (const type of ["metrics_history", "operations"])
				expect(sent(view)).not.toContain(type);
			expect(document.querySelector("[data-kind=error]")).toBeNull();
			expect(document.querySelector("[role=alert]")).toBeNull();
		},
		SLOW,
	);
});
