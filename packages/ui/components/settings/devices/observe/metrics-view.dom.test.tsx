import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	advance,
	byRole,
	click,
	fire,
	inPortal,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { FakeWorkspaceOptions } from "../testing/fake-workspace";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
// Device-lib modules load after the DOM exists (a static import breaks combined runs, W2-FAKES).
const { SAMPLE_IDS, SAMPLE_NOW, SAMPLE_PEOPLE } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { fakeCompact, fakeKeys } = await import("../testing/fake-device-api");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { ACCOUNT_SCOPE } = await import("../routing/devices-route");
const { keyThumbprint } = await import("./use-shared-metrics");
const { MetricsView } = await import("./metrics-view");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const WAREHOUSE = SAMPLE_IDS.warehouse;
const LAB = SAMPLE_IDS.lab;
const SERVICE = "support-bot";
const SLOW = 20_000;
const GIB = 1024 ** 3;
const MIB = 1024 ** 2;

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;
type Agent = ReturnType<Fake["agent"]>;

interface OpenOptions extends FakeWorkspaceOptions {
	serviceId?: string;
	connect?: string;
	arrange?(agent: Agent, fake: Fake): void | Promise<void>;
	people?: Record<string, string>;
}

async function open(deviceId: string, options: OpenOptions = {}) {
	const { serviceId, connect, arrange, people, ...rest } = options;
	const fake = await createFakeWorkspace(undefined, rest);
	if (connect) await fake.unlock(connect, { connectLive: true });
	await arrange?.(fake.agent(deviceId), fake);
	const view = await mountDevices(
		<MetricsView
			deviceId={deviceId}
			serviceId={serviceId}
			scope={ACCOUNT_SCOPE}
		/>,
		{
			fake,
			search: `device=${deviceId}&tab=metrics`,
			overlays: true,
			...(people
				? {
						backend: {
							userState: {
								getProfile: async () => fake.profile,
								getInfo: async () => ({ id: fake.hub.me, dev_mode: false }),
								updateUser: async () => undefined,
								lookupUser: async (id: string) => ({
									id,
									name: people[id] ?? id,
								}),
								lookupUsers: async (ids: string[]) =>
									ids.map((id) => ({ id, name: people[id] ?? id })),
							} as never,
						},
					}
				: {}),
		},
	);
	await view.settle();
	return view;
}

type View = Awaited<ReturnType<typeof open>>;

const text = (root: ParentNode = document.body) =>
	(root.textContent ?? "").replace(/\s+/g, " ");
const sent = (view: View) => view.fake.api.commands.map(([, type]) => type);
const block = (view: View, id: string) =>
	view.container.querySelector<HTMLElement>(`#${id}`) as HTMLElement;
const primaries = () => document.querySelectorAll("[data-dv-primary]").length;
const metric = (root: ParentNode, label: string) =>
	Array.from(root.querySelectorAll<HTMLElement>("[data-metric]")).find((cell) =>
		text(cell).startsWith(label),
	) as HTMLElement;
const stamps = (root: ParentNode) =>
	Array.from(root.querySelectorAll<HTMLElement>("[data-stamp]")).map(
		(stamp) => `${stamp.dataset.src}:${stamp.dataset.age}`,
	);

async function until(check: () => boolean, ms = 6_000) {
	const end = performance.now() + ms;
	while (!check() && performance.now() < end) await advance(50);
	expect(check()).toBe(true);
}

/** R3: wire values and codes never reach the screen. */
const MACHINE =
	/placement|replica|cgroup|process_rss|roster|telemetry|invocations|payload_bytes|concurrency_rejections|memory_used|one_logical_cpu|endpoint_id/;

const usage = {
	scope: "supervised_services",
	window: "current_process_lifetimes",
	reported_replicas: 2,
	expected_replicas: 3,
	invocations_started: 1843,
	invocations_succeeded: 1812,
	invocations_failed: 19,
	invocations_cancelled: 9,
	in_flight: 3,
	runtime_messages: 5210,
	request_payload_bytes: Number.MAX_SAFE_INTEGER + 1,
	response_payload_bytes: 30 * MIB,
	concurrency_rejections: 0,
};

const retained = {
	since: 1_788_220_800,
	through: SAMPLE_NOW - 10,
	counters: {
		invocations_started: 48_211,
		invocations_succeeded: 47_650,
		invocations_failed: 402,
		invocations_cancelled: 156,
		runtime_messages: 30,
		request_payload_bytes: 220 * MIB,
		response_payload_bytes: 710 * MIB,
		concurrency_rejections: 12,
	},
	coverage: {
		registered_runs: 4,
		reported_runs: 3,
		finalized_runs: 1,
		incomplete_runs: 2,
		unreported_runs: 1,
		fresh_active_runs: 0,
		stale_active_runs: 1,
		awaiting_first_report_runs: 0,
	},
	tail_loss_possible: true,
};

const deviceSample = (cpu = 23.4) => ({
	cpu_percent: cpu,
	memory_used_bytes: 5.7 * GIB,
	memory_total_bytes: 16 * GIB,
	agent_cpu_percent_of_one_core: 1.2,
	agent_memory_bytes: null,
	logical_cpus: 8,
	network: { received_bytes: 1.2 * MIB, transmitted_bytes: 822 * 1024 },
	storage_volume: { available_bytes: 281 * GIB, total_bytes: 477 * GIB },
	placements: 3,
	desired_replicas: 3,
	ready_replicas: 2,
	sample_seconds: 5,
	usage,
	usage_retained: retained,
});

const serviceSample = {
	project_id: "app_support_portal",
	process_cohort: "9f1c2a7be04d5a66c1d0",
	config_revision: 7,
	cpu_percent: 175.5,
	cpu_basis: "one_logical_cpu",
	memory_bytes: 616 * MIB,
	memory_basis: "cgroup_current",
	io: {
		basis: "cgroup_block_io",
		read_bytes_per_second: 512.5,
		written_bytes_per_second: null,
		replicas_observed: 2,
	},
	disk_quota: {
		used_bytes: 1024,
		limit_bytes: 8192,
		used_inodes: 12,
		limit_inodes: 64,
	},
	processes_observed: 2,
	desired_replicas: 2,
	running_replicas: 2,
	ready_replicas: 2,
	replicas: [
		{
			slot: 0,
			cpu_percent: 22.1,
			memory_bytes: 301 * MIB,
			processes_and_threads: 14,
		},
		{
			slot: 1,
			cpu_percent: null,
			memory_bytes: 311 * MIB,
			processes_and_threads: 13,
		},
		{ slot: 99, cpu_percent: 1, memory_bytes: 1 },
	],
	sample_seconds: 5,
	usage: { ...usage, expected_replicas: 2 },
	usage_retained: null,
};

/** A metrics snapshot in the encrypted status of `deviceId`, taken `ageS` seconds ago. */
async function snapshot(fake: Fake, deviceId: string, ageS: number) {
	const streams = fake.hub.streams.get(deviceId) ?? [];
	const status = streams.find((stream) => stream.kind === "status");
	if (!status)
		throw new Error("The seed has no status stream for this device.");
	fake.hub.streams.set(deviceId, [
		...streams.filter((stream) => stream.kind !== "metrics"),
		{
			kind: "metrics",
			scope: { kind: "device" },
			grantId: status.grantId,
			sequence: status.sequence,
			bootId: status.bootId,
			observedAt: SAMPLE_NOW - ageS,
			payload: {
				metrics: {
					records: [0, 1, 2].map((index) => ({
						sequence: index + 1,
						timestamp: SAMPLE_NOW - ageS - (2 - index) * 30,
						kind: "sample",
						data: deviceSample(60 + index),
					})),
				},
			},
		},
	]);
	await fake.workspace.fleet.refresh(deviceId);
}

async function openShared(view: View) {
	const details = block(view, "observe-shared-metrics") as HTMLDetailsElement;
	details.open = true;
	await fire(details, new window.Event("toggle"));
	return details;
}

describe("device resources", () => {
	test(
		"live values with the trend of the encrypted status; a value the device didn't send is unavailable, never zero",
		async () => {
			const view = await open(EDGE, {
				arrange: async (agent, fake) => {
					agent.record("metrics:device", deviceSample());
					await snapshot(fake, EDGE, 28);
				},
			});
			const resources = block(view, "observe-resources");
			await until(() => /23\.4/.test(text(resources)));
			expect(text(resources)).toContain("Device resources");
			expect(text(metric(resources, "CPU"))).toContain(
				"8 logical CPUs · 100 % = all of them",
			);
			expect(text(metric(resources, "Memory"))).toContain("of 16.0 GiB");
			expect(text(metric(resources, "Agent's data disk"))).toContain(
				"41 % used",
			);
			expect(text(metric(resources, "Network"))).toContain(
				"not a billing figure",
			);
			expect(text(metric(resources, "Agent memory"))).toContain(
				"Unavailable right now",
			);
			expect(text(metric(resources, "Agent memory"))).not.toMatch(/\b0\b/);
			expect(text(metric(resources, "Instances"))).toContain("2 of 3");
			expect(text(metric(resources, "Sample length"))).toContain("5");
			expect(stamps(resources)).toEqual(["live:live", "snap:current"]);
			expect(
				resources.querySelectorAll("svg[role=img], [role=img]").length,
			).toBeGreaterThan(0);
			expect(text(resources)).not.toMatch(MACHINE);
			expect(primaries()).toBe(0);
		},
		SLOW,
	);

	test(
		"usage is marked as not a billing record; missing counters and partial coverage are spelled out",
		async () => {
			const view = await open(EDGE, {
				arrange: (agent) => agent.record("metrics:device", deviceSample()),
			});
			await until(() => block(view, "observe-usage-current") !== null);
			const current = block(view, "observe-usage-current");
			const kept = block(view, "observe-usage-retained");
			expect(text(current)).toContain(
				"All services on edge-berlin-01 · Not a billing record.",
			);
			expect(text(current)).toContain("2 of 3 running instances");
			expect(text(current)).toContain("1,843");
			expect(text(current)).toContain("Requests receivedUnavailable");
			expect(text(current)).toContain("30.0 MiB");
			expect(text(kept)).toContain(
				"retained across restarts · Not a billing record.",
			);
			expect(text(kept)).toContain("Usage since");
			expect(text(kept)).toContain("48,211");
			expect(text(kept)).toContain(
				"3 of 4 runs reported · 1 finished · 2 ended without a final report · 1 ended before their first report · 1 stopped reporting",
			);
			expect(text(kept)).toContain("Some activity may be missing");
			expect(text(current) + text(kept)).not.toMatch(MACHINE);
		},
		SLOW,
	);

	test("without a live connection the last status keeps the values: current up to 75 s, last known after", async () => {
		const fresh = await open(WAREHOUSE, {
			arrange: (_agent, fake) => snapshot(fake, WAREHOUSE, 60),
		});
		const current = block(fresh, "observe-resources");
		expect(stamps(current)).toEqual(["snap:current"]);
		expect(text(metric(current, "CPU"))).toContain("62.0");
		await cleanupDevices();

		const stale = await open(WAREHOUSE, {
			arrange: (_agent, fake) => snapshot(fake, WAREHOUSE, 100),
		});
		const lastKnown = block(stale, "observe-resources");
		expect(stamps(lastKnown)).toEqual(["snap:lastknown"]);
		expect(text(lastKnown)).toContain("Last known");
		expect(text(metric(lastKnown, "CPU"))).toContain("62.0");
		expect(lastKnown.querySelector("[data-kind]")).toBeNull();
		expect(sent(stale)).not.toContain("metrics");
	});

	test("locked: the block says so and nothing is read", async () => {
		const view = await open(EDGE, { unlock: "none" });
		const resources = block(view, "observe-resources");
		expect(resources.querySelector("[data-kind=locked]")).not.toBeNull();
		expect(text(resources)).toContain(
			"Live resource values need the keys. Unlock to read them.",
		);
		expect(document.querySelector("[data-kind=empty]")).toBeNull();
		expect(sent(view)).toEqual([]);
		expect(block(view, "observe-usage-current")).toBeNull();
	});

	test(
		"shared for one app only: device resources say no access and aren't requested",
		async () => {
			const view = await open(LAB, { connect: LAB });
			const resources = block(view, "observe-resources");
			expect(text(resources)).toContain("No access to metrics.");
			expect(text(resources)).toContain(
				"Needs Read metrics on the whole device.",
			);
			expect(
				view.fake.api.commands.filter(
					([device, type, command]) =>
						device === LAB &&
						type === "metrics" &&
						command.placement_id === null,
				),
			).toEqual([]);
			expect(block(view, "observe-shared-metrics")).toBeNull();
		},
		SLOW,
	);
});

describe("service resources", () => {
	test(
		"one service: bases in words, a per-instance table, usage; unavailable values stay unavailable",
		async () => {
			const view = await open(EDGE, {
				serviceId: SERVICE,
				arrange: (agent) => agent.record(`metrics:${SERVICE}`, serviceSample),
			});
			const resources = block(view, "observe-resources");
			await until(() => /175\.5/.test(text(resources)));
			expect(text(resources)).toContain("Service resources");
			expect(text(metric(resources, "CPU"))).toContain("% of one CPU");
			expect(text(metric(resources, "CPU"))).toContain(
				"Can exceed 100 % on multi-core devices",
			);
			expect(text(metric(resources, "Sandbox memory"))).toContain("616.0");
			expect(text(metric(resources, "Sandbox disk I/O"))).toContain(
				"Unavailable right now",
			);
			expect(text(metric(resources, "Disk quota"))).toContain("of 8.0 KiB");
			expect(text(metric(resources, "Disk quota"))).toContain(
				"12 of 64 files · 13 % used",
			);
			expect(text(metric(resources, "Processes"))).toContain(
				"27 processes and threads in total",
			);
			expect(text(metric(resources, "Instances"))).toContain("of 2 ready");

			const instances = block(view, "observe-instances");
			const rows = Array.from(instances.querySelectorAll("tbody tr")).map(
				(row) => text(row),
			);
			expect(rows.length).toBe(2);
			expect(rows[0]).toContain("#0");
			expect(rows[0]).toContain("22.1 %");
			expect(rows[1]).toContain("–");
			expect(text(block(view, "observe-usage-current"))).toContain(
				"support-bot · Not a billing record.",
			);
			expect(text(block(view, "observe-usage-retained"))).toContain(
				"Not retained",
			);
			expect(text(view.container)).not.toMatch(MACHINE);
		},
		SLOW,
	);

	test(
		"a stopped service has nothing to measure, and says that instead of zeros",
		async () => {
			const view = await open(EDGE, { serviceId: "nightly-sync" });
			const resources = block(view, "observe-resources");
			await until(() =>
				/No running instances to measure/.test(text(resources)),
			);
			expect(text(resources)).toContain(
				"nightly-sync is stopped. Resource values appear once it runs.",
			);
			expect(resources.querySelector("[data-metric]")).toBeNull();
			expect(block(view, "observe-instances")).toBeNull();
		},
		SLOW,
	);
});

describe("trend history (BG16)", () => {
	test(
		"a newer agent is asked for its own history; an older one is not, and the trend is built here",
		async () => {
			const newer = await open(EDGE, {
				serviceId: SERVICE,
				arrange: (agent) => agent.record(`metrics:${SERVICE}`, serviceSample),
			});
			await until(() => sent(newer).includes("metrics_history"));
			const asked = newer.fake.api.commands.find(
				([, type]) => type === "metrics_history",
			);
			expect(asked?.[2]).toMatchObject({
				placement_id: SERVICE,
				fields: ["cpu_percent", "memory_bytes"],
			});
			await until(() =>
				/own metric history/.test(text(block(newer, "observe-resources"))),
			);
			await cleanupDevices();

			const older = await open(EDGE, {
				serviceId: SERVICE,
				agentFeatures: {},
				arrange: (agent) => agent.record(`metrics:${SERVICE}`, serviceSample),
			});
			const resources = block(older, "observe-resources");
			await until(() => /175\.5/.test(text(resources)));
			expect(sent(older)).not.toContain("metrics_history");
			expect(sent(older)).not.toContain("operations");
			expect(text(resources)).toContain(
				"Built from the samples read while this tab is open. Update the device agent to read its own history.",
			);
			expect(document.querySelector("[data-kind=error]")).toBeNull();
			expect(document.querySelector("[role=alert]")).toBeNull();
		},
		SLOW,
	);
});

describe("retained metrics history", () => {
	test(
		"a paused recording says since when and why, with Resume recording next to it",
		async () => {
			const view = await open(EDGE, {
				arrange: (agent) => agent.record("metrics:device", deviceSample()),
			});
			const kept = block(view, "observe-retained-metrics");
			await until(() => /Recording paused/.test(text(kept)));
			expect(text(kept)).toContain("because the readers list expired.");
			expect(text(kept)).toContain(
				"No metrics were retained since then. Live values above still work.",
			);
			const resume = byRole("button", "Resume recording…", kept);
			expect(resume.getAttribute("aria-disabled")).toBeNull();
			expect(text(kept.querySelector("[data-plan-line]") as Element)).toContain(
				"PRO plan · kept 7 days",
			);
			await click(resume);
			expect(text(inPortal("dialog"))).toContain(
				"Retained metrics · Whole device · edge-berlin-01",
			);
			expect(primaries()).toBe(1);
		},
		SLOW,
	);

	test(
		"while the device hasn't applied the newest access rules, Resume recording stays visible with the reason and does nothing",
		async () => {
			const view = await open(EDGE, {
				arrange: (agent, fake) => {
					agent.record("metrics:device", deviceSample());
					const policy = fake.hub.policies.get(EDGE);
					if (!policy) throw new Error("The seed has access rules for edge.");
					policy.appliedVersion = policy.version - 1;
					policy.appliedDigest = "digest-before";
				},
			});
			const kept = block(view, "observe-retained-metrics");
			await until(() => /Recording paused/.test(text(kept)));
			const resume = byRole("button", "Resume recording…", kept);
			await until(() => resume.getAttribute("aria-disabled") === "true");
			expect(kept.querySelector("[data-gate-inline]")).not.toBeNull();
			const commands = view.fake.api.commands.length;
			const writes = view.fake.api.writes().length;
			await click(byRole("button", "Resume recording…", kept));
			expect(queryByRole("dialog")).toBeNull();
			expect(
				view.fake.api.commands
					.slice(commands)
					.filter(([, type]) => type === "archive_policy"),
			).toEqual([]);
			expect(view.fake.api.writes().length).toBe(writes);
		},
		SLOW,
	);

	test("offline and never read: the block says what it can't know instead of looking empty", async () => {
		const view = await open(WAREHOUSE);
		const kept = block(view, "observe-retained-metrics");
		expect(text(kept)).toContain(
			"Whether history is set up for warehouse-pi is read from warehouse-pi over a live connection.",
		);
		expect(kept.querySelector("[data-kind=empty]")).toBeNull();
		expect(kept.querySelector("[data-kind=error]")).toBeNull();
		expect(sent(view)).not.toContain("archive_roster_read");
	});
});

describe("shared live metrics", () => {
	const roster = (
		fake: Fake,
		members: string[],
		expiresAt = SAMPLE_NOW + 3_600,
	) => {
		const publisher = {
			endpoint_id: EDGE,
			signing_key: fakeKeys.identity(EDGE).telemetry_key,
		};
		return fakeCompact(
			{
				version: 1,
				device_id: EDGE,
				scope: "device",
				policy_version: 2,
				previous_policy_digest: null,
				management_policy_digest: null,
				publisher,
				members: [
					publisher,
					...members.map((endpoint_id) => ({
						endpoint_id,
						signing_key: fakeKeys.identity(EDGE).telemetry_key,
					})),
				],
				issued_at: SAMPLE_NOW - 3_600,
				expires_at: expiresAt,
			},
			fake.hub.ownerInvitationKey(EDGE),
		);
	};

	test(
		"collapsed by default: nothing is read until it is opened; an empty group offers Add readers",
		async () => {
			const view = await open(EDGE, {
				arrange: (agent) => agent.record("metrics:device", deviceSample()),
			});
			const shared = block(
				view,
				"observe-shared-metrics",
			) as HTMLDetailsElement;
			expect(shared.open).toBe(false);
			expect(text(shared)).toContain("Shared live metrics");
			expect(text(shared)).toContain("Advanced");
			await advance(300);
			expect(sent(view)).not.toContain("telemetry_roster_read");

			await openShared(view);
			await until(() => sent(view).includes("telemetry_roster_read"));
			await until(() => /Not set up/.test(text(shared)));
			expect(byRole("button", "Add readers…", shared)).not.toBeNull();
			expect(queryByRole("button", "Renew", shared)).toBeNull();
			expect(text(shared)).not.toMatch(MACHINE);
		},
		SLOW,
	);

	test(
		"readers are people when the device says who they are (BG24), endpoint IDs otherwise",
		async () => {
			const view = await open(EDGE, {
				people: { [SAMPLE_PEOPLE.mira]: "Mira Novak" },
				arrange: async (agent, fake) => {
					const mine =
						fake.workspace.keys.controller(EDGE)?.publicBundle().endpoint_id ??
						"";
					const policy = fake.workspace.facts;
					void policy;
					const miraKey = fake.seed.policies[EDGE]?.policy?.grants.find(
						(grant) => grant.user_id === SAMPLE_PEOPLE.mira,
					)?.controller_key;
					if (!miraKey) throw new Error("The seed shares edge with Mira.");
					agent.setRoster(
						"telemetry",
						"device",
						roster(fake, [mine, "reader-mira-01"]),
						{
							confirmed_readers: [mine, "reader-mira-01"],
							reader_bindings: [
								{
									endpoint_id: "reader-mira-01",
									controller_key_thumbprint: await keyThumbprint(miraKey),
								},
							],
						},
					);
				},
			});
			const shared = await openShared(view);
			await until(() => /Mira Novak/.test(text(shared)));
			expect(text(shared)).toContain("You");
			expect(text(shared)).toContain(
				"All 2 readers confirmed the latest sample",
			);
			expect(text(shared)).toContain("Readers list expires");
			expect(text(shared)).not.toContain("reader-m");
			await cleanupDevices();

			const older = await open(EDGE, {
				agentFeatures: {},
				arrange: (agent, fake) => {
					const mine =
						fake.workspace.keys.controller(EDGE)?.publicBundle().endpoint_id ??
						"";
					agent.setRoster(
						"telemetry",
						"device",
						roster(fake, [mine, "reader-mira-01"]),
					);
				},
			});
			const interim = await openShared(older);
			await until(() => /2 readers/.test(text(interim)));
			expect(text(interim)).toContain("reader-m");
			expect(text(interim)).toContain("(you)");
			expect(text(interim)).toContain(
				"Not reported by this version of the device agent",
			);
			expect(document.querySelector("[data-kind=error]")).toBeNull();
		},
		SLOW,
	);

	test(
		"Renew shows what it does first, then signs the same list for the device",
		async () => {
			const view = await open(EDGE, {
				arrange: (agent, fake) => {
					const controller = fake.workspace.keys.controller(EDGE);
					if (!controller) throw new Error("edge is unlocked in the seed.");
					const endpoint = controller.createTelemetry({} as never);
					const joined = () => ({
						...endpoint,
						position: () => ({ joined: true, retired: false, sequence: 4 }),
					});
					controller.createTelemetry = joined;
					controller.openTelemetry = joined;
					const mine = controller.publicBundle().endpoint_id;
					agent.setRoster("telemetry", "device", roster(fake, [mine]), {
						confirmed_readers: [mine],
					});
					agent.handle("telemetry_policy", (command) => ({
						state: "completed",
						result: { sequence: command.sequence },
					}));
				},
			});
			const shared = await openShared(view);
			await until(() => queryByRole("button", "Renew", shared) !== null);
			await click(byRole("button", "Renew", shared));
			const sheet = inPortal();
			expect(text(sheet)).toContain(
				"Signs the same readers list again, valid for one more day.",
			);
			expect(sent(view)).not.toContain("telemetry_policy");
			await click(byRole("button", "Renew shared metric readers", sheet));
			await until(() => sent(view).includes("telemetry_policy"));
			const command = view.fake.api.commands.find(
				([, type]) => type === "telemetry_policy",
			);
			expect(command?.[2].scope).toBe("device");
			expect(command?.[2].key_packages).toEqual([]);
		},
		SLOW,
	);
});
