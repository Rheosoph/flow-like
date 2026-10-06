import { describe, expect, test } from "bun:test";
import type { IIntercomEvent } from "../schema/events/intercom-event";
import type { IEvent } from "../schema/flow/event";
import {
	type Inventory,
	createServiceBackend,
	parseServiceInventory,
	readServiceInventory,
} from "./backend";
import { createServiceRequest } from "./transport";

const event = (more: Partial<IEvent> = {}) =>
	({
		id: "event",
		name: "Deployed interface",
		event_type: "simple_chat",
		board_id: "board",
		node_id: "",
		event_version: [1, 0, 0],
		board_version: [2, 0, 0],
		...more,
	}) as IEvent;

const inventory = (more: Partial<IEvent> = {}): Inventory => ({
	project_id: "project",
	events: [event(more)],
});
const lifetime = () => new AbortController();
const frame = (type: string, payload: unknown) =>
	`event: ${type}\ndata: ${JSON.stringify(payload)}\n\n`;
const completed = () => new Response(frame("done", { completed: true }));

describe("deployed service runtime backend", () => {
	test("validates inventory identity, duplicates and pinned event versions", async () => {
		for (const value of [
			null,
			{ project_id: "../project", events: [] },
			{ project_id: "project", events: [event(), event()] },
			inventory({ id: ".." }),
			inventory({ event_version: [1] }),
			inventory({ config: [256] }),
		])
			expect(() => parseServiceInventory(value)).toThrow("invalid inventory");
		const rest = parseServiceInventory({
			project_id: "project",
			events: [{ ...event(), route: { method: "GET", path: "/report" } }],
		});
		expect(rest.events[0].route).toBeNull();
		expect(rest.events[0].service_route).toEqual({
			method: "GET",
			path: "/report",
		});
		const request = createServiceRequest(null, async () =>
			Response.json(inventory()),
		);
		expect(await readServiceInventory(request, lifetime().signal)).toEqual(
			inventory(),
		);
		const { backend } = createServiceBackend(
			inventory(),
			request,
			lifetime().signal,
			{ visibleAppId: "device-runtime:session" },
		);
		await expect(
			backend.eventState.getEvent("project", "event"),
		).rejects.toThrow("another project");
		expect(
			(
				await backend.eventState.getEvent(
					"device-runtime:session",
					"event",
					[1, 0, 0],
				)
			).id,
		).toBe("event");
		await expect(
			backend.eventState.getEvent("device-runtime:session", "event", [1, 0, 1]),
		).rejects.toThrow("version is not deployed");
		expect(backend.eventState.alwaysRemote).toBe(true);
		expect(backend.helperState.fileToTemporaryFile).toBeUndefined();
		expect(backend.helperState.filesToTemporaryFiles).toBeUndefined();
	});

	test("binds Page bootstrap to project, event, page and both deployed versions", async () => {
		const inv = inventory({
			event_type: "page",
			default_page_id: "page",
			route: "/",
		});
		const good = {
			project_id: "project",
			event_id: "event",
			event: inv.events[0],
			page: { id: "page", components: [] },
			execution_revision: "revision",
			element_demand: { selectors: [], dynamic: false },
		};
		let answer: unknown = good;
		const request = createServiceRequest(null, async () =>
			Response.json(answer),
		);
		let mapped = 0;
		const { backend, bootstrap } = createServiceBackend(
			inv,
			request,
			lifetime().signal,
			{
				visibleAppId: "device-runtime:session",
				mapValue: async (value) => {
					mapped++;
					return value;
				},
			},
		);
		expect(
			(await bootstrap("device-runtime:session", "event")).project_id,
		).toBe("project");
		for (const bad of [
			{ ...good, project_id: "elsewhere" },
			{ ...good, event_id: "foreign" },
			{ ...good, page: { id: "foreign", components: [] } },
			...[
				{ id: "foreign" },
				{ board_id: "foreign" },
				{ node_id: "foreign" },
				{ event_version: [1, 0, 1] },
				{ board_version: [2, 0, 1] },
			].map((changed) => ({ ...good, event: { ...good.event, ...changed } })),
		]) {
			answer = bad;
			await expect(
				bootstrap("device-runtime:session", "event"),
			).rejects.toThrow("does not match");
		}
		expect(mapped).toBe(1);
		answer = good;
		await expect(
			backend.pageState.getPageBootstrap?.("foreign", "/"),
		).rejects.toThrow("another project");
	});

	test("maps chat, rich Page and form results before delivering ordered events", async () => {
		const sent: { path: string; body: unknown }[] = [];
		const request = createServiceRequest(null, async (path, init) => {
			sent.push({ path: String(path), body: JSON.parse(String(init?.body)) });
			return new Response(
				[
					frame("run_initiated", {
						run_id: "run",
						channel: { url: "/channels/run" },
					}),
					frame("a2ui", {
						surfaceUpdate: { src: "/ui/assets/capability_handle" },
					}),
					frame("chat_stream_partial", { text: "Hello" }),
					frame("generic_result", { result: 3 }),
					frame("done", { completed: true }),
				].join(""),
			);
		});
		const { backend } = createServiceBackend(
			inventory(),
			request,
			lifetime().signal,
			{
				mapValue: async (value) => {
					await Promise.resolve();
					return { ...(value as object), mapped: true };
				},
			},
		);
		const results: IIntercomEvent[] = [];
		await backend.eventState.executeEvent(
			"project",
			"event",
			{ id: "ignored", payload: { message: "hi" } },
			false,
			undefined,
			(events) => results.push(...events),
		);
		expect(sent).toEqual([{ path: "/chat/event", body: { message: "hi" } }]);
		expect(results.map((item) => item.event_type)).toEqual([
			"run_initiated",
			"a2ui",
			"chat_stream_partial",
			"generic_result",
			"completed",
		]);
		expect(results.slice(0, -1).every((item) => item.payload.mapped)).toBe(
			true,
		);
	});

	test("cancels the active remote response without a management command", async () => {
		let cancelled = 0;
		let requestedSignal: AbortSignal | undefined;
		const request = createServiceRequest(null, async (_path, init) => {
			requestedSignal = init?.signal as AbortSignal;
			return new Response(
				new ReadableStream<Uint8Array>({
					start(controller) {
						controller.enqueue(
							new TextEncoder().encode(
								frame("run_initiated", { run_id: "run" }),
							),
						);
					},
					cancel() {
						cancelled++;
					},
				}),
			);
		});
		const { backend } = createServiceBackend(
			inventory(),
			request,
			lifetime().signal,
		);
		let ready!: () => void;
		const started = new Promise<void>((resolve) => {
			ready = resolve;
		});
		const execution = backend.eventState.executeEvent(
			"project",
			"event",
			{ id: "", payload: {} },
			false,
			ready,
		);
		const rejected = execution.catch((error: unknown) => error);
		await started;
		await backend.eventState.cancelExecution("run");
		expect(await rejected).toMatchObject({ name: "AbortError" });
		expect(requestedSignal?.aborted).toBe(true);
		expect(cancelled).toBe(1);
	});

	test("keeps storage reads and execution within the live renderer namespace", async () => {
		const controller = lifetime();
		const resolved: string[] = [];
		const request = createServiceRequest(null, async () => completed());
		const { backend } = createServiceBackend(
			inventory(),
			request,
			controller.signal,
			{
				visibleAppId: "device-runtime:session",
				resolveResource: async (path) => {
					resolved.push(path);
					return `blob:${path}`;
				},
			},
		);
		await expect(
			backend.storageState.downloadStorageItems("project", ["image.png"]),
		).rejects.toThrow("another project");
		expect(
			await backend.storageState.downloadStorageItems(
				"device-runtime:session",
				["image.png"],
			),
		).toEqual([{ prefix: "image.png", url: "blob:image.png" }]);
		controller.abort();
		await expect(
			backend.eventState.executeEvent("device-runtime:session", "event", {
				id: "",
			}),
		).rejects.toMatchObject({ name: "AbortError" });
		await expect(
			backend.storageState.downloadStorageItems("device-runtime:session", [
				"later.png",
			]),
		).rejects.toMatchObject({ name: "AbortError" });
		expect(resolved).toEqual(["image.png"]);
	});
});
