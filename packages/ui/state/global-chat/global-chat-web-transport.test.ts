import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	HomeProfileRunError,
	HomeProfileRuns,
	assertHomeProfileRun,
} from "../../components/global-chat/tools/home/profile-run";

const mocks = vi.hoisted(() => ({
	replyToChannel: vi.fn(async () => undefined),
}));

vi.mock("../../lib/channel", () => ({
	cancelChannel: vi.fn(async () => undefined),
	isChannelHandle: vi.fn(() => true),
	replyToChannel: mocks.replyToChannel,
	steerChannel: vi.fn(async () => undefined),
}));

import {
	type WebToolRequest,
	dispatchSpecialistToolRequest,
	webGlobalChatStart,
} from "./global-chat-web-transport";

function toolRequest(parentRequestId?: string) {
	return JSON.stringify({
		requestId: "nested-tool-1",
		toolName: "apply_home_layout",
		arguments: { layout: { version: 1, widgets: [] } },
		channel: {
			channel_id: "specialist-channel",
			request_id: "nested-tool-1",
			expires_at: 4_000_000_000,
			transport: { type: "in_process" },
		},
		...(parentRequestId ? { parentRequestId } : {}),
	});
}

describe("dispatchSpecialistToolRequest", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	it("stamps the known outer request onto hosted specialist tools", async () => {
		const onToolRequest = vi.fn(async (request: WebToolRequest) => ({
			requestId: request.requestId,
			approved: true,
			result: { status: "staged" },
		}));

		await dispatchSpecialistToolRequest({
			data: toolRequest(),
			parentRequestId: "outer-home-1",
			onToolRequest,
		});

		expect(onToolRequest).toHaveBeenCalledWith(
			expect.objectContaining({
				requestId: "nested-tool-1",
				parentRequestId: "outer-home-1",
			}),
		);
		expect(mocks.replyToChannel).toHaveBeenCalledOnce();
	});

	it("keeps an authoritative parent already carried by the frame", async () => {
		const onToolRequest = vi.fn(async (request: WebToolRequest) => ({
			requestId: request.requestId,
			approved: true,
		}));

		await dispatchSpecialistToolRequest({
			data: toolRequest("server-parent"),
			parentRequestId: "fallback-parent",
			onToolRequest,
		});

		expect(onToolRequest).toHaveBeenCalledWith(
			expect.objectContaining({ parentRequestId: "server-parent" }),
		);
	});

	it("pins Home ownership to the launching host even if a frame carries different metadata", async () => {
		const onToolRequest = vi.fn(async (request: WebToolRequest) => ({
			requestId: request.requestId,
			approved: true,
		}));
		await dispatchSpecialistToolRequest({
			data: JSON.stringify({
				...JSON.parse(toolRequest("wrong-parent")),
				context: { profileId: "profile-b" },
			}),
			parentRequestId: "home-parent",
			profileId: "profile-a",
			onToolRequest,
		});
		expect(onToolRequest).toHaveBeenCalledWith(
			expect.objectContaining({
				parentRequestId: "home-parent",
				context: expect.objectContaining({ profileId: "profile-a" }),
			}),
		);
	});

	it("pins forecast fallback app and profile to the specialist launch", async () => {
		const onToolRequest = vi.fn(async (request: WebToolRequest) => ({
			requestId: request.requestId,
			approved: true,
		}));
		await dispatchSpecialistToolRequest({
			data: JSON.stringify({
				...JSON.parse(toolRequest()),
				toolName: "forecast_tool",
				arguments: { operation: "list" },
				context: { appId: "wrong-app", profileId: "wrong-profile" },
			}),
			appId: "launch-app",
			profileId: "launch-profile",
			parentRequestId: "data-studio-parent",
			onToolRequest,
		});
		expect(onToolRequest).toHaveBeenCalledWith(
			expect.objectContaining({
				context: expect.objectContaining({
					appId: "launch-app",
					profileId: "launch-profile",
				}),
			}),
		);
	});

	it.each([
		{
			name: "executes Data Studio reads in their launching profile",
			parentRequestId: "data-studio-parent",
			currentProfileId: "profile-a",
			expectedCode: undefined,
		},
		{
			name: "blocks Data Studio reads after the profile changes",
			parentRequestId: "data-studio-parent",
			currentProfileId: "profile-b",
			expectedCode: "home_profile_changed",
		},
		{
			name: "blocks late reads from a completed Home run",
			parentRequestId: "home-parent",
			currentProfileId: "profile-a",
			expectedCode: "home_run_expired",
		},
	])("$name", async ({ parentRequestId, currentProfileId, expectedCode }) => {
		const runs = new HomeProfileRuns();
		runs.begin("home-parent", "profile-a");
		runs.finish("home-parent");
		const listTables = vi.fn(async () => ({
			status: "ok",
			tables: [{ name: "contacts" }],
		}));

		await dispatchSpecialistToolRequest({
			data: JSON.stringify({
				...JSON.parse(toolRequest()),
				toolName: "database_tool",
				arguments: { operation: "list_tables", app_id: "office" },
			}),
			parentRequestId,
			profileId: "profile-a",
			onToolRequest: async (request) => {
				try {
					await assertHomeProfileRun(
						runs,
						{
							parentRequestId: request.parentRequestId,
							profileId: request.context?.profileId,
						},
						async () => currentProfileId,
						() => currentProfileId,
					);
					return {
						requestId: request.requestId,
						approved: true,
						result: await listTables(),
					};
				} catch (error) {
					if (!(error instanceof HomeProfileRunError)) throw error;
					return {
						requestId: request.requestId,
						approved: true,
						result: error.result,
					};
				}
			},
		});

		expect(listTables).toHaveBeenCalledTimes(expectedCode ? 0 : 1);
		expect(mocks.replyToChannel).toHaveBeenCalledWith(
			expect.anything(),
			expect.objectContaining({
				approved: true,
				result: expectedCode
					? expect.objectContaining({ status: "stale", code: expectedCode })
					: { status: "ok", tables: [{ name: "contacts" }] },
			}),
			expect.anything(),
		);
	});
});

describe("webGlobalChatStart model report", () => {
	const channel = {
		channel_id: "run-channel",
		request_id: "run-1",
		expires_at: 4_000_000_000,
		transport: { type: "in_process" },
	};

	async function runWith(runFrame: Record<string, unknown>) {
		const body = [
			`event: run\ndata: ${JSON.stringify(runFrame)}\n\n`,
			`event: final\ndata: ${JSON.stringify({ message: "ok" })}\n\n`,
		].join("");
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => new Response(body, { status: 200 })),
		);
		const reported: string[] = [];
		await webGlobalChatStart({ baseUrl: "https://api.test", body: {} })(
			() => undefined,
			(modelId) => reported.push(modelId),
		);
		return reported;
	}

	it("reports the model id of the run frame", async () => {
		expect(
			await runWith({ runId: "run-1", channel, modelId: "bit-7" }),
		).toEqual(["bit-7"]);
	});

	it("reports nothing for a run frame from an older server", async () => {
		expect(await runWith({ runId: "run-1", channel })).toEqual([]);
	});
});
