import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

import type { TauriBackend } from "../../tauri-provider";

const APP = "app-1";

function backend() {
	return {
		auth: { user: { access_token: "token-1" } },
	} as unknown as TauriBackend;
}

/** The warning latch is module state, so every test gets a fresh module. */
async function freshState() {
	vi.resetModules();
	const { OfflineWritesState } = await import("../offline-writes-state");
	return new OfflineWritesState(backend());
}

describe("OfflineWritesState.getTableRoute without the routing command", () => {
	let warn: ReturnType<typeof vi.spyOn>;

	beforeEach(() => {
		mocks.invoke.mockReset();
		warn = vi.spyOn(console, "warn").mockImplementation(() => {});
	});

	afterEach(() => {
		warn.mockRestore();
	});

	test("routes through the hub and warns once when Tauri does not know the command", async () => {
		mocks.invoke.mockRejectedValue(
			"Command offline_writes_table_route not found",
		);
		const state = await freshState();

		expect(await state.getTableRoute(APP, "orders")).toBe("hub");
		expect(await state.getTableRoute(APP, "notes", true)).toBe("hub");
		expect(mocks.invoke).toHaveBeenCalledTimes(2);
		expect(warn).toHaveBeenCalledTimes(1);
	});

	test("treats an ACL 'Command not found' rejection as a missing command", async () => {
		mocks.invoke.mockRejectedValue(
			new Error("offline_writes_table_route not allowed. Command not found"),
		);
		const state = await freshState();

		expect(await state.getTableRoute(APP, "orders")).toBe("hub");
	});

	test("rethrows every other routing failure", async () => {
		mocks.invoke.mockRejectedValue(
			"Offline access for table orders is corrupt",
		);
		const state = await freshState();

		await expect(state.getTableRoute(APP, "orders")).rejects.toBe(
			"Offline access for table orders is corrupt",
		);
		expect(warn).not.toHaveBeenCalled();
	});

	test("keeps the route the command answers", async () => {
		mocks.invoke.mockResolvedValue("device");
		const state = await freshState();

		expect(await state.getTableRoute(APP, "orders")).toBe("device");
		expect(mocks.invoke).toHaveBeenCalledWith("offline_writes_table_route", {
			appId: APP,
			token: "token-1",
			table: "orders",
			userScoped: false,
		});
	});
});
