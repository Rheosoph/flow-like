import { afterAll, afterEach, beforeEach, expect, mock, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { ILogLevel } from "../../../lib/schema/flow/log";
import type { IRunLogSummary } from "../../../lib/schema/flow/log-query";
import type { IBackendState } from "../../../state/backend-state";
import type { IEventTimelineRun } from "../../../state/backend-state/event-state";
import {
	byRole,
	click,
	installDom,
	settle,
} from "../devices/testing/dom-harness";
import type { EventHistoryFlowTarget } from "./event-history-run-dialog";

const dom = installDom();
const { BackendContext } = await import("../../../state/backend-state");
const { AppRouterContext } = await import(
	"next/dist/shared/lib/app-router-context.shared-runtime"
);
const { makeBoard, makeLog, makeNode, memoryBoardState } = await import(
	"../../flow/logs/test-fixtures"
);
const { EventHistoryRunDialog } = await import("./event-history-run-dialog");

// The real log virtualizer needs a viewport measurement in happy-dom.
Object.defineProperties(dom.window.HTMLElement.prototype, {
	offsetHeight: {
		configurable: true,
		get() {
			return this.getAttribute("role") === "log" ? 300 : 0;
		},
	},
	offsetWidth: { configurable: true, get: () => 800 },
});

let queryClient: QueryClient;
beforeEach(() => {
	dom.reinstall();
	queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false, gcTime: 0 } },
	});
});
afterEach(async () => {
	await dom.cleanup();
	queryClient.clear();
});
afterAll(dom.restore);

const workerId = "worker&next";
const metadata: IEventTimelineRun = {
	app_id: "app+notes",
	board_id: "board/page",
	event_id: "event",
	node_id: "entry-node",
	run_id: "actual-run-id",
	start: 1_000_000,
	end: 2_000_000,
	log_level: 3,
	nodes: [
		["entry-node", 1],
		[workerId, 3],
	],
	version: "v2-1-3",
	payload: [],
};

async function mountRun(
	run = metadata,
	initialNodeId: string | undefined = workerId,
	onNavigateToFlow?: (target: EventHistoryFlowTarget) => void,
) {
	const otherLog = makeLog({
		node_id: "entry-node",
		message: "Entry started",
	});
	const workerLog = makeLog({
		node_id: workerId,
		message: "Could not load notes",
		log_level: ILogLevel.Error,
		start: 1_500_000,
	});
	const memory = memoryBoardState([otherLog, workerLog]);
	const queryRunLogs = mock(memory.queryRunLogs);
	const countRunLogs = mock(memory.countRunLogs);
	const getRunLogSummary = mock(
		async (): Promise<IRunLogSummary> => ({
			version: 1,
			fingerprinted: false,
			partial: false,
			total: 2,
			levels: [0, 1, 0, 1, 0],
			nodes: { "entry-node": [0, 1, 0, 0, 0], [workerId]: [0, 0, 0, 1, 0] },
			first_error: {
				node_id: workerId,
				log_level: 3,
				start: 1_500_000,
				message: workerLog.message,
			},
			visited: ["entry-node", workerId],
			groups: [],
			groups_truncated: false,
		}),
	);
	const getBoard = mock(async () =>
		makeBoard([
			makeNode("entry-node", "Load page", []),
			makeNode(workerId, "Fetch notes", []),
		]),
	);
	const backend = {
		boardState: { getBoard, queryRunLogs, countRunLogs, getRunLogSummary },
		roleState: {
			getOwnRole: async () => ({
				role_id: "reader",
				role_name: "Flow reader",
				permissions: RolePermissions.ReadBoards.toNumber(),
				is_owner: false,
				can_leave: true,
			}),
		},
	} as unknown as IBackendState;
	const push = mock((_href: string) => {});
	const onClose = mock(() => {});
	const router = {
		bfcacheId: "event-history",
		push,
		replace: () => {},
		back: () => {},
		forward: () => {},
		refresh: () => {},
		prefetch: async () => {},
	};
	await dom.render(
		<QueryClientProvider client={queryClient}>
			<BackendContext.Provider value={backend}>
				<AppRouterContext.Provider value={router}>
					<EventHistoryRunDialog
						run={run}
						name="Notes page"
						initialNodeId={initialNodeId}
						onNavigateToFlow={onNavigateToFlow}
						onClose={onClose}
					/>
				</AppRouterContext.Provider>
			</BackendContext.Provider>
		</QueryClientProvider>,
	);
	await settle();
	await settle();
	return {
		getBoard,
		queryRunLogs,
		countRunLogs,
		getRunLogSummary,
		push,
		onClose,
	};
}

test("a local run opens its recorded flow version and loads only the selected node's logs", async () => {
	const view = await mountRun();
	expect(view.getBoard).toHaveBeenCalledWith(
		"app+notes",
		"board/page",
		[2, 1, 3],
	);
	expect(view.getRunLogSummary).toHaveBeenCalledWith(metadata);
	expect(view.countRunLogs).toHaveBeenCalledWith(metadata, {
		nodes: [workerId],
	});
	expect(view.queryRunLogs).toHaveBeenCalledWith(
		metadata,
		{ nodes: [workerId] },
		0,
		expect.any(Number),
	);
	const logs = byRole("log", "Run logs");
	expect(logs.textContent).toContain("Could not load notes");
	expect(logs.textContent).toContain("Fetch notes");
	expect(logs.textContent).not.toContain("Entry started");
	await click(byRole("button", "Copy run ID"));
	expect(dom.clipboard).toEqual(["actual-run-id"]);
	const detailToggle = byRole("button", "Detail pane");
	expect(detailToggle.getAttribute("aria-pressed")).toBe("true");
	expect(
		document.querySelector('aside[aria-label="Log details"]'),
	).not.toBeNull();
	await click(detailToggle);
	expect(detailToggle.getAttribute("aria-pressed")).toBe("false");
	expect(document.querySelector('aside[aria-label="Log details"]')).toBeNull();
});

test("cloud log denial makes no log requests and keeps the run's recorded failure reason", async () => {
	const run = {
		...metadata,
		is_remote: true,
		error_message: "The trigger was rejected before execution",
	};
	const view = await mountRun(run);
	expect(view.getRunLogSummary).not.toHaveBeenCalled();
	expect(view.countRunLogs).not.toHaveBeenCalled();
	expect(view.queryRunLogs).not.toHaveBeenCalled();
	expect(byRole("dialog").textContent).toContain("Run logs unavailable");
	expect(byRole("dialog").textContent).toContain(run.error_message);
});

test("Show on board navigates to the logged node and recorded version with encoded identifiers", async () => {
	const view = await mountRun();
	await click(byRole("button", "Show on board"));
	expect(view.push).toHaveBeenCalledWith(
		"/flow?id=board%2Fpage&app=app%2Bnotes&node=worker%26next&version=2_1_3",
	);
	expect(view.onClose).toHaveBeenCalledTimes(1);
});

test("embedded navigation receives the recorded board and node without pushing a page route", async () => {
	const onNavigateToFlow = mock((_target: EventHistoryFlowTarget) => {});
	const view = await mountRun(metadata, workerId, onNavigateToFlow);
	await click(byRole("button", "Show on board"));
	expect(onNavigateToFlow).toHaveBeenCalledWith({
		boardId: "board/page",
		appId: "app+notes",
		nodeId: workerId,
		version: [2, 1, 3],
	});
	expect(view.push).not.toHaveBeenCalled();
	expect(view.onClose).toHaveBeenCalledTimes(1);
});

test("a missing flow version keeps the logs without fetching names from the latest board", async () => {
	const view = await mountRun({ ...metadata, version: "" });
	expect(view.getBoard).not.toHaveBeenCalled();
	expect(view.queryRunLogs).toHaveBeenCalled();
	expect(byRole("dialog").textContent).toContain(
		"The flow version for this run is unavailable",
	);
	expect(byRole("log", "Run logs").textContent).toContain(
		"Could not load notes",
	);
});
