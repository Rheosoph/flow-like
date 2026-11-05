import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import { aggregateNodeSeverity } from "../../../lib/event-history";
import { type ILog, ILogLevel } from "../../../lib/schema/flow/log";
import type { IBackendState } from "../../../state/backend-state";
import type { IEventTimelineRun } from "../../../state/backend-state/event-state";
import {
	allByRole,
	byRole,
	click,
	installDom,
	settle,
} from "../devices/testing/dom-harness";

const dom = installDom();
const { BackendContext } = await import("../../../state/backend-state");
const { HistoryRuns, HistoryNodes } = await import("./event-history-activity");

class VisibleRowsObserver {
	static instances: VisibleRowsObserver[] = [];
	private targets = new Set<Element>();

	constructor(private callback: IntersectionObserverCallback) {
		VisibleRowsObserver.instances.push(this);
	}

	observe(target: Element) {
		this.targets.add(target);
	}

	disconnect() {
		this.targets.clear();
	}

	unobserve(target: Element) {
		this.targets.delete(target);
	}

	reveal() {
		this.callback(
			Array.from(this.targets, (target) => ({
				target,
				isIntersecting: true,
			})) as IntersectionObserverEntry[],
			this as unknown as IntersectionObserver,
		);
	}
}

let queryClient: QueryClient;

beforeEach(() => {
	dom.reinstall();
	VisibleRowsObserver.instances = [];
	globalThis.IntersectionObserver =
		VisibleRowsObserver as unknown as typeof IntersectionObserver;
	queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false, gcTime: 0 } },
	});
});

afterEach(async () => {
	await dom.cleanup();
	queryClient.clear();
});
afterAll(dom.restore);

function run(overrides: Partial<IEventTimelineRun> = {}): IEventTimelineRun {
	return {
		app_id: "app",
		board_id: "board",
		event_id: "event",
		event_version: "1.0.0",
		node_id: "entry-node",
		run_id: "run-one",
		start: 1_000_000,
		end: 2_000_000,
		version: "v1-0-0",
		log_level: 1,
		nodes: [["entry-node", 1]],
		payload: [],
		...overrides,
	};
}

function warning(message: string, nodeId = "entry-node"): ILog {
	return {
		log_level: ILogLevel.Warn,
		message,
		node_id: nodeId,
		start: { secs_since_epoch: 1, nanos_since_epoch: 0 },
		end: { secs_since_epoch: 1, nanos_since_epoch: 0 },
	};
}

async function mount(
	children: ReactNode,
	options: {
		queryRunLogs?: IBackendState["boardState"]["queryRunLogs"];
		permissions?: number;
	} = {},
) {
	const backend = {
		boardState: {
			queryRunLogs: options.queryRunLogs ?? (async () => []),
		},
		roleState: {
			getOwnRole: async () => ({
				role_id: "reader",
				role_name: "Reader",
				permissions: options.permissions ?? 1,
				is_owner: false,
				can_leave: true,
			}),
		},
	} as unknown as IBackendState;
	const rendered = await dom.render(
		<QueryClientProvider client={queryClient}>
			<BackendContext.Provider value={backend}>
				{children}
			</BackendContext.Provider>
		</QueryClientProvider>,
	);
	await settle();
	return rendered;
}

async function revealRows() {
	await act(async () => {
		for (const observer of VisibleRowsObserver.instances) observer.reveal();
	});
	await settle();
	await settle();
}

function rowDescription(row: HTMLElement): string {
	const id = row.getAttribute("aria-describedby");
	expect(id).toBeTruthy();
	return document.getElementById(id ?? "")?.textContent?.trim() ?? "";
}

const runName = () => "Load notes";
const runsProps = {
	isLoading: false,
	runsSupported: true,
	runsLocked: false,
	runName,
};

test("runs with the same entry node expose distinct run IDs and independent open and copy actions", async () => {
	const first = run();
	const second = run({ run_id: "run-two" });
	const onOpenRun = mock(() => {});
	const { container } = await mount(
		<HistoryRuns {...runsProps} runs={[first, second]} onOpenRun={onOpenRun} />,
	);

	expect(container.textContent).toContain("run-one");
	expect(container.textContent).toContain("run-two");
	expect(
		allByRole("button", /^Open run /).map((row) =>
			row.textContent?.includes("Load notes"),
		),
	).toEqual([true, true]);
	await click(byRole("button", "Open run run-two"));
	expect(onOpenRun).toHaveBeenCalledTimes(1);
	expect(onOpenRun).toHaveBeenLastCalledWith(second);
	await click(allByRole("button", "Copy run ID")[0]);
	expect(dom.clipboard).toEqual(["run-one"]);
	expect(onOpenRun).toHaveBeenCalledTimes(1);
});

test("a run started by a bot is labelled with the bot and a person run is not", async () => {
	const { container } = await mount(
		<HistoryRuns
			{...runsProps}
			runs={[
				run({
					run_id: "bot-run",
					actor: { kind: "bot", id: "bot_1", name: "Ada" },
				}),
				run({ run_id: "person-run" }),
			]}
			onOpenRun={() => {}}
		/>,
	);
	expect(container.textContent?.match(/by Ada/g)).toHaveLength(1);
});

test("visible warning runs load the first problem and expose it with the severity", async () => {
	const meta = run({ log_level: 2 });
	const queryRunLogs = mock(async () => [warning("The connection timed out")]);
	const { container } = await mount(
		<HistoryRuns {...runsProps} runs={[meta]} onOpenRun={() => {}} />,
		{ queryRunLogs },
	);
	expect(queryRunLogs).not.toHaveBeenCalled();
	await revealRows();
	expect(queryRunLogs).toHaveBeenCalledWith(meta, { levels: [2, 3, 4] }, 0, 1);
	expect(container.textContent).toContain("The connection timed out");
	expect(rowDescription(byRole("button", "Open run run-one"))).toBe(
		"Load notes. Warning: The connection timed out",
	);
});

test("keyboard focus loads and opens the run severity tooltip", async () => {
	const queryRunLogs = mock(async () => [warning("Waiting for authorization")]);
	await mount(
		<HistoryRuns
			{...runsProps}
			runs={[run({ log_level: 2 })]}
			onOpenRun={() => {}}
		/>,
		{ queryRunLogs },
	);
	const row = byRole("button", "Open run run-one");
	expect(queryRunLogs).not.toHaveBeenCalled();
	await act(async () => row.focus());
	await settle();
	await settle();
	expect(document.activeElement).toBe(row);
	expect(queryRunLogs).toHaveBeenCalledTimes(1);
	expect(byRole("tooltip").textContent).toContain(
		"Warning: Waiting for authorization",
	);
	expect(rowDescription(row)).toBe(
		"Load notes. Warning: Waiting for authorization",
	);
});

test("node rows read a problem from a run where that node had a warning and open its runs", async () => {
	const oldWarning = run({
		run_id: "warning-run",
		log_level: 2,
		nodes: [["worker", 2]],
	});
	const newerSuccess = run({
		run_id: "successful-run",
		start: 3_000_000,
		nodes: [["worker", 1]],
	});
	const runs = [newerSuccess, oldWarning];
	const queryRunLogs = mock(async () => [
		warning("Retry limit reached", "worker"),
	]);
	const onSelectNode = mock(() => {});
	const { container } = await mount(
		<HistoryNodes
			runs={runs}
			aggregates={aggregateNodeSeverity(runs)}
			nodeName={() => "Fetch notes"}
			onSelectNode={onSelectNode}
		/>,
		{ queryRunLogs },
	);
	await revealRows();
	expect(queryRunLogs).toHaveBeenCalledWith(
		oldWarning,
		{ levels: [2, 3, 4], nodes: ["worker"] },
		0,
		1,
	);
	expect(container.textContent).toContain("Retry limit reached");
	expect(container.textContent).toContain("1 warn");
	expect(container.textContent).toContain("2 runs");
	await click(byRole("button", /Fetch notes/));
	expect(onSelectNode).toHaveBeenLastCalledWith("worker");
});

test("node-filtered run previews never display another node's stored failure reason", async () => {
	const meta = run({
		log_level: 3,
		error_message: "Failure in a different node",
		nodes: [["worker", 2]],
	});
	const queryRunLogs = mock(async () => [warning("Worker warning", "worker")]);
	const { container } = await mount(
		<HistoryRuns
			{...runsProps}
			runs={[meta]}
			onOpenRun={() => {}}
			nodeId="worker"
		/>,
		{ queryRunLogs },
	);
	await revealRows();
	expect(queryRunLogs).toHaveBeenCalledWith(
		meta,
		{ levels: [2, 3, 4], nodes: ["worker"] },
		0,
		1,
	);
	expect(container.textContent).toContain("Worker warning");
	expect(container.textContent).not.toContain("Failure in a different node");
});

test("a node's error preview comes from a failed run even when a newer run only warned", async () => {
	const failed = run({
		run_id: "failed-run",
		log_level: 3,
		nodes: [["worker", 3]],
	});
	const warned = run({
		run_id: "newer-warning",
		start: 3_000_000,
		log_level: 2,
		nodes: [["worker", 2]],
	});
	const runs = [warned, failed];
	const queryRunLogs = mock(async (meta: IEventTimelineRun) => [
		meta.run_id === failed.run_id
			? {
					...warning("Notes could not be loaded", "worker"),
					log_level: ILogLevel.Error,
				}
			: warning("Trying again", "worker"),
	]);
	await mount(
		<HistoryNodes
			runs={runs}
			aggregates={aggregateNodeSeverity(runs)}
			nodeName={() => "Fetch notes"}
			onSelectNode={() => {}}
		/>,
		{ queryRunLogs },
	);
	await revealRows();
	expect(queryRunLogs).toHaveBeenCalledWith(
		failed,
		{ levels: [2, 3, 4], nodes: ["worker"] },
		0,
		1,
	);
	expect(rowDescription(byRole("button", "Show runs for Fetch notes"))).toBe(
		"Error: Notes could not be loaded",
	);
});

test("cloud previews respect ReadLogs while the stored run failure remains visible", async () => {
	const queryRunLogs = mock(async () => [warning("Private log")]);
	const { container } = await mount(
		<HistoryRuns
			{...runsProps}
			runs={[
				run({
					is_remote: true,
					log_level: 3,
					error_message: "Execution was rejected",
				}),
			]}
			onOpenRun={() => {}}
		/>,
		{ queryRunLogs, permissions: 256 + 4096 },
	);
	await revealRows();
	expect(queryRunLogs).not.toHaveBeenCalled();
	expect(container.textContent).toContain("Execution was rejected");
	expect(container.textContent).not.toContain("Private log");
});

test("a failed preview lookup preserves the warning and explains that its message could not load", async () => {
	const queryRunLogs = mock(async (): Promise<ILog[]> => {
		throw new Error("Log store unavailable");
	});
	const onOpenRun = mock(() => {});
	const { container } = await mount(
		<HistoryRuns
			{...runsProps}
			runs={[run({ log_level: 2 })]}
			onOpenRun={onOpenRun}
		/>,
		{ queryRunLogs },
	);
	await revealRows();
	expect(queryRunLogs).toHaveBeenCalledTimes(1);
	expect(rowDescription(byRole("button", "Open run run-one"))).toBe(
		"Load notes. Warning: Could not load the warning or error message. Open the run to retry.",
	);
	expect(container.innerHTML).not.toContain(
		"No warning or error message was recorded",
	);
	await click(byRole("button", "Open run run-one"));
	expect(onOpenRun).toHaveBeenCalledTimes(1);
});
