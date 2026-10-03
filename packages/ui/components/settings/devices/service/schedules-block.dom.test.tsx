import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";
import type { FakeWorkspaceOptions } from "../testing/fake-workspace";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
// Device-lib modules load after the DOM exists (a static import breaks combined runs, W2-FAKES).
const { SAMPLE_NOW } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { NIGHTLY, serveNightlyOnEdge } = await import(
	"../testing/schedule-scenarios"
);
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { ServiceScreen } = await import("./service-screen");
const { SHOP, openShop, shopRow, stopShop } = await import("./status-test-kit");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

function Page() {
	const { route, scope } = useDevicesRoute();
	if (route.screen !== "service")
		return <p data-left="">{`left:${route.screen}`}</p>;
	return (
		<ServiceScreen
			route={route}
			scope={scope}
			deviceId={route.deviceId}
			serviceId={route.serviceId}
		/>
	);
}

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;
type Mount = NonNullable<Parameters<typeof mountDevices>[1]>;

interface OpenOptions extends FakeWorkspaceOptions {
	/** False: nobody moved the schedule to the service. */
	release?: boolean;
	/** Changes the world after the service took the schedule and before the page opens. */
	arrange?(fake: Fake): void | Promise<void>;
	/**
	 * Who reads the device. `status`: a computer that can't reach it and only
	 * has its published status. `locked`: one without the keys for it.
	 */
	reader?: "status" | "locked";
	backend?: Mount["backend"];
}

/** Read boards, nothing else: a member who can't edit the app's events. */
const VIEWER = {
	role_id: "role-viewer",
	role_name: "Viewer",
	permissions: 256,
	is_owner: false,
	can_leave: true,
};

/** invoice-extractor on edge-berlin-01 with the nightly schedule, opened on its Status tab. */
async function open({
	release,
	arrange,
	reader,
	backend,
	...options
}: OpenOptions = {}) {
	const world = await createFakeWorkspace(undefined, options);
	await serveNightlyOnEdge(world, release === false ? { release } : {});
	await arrange?.(world);
	world.api.hub.publishStatus(NIGHTLY.device, world.agent(NIGHTLY.device));
	if (reader === "status") world.agent(NIGHTLY.device).online = false;
	// The same hub and device, read by another computer.
	const fake = reader
		? await createFakeWorkspace(undefined, {
				api: world.api,
				viewFacts: false,
				...(reader === "locked" ? { unlock: "none" as const } : {}),
			})
		: world;
	const view = await mountDevices(<Page />, {
		fake,
		search: `device=${NIGHTLY.device}&service=${NIGHTLY.service}&tab=status`,
		...(backend ? { backend } : {}),
	});
	await view.settle();
	return view;
}

const text = (root: ParentNode | null | undefined) =>
	(root?.textContent ?? "").replace(/\s+/g, " ").trim();
const has = (root: ParentNode, selector: string) =>
	root.querySelector(selector) !== null;
const block = (root: ParentNode) =>
	root.querySelector<HTMLElement>("#service-schedules");
const schedule = (root: ParentNode) =>
	root.querySelector<HTMLElement>(`[data-schedule="${NIGHTLY.event}"]`);
const lines = (root: ParentNode) =>
	[...(schedule(root)?.querySelectorAll("[data-schedule-run]") ?? [])].map(
		(line) => text(line),
	);
const entry = (fake: Fake) => {
	const found = fake.agent(NIGHTLY.device).placement(NIGHTLY.service)
		?.schedules?.[0];
	if (!found) throw new Error("The service reports no schedule.");
	return found;
};
const NOW_S = SAMPLE_NOW;

describe("Service › Status · Schedules", () => {
	test("a schedule the service runs: what it is, and the device's own next run", async () => {
		const { container } = await open();
		const item = schedule(container);
		expect(item?.dataset.scheduleState).toBe("armed");
		expect(text(item)).toContain("Nightly reconciliation");
		expect(text(item)).toContain("At 02:00 every day · Europe/Berlin");
		expect(lines(container)).toEqual([
			"Next run Oct 1 at 02:00 GMT+2 · in 12 hr. · Oct 1 at 00:00 your time",
		]);
		expect(text(block(container))).toContain(
			"A run is skipped when the device is off, asleep or restarting at its time, and when the previous run is still going.",
		);
		expect(text(block(container))).toContain(
			"When the clocks change, a time that comes twice runs once",
		);
		expect(queryByRole("button", "Run it here", container) === null).toBe(true);
	});

	test("the last run, a run in progress and skipped times are the device's numbers", async () => {
		const { container } = await open({
			arrange: (fake) => {
				Object.assign(entry(fake), {
					running: true,
					last_at: NOW_S - 12 * 3_600,
					last_outcome: "failed",
					skipped: 2,
					last_skip: { at: NOW_S - 36 * 3_600, reason: "overlap" },
				});
			},
		});
		const [running, next, last] = lines(container);
		expect(running).toBe("A run is going now.");
		expect(next).toMatch(/^Next run Oct 1 at 02:00 GMT\+2 · in 12 hr\./);
		expect(last).toBe(
			"Last run 02:00 GMT+2 · 12 hr. ago · 00:00 your time · failed",
		);
		expect(
			text(schedule(container)?.querySelector("[data-schedule-skipped]")),
		).toBe("Skipped 2 · last: the previous run was still going");
	});

	test("from the device's published status the next run is computed from the schedule, and says so", async () => {
		const { container } = await open({
			reader: "status",
			arrange: (fake) => {
				Object.assign(entry(fake), { skipped: 2, last_outcome: "succeeded" });
			},
		});
		expect(schedule(container)?.dataset.scheduleState).toBe("armed");
		// A status carries the result and neither a time nor a counter.
		expect(lines(container)).toEqual([
			"Next run by its schedule: Oct 1 at 02:00 GMT+2 · in 12 hr. · Oct 1 at 00:00 your time",
			"Last run succeeded",
		]);
		expect(has(container, "[data-schedule-skipped]")).toBe(false);
	});

	test("a device that dropped off after it was read: no next run from a status that old, and no counters", async () => {
		const { container } = await open({
			arrange: async (fake) => {
				Object.assign(entry(fake), { skipped: 2 });
				await fake.workspace.live.refreshInspection(NIGHTLY.device);
				fake.agent(NIGHTLY.device).online = false;
			},
		});
		expect(schedule(container)?.dataset.scheduleState).toBe("armed");
		expect(lines(container)).toEqual([
			"This status is too old to say when it runs next.",
		]);
		expect(has(container, "[data-schedule-skipped]")).toBe(false);
	});

	test("without this computer's keys nothing is said about its runs", async () => {
		const { container } = await open({ reader: "locked" });
		expect(text(container)).not.toContain("Next run");
		expect(text(container)).not.toContain("No runs while");
		const item = schedule(container);
		// A locked device may list no service at all; a row it does show says only that it is locked.
		if (item) {
			expect(item.dataset.scheduleState).toBe("locked");
			expect(lines(container)).toEqual(["Unknown until unlocked"]);
		}
	});

	test("a stopped service: no next run and no counters, even when its row still carries them", async () => {
		const { container } = await open({
			arrange: async (fake) => {
				const row = fake.agent(NIGHTLY.device).placement(NIGHTLY.service);
				const kept = { ...entry(fake), skipped: 4, runs: 9 };
				Object.assign(row ?? {}, {
					desired_state: "stopped",
					observed_state: "stopped",
					running_replicas: 0,
					ready_replicas: 0,
					schedules: [kept],
				});
			},
		});
		expect(schedule(container)?.dataset.scheduleState).toBe("stopped");
		expect(lines(container)).toEqual([
			"No runs while the service is not running.",
		]);
		expect(text(block(container))).not.toContain("Next run");
		expect(has(container, "[data-schedule-skipped]")).toBe(false);
	});

	test("a running service that has not said anything about the schedule yet", async () => {
		const { container } = await open({
			arrange: (fake) => {
				const row = fake.agent(NIGHTLY.device).placement(NIGHTLY.service);
				if (row) row.schedules = undefined;
			},
		});
		expect(schedule(container)?.dataset.scheduleState).toBe("not_reported");
		expect(lines(container)).toEqual(["Not reported yet."]);
	});

	test("an agent from before schedules can't say: the row asks for the agent update", async () => {
		const { container } = await open({
			agentFeatures: { placement_events: 1, placement_diagnostics: 1 },
		});
		expect(schedule(container)?.dataset.scheduleState).toBe("needs_agent");
		expect(lines(container)).toEqual([
			"Update the device agent to see its schedules.",
		]);
	});
});

describe("Service · Stop and Start say what happens to its schedules", () => {
	const bar = (root: ParentNode) =>
		root.querySelector("[data-service-actions]") as HTMLElement;
	const confirm = (root: ParentNode) =>
		text(root.querySelector("[data-inline-confirm]"));

	test("Stop: they stop, and the hub doesn't take them over by itself", async () => {
		const { container, fake } = await open();
		await click(byRole("button", "Stop…", bar(container)));
		expect(confirm(container)).toContain(
			"Its schedules stop. A run in progress is cut off. The hub doesn't take them over. To run one on the hub meanwhile, choose Run it on the hub again in Events.",
		);
		expect(fake.api.commands.some(([, type]) => type === "stop")).toBe(false);
	});

	test("Start: they run again from their next time, also when the stopped service reports none", async () => {
		const { container } = await open({
			viewFacts: false,
			arrange: (fake) => {
				const agent = fake.agent(NIGHTLY.device);
				const row = agent.placement(NIGHTLY.service);
				// The sample's update of this service is over: only then Start is allowed.
				agent.rollouts.length = 0;
				Object.assign(row ?? {}, {
					desired_state: "stopped",
					observed_state: "stopped",
					applied_revision: row?.config_revision,
					running_replicas: 0,
					ready_replicas: 0,
					schedules: undefined,
				});
			},
		});
		await click(byRole("button", "Start", bar(container)));
		expect(confirm(container)).toContain(
			"Schedules run again from their next time. Missed runs are not made up.",
		);
	});

	test("a service without a schedule says nothing about schedules", async () => {
		const view = await mountDevices(<Page />, {
			search: `device=${NIGHTLY.device}&service=support-bot&tab=status`,
		});
		await view.settle();
		expect(has(view.container, "#service-schedules")).toBe(false);
		await click(byRole("button", "Stop…", bar(view.container)));
		expect(confirm(view.container)).toContain("The service stops answering");
		expect(confirm(view.container)).not.toContain("schedules");
	});
});

describe("Service › Status · a schedule nobody moved here", () => {
	const HELD =
		"Not running here: nobody who can edit this app's events has moved it to this service. The hub runs it.";

	test("it says the hub runs it; Run it here moves it after a confirm that says when", async () => {
		const { container, fake, settle } = await open({ release: false });
		expect(schedule(container)?.dataset.scheduleState).toBe("held");
		expect(lines(container)).toEqual([HELD]);
		expect(text(block(container))).not.toContain("Next run");

		await click(byRole("button", "Run it here", container));
		const item = schedule(container) as HTMLElement;
		expect(text(item)).toContain(
			`${NIGHTLY.service} takes Nightly reconciliation over at its next check, within 5 minutes. The hub stops running it then.`,
		);
		expect(text(item)).toContain(
			"Until then the hub keeps running it. No run happens twice.",
		);
		expect(fake.api.sent("PUT", /device-schedules/).length).toBe(0);

		const confirm = [...item.querySelectorAll("button")]
			.filter((button) => text(button) === "Run it here")
			.at(-1);
		await click(confirm as HTMLElement);
		await settle();
		const [call] = fake.api.sent("PUT", /device-schedules/);
		expect(call?.[1]).toContain(
			`apps/${NIGHTLY.app}/device-schedules/${NIGHTLY.event}`,
		);
		expect(call?.[2]).toEqual({
			device_id: NIGHTLY.device,
			placement_id: NIGHTLY.service,
		});
		expect(text(schedule(container))).toContain(
			`${NIGHTLY.service} takes Nightly reconciliation over at its next check, within 5 minutes.`,
		);
	});

	test("someone who can't edit the app's events sees the button disabled with who can", async () => {
		const { container, fake } = await open({
			release: false,
			backend: { roleState: { getOwnRole: async () => VIEWER } } as never,
		});
		expect(lines(container)).toEqual([HELD]);
		const button = byRole("button", "Run it here", container);
		expect(button.getAttribute("aria-disabled")).toBe("true");
		expect(text(schedule(container))).toContain(
			"Only someone who can edit this app's events can move it.",
		);
		await click(button);
		expect(fake.api.sent("PUT", /device-schedules/).length).toBe(0);
		expect(text(schedule(container))).not.toContain("within 5 minutes");
	});

	test("the hub's refusal is said in the confirm, and nothing reads as moved", async () => {
		const { container, fake, settle } = await open({ release: false });
		fake.hub.schedules.canEditEvents = false;
		await click(byRole("button", "Run it here", container));
		const item = schedule(container) as HTMLElement;
		const confirm = [...item.querySelectorAll("button")]
			.filter((button) => text(button) === "Run it here")
			.at(-1);
		await click(confirm as HTMLElement);
		await settle();
		expect(text(schedule(container))).toContain(
			"Only someone who can edit this app's events can move it.",
		);
		expect(text(schedule(container))).not.toContain(
			"takes Nightly reconciliation over at its next check, within 5 minutes.The hub",
		);
		expect(lines(container)).toEqual([HELD]);
	});
});

describe("Service › Status · a one-time schedule", () => {
	type ShopFake = Parameters<typeof shopRow>[0];
	/** Two hours before the sample's clock: 12:00 in Berlin, the same day. */
	const PAST = NOW_S - 2 * 3_600;
	const onceItem = (root: ParentNode) =>
		root.querySelector<HTMLElement>(`[data-schedule="${SHOP.once}"]`);
	const onceLines = (root: ParentNode) =>
		[...(onceItem(root)?.querySelectorAll("[data-schedule-run]") ?? [])].map(
			(line) => text(line),
		);
	const onceEntry = (fake: ShopFake) => {
		const found = shopRow(fake).schedules?.find(
			(value) => value.event_id === SHOP.once,
		);
		if (!found) throw new Error("The service reports no one-time schedule.");
		return found;
	};
	/** The device's record of a run two hours ago, as it reports a finished entry. */
	const finished =
		(state: "ran" | "missed" | "passed", outcome?: string) =>
		(fake: ShopFake) => {
			Object.assign(onceEntry(fake), {
				once_at: PAST,
				once_state: state,
				next_at: null,
				running: false,
				last_at: state === "ran" ? PAST + 4 : null,
				last_outcome: outcome ?? null,
			});
		};
	const open = (options: Parameters<typeof openShop>[0] = {}) =>
		openShop({ events: [SHOP.once], ...options });

	test("armed before its time: once on its own date and time, when it runs, and only the one-time rule", async () => {
		const { container } = await open();
		const item = onceItem(container);
		expect(item?.dataset.scheduleState).toBe("armed");
		expect(item?.dataset.once).toBe("pending");
		expect(text(item)).toContain("Price update");
		expect(text(item)).toContain("Once on 2026-10-15 at 09:00 · Europe/Berlin");
		const [line, ...rest] = onceLines(container);
		expect(line).toMatch(/^Runs once Oct 15 at 09:00 GMT\+2 · in /);
		expect(rest).toEqual([]);
		const foot = text(block(container));
		expect(foot).toContain(
			"A one-time schedule runs once. If edge-berlin-01 isn't running at its time, or within 15 minutes after, it doesn't run at all.",
		);
		expect(foot).not.toContain("A run is skipped when the device is off");
	});

	test("a run that is going says so instead of the time", async () => {
		const { container } = await open({
			arrange: (fake) => {
				Object.assign(onceEntry(fake), {
					once_state: "started",
					running: true,
				});
			},
		});
		expect(onceItem(container)?.dataset.once).toBe("started");
		expect(onceLines(container)).toEqual(["A run is going now."]);
	});

	test("it ran: when and how, nothing more to run, and the device's own time in the head", async () => {
		const { container } = await open({ arrange: finished("ran", "succeeded") });
		const item = onceItem(container);
		expect(item?.dataset.scheduleState).toBe("finished");
		expect(item?.dataset.once).toBe("ran");
		// The device reports the instant it ran for; the head says it in the schedule's zone.
		expect(text(item)).toContain("Once on 2026-09-30 at 12:00 · Europe/Berlin");
		const [ran, done] = onceLines(container);
		expect(ran).toMatch(/^Ran 12:00 GMT\+2 · 2 hr\. ago · .* · succeeded$/);
		expect(done).toBe("Nothing more to run.");
	});

	test("a run that was cut off isn't started again", async () => {
		const { container } = await open({ arrange: finished("ran", "cancelled") });
		const [line, done] = onceLines(container);
		expect(line).toMatch(
			/^Was cut off 12:00 GMT\+2 · 2 hr\. ago.*; it isn't started again\.$/,
		);
		expect(done).toBe("Nothing more to run.");
	});

	test("missed: final, and why", async () => {
		const { container } = await open({ arrange: finished("missed") });
		expect(onceItem(container)?.dataset.once).toBe("missed");
		const [line, done] = onceLines(container);
		expect(line).toMatch(
			/^Missed: edge-berlin-01 wasn't running at its time \(12:00 GMT\+2 · 2 hr\. ago.*\)\.$/,
		);
		expect(done).toBe("Nothing more to run.");
	});

	test("its time had passed when the service could first run it: final, and why", async () => {
		const { container } = await open({ arrange: finished("passed") });
		expect(onceItem(container)?.dataset.once).toBe("passed");
		const [line, done] = onceLines(container);
		expect(line).toMatch(
			/^Its time \(12:00 GMT\+2 · 2 hr\. ago.*\) had passed when shop-assistant could first run it\.$/,
		);
		expect(done).toBe("Nothing more to run.");
	});

	test("a stopped service still reports one that finished", async () => {
		const { container } = await open({
			arrange: (fake) => {
				finished("ran", "succeeded")(fake);
				stopShop(fake);
			},
		});
		expect(onceItem(container)?.dataset.scheduleState).toBe("finished");
		const [ran, done] = onceLines(container);
		expect(ran).toMatch(/^Ran 12:00 GMT\+2 · 2 hr\. ago.* · succeeded$/);
		expect(done).toBe("Nothing more to run.");
	});

	test("one still to come does nothing while the service is stopped", async () => {
		const { container } = await open({
			arrange: (fake) => {
				stopShop(fake);
				shopRow(fake).schedules = undefined;
			},
		});
		expect(onceItem(container)?.dataset.scheduleState).toBe("stopped");
		expect(text(onceItem(container))).toContain(
			"Once on 2026-10-15 at 09:00 · Europe/Berlin",
		);
		expect(onceLines(container)).toEqual([
			"No runs while the service is not running.",
		]);
	});

	test("from the published status: its time from the schedule, without the device's live numbers", async () => {
		const { container } = await open({ reader: "status" });
		expect(onceItem(container)?.dataset.once).toBe("pending");
		expect(onceLines(container)[0]).toMatch(
			/^Runs once Oct 15 at 09:00 GMT\+2/,
		);
	});

	test("without this computer's keys nothing is said about it", async () => {
		const { container } = await open({ reader: "locked" });
		expect(text(container)).not.toContain("Runs once");
		const item = onceItem(container);
		if (item) {
			expect(item.dataset.scheduleState).toBe("locked");
			expect(onceLines(container)).toEqual(["Unknown until unlocked"]);
		}
	});

	test("nobody moved it here: the hub runs it, and Run it here moves it", async () => {
		const { container } = await open({ release: false });
		expect(onceItem(container)?.dataset.scheduleState).toBe("held");
		expect(onceLines(container)).toEqual([
			"Not running here: nobody who can edit this app's events has moved it to this service. The hub runs it.",
		]);
		expect(
			byRole("button", "Run it here", onceItem(container) as HTMLElement),
		).toBeTruthy();
	});

	test("a running service that has not said anything about it yet", async () => {
		const { container } = await open({
			arrange: (fake) => {
				shopRow(fake).schedules = undefined;
			},
		});
		expect(onceItem(container)?.dataset.scheduleState).toBe("not_reported");
		expect(onceLines(container)).toEqual(["Not reported yet."]);
	});

	test("an agent from before schedules can't say: the row asks for the agent update", async () => {
		const { container } = await open({
			agentFeatures: { placement_events: 1, placement_diagnostics: 1 },
		});
		expect(onceItem(container)?.dataset.scheduleState).toBe("needs_agent");
		expect(text(onceItem(container))).toContain(
			"Once on 2026-10-15 at 09:00 · Europe/Berlin",
		);
		expect(onceLines(container)).toEqual([
			"Update the device agent to see its schedules.",
		]);
	});
});
