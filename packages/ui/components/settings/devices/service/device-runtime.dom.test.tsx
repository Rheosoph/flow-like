import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act } from "react";
import type { TunnelFetchOptions } from "../../../../lib/device-management/tunnel-fetch";
import type { RuntimeSession } from "../../../../lib/service-runtime/session";
import { ServiceRequestError } from "../../../../lib/service-runtime/transport";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";

const dom = installDom();
const attempts: { options: TunnelFetchOptions; token: string | null }[] = [];
const sessions: { appId: string; close: ReturnType<typeof mock> }[] = [];
let protectedService = false;
mock.module("../../../../lib/service-runtime/session", () => ({
	openRuntimeSession: async (
		options: TunnelFetchOptions,
		token: string | null,
	) => {
		attempts.push({ options, token });
		await options.open(options.signal);
		if (protectedService && token === null)
			throw new ServiceRequestError("Access token required", 401);
		const session = {
			appId: crypto.randomUUID(),
			close: mock(() => {}),
			signal: options.signal,
		};
		sessions.push(session);
		return session;
	},
}));
mock.module("./runtime-view", () => ({
	RuntimeView: ({ session }: { session: RuntimeSession }) => (
		<p>Runtime session {session.appId}</p>
	),
}));
const { cleanupDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { STUDIO, openTab, until } = await import("./config-test-kit");
const { useBackendStore } = await import("../../../../state/backend-state");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	attempts.length = 0;
	sessions.length = 0;
	protectedService = false;
	localStorage.clear();
});
afterAll(() => {
	mock.restore();
	dom.restore();
});

async function open(platform: "web" | "desktop") {
	const opened: unknown[][] = [];
	let held = 0;
	const view = await openTab({
		tab: "endpoint",
		device: STUDIO,
		service: "field-notes",
		platform,
		arrange(fake) {
			fake.workspace.live.openService = async (...args) => {
				opened.push(args);
				return {} as never;
			};
			const acquire = fake.workspace.live.acquire.bind(fake.workspace.live);
			fake.workspace.live.acquire = (id, reason) => {
				const release = acquire(id, reason);
				if (reason === "stream") held++;
				let active = true;
				return () => {
					if (!active) return;
					active = false;
					if (reason === "stream") held--;
					release();
				};
			};
		},
	});
	await until(
		() => !!queryByRole("button", "Open deployed app", view.container),
	);
	await click(byRole("button", "Open deployed app", view.container));
	await until(() => attempts.length > 0);
	return { view, opened, held: () => held };
}

test("Web opens the configured hosting service and close releases the session without replacing Studio's backend", async () => {
	const { view, opened, held } = await open("web");
	const backend = useBackendStore.getState().backend;
	await until(
		() => document.body.textContent?.includes("Runtime session") ?? false,
	);
	expect(opened[0]?.slice(0, 3)).toEqual([STUDIO, "field-notes", "hosting"]);
	expect((opened[0]?.[3] as { mode: string }).mode).toBe("http");
	expect(held()).toBe(1);
	expect(attempts[0].token).toBeNull();
	await click(byRole("button", "Close", byRole("dialog", "Deployed app")));
	expect(attempts[0].options.signal?.aborted).toBe(true);
	expect(sessions[0].close).toHaveBeenCalled();
	expect(held()).toBe(0);
	expect(useBackendStore.getState().backend).toBe(backend);
	expect(
		queryByRole("button", "Open deployed app", view.container),
	).not.toBeNull();
});

test("Desktop requests the service token and key locking removes the runtime and cancels its connection", async () => {
	protectedService = true;
	const { view, held } = await open("desktop");
	await until(() => !!queryByRole("textbox", "Service access token"));
	expect(held()).toBe(0);
	const token = "s".repeat(32);
	await typeInto(byRole("textbox", "Service access token"), token);
	await click(byRole("button", "Open app"));
	await until(
		() => document.body.textContent?.includes("Runtime session") ?? false,
	);
	expect(attempts.map((attempt) => attempt.token)).toEqual([null, token]);
	expect(held()).toBe(1);
	expect(document.body.textContent).not.toContain(token);
	await act(async () => view.fake.workspace.keys.lock(STUDIO));
	expect(attempts[1].options.signal?.aborted).toBe(true);
	expect(held()).toBe(0);
	expect(queryByRole("dialog", "Deployed app")).toBeNull();
	for (let index = 0; index < localStorage.length; index++) {
		const key = localStorage.key(index);
		expect(key && localStorage.getItem(key)).not.toContain(token);
	}
});
