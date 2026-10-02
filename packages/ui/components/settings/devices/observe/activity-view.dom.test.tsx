import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	advance,
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { FakeWorkspaceOptions } from "../testing/fake-workspace";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
// Device-lib modules load after the DOM exists (a static import breaks combined runs, W2-FAKES).
const { SAMPLE_IDS, SAMPLE_NOW, SAMPLE_PEOPLE, sampleFleet } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { FAKE_PASSWORD, readFakeCompact } = await import(
	"../testing/fake-device-api"
);
const { useOverlayStore } = await import("../workspace/overlay-store");
const { ACCOUNT_SCOPE } = await import("../routing/devices-route");
const { READER_REQUEST_KIND } = await import("./reader-request");
const { ActivityView } = await import("./activity-view");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const WAREHOUSE = SAMPLE_IDS.warehouse;
const LAB = SAMPLE_IDS.lab;
const SERVICE = "invoice-extractor";
const COMMAND = "a0e5259e-a9bf-4eef-99df-f9666dffbab4";
const SLOW = 20_000;

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;
type Agent = ReturnType<Fake["agent"]>;

interface OpenOptions extends FakeWorkspaceOptions {
	serviceId?: string;
	errorsFirst?: boolean;
	/** Also unlock this device and connect live (a device the seed leaves locked). */
	connect?: string;
	arrange?(agent: Agent, fake: Fake): void;
	people?: Record<string, string>;
}

async function open(deviceId: string, options: OpenOptions = {}) {
	const { serviceId, errorsFirst, connect, arrange, people, ...rest } = options;
	const fake = await createFakeWorkspace(undefined, rest);
	if (connect) await fake.unlock(connect, { connectLive: true });
	arrange?.(fake.agent(deviceId), fake);
	const view = await mountDevices(
		<ActivityView
			deviceId={deviceId}
			serviceId={serviceId}
			scope={ACCOUNT_SCOPE}
			errorsFirst={errorsFirst}
		/>,
		{
			fake,
			search: `device=${deviceId}&tab=activity`,
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
const count = (view: View, type: string) =>
	sent(view).filter((entry) => entry === type).length;
const block = (view: View, id: string) =>
	view.container.querySelector<HTMLElement>(`#${id}`) as HTMLElement;
const primaries = () => document.querySelectorAll("[data-dv-primary]").length;

async function until(check: () => boolean, ms = 6_000) {
	const end = performance.now() + ms;
	while (!check() && performance.now() < end) await advance(50);
	expect(check()).toBe(true);
}

/** R3: wire values and codes never reach the screen. */
const MACHINE =
	/placement|replica|backoff|\bstdout\b|\bstderr\b|roster|outbox|evicted_through|config_revision|source_id|rolled_back|unauthorized|cgroup|archive_policy|telemetry/;

const message = (data: Record<string, unknown>) => ({ version: 1, ...data });

/** A command with two states, an instance change, and two records the timeline must leave out. */
const ACTIVITY: Record<string, unknown>[] = [
	{
		kind: "operation",
		source_id: COMMAND,
		placement_id: SERVICE,
		state: "accepted",
		secret: "do-not-render-this",
	},
	{
		kind: "replica",
		source_id: SERVICE,
		placement_id: SERVICE,
		state: "starting",
		config_revision: 12,
		replica_slot: 0,
		process_id: 48211,
	},
	{
		kind: "operation",
		source_id: COMMAND,
		placement_id: SERVICE,
		state: "completed",
	},
	{ kind: "arbitrary", state: "completed", source_id: "unknown-kind" },
	{
		kind: "replica",
		placement_id: SERVICE,
		state: "<script>unknown-state</script>",
	},
];

function activity(agent: Agent) {
	for (const data of ACTIVITY)
		agent.record("messages:device", message(data), "message");
}

function logs(agent: Agent, key = "logs:device") {
	agent.record(
		key,
		{ stream: "stdout", message: "check-in accepted", truncated: false },
		"log",
	);
	agent.record(
		key,
		{
			stream: "stderr",
			message: "certificate renewal failed",
			truncated: true,
		},
		"log",
	);
	agent.record(
		key,
		{
			stream: "stdout",
			message: "[flow-like] 37 log lines dropped by the capture rate limit",
			truncated: false,
			dropped_lines: 37,
		},
		"log",
	);
}

const encode = (value: unknown) =>
	btoa(JSON.stringify(value))
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replaceAll("=", "");

function archives(fake: Fake) {
	const lines = [
		{
			sequence: 1,
			timestamp: SAMPLE_NOW - 500,
			kind: "log",
			data: { stream: "stdout", message: "published encrypted status #5509" },
		},
		{
			sequence: 2,
			timestamp: SAMPLE_NOW - 480,
			kind: "log",
			data: { stream: "stderr", message: "renewal failed" },
		},
	];
	fake.hub.archives.set(EDGE, [
		{
			archive_id: "3a6f2c10-9d1b-4f7e-8a55-0c2d6b7e9f01",
			sequence: 5508,
			scope: "device",
			kind: "logs",
			created_at: SAMPLE_NOW - 600,
			expires_at: SAMPLE_NOW + 7 * 86_400,
			manifest_jws: "m",
			roster_jws: "r",
			ciphertext: encode({
				records: lines,
				next: 3,
				gap: { after: 0, dropped: 3 },
			}),
			recipient_keys: [],
		},
		{
			archive_id: "7be1d0aa-41c9-4d12-b6f3-5e8a9c0d1f02",
			sequence: 5509,
			scope: "device",
			kind: "logs",
			created_at: SAMPLE_NOW - 300,
			expires_at: SAMPLE_NOW + 7 * 86_400,
			manifest_jws: "m",
			roster_jws: "r",
			ciphertext: encode({ records: [], next: 3 }),
			recipient_keys: [],
		},
	]);
}

/** The device stores a signed readers list and answers like the agent. */
function acceptRosters(agent: Agent, fake: Fake) {
	agent.handle("archive_policy", (command) => {
		const roster = readFakeCompact<{ scope: string; kind: "logs" | "metrics" }>(
			String(command.policy_jws),
			fake.hub.ownerInvitationKey(agent.deviceId),
		);
		agent.setRoster(roster.kind, roster.scope, String(command.policy_jws), {
			status: { state: "recording", reason: null, since: fake.hub.now() },
		});
		return { state: "completed", result: {} };
	});
}

describe("timeline", () => {
	test(
		"live: instance changes and commands, newest first, typed fields only; the filter keeps markers",
		async () => {
			const view = await open(EDGE, { arrange: activity });
			const timeline = block(view, "observe-timeline");
			await until(() => /Instance #0/.test(text(timeline)));
			const shown = text(timeline);
			expect(shown).toContain(
				"Instance #0 of invoice-extractor is starting with settings v12.",
			);
			expect(shown).toContain("process 48211");
			expect(shown).toContain("A command for invoice-extractor finished.");
			// One entry per command: the newest state wins.
			expect(shown).not.toContain("The device recorded a command");
			expect(shown).not.toContain("do-not-render-this");
			expect(shown).not.toContain("unknown-kind");
			expect(shown).not.toContain("unknown-state");
			expect(shown).not.toMatch(MACHINE);
			expect(timeline.querySelector("[data-stamp]")?.textContent).toContain(
				"following",
			);
			expect(
				timeline.querySelector("[data-timeline] a")?.getAttribute("href"),
			).toContain(`service=${SERVICE}`);
			expect(primaries()).toBe(0);

			await click(byRole("button", "Instances", timeline));
			expect(text(timeline)).toContain("Instance #0");
			expect(text(timeline)).not.toContain("A command for");
			await click(byRole("button", "Commands", timeline));
			expect(text(timeline)).toContain("A command for");
			expect(text(timeline)).not.toContain("Instance #0");
		},
		SLOW,
	);

	test(
		"loss markers: changes lost before storing and entries the device deleted",
		async () => {
			const view = await open(EDGE, {
				arrange: (agent) => {
					activity(agent);
					agent.handle("messages", (command, context) => {
						const after = typeof command.after === "number" ? command.after : 0;
						const rows = (context.agent.records.get("messages:device") ?? [])
							.filter((row) => row.sequence > after)
							.slice(0, 100);
						return {
							state: "completed",
							result: {
								records: rows,
								next: rows.at(-1)?.sequence ?? after,
								retention_limit: 2000,
								outbox_dropped: 3,
								...(after === 0 ? { evicted_through: 18_399 } : {}),
							},
						};
					});
				},
			});
			const timeline = block(view, "observe-timeline");
			await until(() => /Older entries/.test(text(timeline)));
			expect(text(timeline)).toContain(
				"3 changes were lost before the device could store them.",
			);
			expect(text(timeline)).toContain(
				"Older entries were deleted to save space (before #18,400). The device keeps the last 2,000 changes.",
			);
			// Markers never hide behind a filter.
			await click(byRole("button", "Access", timeline));
			expect(text(timeline)).toContain("Older entries were deleted");
		},
		SLOW,
	);

	test(
		"who did what (owner, newer agent): names, the person filter, and commands sent from here read by their name",
		async () => {
			const view = await open(EDGE, {
				people: { [SAMPLE_PEOPLE.mira]: "Mira Novak" },
				arrange: (agent, fake) => {
					activity(agent);
					agent.record(
						"messages:device",
						message({
							kind: "operation",
							source_id: "op-mine",
							placement_id: "support-bot",
							state: "completed",
						}),
						"message",
					);
					agent.handle("operations", () => ({
						state: "completed",
						result: {
							operations: [
								{
									operation_id: COMMAND,
									kind: null,
									actor: {
										role: "grant",
										user_id: SAMPLE_PEOPLE.mira,
										grant_id: "g",
									},
									project_id: null,
									placement_id: SERVICE,
									accepted_at: SAMPLE_NOW,
									state: "completed",
								},
							],
							next: null,
						},
					}));
					fake.workspace.activity.start({
						kind: "command",
						target: { deviceId: EDGE, serviceId: "support-bot" },
						state: "done",
						label: { code: "command", params: { command: "stop" } },
						startedBy: "you",
						actions: [],
						resume: {
							type: "operation",
							operationId: "op-mine",
							command: "stop",
							issuedAt: SAMPLE_NOW,
						},
					});
				},
			});
			const timeline = block(view, "observe-timeline");
			await until(() => /by Mira Novak/.test(text(timeline)));
			expect(text(timeline)).toContain("Stop support-bot: done.");
			expect(text(timeline)).toContain("by you");
			expect(count(view, "operations")).toBeGreaterThan(0);
			expect(queryByRole("combobox", "Who sent it", timeline)).not.toBeNull();
			expect(text(timeline)).not.toMatch(MACHINE);
		},
		SLOW,
	);

	test(
		"older agent: the timeline reads without asking who did what",
		async () => {
			const view = await open(EDGE, { arrange: activity, agentFeatures: {} });
			const timeline = block(view, "observe-timeline");
			await until(() => /Instance #0/.test(text(timeline)));
			expect(queryByRole("combobox", "Who sent it", timeline)).toBeNull();
			expect(text(timeline)).not.toContain("by Mira");
			expect(sent(view)).not.toContain("operations");
			expect(sent(view)).not.toContain("metrics_history");
			expect(document.querySelector("[data-kind=error]")).toBeNull();
			expect(document.querySelector("[role=alert]")).toBeNull();
		},
		SLOW,
	);

	test(
		"a service sees only its own entries",
		async () => {
			const view = await open(EDGE, {
				serviceId: SERVICE,
				arrange: (agent) => {
					for (const row of [
						message({
							kind: "replica",
							placement_id: SERVICE,
							state: "running",
							replica_slot: 0,
						}),
					])
						agent.record(`messages:${SERVICE}`, row, "message");
				},
			});
			const timeline = block(view, "observe-timeline");
			await until(() => /is running/.test(text(timeline)));
			expect(text(timeline)).toContain("Only entries for invoice-extractor.");
			expect(text(timeline)).not.toContain("edge-berlin-01 started");
			expect(text(timeline)).not.toContain("Access rules");
			const read = view.fake.api.commands.find(
				([, type]) => type === "messages",
			);
			expect(read?.[2].placement_id).toBe(SERVICE);
		},
		SLOW,
	);
});

describe("without a live read", () => {
	test("locked: every block says locked, nothing is empty, Unlock opens the sheet and nothing is sent", async () => {
		const view = await open(EDGE, { unlock: "none" });
		const before = view.fake.api.commands.length;
		for (const id of ["observe-timeline", "observe-logs"])
			expect(
				block(view, id).querySelector("[data-kind=locked]"),
			).not.toBeNull();
		expect(text()).toContain(
			"Activity is cleared when you lock. Unlock to read it again.",
		);
		expect(text()).toContain(
			"Logs are cleared when you lock. Unlock to read them again.",
		);
		expect(document.querySelector("[data-kind=empty]")).toBeNull();
		const lookup = text(block(view, "observe-lookup"));
		expect(lookup).toContain("Unlock edge-berlin-01 to look up a command.");
		expect(lookup).not.toContain("to change settings");
		await click(byRole("button", "Unlock…", block(view, "observe-logs")));
		expect(useOverlayStore.getState().overlay).toMatchObject({
			kind: "unlock",
			deviceId: EDGE,
		});
		expect(view.fake.api.commands.length).toBe(before);
		expect(primaries()).toBeLessThanOrEqual(1);
	});

	test("unlocked and offline: needs a live connection, says why, and no connection is opened", async () => {
		const view = await open(WAREHOUSE);
		const logsBlock = block(view, "observe-logs");
		expect(text(logsBlock)).toContain("Needs a live connection");
		expect(text(logsBlock)).toContain("warehouse-pi is offline.");
		expect(queryByRole("button", "Connect live", logsBlock)).toBeNull();
		expect(sent(view)).not.toContain("logs");
		expect(sent(view)).not.toContain("messages");
		expect(view.fake.agent(WAREHOUSE).sessions).toBe(0);
	});
});

describe("logs", () => {
	test(
		"agent logs follow live with stream labels and markers; Pause counts what arrives",
		async () => {
			const view = await open(EDGE, { arrange: (agent) => logs(agent) });
			const logsBlock = block(view, "observe-logs");
			await until(() => /check-in accepted/.test(text(logsBlock)));
			const shown = text(logsBlock);
			expect(shown).toContain("Agent logs");
			expect(shown).toContain("Output");
			expect(shown).toContain("Errors");
			expect(shown).toContain("line cut at 2 048 bytes");
			expect(shown).toContain("37 lines dropped here");
			expect(shown).toContain("Agent logs only.");
			expect(shown).not.toMatch(MACHINE);
			expect(logsBlock.querySelector("[data-stamp]")?.textContent).toContain(
				"following",
			);

			await click(byRole("button", "Pause", logsBlock));
			const agent = view.fake.agent(EDGE);
			for (const line of ["late one", "late two"])
				agent.record(
					"logs:device",
					{ stream: "stdout", message: line, truncated: false },
					"log",
				);
			await until(() => /2 behind/.test(text(logsBlock)), 12_000);
			expect(text(logsBlock)).not.toContain("late one");
			await click(byRole("button", "Follow", logsBlock));
			await until(() => /late two/.test(text(logsBlock)));
		},
		SLOW,
	);

	test(
		"stream=errors: Errors only is preselected and the block scrolls into view",
		async () => {
			const scrolled: Element[] = [];
			const proto = window.HTMLElement.prototype as unknown as {
				scrollIntoView?: () => void;
			};
			const original = proto.scrollIntoView;
			proto.scrollIntoView = function (this: Element) {
				scrolled.push(this);
			};
			try {
				const view = await open(EDGE, {
					serviceId: SERVICE,
					errorsFirst: true,
					arrange: (agent) => logs(agent, `logs:${SERVICE}`),
				});
				const logsBlock = block(view, "observe-logs");
				await until(() => /certificate renewal failed/.test(text(logsBlock)));
				expect(
					logsBlock
						.querySelector("[data-log-viewer]")
						?.getAttribute("data-errors"),
				).toBe("true");
				expect(text(logsBlock)).not.toContain("check-in accepted");
				expect(text(logsBlock)).toContain("37 lines dropped here");
				expect(text(logsBlock)).toContain("Logs of invoice-extractor");
				expect(text(logsBlock)).toContain("Agent logs are on");
				expect(text(logsBlock)).not.toMatch(MACHINE);
				expect(scrolled.some((el) => el.contains(logsBlock))).toBe(true);
			} finally {
				proto.scrollIntoView = original;
			}
		},
		SLOW,
	);

	test(
		"Download saves the lines on screen and says so next to the viewer",
		async () => {
			const urls = URL as unknown as {
				createObjectURL?: (blob: Blob) => string;
				revokeObjectURL?: (url: string) => void;
			};
			const original = [urls.createObjectURL, urls.revokeObjectURL] as const;
			const blobs: Blob[] = [];
			urls.createObjectURL = (blob) => {
				blobs.push(blob);
				return "blob:logs";
			};
			urls.revokeObjectURL = () => undefined;
			try {
				const view = await open(EDGE, { arrange: (agent) => logs(agent) });
				const logsBlock = block(view, "observe-logs");
				await until(() => /check-in accepted/.test(text(logsBlock)));
				await click(byRole("button", "Download", logsBlock));
				expect(text(logsBlock)).toContain(
					"Saved 2 lines as edge-berlin-01-agent-2026-09-30.log.",
				);
				expect(await blobs[0]?.text()).toContain(
					"ERR certificate renewal failed",
				);
			} finally {
				[urls.createObjectURL, urls.revokeObjectURL] = original;
			}
		},
		SLOW,
	);

	test(
		"shared for one app only: agent logs say no access and aren't requested",
		async () => {
			const view = await open(LAB, { connect: LAB });
			const logsBlock = block(view, "observe-logs");
			expect(text(logsBlock)).toContain("No access to logs.");
			expect(logsBlock.querySelector("[data-kind=empty]")).toBeNull();
			const agentLogs = view.fake.api.commands.filter(
				([device, type, command]) =>
					device === LAB && type === "logs" && command.placement_id === null,
			);
			expect(agentLogs).toEqual([]);
			// History access instead of History settings for a person who isn't the owner.
			expect(block(view, "observe-history-settings")).toBeNull();
			const access = block(view, "observe-history-access");
			expect(text(access)).toContain(
				"You aren't a reader of this device's retained history.",
			);
		},
		SLOW,
	);
});

describe("look up a command", () => {
	const unconfirmed = (fake: Fake) =>
		fake.workspace.activity.start({
			kind: "command",
			target: { deviceId: EDGE, serviceId: SERVICE },
			state: "unknown",
			label: { code: "command", params: { command: "restart" } },
			startedBy: "you",
			actions: [],
			resume: {
				type: "operation",
				operationId: COMMAND,
				command: "restart",
				issuedAt: SAMPLE_NOW - 60,
			},
		});

	test(
		"prefilled from the command without a seen result; the answer shows state, command and target",
		async () => {
			const view = await open(EDGE, {
				arrange: (agent, fake) => {
					unconfirmed(fake);
					agent.journal.set(COMMAND, {
						state: "completed",
						result: { placement_id: SERVICE, config_revision: 12 },
						type: "restart",
						at: SAMPLE_NOW - 60,
					});
				},
			});
			const lookup = block(view, "observe-lookup");
			const input = byRole("textbox", "Command ID", lookup) as HTMLInputElement;
			expect(input.value).toBe(COMMAND);
			expect(text(lookup)).toContain(
				"Prefilled from the last command you sent whose result you haven't seen.",
			);
			const before = count(view, "operation");
			await click(byRole("button", "Look up", lookup));
			await until(() => /Done/.test(text(lookup)));
			expect(count(view, "operation")).toBe(before + 1);
			expect(text(lookup)).toContain("Restart");
			expect(text(lookup)).toContain(SERVICE);
			expect(text(lookup)).toContain("v12");
			const stamp = lookup.querySelector("[data-stamp]")?.textContent ?? "";
			expect(stamp).toContain("on demand");
			expect(stamp).not.toContain("every");
			expect(text(lookup)).toContain(
				"Results are kept for 24 hours and only for the person who sent the command.",
			);
			expect(text(lookup)).not.toMatch(MACHINE);
		},
		SLOW,
	);

	test(
		"an ID the device doesn't know says so instead of failing",
		async () => {
			const view = await open(EDGE);
			const lookup = block(view, "observe-lookup");
			await typeInto(byRole("textbox", "Command ID", lookup), "not-a-known-id");
			await click(byRole("button", "Look up", lookup));
			await until(() => /has no result for this command/.test(text(lookup)));
			expect(lookup.querySelector("[data-kind=error]")).toBeNull();
		},
		SLOW,
	);

	test("without a live connection Look up stays visible, says why, and sends nothing", async () => {
		const view = await open(WAREHOUSE, {
			arrange: (_agent, fake) =>
				fake.workspace.activity.start({
					kind: "command",
					target: { deviceId: WAREHOUSE },
					state: "unknown",
					label: { code: "command", params: { command: "stop" } },
					startedBy: "you",
					actions: [],
					resume: {
						type: "operation",
						operationId: COMMAND,
						command: "stop",
						issuedAt: SAMPLE_NOW - 60,
					},
				}),
		});
		const lookup = block(view, "observe-lookup");
		const button = byRole("button", "Look up", lookup);
		expect(button.getAttribute("aria-disabled")).toBe("true");
		expect(lookup.querySelector("[data-gate-inline]")).not.toBeNull();
		const commands = view.fake.api.commands.length;
		const calls = view.fake.api.calls.length;
		await click(button);
		expect(view.fake.api.commands.length).toBe(commands);
		expect(view.fake.api.calls.length).toBe(calls);
	});
});

describe("retained history", () => {
	test(
		"the hub's chunks are listed newest first; Read opens one with this computer's key",
		async () => {
			const view = await open(EDGE, {
				arrange: (_agent, fake) => archives(fake),
			});
			const retained = block(view, "observe-retained-logs");
			await until(() => retained.querySelectorAll("[data-chunk]").length === 2);
			expect(text(retained)).toContain("stored on the hub");
			expect(
				retained.querySelector("[data-stamp]")?.getAttribute("data-src"),
			).toBe("hub");
			expect(text(retained.querySelector("[data-plan-line]") as Element)).toBe(
				"PRO plan · kept 7 days · up to 256.0 MiB · 87.0 MiB used",
			);
			const older = retained.querySelectorAll<HTMLElement>("[data-chunk]")[1];
			await click(byRole("button", "Read", older));
			await until(() => /published encrypted status/.test(text(retained)));
			expect(text(retained)).toContain(
				"3 records before this chunk were dropped on the device.",
			);
			expect(text(retained)).toContain("renewal failed");
			expect(byRole("button", "Hide", older)).not.toBeNull();
			expect(text(retained)).not.toMatch(MACHINE);
		},
		SLOW,
	);

	test(
		"older hub: the plan line states what is known, without an error, asking once",
		async () => {
			const view = await open(EDGE, {
				hubVersion: "old",
				arrange: (_agent, fake) => archives(fake),
			});
			const retained = block(view, "observe-retained-logs");
			await until(() => retained.querySelectorAll("[data-chunk]").length === 2);
			expect(text(retained.querySelector("[data-plan-line]") as Element)).toBe(
				"Depending on your plan, the hub keeps history up to 7 days and up to 256.0 MiB. This hub doesn't report how much you use.",
			);
			expect(document.querySelector("[data-kind=error]")).toBeNull();
			expect(document.querySelector("[role=alert]")).toBeNull();
			expect(
				view.fake.api.sent("GET", "devices/archive-usage").length,
			).toBeLessThanOrEqual(1);
		},
		SLOW,
	);

	test("no keys on this computer: says so and leads to the keys, instead of asking to unlock", async () => {
		const seed = sampleFleet();
		seed.local.vaults = seed.local.vaults.filter(
			(vault) => vault.deviceId !== EDGE,
		);
		seed.keys = seed.keys.filter((session) => session.deviceId !== EDGE);
		delete seed.local.backups[EDGE];
		const fake = await createFakeWorkspace(seed);
		const view = await mountDevices(
			<ActivityView deviceId={EDGE} scope={ACCOUNT_SCOPE} />,
			{ fake, search: `device=${EDGE}&tab=activity`, overlays: true },
		);
		await view.settle();
		const retained = block(view, "observe-retained-logs");
		expect(text(retained)).toContain(
			"This computer has no keys for edge-berlin-01.",
		);
		expect(retained.querySelector("[data-kind=locked]")).toBeNull();
		expect(text(retained)).not.toContain("Unlock edge-berlin-01");
		expect(
			byRole("link", "Restore keys…", retained).getAttribute("href"),
		).toContain("view=keys");
		expect(view.fake.api.sent("GET", /archives/)).toEqual([]);
	});
});

describe("history settings (owner)", () => {
	test(
		"one row per readers list with its recording state; services without one read Not set up",
		async () => {
			const view = await open(EDGE);
			const settings = block(view, "observe-history-settings");
			await until(
				() => settings.querySelectorAll("[data-history-row]").length > 0,
			);
			const row = (key: string) =>
				text(settings.querySelector(`[data-history-row="${key}"]`) as Element);
			expect(row("device|logs")).toContain("Whole device");
			expect(row("device|logs")).toContain("Recording");
			expect(row("device|logs")).toContain("Change readers…");
			expect(row("device|metrics")).toContain("Paused: readers list expired");
			expect(row("device|metrics")).toContain("none: the list expired");
			expect(row("device|metrics")).toContain("Resume recording…");
			expect(row(SERVICE)).toContain("Logs and metrics");
			expect(row(SERVICE)).toContain("Not set up");
			expect(row(SERVICE)).toContain("Set up…");
			expect(
				settings.querySelector("[data-stamp]")?.getAttribute("data-src"),
			).toBe("live");
			expect(text(settings)).not.toMatch(MACHINE);
			// The readers lists are old commands; a current agent adds only the state.
			expect(sent(view)).toContain("archive_roster_read");
			expect(block(view, "observe-history-access")).toBeNull();
		},
		SLOW,
	);

	test(
		"the device's own word for a pause wins; an older agent's state follows from expiry and the rules it was signed for",
		async () => {
			const newer = await open(EDGE, {
				arrange: (agent) => {
					const roster = agent.rosters.get("logs:device");
					if (roster)
						agent.setRoster("logs", "device", roster.text, {
							status: {
								state: "paused",
								reason: "quota_reached",
								since: SAMPLE_NOW - 3_600,
							},
						});
				},
			});
			const settings = block(newer, "observe-history-settings");
			await until(() => /Paused: history storage is full/.test(text(settings)));
			await cleanupDevices();

			const older = await open(EDGE, { agentFeatures: {} });
			const interim = block(older, "observe-history-settings");
			await until(
				() => interim.querySelectorAll("[data-history-row]").length > 0,
			);
			expect(
				text(
					interim.querySelector(
						'[data-history-row="device|metrics"]',
					) as Element,
				),
			).toContain("Paused: readers list expired");
			expect(
				text(
					interim.querySelector('[data-history-row="device|logs"]') as Element,
				),
			).toContain("Paused: access changed");
			expect(document.querySelector("[data-kind=error]")).toBeNull();
		},
		SLOW,
	);

	test(
		"expired access rules (BG31): one notice says why and offers Renew access rules; no list can be signed",
		async () => {
			const view = await open(EDGE, {
				arrange: (_agent, fake) => {
					const rules = fake.seed.policies[EDGE]?.policy;
					if (!rules) throw new Error("The seed has access rules for edge.");
					fake.hub.setPolicy(EDGE, {
						...rules,
						expires_at: SAMPLE_NOW - 3_600,
					});
				},
			});
			const settings = block(view, "observe-history-settings");
			await until(() =>
				/Access rules on edge-berlin-01 expired/.test(text(settings)),
			);
			expect(settings.querySelectorAll("[data-gate=policy]").length).toBe(1);
			const renew = byRole("link", "Renew access rules", settings);
			expect(renew.getAttribute("href")).toContain("tab=access");
			// The reason is said once; every row's action is disabled and points to it.
			const actions = [
				...settings.querySelectorAll<HTMLElement>("[data-history-row] button"),
			];
			expect(actions.length).toBeGreaterThan(1);
			for (const action of actions) {
				expect(action.getAttribute("aria-disabled")).toBe("true");
				const reason = document.getElementById(
					action.getAttribute("aria-describedby") ?? "",
				);
				expect(text(reason as Element)).toContain("Access rules on");
			}
			expect(settings.querySelector("[data-gate-inline]")).toBeNull();
			const change = byRole("button", "Change readers…", settings);
			const before = view.fake.api.commands.length;
			await click(change);
			expect(queryByRole("dialog")).toBeNull();
			expect(view.fake.api.commands.length).toBe(before);
			expect(primaries()).toBeLessThanOrEqual(1);
		},
		SLOW,
	);

	test(
		"rules not applied by the device yet: the reason is said once above the table and no row action opens the sheet",
		async () => {
			const view = await open(EDGE, {
				arrange: (_agent, fake) => {
					const policy = fake.hub.policies.get(EDGE);
					if (!policy) throw new Error("The seed has access rules for edge.");
					policy.appliedVersion = policy.version - 1;
					policy.appliedDigest = "digest-before";
				},
			});
			const settings = block(view, "observe-history-settings");
			await until(
				() => settings.querySelectorAll("[data-history-row]").length > 0,
			);
			await until(
				() => settings.querySelectorAll("[data-gate-inline]").length === 1,
			);
			const reason = settings.querySelector("[data-gate-inline]") as Element;
			const actions = [
				...settings.querySelectorAll<HTMLElement>("[data-history-row] button"),
			];
			expect(actions.length).toBeGreaterThan(1);
			for (const action of actions) {
				expect(action.getAttribute("aria-disabled")).toBe("true");
				expect(action.getAttribute("aria-describedby")).toBe(reason.id);
			}
			const before = view.fake.api.commands.length;
			await click(actions[0] as HTMLElement);
			expect(queryByRole("dialog")).toBeNull();
			expect(view.fake.api.commands.length).toBe(before);
		},
		SLOW,
	);

	test(
		"Resume recording: the sheet shows the consequences, one primary, and signs a new list for the device",
		async () => {
			const view = await open(EDGE, { arrange: acceptRosters });
			const settings = block(view, "observe-history-settings");
			await until(
				() => settings.querySelectorAll("[data-history-row]").length > 0,
			);
			const metrics = settings.querySelector(
				'[data-history-row="device|metrics"]',
			) as HTMLElement;
			await click(byRole("button", "Resume recording…", metrics));
			const sheet = inPortal("dialog");
			expect(text(sheet)).toContain("Resume recording");
			expect(text(sheet)).toContain(
				"Retained metrics · Whole device · edge-berlin-01",
			);
			expect(text(sheet)).toContain("What happens");
			expect(text(sheet)).toContain("Can you undo it?");
			expect(text(sheet)).toContain("owner, always a reader");
			expect(primaries()).toBe(1);
			expect(queryByRole("textbox", "Device password", sheet)).toBeNull();

			await click(byRole("button", "Resume recording", sheet));
			await until(() => sent(view).includes("archive_policy"));
			const signed = view.fake.api.commands.find(
				([, type]) => type === "archive_policy",
			);
			const roster = readFakeCompact<{
				kind: string;
				scope: string;
				policy_version: number;
				recipients: { user_id: string }[];
				expires_at: number;
			}>(
				String(signed?.[2].policy_jws),
				view.fake.hub.ownerInvitationKey(EDGE),
			);
			expect(roster).toMatchObject({
				kind: "metrics",
				scope: "device",
				policy_version: 4,
			});
			expect(roster.recipients.map((reader) => reader.user_id)).toEqual([
				view.fake.hub.me,
			]);
			expect(roster.expires_at).toBeGreaterThan(SAMPLE_NOW);
			await until(() => queryByRole("dialog") === null);
			await until(() =>
				/Recording/.test(
					text(
						settings.querySelector(
							'[data-history-row="device|metrics"]',
						) as Element,
					),
				),
			);
			expect(text(settings)).toContain("done.");
		},
		SLOW,
	);

	test(
		"without a held signature the sheet asks for the device password before it signs",
		async () => {
			const view = await open(EDGE, {
				heldSigner: false,
				arrange: acceptRosters,
			});
			const settings = block(view, "observe-history-settings");
			await until(
				() => settings.querySelectorAll("[data-history-row]").length > 0,
			);
			await click(
				byRole(
					"button",
					"Resume recording…",
					settings.querySelector(
						'[data-history-row="device|metrics"]',
					) as HTMLElement,
				),
			);
			const sheet = inPortal("dialog");
			const confirm = byRole("button", "Resume recording", sheet);
			expect(confirm.hasAttribute("disabled")).toBe(true);
			await click(confirm);
			expect(sent(view)).not.toContain("archive_policy");
			const password = sheet.querySelector<HTMLInputElement>(
				"input[autocomplete=current-password]",
			) as HTMLInputElement;
			await typeInto(password, FAKE_PASSWORD);
			await click(byRole("button", "Resume recording", sheet));
			await until(() => sent(view).includes("archive_policy"));
		},
		SLOW,
	);

	test(
		"a list the device refuses: the sheet stays open with the device's words, and the typed password has left the field",
		async () => {
			const view = await open(EDGE, {
				heldSigner: false,
				arrange: (agent) =>
					agent.handle("archive_policy", () => ({
						state: "rejected",
						result: {
							code: "conflict",
							error: "A newer readers list is already in place.",
						},
					})),
			});
			const settings = block(view, "observe-history-settings");
			await until(
				() => settings.querySelectorAll("[data-history-row]").length > 0,
			);
			await click(
				byRole(
					"button",
					"Resume recording…",
					settings.querySelector(
						'[data-history-row="device|metrics"]',
					) as HTMLElement,
				),
			);
			const sheet = inPortal("dialog");
			const field = () =>
				sheet.querySelector<HTMLInputElement>(
					"input[autocomplete=current-password]",
				) as HTMLInputElement;
			await typeInto(field(), FAKE_PASSWORD);
			await click(byRole("button", "Resume recording", sheet));
			await until(() => /refused the list/.test(text(sheet)));
			expect(text(sheet)).toContain(
				"edge-berlin-01 refused the list: “A newer readers list is already in place.”",
			);
			expect(field().value).toBe("");
			expect(sheet.innerHTML).not.toContain(FAKE_PASSWORD);
			expect(
				byRole("button", "Resume recording", sheet).hasAttribute("disabled"),
			).toBe(true);
		},
		SLOW,
	);

	test(
		"a pasted reader request joins the list only with permission to read there",
		async () => {
			const view = await open(EDGE, {
				people: {
					[SAMPLE_PEOPLE.mira]: "Mira Novak",
					[SAMPLE_PEOPLE.partner]: "Pat Partner",
				},
			});
			const settings = block(view, "observe-history-settings");
			await until(
				() => settings.querySelectorAll("[data-history-row]").length > 0,
			);
			await click(
				byRole(
					"button",
					"Change readers…",
					settings.querySelector(
						'[data-history-row="device|logs"]',
					) as HTMLElement,
				),
			);
			const sheet = inPortal("dialog");
			const paste = byRole("textbox", "Paste a reader request", sheet);
			await typeInto(paste, "not a request");
			await click(byRole("button", "Add reader", sheet));
			expect(text(sheet)).toContain("This isn't a reader request.");

			const request = (user: string, key: string) =>
				JSON.stringify({
					kind: READER_REQUEST_KIND,
					version: 1,
					device_id: EDGE,
					recipient: {
						recipient_id: key,
						user_id: user,
						public_key: [1, 2, 3],
					},
				});
			await typeInto(paste, request(SAMPLE_PEOPLE.mira, "mira-key"));
			await click(byRole("button", "Add reader", sheet));
			await until(() => /Mira Novak/.test(text(sheet)));
			expect(text(sheet)).toContain("may read logs here until");
			await typeInto(paste, request(SAMPLE_PEOPLE.partner, "partner-key"));
			await click(byRole("button", "Add reader", sheet));
			await until(() => /Pat Partner/.test(text(sheet)));
			expect(text(sheet)).toContain(
				"has no permission to read logs here: can't be a reader",
			);
			const boxes = allByRole("checkbox", undefined, sheet);
			expect(boxes.at(-1)?.getAttribute("aria-checked")).toBe("false");
			expect(boxes.at(-1)?.hasAttribute("disabled")).toBe(true);
			expect(primaries()).toBe(1);
		},
		SLOW,
	);
});

describe("history access (not the owner)", () => {
	test(
		"the reader request is copied as a file without a secret",
		async () => {
			const copied: string[] = [];
			Object.defineProperty(navigator, "clipboard", {
				configurable: true,
				value: {
					writeText: async (value: string) => {
						copied.push(value);
					},
				},
			});
			const view = await open(LAB, { connect: LAB });
			const access = block(view, "observe-history-access");
			await click(byRole("button", "Copy reader request", access));
			await until(() => copied.length === 1);
			const request = JSON.parse(copied[0] ?? "{}");
			expect(request.kind).toBe(READER_REQUEST_KIND);
			expect(request.device_id).toBe(LAB);
			expect(request.recipient.user_id).toBe(view.fake.hub.me);
			expect(JSON.stringify(request)).not.toMatch(/secret|private|vault/i);
			expect(
				byRole("button", "Download reader request", access),
			).not.toBeNull();
			expect(sent(view)).not.toContain("archive_policy");
		},
		SLOW,
	);

	test("locked: it doesn't claim you aren't a reader, and the request waits for the keys", async () => {
		const view = await open(LAB);
		const access = block(view, "observe-history-access");
		expect(text(access)).toContain(
			"is read from the device over a live connection",
		);
		expect(text(access)).not.toContain("You aren't a reader");
		expect(text(access)).toContain(
			"Unlock lab-gpu-02 to create a reader request.",
		);
		const before = view.fake.api.commands.length;
		await click(byRole("button", "Copy reader request", access));
		expect(view.fake.api.commands.length).toBe(before);
		expect(queryByRole("button", "Download reader request", access)).toBeNull();
	});
});
