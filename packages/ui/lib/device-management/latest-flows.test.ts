import { expect, test } from "bun:test";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { ApiResponseError } from "../api-error";
import { HubError } from "./hub/endpoints";
import {
	type FlowVersionCommands,
	LatestFlowError,
	type LatestFlows,
	desktopLatestFlows,
	hubLatestFlows,
	latestPinsOffline,
	latestPinsOnline,
	publishFlow,
	publishLatestFlows,
} from "./latest-flows";

const profile = { id: "profile", hub: "hub.example.com" } as IProfile;
const refusal = (status: number, code?: string, message = "refused") =>
	new ApiResponseError({ status, code, message });

/** A hub that answers each request with the next queued value. */
function hub(answers: Record<string, unknown[]>) {
	const calls: string[] = [];
	const answer = (method: string) => async (_: IProfile, path: string) => {
		const key = `${method} ${path}`;
		calls.push(key);
		const queue = answers[key];
		if (!queue?.length) throw new Error(`unexpected ${key}`);
		const value = queue.length > 1 ? queue.shift() : queue[0];
		if (value instanceof Error) throw value;
		return value;
	};
	const api = { get: answer("GET"), post: answer("POST") };
	return {
		flows: hubLatestFlows(api as unknown as IApiState, profile),
		calls,
	};
}

const PATH = "apps/app/board/board/version/current";
const READ = `GET ${PATH}`;
const PUBLISH = `POST ${PATH}`;
const noWait = { sleep: async () => undefined };

test("a flow with edits gets a version; a flow without edits keeps the one it has", async () => {
	const edited = hub({ [PUBLISH]: [{ version: [0, 0, 8], created: true }] });
	expect(await publishFlow(edited.flows, "app", "board")).toEqual({
		kind: "created",
		version: [0, 0, 8],
	});
	expect(edited.calls).toEqual([PUBLISH]);
	const same = hub({ [PUBLISH]: [{ version: [0, 0, 7], created: false }] });
	expect(await publishFlow(same.flows, "app", "board")).toEqual({
		kind: "unchanged",
		version: [0, 0, 7],
	});
});

test("an older hub can't deploy events that follow Latest", async () => {
	for (const status of [404, 405]) {
		const older = hub({ [PUBLISH]: [refusal(status)] });
		expect(await publishFlow(older.flows, "app", "board")).toEqual({
			kind: "missing_on_hub",
		});
		expect(older.calls).toEqual([PUBLISH]);
	}
	const reads = hub({ [READ]: [refusal(404)] });
	expect(await reads.flows.read("app", "board")).toEqual({
		kind: "missing_on_hub",
	});
});

test("a person who can't publish deploys a flow that already has an equal version, and is stopped otherwise", async () => {
	const published = hub({
		[PUBLISH]: [refusal(403, "FORBIDDEN")],
		[READ]: [{ current: [0, 0, 7], newest: [0, 0, 7] }],
	});
	expect(await publishFlow(published.flows, "app", "board")).toEqual({
		kind: "unchanged",
		version: [0, 0, 7],
	});
	expect(published.calls).toEqual([PUBLISH, READ]);
	const edited = hub({
		[PUBLISH]: [refusal(403, "FORBIDDEN")],
		[READ]: [{ current: null, newest: [0, 0, 7] }],
	});
	expect(await publishFlow(edited.flows, "app", "board")).toEqual({
		kind: "role",
	});
	// The last published version is never deployed in place of newer edits.
	expect(edited.calls).toEqual([PUBLISH, READ]);
});

test("a flow that is being edited is tried three more times, then the deploy says so", async () => {
	const waits: number[] = [];
	const sleep = async (ms: number) => {
		waits.push(ms);
	};
	const busy = hub({ [PUBLISH]: [refusal(423, "BOARD_LOCKED")] });
	expect(await publishFlow(busy.flows, "app", "board", { sleep })).toEqual({
		kind: "busy",
	});
	expect(busy.calls).toEqual([PUBLISH, PUBLISH, PUBLISH, PUBLISH]);
	expect(waits).toEqual([2_000, 2_000, 2_000]);
	const freed = hub({
		[PUBLISH]: [
			refusal(423, "BOARD_LOCKED"),
			refusal(423, "BOARD_LOCKED"),
			{ version: [0, 0, 8], created: true },
		],
	});
	expect(await publishFlow(freed.flows, "app", "board", noWait)).toEqual({
		kind: "created",
		version: [0, 0, 8],
	});
	expect(freed.calls).toEqual([PUBLISH, PUBLISH, PUBLISH]);
});

test("a flow the hub can't compare with its versions stops with the hub's sentence", async () => {
	const sentence =
		"This flow can't be compared with its published version. Pin a flow version in Events to deploy this event.";
	const incomparable = hub({
		[PUBLISH]: [refusal(422, "UNPROCESSABLE", sentence)],
	});
	expect(await publishFlow(incomparable.flows, "app", "board")).toEqual({
		kind: "incomparable",
		message: sentence,
	});
});

test("other failures stay errors, and a cancelled deploy publishes nothing", async () => {
	const failing = hub({ [PUBLISH]: [refusal(500)] });
	await expect(publishFlow(failing.flows, "app", "board")).rejects.toThrow(
		HubError,
	);
	const controller = new AbortController();
	controller.abort();
	const idle = hub({});
	await expect(
		publishFlow(idle.flows, "app", "board", { signal: controller.signal }),
	).rejects.toThrow();
	expect(idle.calls).toEqual([]);
});

test("a local-only app publishes on this computer, for the signed-in account", async () => {
	const calls: unknown[][] = [];
	const commands: FlowVersionCommands = {
		read: async (...args) => {
			calls.push(["read", ...args]);
			return { current: null, newest: [0, 0, 7] };
		},
		publish: async (...args) => {
			calls.push(["publish", ...args]);
			return { version: [0, 0, 8], created: true };
		},
	};
	const flows: LatestFlows = desktopLatestFlows("usr_me", async () => commands);
	expect(await flows.read("app", "board")).toEqual({
		kind: "ok",
		data: { current: null, newest: [0, 0, 7] },
	});
	expect(await publishFlow(flows, "app", "board")).toEqual({
		kind: "created",
		version: [0, 0, 8],
	});
	expect(calls).toEqual([
		["read", "app", "board"],
		["publish", "app", "board", "usr_me"],
	]);
	const broken = desktopLatestFlows(undefined, async () => ({
		read: async () => ({ current: "latest" }),
		publish: async () => {
			throw new Error("The flow is open in another window.");
		},
	}));
	await expect(broken.read("app", "board")).rejects.toThrow();
	await expect(publishFlow(broken, "app", "board")).rejects.toThrow(
		"The flow is open in another window.",
	);
});

/** Flows by id: what `publish` answers, and what `read` says is current. */
function flowsOf(
	states: Record<
		string,
		{ publish: unknown; current?: [number, number, number] | null }
	>,
): { flows: LatestFlows; calls: string[] } {
	const calls: string[] = [];
	return {
		calls,
		flows: {
			read: async (_app, boardId) => {
				calls.push(`read ${boardId}`);
				const { current = null } = states[boardId] ?? {};
				return { kind: "ok", data: { current, newest: current } };
			},
			publish: async (_app, boardId) => {
				calls.push(`publish ${boardId}`);
				return states[boardId]?.publish as Awaited<
					ReturnType<LatestFlows["publish"]>
				>;
			},
		},
	};
}

const created = (version: [number, number, number], made = true) => ({
	kind: "ok",
	data: { version, created: made },
});

const caught = async (run: () => unknown): Promise<LatestFlowError> => {
	try {
		await run();
	} catch (error) {
		if (error instanceof LatestFlowError) return error;
		throw error;
	}
	throw new Error("expected a LatestFlowError");
};

test("a deploy publishes each flow of its Latest events once, in a stable order", async () => {
	const { flows, calls } = flowsOf({
		flow_a: { publish: created([0, 0, 8]) },
		flow_b: { publish: created([1, 2, 0], false) },
	});
	const latest = [
		{ eventId: "evt_two", boardId: "flow_b" },
		{ eventId: "evt_one", boardId: "flow_a" },
		{ eventId: "evt_three", boardId: "flow_a" },
	];
	expect(await publishLatestFlows(flows, "app", latest)).toEqual([
		{ boardId: "flow_a", version: [0, 0, 8], created: true },
		{ boardId: "flow_b", version: [1, 2, 0], created: false },
	]);
	expect(calls).toEqual(["publish flow_a", "publish flow_b"]);
	expect(await publishLatestFlows(flows, "app", [])).toEqual([]);
});

test("the first flow that can't become a version stops the deploy with its cause and an event that follows it", async () => {
	const stop = async (publish: unknown, current?: [number, number, number]) =>
		caught(() =>
			publishLatestFlows(
				flowsOf({ flow_a: { publish, ...(current ? { current } : {}) } }).flows,
				"app",
				[{ eventId: "evt_one", boardId: "flow_a" }],
				noWait,
			),
		);
	expect(await stop({ kind: "flow_busy" })).toMatchObject({
		problem: "busy",
		at: { boardId: "flow_a", eventId: "evt_one" },
	});
	expect((await stop({ kind: "flow_role" })).problem).toBe("role");
	expect((await stop({ kind: "missing_on_hub" })).problem).toBe("hub");
	const incomparable = await stop({
		kind: "flow_incomparable",
		message: "This flow can't be compared with its published version.",
	});
	expect(incomparable).toMatchObject({
		problem: "incomparable",
		detail: "This flow can't be compared with its published version.",
	});
});

test("online: an event the bundle left out is asked about once more: the flow moved, or the event no longer fits it", async () => {
	const latest = [
		{ eventId: "evt_one", boardId: "flow_a" },
		{ eventId: "evt_two", boardId: "flow_b" },
	];
	const pins: Record<string, [number, number, number]> = {
		evt_one: [0, 0, 8],
		evt_two: [1, 2, 0],
	};
	const all = flowsOf({});
	expect(
		await latestPinsOnline(all.flows, "app", latest, (id) => pins[id]),
	).toEqual(pins);
	expect(all.calls).toEqual([]);
	const moved = flowsOf({ flow_b: { publish: null, current: null } });
	expect(
		await caught(() =>
			latestPinsOnline(moved.flows, "app", latest, (id) =>
				id === "evt_one" ? pins[id] : undefined,
			),
		),
	).toMatchObject({ problem: "moved", at: { eventId: "evt_two" } });
	expect(moved.calls).toEqual(["read flow_b"]);
	const gone = flowsOf({ flow_b: { publish: null, current: [1, 2, 0] } });
	expect(
		(
			await caught(() =>
				latestPinsOnline(gone.flows, "app", latest, (id) =>
					id === "evt_one" ? pins[id] : null,
				),
			)
		).problem,
	).toBe("target");
});

test("local-only: the export says which version each event was pinned to, or why it was left out", async () => {
	const latest = [{ eventId: "evt_one", boardId: "flow_a" }];
	const entry = (
		board_version: [number, number, number] | null,
		problem: "edited" | "target_missing" | null,
	) => [{ event_id: "evt_one", board_version, problem }];
	expect(latestPinsOffline(latest, entry([0, 0, 8], null))).toEqual({
		evt_one: [0, 0, 8],
	});
	expect(
		(await caught(() => latestPinsOffline(latest, entry(null, "edited"))))
			.problem,
	).toBe("moved");
	expect(
		(
			await caught(() =>
				latestPinsOffline(latest, entry(null, "target_missing")),
			)
		).problem,
	).toBe("target");
	// An export that names no Latest event did not pin this one: preparing again is the way.
	expect((await caught(() => latestPinsOffline(latest, []))).problem).toBe(
		"moved",
	);
	expect((await caught(() => latestPinsOffline(latest, null))).problem).toBe(
		"moved",
	);
	expect(latestPinsOffline([], null)).toEqual({});
});
