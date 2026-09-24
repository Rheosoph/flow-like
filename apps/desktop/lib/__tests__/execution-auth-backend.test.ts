import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
}));
vi.mock("sonner", () => ({
	toast: Object.assign(vi.fn(), {
		success: vi.fn(),
		error: vi.fn(),
		info: vi.fn(),
		warning: vi.fn(),
		dismiss: vi.fn(),
	}),
}));

import { TauriBackend } from "../../components/tauri-provider";

const auth = {
	isAuthenticated: true,
	isLoading: false,
	user: { access_token: "first", profile: { sub: "user-a" } },
};
const hub = "https://first-hub.test";

function updates() {
	return mocks.invoke.mock.calls
		.filter(([command]) => command === "execution_set_auth")
		.map(([, args]) => args);
}

beforeEach(() => mocks.invoke.mockClear());

describe("desktop execution session bridge", () => {
	test("renewed tokens reach native execution with the original session ID", async () => {
		const backend = new TauriBackend(() => undefined);
		backend.pushProfile({ hub } as never);
		backend.pushAuthContext(auth as never, hub);
		expect(await backend.prepareExecutionAuth()).toBe(hub);
		const sessionId = updates().at(-1).sessionId;
		backend.pushAuthContext(
			{ ...auth, user: { ...auth.user, access_token: "renewed" } } as never,
			hub,
		);
		await backend.prepareExecutionAuth();
		expect(updates().at(-1)).toMatchObject({
			sessionId,
			hub,
			subject: "user-a",
			token: "renewed",
		});
	});

	test("sign-out clears the native session and blocks new online runs", async () => {
		const backend = new TauriBackend(() => undefined);
		backend.pushProfile({ hub } as never);
		backend.pushAuthContext(auth as never, hub);
		await backend.prepareExecutionAuth();
		backend.pushAuthContext(
			{ ...auth, isAuthenticated: false, user: null } as never,
			hub,
		);
		await expect(backend.prepareExecutionAuth()).rejects.toThrow("Sign in");
		expect(updates().at(-1)).toMatchObject({ hub, subject: null, token: null });
	});

	test("a profile switch cannot send the previous hub's token to the new hub", async () => {
		const backend = new TauriBackend(() => undefined);
		backend.pushProfile({ hub } as never);
		backend.pushAuthContext(auth as never, hub);
		await backend.prepareExecutionAuth();
		backend.pushProfile({ hub: "https://second-hub.test" } as never);
		backend.pushAuthContext(auth as never, hub);
		await expect(backend.prepareExecutionAuth()).rejects.toThrow("Sign in");
		expect(
			updates()
				.filter((update) => update.hub === "https://second-hub.test")
				.every((update) => update.token === null),
		).toBe(true);
	});
});
