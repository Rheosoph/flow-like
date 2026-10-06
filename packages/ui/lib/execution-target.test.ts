import { afterEach, describe, expect, test } from "bun:test";
import type { IPrerunEventResponse } from "../state/backend-state";
import { invalidatePrerunCache } from "../state/backend-state/prerun-cache";
import {
	type ExecutionTargetBackend,
	executionTargetOf,
	resolveExecutionTarget,
} from "./execution-target";
import { IExecutionMode } from "./schema/flow/board";
import { IEventExecutionMode } from "./schema/flow/event";

const LOCAL_PRERUN: IPrerunEventResponse = {
	board_id: "board-1",
	runtime_variables: [],
	oauth_requirements: [],
	requires_local_execution: false,
	execution_mode: IExecutionMode.Hybrid,
	event_execution_mode: IEventExecutionMode.Local,
	can_execute_locally: true,
};

interface FakeBackend extends ExecutionTargetBackend {
	readonly calls: string[];
}

function backendWith(
	answer: () => Promise<IPrerunEventResponse>,
	alwaysRemote = false,
): FakeBackend {
	const calls: string[] = [];
	return {
		calls,
		eventState: {
			alwaysRemote,
			prerunEvent: (appId: string, eventId: string) => {
				calls.push(`${appId}/${eventId}`);
				return answer();
			},
		},
	};
}

afterEach(() => invalidatePrerunCache());

describe("executionTargetOf", () => {
	test.each([
		["a board this device may run", LOCAL_PRERUN, "local"],
		[
			"a viewer without ReadBoards",
			{ ...LOCAL_PRERUN, can_execute_locally: false },
			"remote",
		],
		[
			"a board pinned to Remote",
			{ ...LOCAL_PRERUN, execution_mode: IExecutionMode.Remote },
			"remote",
		],
		[
			"an event pinned to Remote",
			{ ...LOCAL_PRERUN, event_execution_mode: IEventExecutionMode.Remote },
			"remote",
		],
		[
			"a board pinned to Local",
			{ ...LOCAL_PRERUN, execution_mode: IExecutionMode.Local },
			"local",
		],
		[
			"a prerun without its local flag",
			{ execution_mode: IExecutionMode.Hybrid },
			"remote",
		],
		["no prerun", null, "remote"],
	] as const)("%s runs %s", (_label, prerun, target) => {
		expect(executionTargetOf(false, prerun)).toBe(target);
	});

	test("a backend that always runs remotely wins over a local prerun", () => {
		expect(executionTargetOf(true, LOCAL_PRERUN)).toBe("remote");
	});

	test("a malformed answer is the cloud", () => {
		expect(executionTargetOf(false, "local" as never)).toBe("remote");
	});
});

describe("resolveExecutionTarget", () => {
	test("the web never asks for a prerun", async () => {
		const backend = backendWith(async () => LOCAL_PRERUN, true);
		expect(await resolveExecutionTarget(backend, "app", "event")).toBe(
			"remote",
		);
		expect(backend.calls).toEqual([]);
	});

	test("a desktop prerun that allows local runs keeps the run here", async () => {
		const backend = backendWith(async () => LOCAL_PRERUN);
		expect(await resolveExecutionTarget(backend, "app", "event")).toBe("local");
		expect(backend.calls).toEqual(["app/event"]);
	});

	test("an event pinned to Remote goes to the cloud", async () => {
		const backend = backendWith(async () => ({
			...LOCAL_PRERUN,
			event_execution_mode: IEventExecutionMode.Remote,
		}));
		expect(await resolveExecutionTarget(backend, "app", "event")).toBe(
			"remote",
		);
	});

	test("a failed prerun goes to the cloud", async () => {
		const backend = backendWith(async () => {
			throw new Error("Hub unavailable");
		});
		expect(await resolveExecutionTarget(backend, "app", "event")).toBe(
			"remote",
		);
	});

	test("a prerun that throws synchronously goes to the cloud", async () => {
		const backend: ExecutionTargetBackend = {
			eventState: {
				prerunEvent: () => {
					throw new Error("not available on this page");
				},
			},
		};
		expect(await resolveExecutionTarget(backend, "app", "event")).toBe(
			"remote",
		);
	});

	test("a backend without a prerun goes to the cloud", async () => {
		const backend: ExecutionTargetBackend = { eventState: {} };
		expect(await resolveExecutionTarget(backend, "app", "event")).toBe(
			"remote",
		);
	});

	test("the answer is shared with the execution service's prerun cache", async () => {
		const backend = backendWith(async () => LOCAL_PRERUN);
		await resolveExecutionTarget(backend, "app", "event");
		await resolveExecutionTarget(backend, "app", "event");
		await resolveExecutionTarget(backend, "app", "other");
		expect(backend.calls).toEqual(["app/event", "app/other"]);
	});

	test("the prerun is called on the event state", async () => {
		const seen: unknown[] = [];
		const eventState = {
			prerunEvent(this: unknown) {
				seen.push(this);
				return Promise.resolve(LOCAL_PRERUN);
			},
		};
		await resolveExecutionTarget({ eventState }, "app", "event");
		expect(seen).toEqual([eventState]);
	});
});
