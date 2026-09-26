import { describe, expect, test } from "bun:test";
import type { IIntercomEvent } from "@flow-like/flow-like-ui/lib/schema/events/intercom-event";
import {
	type Inventory,
	type PageBootstrap,
	createServiceBackend,
} from "./backend";
import { consumeServiceStream, createServiceRequest } from "./transport";

function stream(text: string, size = 1) {
	const bytes = new TextEncoder().encode(text);
	return new ReadableStream<Uint8Array>({
		start(controller) {
			for (let at = 0; at < bytes.length; at += size)
				controller.enqueue(bytes.slice(at, at + size));
			controller.close();
		},
	});
}

describe("standalone service transport", () => {
	test("scopes the in-memory bearer to local service paths and refuses redirects", async () => {
		const sent: Array<{ path: string; init: RequestInit }> = [];
		const request = createServiceRequest("t".repeat(32), (async (
			path,
			init,
		) => {
			sent.push({ path: String(path), init: init ?? {} });
			return new Response("{}");
		}) as typeof fetch);
		await request("/services");
		expect(sent[0].init.credentials).toBe("omit");
		expect(sent[0].init.redirect).toBe("error");
		expect(new Headers(sent[0].init.headers).get("Authorization")).toBe(
			`Bearer ${"t".repeat(32)}`,
		);
		for (const path of [
			"https://other.example/services",
			"//other.example/services",
			"/services?token=anything",
			"/pages/../bootstrap",
			"/instances/project/storage/files",
			"/channels/run",
		])
			await expect(request(path)).rejects.toThrow("Unsupported");
		expect(sent).toHaveLength(1);
	});
	test("preserves fragmented SSE payloads and requires explicit successful completion", async () => {
		const events: IIntercomEvent[] = [];
		const ids: string[] = [];
		await consumeServiceStream(
			stream(
				'event: run_initiated\r\ndata: {"run_id":"run"}\r\n\r\nevent: chat_stream_partial\ndata: {"text":" héllo "}\n\nevent: done\ndata: {"completed":true}\n\n',
			),
			(batch) => events.push(...batch),
			(id) => ids.push(id),
		);
		expect(ids).toEqual(["run"]);
		expect(events[1].payload.text).toBe(" héllo ");
		expect(events[2].event_type).toBe("completed");
		expect(new Set(events.map((e) => e.event_id)).size).toBe(3);
		await expect(
			consumeServiceStream(
				stream('event: chat_stream_partial\ndata: "partial"\n\n'),
			),
		).rejects.toThrow("before the workflow completed");
		await expect(
			consumeServiceStream(
				stream('event: error\ndata: {"completed":false}\n\n'),
			),
		).rejects.toThrow("workflow failed");
	});
	test("the renderer adapter never exchanges raw board targets or stale Page grants", async () => {
		const event = {
			id: "event",
			active: true,
			board_id: "board",
			board_version: [1, 0, 0],
			event_version: [1, 0, 0],
			default_page_id: "page",
			event_type: "page",
			name: "Page",
			description: "",
			node_id: "",
			variables: {},
			inputs: [],
			config: [],
			created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
			updated_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
			priority: 0,
		};
		const inventory: Inventory = { project_id: "project", events: [event] };
		const page = {
			project_id: "project",
			event_id: "event",
			event,
			page: { id: "page", components: [] },
			execution_revision: "revision",
			element_demand: { selectors: [], dynamic: false },
		} as unknown as PageBootstrap;
		const posted: Array<{ path: string; body: unknown }> = [];
		const request = createServiceRequest("t".repeat(32), (async (
			path,
			init,
		) => {
			if (init?.method === "POST") {
				posted.push({
					path: String(path),
					body: JSON.parse(String(init.body)),
				});
				return new Response(
					stream('event: done\ndata: {"completed":true}\n\n'),
				);
			}
			return Response.json(page);
		}) as typeof fetch);
		const { backend } = createServiceBackend(
			inventory,
			request,
			new AbortController().signal,
		);
		expect(backend.eventState.checkEventOAuth).toBeUndefined();
		const payload = { id: "caller-selected-node", payload: { value: 2 } };
		await expect(
			backend.eventState.executeEvent("other", "event", payload),
		).rejects.toThrow("another project");
		await expect(
			backend.eventState.executeEvent("project", "event", payload),
		).rejects.toThrow("requires a Page action");
		await expect(
			backend.eventState.executeEvent(
				"project",
				"event",
				payload,
				false,
				undefined,
				undefined,
				false,
				{ kind: "action", actionId: "action", manifestRevision: "stale" },
			),
		).rejects.toThrow("stale");
		await expect(
			backend.eventState.executeEvent(
				"project",
				"event",
				payload,
				false,
				undefined,
				undefined,
				false,
				{
					kind: "action",
					actionId: "action",
					manifestRevision: "revision",
					capabilityJwt: "foreign",
				},
			),
		).rejects.toThrow("unsupported");
		expect(posted).toHaveLength(0);
		await backend.eventState.executeEvent(
			"project",
			"event",
			payload,
			false,
			undefined,
			undefined,
			false,
			{ kind: "action", actionId: "action", manifestRevision: "revision" },
		);
		expect(posted).toEqual([
			{
				path: "/pages/event/invoke",
				body: {
					manifest_revision: "revision",
					trigger: { kind: "action", action_id: "action" },
					payload: { value: 2 },
				},
			},
		]);
		const dynamic = {
			kind: "action" as const,
			actionId: `da1_${"a".repeat(32)}`,
			manifestRevision: "revision",
			capabilityJwt: `sac1.${"b".repeat(32)}.${"c".repeat(43)}`,
		};
		await backend.eventState.executeEvent(
			"project",
			"event",
			payload,
			false,
			undefined,
			undefined,
			false,
			dynamic,
		);
		expect(posted[1]).toEqual({
			path: "/pages/event/invoke",
			body: {
				manifest_revision: "revision",
				trigger: {
					kind: "action",
					action_id: dynamic.actionId,
					capability_jwt: dynamic.capabilityJwt,
				},
				payload: { value: 2 },
			},
		});
	});
});
