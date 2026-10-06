import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import { sampleFleet } from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import type { DeviceServiceStream } from "../../../../lib/device-management/tunnel";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";

const dom = installDom();
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { STUDIO, LAB, commandsOf, openTab, patchConfig, text, until, writes } =
	await import("./config-test-kit");
const { fakeDeviceApi } = await import("../testing/fake-device-api");
type Kit = typeof import("./config-test-kit");
type View = Awaited<ReturnType<Kit["openTab"]>>;
type OpenService = View["fake"]["workspace"]["live"]["openService"];

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	localStorage.clear();
});
afterAll(dom.restore);

class ScriptedService {
	private readonly chunks: (Uint8Array | null)[] = [];
	private pending?: {
		resolve: (bytes: Uint8Array | null) => void;
		reject: (error: Error) => void;
	};
	private complete!: () => void;
	private failure?: Error;
	readonly closed = new Promise<void>((resolve) => {
		this.complete = resolve;
	});
	readonly writes: Uint8Array[] = [];
	readCalls = 0;
	resets = 0;
	finished = false;
	read(): Promise<Uint8Array | null> {
		this.readCalls++;
		if (this.failure) return Promise.reject(this.failure);
		if (this.chunks.length) return Promise.resolve(this.chunks.shift() ?? null);
		return new Promise((resolve, reject) => {
			this.pending = { resolve, reject };
		});
	}
	push(text: string) {
		const bytes = new TextEncoder().encode(text);
		if (this.pending) {
			const reader = this.pending;
			this.pending = undefined;
			reader.resolve(bytes);
		} else this.chunks.push(bytes);
	}
	async write(bytes: Uint8Array, progress?: () => void) {
		this.writes.push(bytes.slice());
		progress?.();
	}
	async end() {}
	reset() {
		if (this.failure) return;
		this.resets++;
		this.failure = new Error("The service stream was reset.");
		this.pending?.reject(this.failure);
		this.pending = undefined;
		this.chunks.length = 0;
		this.complete();
	}
	asStream() {
		return this as unknown as DeviceServiceStream;
	}
}

async function open(options: Partial<Parameters<Kit["openTab"]>[0]> = {}) {
	const view = await openTab({
		tab: "endpoint",
		device: STUDIO,
		service: "field-notes",
		platform: "web",
		...options,
	});
	await until(
		() => queryByRole("button", "Send request", view.container) !== null,
	);
	return view;
}

async function scripted() {
	const service = new ScriptedService();
	const opens: Parameters<OpenService>[] = [];
	const view = await open({
		arrange(fake) {
			fake.workspace.live.openService = async (...args) => {
				opens.push(args);
				return service.asStream();
			};
		},
	});
	return { view, service, opens };
}

const field = (view: View, name: string) =>
	byRole("textbox", name, view.container) as
		| HTMLInputElement
		| HTMLTextAreaElement;
const response = (view: View) =>
	view.container.querySelector<HTMLElement>("[data-tunnel-response]");

describe("Endpoint service tunnel requests", () => {
	test("caps displayed text at the latest 1 MiB while draining the rest through EOF", async () => {
		const { view, service } = await scripted();
		const prefix = "old-response-start\n";
		const part = "x".repeat(16_000);
		const suffix = "\nlatest-response-end";
		const count = 80;
		const length = prefix.length + part.length * count + suffix.length;
		await click(byRole("button", "Send request", view.container));
		await act(async () => {
			service.push(
				`HTTP/1.1 200 OK\r\nContent-Length: ${length}\r\n\r\n${prefix}`,
			);
			for (let index = 0; index < count; index++) service.push(part);
		});
		await until(() =>
			text(view.container).includes("Showing the most recent response text"),
		);
		expect(response(view)?.textContent?.length).toBe(1024 * 1024);
		expect(response(view)?.textContent).not.toContain(prefix);
		expect(service.resets).toBe(0);
		expect(
			queryByRole("button", "Stop request", view.container),
		).not.toBeNull();
		await act(async () => service.push(suffix));
		await until(() => response(view)?.textContent?.endsWith(suffix) ?? false);
		expect(response(view)?.textContent?.length).toBe(1024 * 1024);
		expect(service.readCalls).toBeGreaterThanOrEqual(count + 2);
		expect(queryByRole("button", "Stop request", view.container)).toBeNull();
		expect(service.resets).toBe(1);
	});

	test("service-connect-only access discovers listeners and sends without Deploy or Status", async () => {
		const seed = sampleFleet();
		const grant = seed.myAccess?.[LAB]?.grants[0];
		const placements = seed.live[STUDIO]?.inspection?.value.placements;
		if (!grant || !placements)
			throw new Error("The shared-device fixture is incomplete.");
		grant.capabilities = ["service_connect"];
		grant.scope = { kind: "device" };
		seed.live[LAB] = { state: { kind: "idle" } };
		const api = fakeDeviceApi({ seed });
		const service = new ScriptedService();
		const opens: Parameters<OpenService>[] = [];
		const view = await openTab({
			tab: "endpoint",
			device: LAB,
			service: "field-notes",
			platform: "web",
			api,
			unlock: [LAB],
			arrange(fake) {
				fake.agent(LAB).placements = structuredClone(placements);
				fake.agent(LAB).configs.set("field-notes", {
					hosting: { host: "127.0.0.1", port: 8090 },
					variables: { private_setting: "must-not-be-disclosed" },
				});
				fake
					.agent(LAB)
					.reject(
						"placement_configuration",
						"unauthorized",
						"Deploy capability required.",
					);
				fake
					.agent(LAB)
					.reject(
						"inspect_page",
						"unauthorized",
						"Status capability required.",
					);
				fake.workspace.live.openService = async (...args) => {
					opens.push(args);
					return service.asStream();
				};
			},
		});
		await until(
			() => queryByRole("button", "Send request", view.container) !== null,
		);
		expect(view.fake.hub.myAccess(LAB).grants[0]?.capabilities).toEqual([
			"service_connect",
		]);
		expect(commandsOf(view, "service_listeners")).toContainEqual({
			type: "service_listeners",
			placement_id: "field-notes",
		});
		expect(queryByRole("button", "Add listener", view.container)).toBeNull();
		expect(text(view.container)).not.toContain("must-not-be-disclosed");
		await click(byRole("button", "Send request", view.container));
		await act(async () =>
			service.push("HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nallowed"),
		);
		await until(() => response(view)?.textContent === "allowed");
		expect(opens[0]?.slice(0, 3)).toEqual([LAB, "field-notes", "hosting"]);
		expect(writes(view)).toEqual([]);
	});

	test("shows streamed text before completion, escapes HTML and stops the active stream", async () => {
		const { view, service, opens } = await scripted();
		await typeInto(field(view, "Request path"), "/events?limit=1");
		await typeInto(
			field(view, "Request headers"),
			"Authorization: Bearer local-token",
		);
		await click(byRole("button", "Send request", view.container));
		expect(opens).toHaveLength(1);
		expect(opens[0]?.slice(0, 3)).toEqual([STUDIO, "field-notes", "hosting"]);
		expect(opens[0]?.[3]?.mode).toBe("http");
		await act(async () =>
			service.push(
				"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: first\n\n",
			),
		);
		await until(
			() => response(view)?.textContent?.includes("data: first") ?? false,
		);
		expect(service.resets).toBe(0);
		expect(
			queryByRole("button", "Stop request", view.container),
		).not.toBeNull();
		const html = '<img src=x onerror="alert(1)"><script>alert(2)</script>';
		await act(async () => service.push(html));
		await until(() => response(view)?.textContent?.includes(html) ?? false);
		expect(response(view)?.querySelector("img,script,iframe")).toBeNull();
		expect(response(view)?.innerHTML).toContain("&lt;img");
		const request = service.writes
			.map((bytes) => new TextDecoder().decode(bytes))
			.join("");
		expect(request).toContain("GET /events?limit=1 HTTP/1.1\r\n");
		expect(request).toContain("authorization: Bearer local-token\r\n");
		await click(byRole("button", "Stop request", view.container));
		await until(() => service.resets === 1);
		expect(opens[0]?.[3]?.signal?.aborted).toBe(true);
		expect(queryByRole("button", "Stop request", view.container)).toBeNull();
		expect(text(view.container)).toContain("cancelled");
		expect(response(view)?.textContent).toContain("data: first");
		expect(writes(view)).toEqual([]);
	});

	test("locking keys resets the stream and clears request credentials, body and response", async () => {
		const { view, service, opens } = await scripted();
		await typeInto(
			field(view, "Request headers"),
			"Authorization: Bearer private-token",
		);
		await typeInto(field(view, "Request body"), '{"private":"request-body"}');
		await typeInto(field(view, "Request path"), "/private/path");
		await click(byRole("button", "Send request", view.container));
		await act(async () =>
			service.push(
				"HTTP/1.1 200 OK\r\nX-Private: response-header\r\n\r\nprivate-response",
			),
		);
		await until(() => response(view)?.textContent === "private-response");
		await act(async () => view.fake.workspace.keys.lock(STUDIO));
		await until(() => service.resets === 1);
		expect(opens[0]?.[3]?.signal?.aborted).toBe(true);
		expect(text(view.container)).not.toContain("private-response");
		expect(text(view.container)).not.toContain("response-header");
		await act(async () => view.fake.unlock(STUDIO, { connectLive: true }));
		await until(
			() => queryByRole("textbox", "Request headers", view.container) !== null,
		);
		expect(field(view, "Request headers").value).toBe("");
		expect(field(view, "Request body").value).toBe("");
		expect(field(view, "Request path").value).toBe("/");
		expect(response(view)).toBeNull();
		expect(
			view.container.querySelector("[data-tunnel-response-head]"),
		).toBeNull();
	});

	test("an aborted response cannot republish after lock and unlock notifications in one turn", async () => {
		const service = new ScriptedService();
		const listeners = new Set<() => void>();
		let locked = false;
		const view = await open({
			arrange(fake) {
				fake.workspace.live.openService = async () => service.asStream();
				const keys = fake.workspace.keys;
				const snapshot = keys.snapshot.bind(keys);
				const subscribe = keys.subscribe.bind(keys);
				const lockedSnapshot = {
					...snapshot(STUDIO),
					state: "locked" as const,
				};
				keys.snapshot = (deviceId) =>
					locked && deviceId === STUDIO ? lockedSnapshot : snapshot(deviceId);
				keys.subscribe = (listener) => {
					listeners.add(listener);
					const off = subscribe(listener);
					return () => {
						listeners.delete(listener);
						off();
					};
				};
			},
		});
		await typeInto(
			field(view, "Request headers"),
			"Authorization: Bearer old-credential",
		);
		await click(byRole("button", "Send request", view.container));
		await act(async () => service.push("HTTP/1.1 200 OK\r\n\r\nold-response"));
		await until(() => response(view)?.textContent === "old-response");
		await act(async () => {
			locked = true;
			for (const listener of listeners) listener();
			locked = false;
			for (const listener of listeners) listener();
		});
		expect(service.resets).toBe(1);
		expect(field(view, "Request headers").value).toBe("");
		expect(response(view)).toBeNull();
		expect(text(view.container)).not.toContain("old-response");
		expect(text(view.container)).not.toContain("cancelled");
		expect(queryByRole("button", "Stop request", view.container)).toBeNull();
	});

	test("web Studio explains desktop local ports without showing a port action", async () => {
		const view = await open();
		expect(text(view.container)).toContain(
			"Desktop Studio can also expose a local port",
		);
		expect(queryByRole("button", "Open local port", view.container)).toBeNull();
		expect(queryByRole("textbox", "Local port", view.container)).toBeNull();
		expect(
			queryByRole("button", "Send request", view.container),
		).not.toBeNull();
		expect(writes(view)).toEqual([]);
	});

	test("leaving the Endpoint tab ends an unfinished response", async () => {
		const { view, service } = await scripted();
		await click(byRole("button", "Send request", view.container));
		await act(async () => service.push("HTTP/1.1 200 OK\r\n\r\nstreaming"));
		await until(() => response(view)?.textContent === "streaming");
		await view.unmount();
		expect(service.resets).toBe(1);
	});
});

describe("Endpoint listener configuration", () => {
	async function addListener(view: View, host = "127.0.0.1") {
		await click(byRole("button", "Add listener", view.container));
		await typeInto(field(view, "Service ID"), "database");
		await typeInto(field(view, "Device loopback address"), host);
		await typeInto(field(view, "Device port"), "5432");
	}

	test("listener edits require Save and retain the existing revision and settings", async () => {
		const view = await open({
			arrange: (fake) =>
				patchConfig(fake, STUDIO, "field-notes", (config) => {
					config.variables = { keep: "stored-value" };
					config.future_setting = { preserve: true };
				}),
		});
		await addListener(view);
		expect(writes(view)).toEqual([]);
		const checks = byRole(
			"checkbox",
			"Apply with health checks",
			view.container,
		);
		expect((checks as HTMLInputElement).checked).toBe(true);
		await click(checks);
		expect(writes(view)).toEqual([]);
		await click(byRole("button", "Save and restart service", view.container));
		await until(() => commandsOf(view, "apply").length === 1);
		const [command] = commandsOf(view, "apply");
		expect(command).toMatchObject({ expected_revision: 9, start: true });
		expect(command?.config).toMatchObject({
			variables: { keep: "stored-value" },
			future_setting: { preserve: true },
			hosting: {
				host: "127.0.0.1",
				port: 8090,
				auth_secret: "field-notes-auth",
			},
			tunnel_services: [
				{ id: "database", host: "127.0.0.1", port: 5432, protocol: "tcp" },
			],
		});
		expect(commandsOf(view, "stage_rollout")).toEqual([]);
		expect(writes(view)).toEqual(["apply"]);
	});

	test("a non-loopback listener is rejected before any configuration call", async () => {
		const view = await open();
		await addListener(view, "192.168.1.40");
		await click(byRole("button", "Save with health checks", view.container));
		expect(text(view.container)).toContain(
			"Use a device loopback address, such as 127.0.0.1 or ::1.",
		);
		expect(writes(view)).toEqual([]);
		expect(commandsOf(view, "apply")).toEqual([]);
		expect(commandsOf(view, "stage_rollout")).toEqual([]);
		expect(field(view, "Device loopback address").value).toBe("192.168.1.40");
	});
});
