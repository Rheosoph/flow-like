import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import {
	advance,
	byRole,
	click,
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
const { SAMPLE_IDS } = await import(
	"../../../../lib/device-management/model/__fixtures__/sample-fleet"
);
const { createFakeWorkspace } = await import("../testing/fake-workspace");
const { useOverlayStore } = await import("../workspace/overlay-store");
const { ACCOUNT_SCOPE } = await import("../routing/devices-route");
const { ActivityView } = await import("./activity-view");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
});
afterAll(dom.restore);

const DEVICE = SAMPLE_IDS.edge;
const SERVICE = "invoice-extractor";
const RUN = "019abcde-1234-7000-8000-123456789abc";
const SECOND_RUN = "019abcde-1234-7000-8000-123456789abd";
const START = 1_791_541_800_123_456;
const PRINT_INFO =
	'Print Info: invoice approved <script>alert("literal")</script>';
const SLOW = 20_000;

type Fake = Awaited<ReturnType<typeof createFakeWorkspace>>;
type Agent = ReturnType<Fake["agent"]>;

function run(runId = RUN) {
	return {
		run_id: runId,
		board_id: "invoice-flow",
		event_id: "invoice-event",
		node_id: "start-node",
		version: "v1-2-3",
		event_version: "1.0.0",
		start: START,
		end: START + 125_000,
		log_level: 1,
		logs: 2,
	};
}

function log(message = PRINT_INFO, nodeId = "print-info-node") {
	return {
		message,
		node_id: nodeId,
		operation_id: "operation-1",
		log_level: 1,
		start: START,
		end: START + 1_000,
		truncated: false,
	};
}

function histories(agent: Agent) {
	agent.handle("execution_runs", (command) => ({
		state: "completed",
		result: {
			placement_id: command.placement_id,
			runs: [run()],
			next_offset: null,
		},
	}));
	agent.handle("execution_logs", (command) => ({
		state: "completed",
		result: {
			placement_id: command.placement_id,
			run_id: command.run_id,
			logs: [log()],
			next_offset: null,
		},
	}));
}

async function open(
	arrange?: (agent: Agent, fake: Fake) => void,
	options: FakeWorkspaceOptions = {},
) {
	const fake = await createFakeWorkspace(undefined, {
		agentFeatures: { execution_logs: 1 },
		...options,
	});
	histories(fake.agent(DEVICE));
	arrange?.(fake.agent(DEVICE), fake);
	const view = await mountDevices(
		<ActivityView
			deviceId={DEVICE}
			serviceId={SERVICE}
			scope={ACCOUNT_SCOPE}
		/>,
		{ fake, search: `device=${DEVICE}&tab=activity` },
	);
	await view.settle();
	return view;
}

type View = Awaited<ReturnType<typeof open>>;
const block = (view: View) =>
	view.container.querySelector<HTMLElement>(
		"#observe-executions",
	) as HTMLElement;
const text = (root: ParentNode) =>
	(root.textContent ?? "").replace(/\s+/g, " ");
const commands = (view: View, type: string) =>
	view.fake.api.commands
		.filter(([, sent]) => sent === type)
		.map(([, , body]) => body);

async function until(check: () => boolean) {
	const end = performance.now() + 6_000;
	while (!check() && performance.now() < end) await advance(25);
	expect(check()).toBe(true);
}

async function inspect(view: View, id = RUN) {
	await until(
		() => !!queryByRole("button", `Inspect execution ${id}`, block(view)),
	);
	await click(byRole("button", `Inspect execution ${id}`, block(view)));
}

describe("workflow execution logs in Activity", () => {
	test(
		"paging boundaries explain remaining local history for runs and logs",
		async () => {
			const view = await open((agent) => {
				agent.handle("execution_runs", () => ({
					state: "completed",
					result: {
						placement_id: SERVICE,
						runs: [run()],
						next_offset: null,
						limit_reached: true,
					},
				}));
				agent.handle("execution_logs", () => ({
					state: "completed",
					result: {
						placement_id: SERVICE,
						run_id: RUN,
						logs: [log()],
						next_offset: null,
						limit_reached: true,
					},
				}));
			});
			const note =
				"The device's paging limit has been reached. More entries remain in its local log store.";
			await until(() => text(block(view)).includes(note));
			expect(
				queryByRole("button", "Load older executions", block(view)),
			).toBeNull();
			await inspect(view);
			await until(() => text(block(view)).includes(PRINT_INFO));
			expect(text(block(view))).toContain(note);
			expect(
				queryByRole("button", "Load more log entries", block(view)),
			).toBeNull();
		},
		SLOW,
	);

	test(
		"selecting an execution reads its node logs and renders Print Info as literal text",
		async () => {
			const view = await open();
			await inspect(view);
			await until(() => text(block(view)).includes(PRINT_INFO));
			expect(commands(view, "execution_runs")).toEqual([
				{ type: "execution_runs", placement_id: SERVICE, offset: 0, limit: 20 },
			]);
			expect(commands(view, "execution_logs")).toEqual([
				{
					type: "execution_logs",
					placement_id: SERVICE,
					run_id: RUN,
					offset: 0,
					limit: 50,
					min_level: 0,
				},
			]);
			expect(block(view).querySelector("pre")?.textContent).toBe(PRINT_INFO);
			expect(block(view).querySelector("script")).toBeNull();
			expect(text(block(view))).toContain("Node print-info-node");
			expect(text(block(view))).toContain("Operation operation-1");
			expect(text(block(view))).toContain("Flow invoice-flow, version v1-2-3");
			expect(text(block(view))).toContain("Info");
			expect(block(view).querySelector("time")?.title).toContain(
				"10:30:00 UTC",
			);
			await click(byRole("button", "All executions", block(view)));
			expect(text(block(view))).toContain("125 ms");
			expect(text(block(view))).not.toContain(PRINT_INFO);
		},
		SLOW,
	);

	test(
		"a newly completed run shifting pagination does not duplicate rows and refresh shows it",
		async () => {
			const newestRun = "newest-run";
			let completedAnotherRun = false;
			const view = await open((agent) => {
				agent.handle("execution_runs", (command) => {
					const available = completedAnotherRun
						? [newestRun, RUN, SECOND_RUN]
						: [RUN, SECOND_RUN];
					const offset = Number(command.offset);
					return {
						state: "completed",
						result: {
							placement_id: SERVICE,
							runs: available.slice(offset, offset + 1).map((id) => run(id)),
							next_offset: offset + 1 < available.length ? offset + 1 : null,
						},
					};
				});
			});
			const ids = () =>
				Array.from(
					block(view).querySelectorAll("[data-execution-run]"),
					(row) => row.getAttribute("data-execution-run"),
				);
			await until(() => ids().includes(RUN));
			completedAnotherRun = true;
			await click(byRole("button", "Load older executions", block(view)));
			await until(
				() =>
					commands(view, "execution_runs").length === 2 &&
					byRole("button", "Load older executions", block(view)).getAttribute(
						"aria-busy",
					) !== "true",
			);
			expect(ids()).toEqual([RUN]);
			await click(byRole("button", "Load older executions", block(view)));
			await until(() => ids().includes(SECOND_RUN));
			expect(ids()).toEqual([RUN, SECOND_RUN]);
			expect(
				queryByRole("button", "Load older executions", block(view)),
			).toBeNull();
			await click(byRole("button", "Refresh executions", block(view)));
			await until(() => ids().includes(newestRun));
			expect(ids()).toEqual([newestRun]);
			expect(
				commands(view, "execution_runs").map((command) => command.offset),
			).toEqual([0, 1, 2, 0]);
		},
		SLOW,
	);

	test(
		"run and log pages append, while node and level changes reset the cursor",
		async () => {
			const view = await open((agent) => {
				agent.handle("execution_runs", (command) => ({
					state: "completed",
					result: {
						placement_id: SERVICE,
						runs: [run(command.offset === 0 ? RUN : SECOND_RUN)],
						next_offset: command.offset === 0 ? 1 : null,
					},
				}));
				agent.handle("execution_logs", (command) => ({
					state: "completed",
					result: {
						placement_id: SERVICE,
						run_id: command.run_id,
						logs: [
							log(
								command.node_id
									? "Filtered Print Info"
									: command.offset === 0
										? PRINT_INFO
										: "Second page message",
							),
						],
						next_offset: !command.node_id && command.offset === 0 ? 1 : null,
					},
				}));
			});
			await until(
				() => !!queryByRole("button", "Load older executions", block(view)),
			);
			await click(byRole("button", "Load older executions", block(view)));
			await until(
				() =>
					!!queryByRole(
						"button",
						`Inspect execution ${SECOND_RUN}`,
						block(view),
					),
			);
			expect(block(view).querySelectorAll("[data-execution-run]")).toHaveLength(
				2,
			);
			await inspect(view);
			await until(
				() => !!queryByRole("button", "Load more log entries", block(view)),
			);
			await click(byRole("button", "Load more log entries", block(view)));
			await until(() => text(block(view)).includes("Second page message"));
			expect(block(view).querySelectorAll("[data-execution-log]")).toHaveLength(
				2,
			);
			await typeInto(
				byRole("textbox", "Node ID", block(view)),
				"print-info-node",
			);
			await click(byRole("button", "Filter", block(view)));
			await until(() => text(block(view)).includes("Filtered Print Info"));
			expect(text(block(view))).not.toContain("Second page message");
			expect(block(view).querySelectorAll("[data-execution-log]")).toHaveLength(
				1,
			);
			await click(byRole("combobox", "Minimum log level", block(view)));
			await click(byRole("option", "Warnings and above"));
			await until(
				() => commands(view, "execution_logs").at(-1)?.min_level === 2,
			);
			expect(
				commands(view, "execution_runs").map((command) => command.offset),
			).toEqual([0, 1]);
			expect(
				commands(view, "execution_logs").map((command) => [
					command.offset,
					command.node_id,
					command.min_level,
				]),
			).toEqual([
				[0, undefined, 0],
				[1, undefined, 0],
				[0, "print-info-node", 0],
				[0, "print-info-node", 2],
			]);
		},
		SLOW,
	);

	test(
		"locking removes loaded logs and a delayed response cannot restore them",
		async () => {
			const view = await open((agent) => {
				agent.handle("execution_logs", (command) => ({
					state: "completed",
					result: {
						placement_id: SERVICE,
						run_id: RUN,
						logs: [log()],
						next_offset: command.offset === 0 ? 1 : null,
					},
				}));
			});
			await inspect(view);
			await until(() => text(block(view)).includes(PRINT_INFO));
			const release = view.fake.agent(DEVICE).hold("execution_logs");
			try {
				await click(byRole("button", "Load more log entries", block(view)));
				await until(() => commands(view, "execution_logs").length === 2);
				expect(text(block(view))).toContain(PRINT_INFO);
				await act(async () => view.fake.workspace.keys.lock(DEVICE));
				await view.settle();
				expect(block(view).querySelector("[data-kind=locked]")).not.toBeNull();
				expect(text(block(view))).not.toContain(PRINT_INFO);
				expect(block(view).querySelector("[data-execution-detail]")).toBeNull();
				await act(async () => release());
				await view.settle();
				expect(block(view).querySelector("[data-kind=locked]")).not.toBeNull();
				expect(block(view).querySelector("[data-execution-log]")).toBeNull();
				expect(text(block(view))).not.toContain(PRINT_INFO);
			} finally {
				release();
			}
		},
		SLOW,
	);

	test(
		"a locked service never requests decrypted execution history",
		async () => {
			const view = await open(undefined, { unlock: "none" });
			expect(block(view).querySelector("[data-kind=locked]")).not.toBeNull();
			expect(commands(view, "execution_runs")).toEqual([]);
			expect(commands(view, "execution_logs")).toEqual([]);
			expect(text(block(view))).not.toContain("No finished executions");
		},
		SLOW,
	);

	test(
		"an older agent shows update guidance without sending execution requests",
		async () => {
			const view = await open(undefined, { agentFeatures: {} });
			await until(() => text(block(view)).includes("Update the device agent"));
			expect(commands(view, "execution_runs")).toEqual([]);
			expect(commands(view, "execution_logs")).toEqual([]);
			expect(text(block(view))).not.toContain("No finished executions");
		},
		SLOW,
	);

	for (const type of ["execution_runs", "execution_logs"]) {
		test(
			`${type} authorization refusal is displayed and retry reads again`,
			async () => {
				let restore = () => {};
				const view = await open((agent) => {
					restore = agent.reject(
						type,
						"unauthorized",
						"Logs access was revoked.",
					);
				});
				if (type === "execution_logs") await inspect(view);
				await until(() =>
					text(block(view)).includes("Logs access was revoked."),
				);
				expect(text(block(view))).toContain("Execution logs could not be read");
				expect(text(block(view))).not.toContain("No finished executions");
				restore();
				histories(view.fake.agent(DEVICE));
				await click(byRole("button", "Try again", block(view)));
				await until(() =>
					type === "execution_runs"
						? !!queryByRole("button", `Inspect execution ${RUN}`, block(view))
						: text(block(view)).includes(PRINT_INFO),
				);
				expect(commands(view, type)).toHaveLength(2);
			},
			SLOW,
		);
	}
});
