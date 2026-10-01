import { afterAll, afterEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { BoardEditJob, BoardEditJobPhase } from "../../lib/schema/copilot";

const window = new Window({ url: "https://localhost" });
Object.assign(window, { SyntaxError, TypeError, Error });
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	Node: window.Node,
	NodeFilter: window.NodeFilter,
	MutationObserver: window.MutationObserver,
	CustomEvent: window.CustomEvent,
	getComputedStyle: window.getComputedStyle.bind(window),
	IS_REACT_ACT_ENVIRONMENT: true,
};
const globalDescriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);

const actualBackend = { ...(await import("../../state/backend-state")) };
const actualLocales = { ...(await import("@flow-like/locales")) };
let backend: ReturnType<typeof recoveryBackend>;
mock.module("../../state/backend-state", () => ({
	...actualBackend,
	useBackend: () => ({ boardState: backend }),
}));
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({
		t: (
			key: string,
			fallback: string | { count: number; defaultValue_other: string },
		) =>
			typeof fallback === "string"
				? fallback
				: fallback?.defaultValue_other.replace(
						"{{count}}",
						`${fallback.count}`,
					) || key,
	}),
}));

const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { createRoot } = await import("react-dom/client");
const { BoardEditRecovery } = await import("./board-edit-recovery");
const roots: ReturnType<typeof createRoot>[] = [];
const queryClients: InstanceType<typeof QueryClient>[] = [];

afterEach(async () => {
	await act(async () => {
		for (const root of roots.splice(0)) root.unmount();
	});
	for (const client of queryClients.splice(0)) client.clear();
	window.document.body.innerHTML = "";
});

afterAll(async () => {
	mock.module("../../state/backend-state", () => actualBackend);
	mock.module("@flow-like/locales", () => actualLocales);
	await window.happyDOM.close();
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

function job(phase: BoardEditJobPhase = "failed"): BoardEditJob {
	return {
		schemaVersion: "flowpilot.board-edit-job/v1",
		jobId: "retained-edit",
		appId: "app-1",
		boardId: "board-1",
		requestId: "flowpilot:closed-chat",
		phase,
		createdAtMs: 1,
		updatedAtMs: 2,
		expiresAtMs: 10_000,
		token: {
			board_id: "board-1",
			draft_id: "draft-1",
			revision: 1,
			base_fingerprint: "fingerprint-1",
			claim_id: "claim-1",
		},
		approval: {
			kind: "execute",
			title: "Approve edit",
			description: "Apply the reviewed changes.",
			sessionKey: "flowpilot_board",
			timing: "before_apply",
		},
		review: {
			commandCount: 1,
			commandCounts: { AddNode: 1 },
			commandSummaries: ["Add the retained node"],
			replacementMode: false,
			destructiveEffects: [],
		},
	};
}

function recoveryBackend(initialJobs: BoardEditJob[]) {
	let retained = initialJobs;
	const findJob = (jobId: string) => {
		const retainedJob = retained.find((entry) => entry.jobId === jobId);
		if (!retainedJob) throw new Error(`Missing retained edit: ${jobId}`);
		return retainedJob;
	};
	return {
		listBoardEditJobs: mock(async () => retained),
		getBoardEditJob: mock(async (jobId: string) =>
			retained.find((entry) => entry.jobId === jobId),
		),
		resolveBoardEditJob: mock(
			async (jobId: string, approved: boolean, _allowDestructive: boolean) => {
				const current = findJob(jobId);
				const resolved: BoardEditJob = {
					...current,
					phase: approved ? "applied_pending_delivery" : "denied",
				};
				retained = retained.map((entry) =>
					entry.jobId === jobId ? resolved : entry,
				);
				return { job: resolved, transitioned: true };
			},
		),
		claimBoardEditJobDelivery: mock(async (jobId: string) => ({
			job: findJob(jobId),
			claimed: true,
			deliveryLeaseId: "lease-1",
		})),
		ackBoardEditJobDelivery: mock(async (jobId: string, _leaseId: string) => {
			const resolved: BoardEditJob = {
				...findJob(jobId),
				phase: "applied",
			};
			retained = retained.map((entry) =>
				entry.jobId === jobId ? resolved : entry,
			);
			return resolved;
		}),
	};
}

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 10));
	});
}

async function mount(initialJobs: BoardEditJob[]) {
	backend = recoveryBackend(initialJobs);
	const replayReceipt = mock(async () => ({
		status: "applied" as const,
		delivery_complete: true,
		message: "Applied.",
		commands: [],
		board_commands: [],
		diagnostics: [],
	}));
	const client = new QueryClient({
		defaultOptions: {
			queries: { retry: false, gcTime: Number.POSITIVE_INFINITY },
		},
	});
	queryClients.push(client);
	const container = window.document.createElement("div");
	window.document.body.appendChild(container);
	const root = createRoot(container as unknown as HTMLElement);
	roots.push(root);
	// The board recovery control mounts independently of the FlowPilot panel.
	await act(async () => {
		root.render(
			<QueryClientProvider client={client}>
				<BoardEditRecovery
					appId="app-1"
					boardId="board-1"
					onApplyFlowIrCommit={replayReceipt}
				/>
			</QueryClientProvider>,
		);
	});
	await settle();
	return { backend, replayReceipt };
}

function button(label: string) {
	return [...window.document.querySelectorAll("button")].find(
		(element) => element.textContent === label,
	);
}

async function click(label: string) {
	const target = button(label);
	expect(target).toBeDefined();
	if (!target) throw new Error(`Missing button: ${label}`);
	await act(async () => target.click());
	await settle();
}

describe("BoardEditRecovery", () => {
	test("offers a recovery dialog for a direct edit when FlowPilot is closed", async () => {
		const { backend } = await mount([
			{ ...job(), error: "The previous apply was interrupted." },
		]);
		expect(backend.listBoardEditJobs).toHaveBeenCalledWith(
			"app-1",
			"board-1",
			false,
		);
		expect(
			window.document.querySelector('[aria-live="polite"]')?.textContent,
		).toContain("A FlowPilot edit is blocking changes to this board.");
		expect(window.document.querySelector('[role="dialog"]')).toBeNull();

		await click("Resolve edit");

		const dialog = window.document.querySelector('[role="dialog"]');
		expect(dialog?.textContent).toContain("Recover FlowPilot edits");
		expect(dialog?.textContent).toContain("Add the retained node");
		expect(dialog?.textContent).toContain(
			"The previous apply was interrupted.",
		);
		expect(button("Dismiss edit")).toBeDefined();
		expect(button("Retry apply")).toBeDefined();
	});

	test("dismisses a failed edit and removes the board blocker", async () => {
		const { backend, replayReceipt } = await mount([job()]);
		await click("Resolve edit");
		await click("Dismiss edit");

		expect(backend.resolveBoardEditJob).toHaveBeenCalledWith(
			"retained-edit",
			false,
			false,
		);
		expect(replayReceipt).not.toHaveBeenCalled();
		expect(window.document.querySelector('[aria-live="polite"]')).toBeNull();
		expect(window.document.querySelector('[role="dialog"]')).toBeNull();
	});

	test("retries a failed edit, delivers its receipt, and removes the blocker", async () => {
		const retained = job();
		const { backend, replayReceipt } = await mount([retained]);
		await click("Resolve edit");
		await click("Retry apply");

		expect(backend.resolveBoardEditJob).toHaveBeenCalledWith(
			"retained-edit",
			true,
			true,
		);
		expect(replayReceipt).toHaveBeenCalledWith(
			retained.token,
			"flowpilot-board-edit:claim:claim-1",
			"invalidate",
		);
		expect(backend.ackBoardEditJobDelivery).toHaveBeenCalledWith(
			"retained-edit",
			"lease-1",
		);
		expect(window.document.querySelector('[aria-live="polite"]')).toBeNull();
	});

	test("keeps recovery available after an error and allows a later retry", async () => {
		const { backend } = await mount([job()]);
		backend.resolveBoardEditJob.mockRejectedValueOnce(
			new Error("The board could not be loaded. Try again."),
		);
		await click("Resolve edit");
		await click("Retry apply");

		expect(window.document.querySelector('[role="alert"]')?.textContent).toBe(
			"The board could not be loaded. Try again.",
		);
		expect(
			window.document.querySelector('[aria-live="polite"]'),
		).not.toBeNull();
		expect(button("Retry apply")?.disabled).toBe(false);
		expect(button("Dismiss edit")?.disabled).toBe(false);

		await click("Retry apply");

		expect(backend.resolveBoardEditJob).toHaveBeenCalledTimes(2);
		expect(window.document.querySelector('[aria-live="polite"]')).toBeNull();
	});

	test.each([
		["applying", "Recover edit"],
		["applied_pending_delivery", "Finish sync"],
	] as const)(
		"offers recovery without dismissal for %s edits",
		async (phase, label) => {
			await mount([job(phase)]);
			await click("Resolve edit");

			expect(button(label)).toBeDefined();
			expect(button("Dismiss edit")).toBeUndefined();
		},
	);

	test("ignores other boards and edits that do not block mutations", async () => {
		await mount([
			{ ...job(), jobId: "other-board", boardId: "board-2" },
			{ ...job(), jobId: "other-app", appId: "app-2" },
			...(
				[
					"preparing",
					"awaiting_approval",
					"applied",
					"denied",
					"stale",
					"cancelled",
				] as const
			).map((phase) => ({ ...job(phase), jobId: phase })),
		]);

		expect(window.document.querySelector('[aria-live="polite"]')).toBeNull();
		expect(button("Resolve edit")).toBeUndefined();
	});
});
