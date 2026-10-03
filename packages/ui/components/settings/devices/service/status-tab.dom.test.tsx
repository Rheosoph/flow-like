import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	advance,
	byRole,
	click,
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
const { SAMPLE_IDS, SAMPLE_NOW } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { rejected } = await import("../testing/fake-device-api");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useActivityTray } = await import("../shell/activity-tray");
const { ServiceScreen } = await import("./service-screen");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useActivityTray.getState().setOpen(false);
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const SERVICE = "invoice-extractor";
/** Following polls the device once a second, so these tests outlast bun's 5 s default. */
const SLOW = 20_000;

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
type Agent = ReturnType<Fake["agent"]>;

/** The sample's invoice-extractor with its update in `state`; nothing about it is known on this computer yet. */
async function open(
	arrange: (agent: Agent) => void = () => undefined,
	options: FakeWorkspaceOptions = {},
	serviceId = SERVICE,
) {
	const fake = await createFakeWorkspace(undefined, {
		viewFacts: false,
		...options,
	});
	arrange(fake.agent(EDGE));
	await fake.workspace.live.refreshInspection(EDGE);
	const view = await mountDevices(<Page />, {
		fake,
		search: `device=${EDGE}&service=${serviceId}&tab=status`,
	});
	await view.settle();
	return view;
}

type View = Awaited<ReturnType<typeof open>>;

const text = (root: ParentNode = document.body) =>
	(root.textContent ?? "").replace(/\s+/g, " ");
const sent = (view: View) => view.fake.api.commands.map(([, type]) => type);
const count = (view: View, type: string) =>
	sent(view).filter((entry) => entry === type).length;

async function until(check: () => boolean, ms = 6_000) {
	const end = performance.now() + ms;
	while (!check() && performance.now() < end) await advance(50);
	expect(check()).toBe(true);
}

const rollout = (agent: Agent) => agent.rollouts[0];

/** The service as it runs before and after an update that didn't switch: settings v11, one instance ready. */
function runsPrevious(agent: Agent) {
	Object.assign(agent.placement(SERVICE) ?? {}, {
		observed_state: "running",
		config_revision: 11,
		applied_revision: 11,
		running_replicas: 1,
		ready_replicas: 1,
	});
}

const update = (view: View) =>
	view.container.querySelector<HTMLElement>("[data-rollout]");

describe("current update", () => {
	test("a running safe update: timeline, deadline, no discard once switching started, follow opens the tray", async () => {
		const view = await open();
		await until(() => update(view)?.dataset.rollout === "activating");
		const block = update(view) as HTMLElement;
		expect(text(block)).toContain("settings v11 → v12");
		expect(text(block)).toContain("Switching over");
		expect(text(block)).toContain("Time limit to start");
		expect(text(block)).toContain("previous 1 → new 1");
		const discard = byRole("button", "Discard update…", block);
		expect(discard.getAttribute("aria-disabled")).toBe("true");
		expect(text(block)).toContain("Can't discard once switching has started.");
		await click(discard);
		expect(count(view, "cancel_rollout")).toBe(0);
		await click(byRole("button", "Follow in activity", block));
		expect(useActivityTray.getState().open).toBe(true);
		expect(document.querySelectorAll("[data-dv-primary]").length).toBe(0);
	});

	test("a restored version is reported as rolled back, never as updated", async () => {
		const view = await open((agent) => {
			runsPrevious(agent);
			Object.assign(rollout(agent), {
				state: "rolled_back",
				failure_code: "candidate_failed",
				updated_at: SAMPLE_NOW - 60,
				deadline_at: null,
			});
		});
		await until(() => update(view)?.dataset.rollout === "rolled_back");
		const page = text(view.container);
		expect(page).toContain("Rolled back");
		expect(page).toContain("New version crashed after switching");
		expect(page).toContain("The previous version is running again.");
		expect(page).toContain("Runs settings v11 again.");
		expect(page).not.toContain("Updated at");
		expect(text(byRole("tab", /^Status/, view.container))).toContain(
			"The last update didn't apply",
		);
		const block = update(view) as HTMLElement;
		const logs = byRole("link", "View logs (errors only)", block);
		expect(logs.getAttribute("href")).toContain("tab=activity");
		expect(logs.getAttribute("href")).toContain("stream=errors");
		expect(byRole("link", "Update…", block).getAttribute("href")).toContain(
			"flow=deploy",
		);
		expect(
			view.container.querySelector("[data-history=rolled_back]"),
		).not.toBeNull();
	});

	test(
		"reopening reads the device's durable status and follows it without resubmitting anything",
		async () => {
			const view = await open((agent) => {
				agent.settling.add(rollout(agent).rollout_id);
			});
			await until(() => update(view) !== null);
			expect(count(view, "rollout_history")).toBeGreaterThanOrEqual(1);
			await until(() => update(view)?.dataset.rollout === "healthy");
			const page = text(view.container);
			expect(page).toContain("Updated");
			expect(page).toMatch(/v12 is running/);
			for (const write of ["apply", "stage_rollout", "activate_rollout"])
				expect(sent(view)).not.toContain(write);
			expect(view.fake.workspace.facts.get(EDGE)?.rollouts?.[0]?.state).toBe(
				"healthy",
			);
		},
		SLOW,
	);

	test(
		"a staged update from an earlier session can be discarded while the current version keeps running",
		async () => {
			const view = await open((agent) => {
				runsPrevious(agent);
				Object.assign(rollout(agent), {
					state: "staged",
					created_at: SAMPLE_NOW - 600,
					updated_at: SAMPLE_NOW - 600,
					deadline_at: null,
				});
			});
			await until(() => update(view)?.dataset.rollout === "staged");
			expect(text(view.container)).toContain(
				"Settings v12 is ready to switch over.",
			);
			expect(text(view.container)).toContain(
				"Settings v11 keeps running until you activate it.",
			);
			expect(
				text(
					view.container.querySelector("[data-service-actions]") as HTMLElement,
				),
			).toContain(
				"An update is staged. Activate or discard it under Status first. Only Stop is allowed.",
			);
			const block = update(view) as HTMLElement;
			expect(byRole("button", "Activate", block)).toBeTruthy();
			await click(byRole("button", "Discard update…", block));
			const sheet = inPortal();
			expect(text(sheet)).toContain(
				"The staged version is discarded. The current version keeps running.",
			);
			expect(count(view, "cancel_rollout")).toBe(0);
			await click(
				byRole("button", "Discard the update of invoice-extractor", sheet),
			);
			await until(() => count(view, "cancel_rollout") === 1);
			await until(() => update(view) === null);
			for (const write of ["stop", "start", "stage_rollout", "apply"])
				expect(sent(view)).not.toContain(write);
			expect(text(view.container)).toMatch(
				/Discard the update of invoice-extractor: done/,
			);
			expect(
				view.container.querySelector("[data-history=cancelled]"),
			).not.toBeNull();
			expect(text(view.container)).toContain("Runs as you asked: settings v11");
		},
		SLOW,
	);

	test("Cancel in the discard sheet sends nothing", async () => {
		const view = await open((agent) => {
			runsPrevious(agent);
			Object.assign(rollout(agent), { state: "staged", deadline_at: null });
		});
		await until(() => update(view)?.dataset.rollout === "staged");
		await click(
			byRole("button", "Discard update…", update(view) as HTMLElement),
		);
		await click(byRole("button", "Cancel", inPortal()));
		await view.settle();
		expect(count(view, "cancel_rollout")).toBe(0);
		expect(update(view)?.dataset.rollout).toBe("staged");
	});

	test(
		"Activate switches a staged update over and tracks it in the tray until it is healthy",
		async () => {
			const view = await open((agent) => {
				runsPrevious(agent);
				Object.assign(rollout(agent), { state: "staged", deadline_at: null });
			});
			await until(() => update(view)?.dataset.rollout === "staged");
			await click(byRole("button", "Activate", update(view) as HTMLElement));
			await until(() => count(view, "activate_rollout") === 1);
			await until(() => update(view)?.dataset.rollout === "healthy");
			expect(count(view, "activate_rollout")).toBe(1);
			const tracked = () =>
				view.fake.workspace.activity
					.list()
					.find((item) => item.label.params?.action === "activate_staged");
			expect(tracked()?.kind).toBe("safe_update");
			expect(tracked()?.target.serviceId).toBe(SERVICE);
			await until(() => tracked()?.state === "done");
			expect(text(view.container)).toContain(
				"Runs as you asked: settings v12, 1 of 1 instance ready.",
			);
		},
		SLOW,
	);

	for (const terminal of ["rolled_back", "healthy"] as const)
		test(
			`following survives a failed status read and ends ${terminal}`,
			async () => {
				let reads = 0;
				const view = await open((agent) => {
					agent.handle("rollout", () => {
						reads += 1;
						const row = rollout(agent);
						if (reads === 1) return rejected("busy", "Device busy.", true);
						if (reads >= 3)
							Object.assign(row, {
								state: terminal,
								updated_at: SAMPLE_NOW,
								...(terminal === "rolled_back"
									? { failure_code: "activation_timeout" }
									: {}),
							});
						return { state: "completed", result: { ...row } };
					});
				});
				await until(() => reads >= 1);
				await until(
					() => update(view)?.querySelector("[data-age=error]") != null,
				);
				expect(update(view)?.dataset.rollout).toBe("activating");
				await until(() => update(view)?.dataset.rollout === terminal, 8_000);
				expect(view.container.querySelector("[role=alert]")).toBeNull();
				expect(text(view.container)).toContain(
					terminal === "healthy"
						? "Updated"
						: "New version didn't become healthy in time",
				);
				expect(count(view, "stage_rollout")).toBe(0);
				expect(count(view, "activate_rollout")).toBe(0);
			},
			SLOW,
		);
});

describe("update history and instances", () => {
	test("finished updates are listed newest first with their result", async () => {
		const view = await open((agent) => {
			runsPrevious(agent);
			Object.assign(rollout(agent), {
				state: "healthy",
				updated_at: SAMPLE_NOW - 3 * 86_400,
			});
			agent.rollouts.push({
				...rollout(agent),
				rollout_id: "0b0a1f7e-55a1-4a6c-9d0e-2f1f3f5a7b9c",
				state: "rolled_back",
				failure_code: "candidate_failed",
				created_at: SAMPLE_NOW - 7 * 86_400,
				updated_at: SAMPLE_NOW - 7 * 86_400 + 180,
			});
		});
		await until(
			() => view.container.querySelectorAll("[data-history]").length === 2,
		);
		const rows = [...view.container.querySelectorAll("[data-history]")].map(
			(row) => row.getAttribute("data-history"),
		);
		expect(rows).toEqual(["healthy", "rolled_back"]);
		expect(text(view.container)).toContain(
			"The device keeps the last 32 finished updates.",
		);
		expect(update(view)).toBeNull();
		expect(text(view.container)).toContain("Requested vs actual");
	});

	test("older agent: only the latest known update, read without the newer command", async () => {
		const view = await open(
			(agent) => {
				runsPrevious(agent);
				Object.assign(rollout(agent), {
					state: "rolled_back",
					failure_code: "candidate_failed",
					updated_at: SAMPLE_NOW - 120,
					deadline_at: null,
				});
			},
			{ agentFeatures: {} },
		);
		await until(() => view.container.querySelector("[data-history]") !== null);
		expect(text(view.container)).toContain(
			"Only the latest finished update is shown.",
		);
		expect(sent(view)).not.toContain("rollout_history");
		expect(sent(view)).toContain("rollout");
		expect(view.container.querySelector("[role=alert]")).toBeNull();
	});

	test("instances: one row per instance with state, settings and readiness; last error from the agent", async () => {
		const view = await open(
			(agent) => {
				Object.assign(agent.placement("support-bot") ?? {}, {
					replicas: [
						{
							slot: 0,
							observed_state: "running",
							applied_revision: 7,
							last_error: null,
							has_error: false,
						},
						{
							slot: 1,
							observed_state: "backoff",
							applied_revision: 7,
							last_error: "listener exited with code 3",
							has_error: true,
							restarts: {
								failures: 2,
								max_restarts: 5,
								crash_looping: false,
								retry_in_seconds: 4,
								last_started_at: SAMPLE_NOW - 30,
							},
						},
					],
					ready_replicas: 1,
				});
			},
			{},
			"support-bot",
		);
		await until(
			() => view.container.querySelectorAll("[data-observed]").length === 2,
		);
		const table = byRole("table", "Instances of support-bot", view.container);
		const cells = text(table);
		expect(cells).toContain("#0");
		expect(cells).toContain("#1");
		expect(cells).toContain("Restarting after a crash");
		expect(cells).toContain("listener exited with code 3");
		expect(cells).toContain("Restarted 2 times of 5 allowed");
		expect(table.getAttribute("data-stack")).toBe("900");
		expect(queryByRole("columnheader", "Last error", table)).not.toBeNull();
	});

	test("a stopped service says why no instance runs instead of an empty table", async () => {
		const view = await open(undefined, {}, "nightly-sync");
		expect(text(view.container)).toContain("No instances are running");
		expect(text(view.container)).toContain("It's stopped, as you asked.");
		expect(queryByRole("table", /Instances of/, view.container)).toBeNull();
	});

	test("secret writes sent from this computer are listed with their state", async () => {
		const view = await open();
		expect(text(view.container)).toContain("None pending.");
		const { act } = await import("react");
		await act(async () => {
			view.fake.workspace.activity.start({
				kind: "secret_write",
				target: { deviceId: EDGE, serviceId: SERVICE },
				state: "waiting",
				label: { code: "secret_write" },
				startedBy: "you",
				actions: [],
				resume: {
					type: "secret",
					operationId: "5a1f2c3d-7e8b-4c90-a1b2-c3d4e5f60718",
					placementId: SERVICE,
					name: "OPENAI_API_KEY",
				},
			});
		});
		await view.settle();
		const row = view.container.querySelector("[data-secret]") as HTMLElement;
		expect(text(row)).toContain("OPENAI_API_KEY");
		expect(text(row)).toContain("Waiting for device");
	});

	test(
		"when the hub can't refresh the cloud leases, the rows stay and the Hub stamp says so",
		async () => {
			const view = await open();
			const instances = () =>
				byRole(
					"table",
					"Instances of invoice-extractor",
					view.container,
				).closest("[data-block]") as HTMLElement;
			await until(() => text(instances()).includes("until"));
			const evidence = byRole("list", "Evidence", view.container);
			expect(text(evidence)).toContain("Cloud access active until");
			expect(
				evidence
					.querySelector("[data-stamp][data-src=hub]")
					?.getAttribute("data-age"),
			).not.toBe("error");
			view.fake.api.fail({ method: "GET", path: /resource-grants/ });
			const { act } = await import("react");
			await act(async () => {
				await view.fake.queryClient.refetchQueries({
					predicate: (query) => query.queryKey[2] === "resources",
				});
			});
			await until(
				() =>
					instances().querySelector("[data-src=hub][data-age=error]") !== null,
				8_000,
			);
			expect(text(instances())).toContain("until");
			expect(
				byRole("list", "Evidence", view.container).querySelector(
					"[data-stamp][data-src=hub][data-age=error]",
				),
			).not.toBeNull();
			expect(text(view.container)).toContain("Cloud access active until");
			expect(view.container.querySelector("[role=alert]")).toBeNull();
		},
		SLOW,
	);
});
