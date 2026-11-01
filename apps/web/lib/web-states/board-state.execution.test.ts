import {
	BOARD_FORMAT_HEADER,
	CURRENT_BOARD_FORMAT_VERSION,
} from "@flow-like/flow-like-ui/lib/board-format";
import {
	getExecutionRuntime,
	resetExecutionRuntimes,
} from "@flow-like/flow-like-ui/lib/execution/runtime-client";
import { EXECUTION_METADATA } from "@flow-like/flow-like-ui/lib/execution/types";
import { afterEach, describe, expect, test, vi } from "vitest";

vi.mock("@flow-like/flow-like-ui", () => ({
	finishAllProgressToasts: vi.fn(),
}));
vi.mock("../oauth-db", () => ({
	oauthConsentStore: {},
	oauthTokenStore: {},
}));
vi.mock("../oauth-service", () => ({}));
vi.mock(
	"@flow-like/flow-like-ui/state/global-chat/global-chat-tool-registry",
	() => ({ runGlobalChatTool: vi.fn() }),
);
vi.mock(
	"@flow-like/flow-like-ui/state/global-chat/global-chat-web-transport",
	() => ({ dispatchSpecialistToolRequest: vi.fn() }),
);
vi.mock("./api-utils", () => ({
	getApiBaseUrl: () => "https://api.example.test",
}));

import { WebBoardState } from "./board-state";

function boardState() {
	return new WebBoardState({
		auth: {
			isAuthenticated: true,
			user: {
				access_token: "client-token",
				profile: { sub: "viewer", iss: "issuer" },
			},
		},
		profile: { id: "profile-1" },
	} as never);
}

const runStarted =
	'data: {"event_id":"start","event_type":"run_initiated","payload":{"run_id":"run-1"}}\n\n';

function eventStream(body: string) {
	return new Response(body, {
		headers: { "Content-Type": "text/event-stream" },
	});
}

describe("web board execution transport", () => {
	afterEach(async () => {
		await resetExecutionRuntimes();
		vi.unstubAllGlobals();
	});

	test("can invoke a format-v2 board with authentication and the selected profile", async () => {
		const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
			const supported = Number(
				new Headers(init?.headers).get(BOARD_FORMAT_HEADER) ?? 1,
			);
			return supported < 2
				? Response.json(
						{ error: { code: "BOARD_FORMAT_UPGRADE_REQUIRED" } },
						{ status: 426 },
					)
				: new Response(
						'data: {"event_id":"done","event_type":"completed","payload":{"status":"completed"}}\n\n',
						{ headers: { "Content-Type": "text/event-stream" } },
					);
		});
		vi.stubGlobal("fetch", fetchMock);

		await expect(
			boardState().executeBoardRemote("app-1", "board-1", { id: "start" }),
		).resolves.toBeUndefined();

		const [url, init] = fetchMock.mock.calls[0];
		expect(url).toBe(
			"https://api.example.test/api/v1/apps/app-1/board/board-1/invoke",
		);
		const headers = new Headers(init?.headers);
		expect(headers.get(BOARD_FORMAT_HEADER)).toBe(
			String(CURRENT_BOARD_FORMAT_VERSION),
		);
		expect(headers.get("Authorization")).toBe("Bearer client-token");
		expect(JSON.parse(init?.body as string)).toMatchObject({
			node_id: "start",
			profile_id: "profile-1",
		});
	});

	test("preserves the server reason and reference when a board invocation fails", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn(async () =>
				Response.json(
					{
						error: {
							code: "BOARD_FORMAT_UPGRADE_REQUIRED",
							message: "This board requires format version 3.",
							id: "error-1",
						},
					},
					{ status: 426 },
				),
			),
		);

		await expect(
			boardState().executeBoardRemote("app-1", "board-1", { id: "start" }),
		).rejects.toMatchObject({
			status: 426,
			code: "BOARD_FORMAT_UPGRADE_REQUIRED",
			errorId: "error-1",
			serverMessage: expect.stringContaining(
				"This board requires format version 3.",
			),
		});
	});

	test("delivers trailing output in the same network chunk as completion", async () => {
		const events = [
			{ event_type: "completed", payload: {} },
			{ event_type: "chat_out", payload: { text: "Final answer" } },
			{ event_type: "usage", payload: { tokens: 12 } },
		];
		vi.stubGlobal(
			"fetch",
			vi.fn(
				async () =>
					new Response(
						events
							.map((event) => `data: ${JSON.stringify(event)}\n\n`)
							.join(""),
						{ headers: { "content-type": "text/event-stream" } },
					),
			),
		);
		const onEvents = vi.fn();

		await boardState().executeBoardRemote(
			"app-1",
			"board-1",
			{ id: "start" },
			true,
			undefined,
			onEvents,
		);

		expect(onEvents.mock.calls.flatMap(([batch]) => batch)).toMatchObject(
			events,
		);
	});

	test("follows a queued board receipt through the account bus", async () => {
		const calls: string[] = [];
		vi.stubGlobal(
			"fetch",
			vi.fn(async (input: string, init?: RequestInit) => {
				calls.push(String(input));
				if (String(input).endsWith("/connect"))
					return Response.json({
						protocol_version: 1,
						expires_at: Math.floor(Date.now() / 1000) + 900,
						transport: { type: "http" },
					});
				if (String(input).endsWith("/poll")) {
					expect(JSON.parse(String(init?.body)).runs).toEqual([
						{
							run_id: "queued-board",
							poll_token: "run-capability",
							after_sequence: -1,
						},
					]);
					return Response.json({
						protocol_version: 1,
						errors: [],
						runs: [
							{
								run_id: "queued-board",
								revision: "r1",
								status: "completed",
								progress: 100,
								events: [
									{
										sequence: 0,
										event_id: "output",
										event_type: "chat_out",
										payload: { text: "Queued answer" },
										created_at: "2026-11-01T00:00:00Z",
									},
								],
								after_sequence: 0,
								has_more: false,
							},
						],
					});
				}
				return Response.json({
					run_id: "queued-board",
					status: "pending",
					poll_token: "run-capability",
				});
			}),
		);
		const started = vi.fn();
		const events = vi.fn();
		await boardState().executeBoardRemote(
			"app-1",
			"board-1",
			{ id: "start" },
			true,
			started,
			events,
		);
		expect(started).toHaveBeenCalledWith("queued-board");
		expect(events.mock.calls.flatMap(([batch]) => batch)).toContainEqual(
			expect.objectContaining({ event_id: "output", event_type: "chat_out" }),
		);
		expect(calls.filter((url) => url.endsWith("/poll"))).toHaveLength(1);
	});

	test.each([
		[
			"failed",
			{
				event_id: "end",
				event_type: "completed",
				payload: {
					status: "failed",
					error_message: "Node failed",
					log_level: 4,
				},
			},
		],
		[
			"timed out",
			{
				event_id: "end",
				event_type: "completed",
				payload: { status: "timeout" },
			},
		],
		[
			"lost its worker",
			{
				event_id: "end",
				event_type: "error",
				payload: { message: "Runtime.ExitError" },
			},
		],
	])(
		"a run that %s resolves so the board loads its logs by run id",
		async (_, outcome) => {
			vi.stubGlobal(
				"fetch",
				vi.fn(async () =>
					eventStream(`${runStarted}data: ${JSON.stringify(outcome)}\n\n`),
				),
			);
			const started = vi.fn();

			await expect(
				boardState().executeBoardRemote(
					"app-1",
					"board-1",
					{ id: "start" },
					true,
					started,
				),
			).resolves.toBeUndefined();
			expect(started).toHaveBeenCalledWith("run-1");
		},
	);

	test("a stopped run resolves instead of failing the board", async () => {
		const metadata = {
			taskId: "task-1",
			streamId: "task-1",
			scope: "board-stop-test",
			appId: "app-1",
			eventId: "",
			title: "Workflow",
			startedAt: new Date(0).toISOString(),
		};
		const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
			if (init?.method === "DELETE")
				return Response.json({
					run_id: "run-1",
					status: "Cancelled",
					cancelled: true,
				});
			const signal = init?.signal;
			return new Response(
				new ReadableStream<Uint8Array>({
					start(controller) {
						controller.enqueue(new TextEncoder().encode(runStarted));
						signal?.addEventListener("abort", () =>
							controller.error(signal.reason),
						);
					},
				}),
				{ headers: { "Content-Type": "text/event-stream" } },
			);
		});
		vi.stubGlobal("fetch", fetchMock);
		const payload = { id: "start", [EXECUTION_METADATA]: metadata };
		const started = vi.fn();

		const running = boardState().executeBoardRemote(
			"app-1",
			"board-1",
			payload,
			true,
			started,
		);
		await vi.waitFor(() => expect(started).toHaveBeenCalledWith("run-1"));
		await (await getExecutionRuntime(metadata.scope)).cancel("run-1");

		await expect(running).resolves.toBeUndefined();
		expect(fetchMock).toHaveBeenCalledWith(
			"https://api.example.test/api/v1/execution/run/run-1",
			expect.objectContaining({ method: "DELETE" }),
		);
	});

	test("a run whose stream breaks before any outcome still rejects", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => eventStream(runStarted)),
		);
		const started = vi.fn();

		await expect(
			boardState().executeBoardRemote(
				"app-1",
				"board-1",
				{ id: "start" },
				true,
				started,
			),
		).rejects.toThrow();
		expect(started).toHaveBeenCalledWith("run-1");
	});
});
