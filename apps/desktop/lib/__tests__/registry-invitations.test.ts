import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ fetcher: vi.fn(), invoke: vi.fn() }));
vi.mock("../api", () => ({ fetcher: mocks.fetcher }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock(
	"@flow-like/flow-like-ui/components/a2ui/use-micro-widget-grant",
	() => ({ forgetMicroWidgetGrants: vi.fn() }),
);

import { RegistryState } from "../../components/tauri-provider/registry-state";

const auth = { isAuthenticated: true, user: { access_token: "token" } };
const profile = { hub: "hub.example" };
const registry = new RegistryState({ auth, profile } as never);

describe("desktop package invitations", () => {
	beforeEach(() => vi.resetAllMocks());

	test("loads invitations from the current profile's hub", async () => {
		const invitations = [{ id: "invite-1", packageId: "com.example.sales" }];
		mocks.fetcher.mockResolvedValue(invitations);
		await expect(registry.listMyInvitations()).resolves.toEqual(invitations);
		expect(mocks.fetcher).toHaveBeenCalledWith(
			profile,
			"registry/invitations/me",
			{ method: "GET" },
			auth,
		);
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("accepts and declines through authenticated package endpoints", async () => {
		const membership = { userId: "recipient", permission: 4 };
		mocks.fetcher.mockResolvedValueOnce(membership);
		await expect(registry.acceptInvitation("invite/1")).resolves.toEqual(
			membership,
		);
		expect(mocks.fetcher).toHaveBeenLastCalledWith(
			profile,
			"registry/invitation/invite%2F1/accept",
			{ method: "POST" },
			auth,
		);
		await registry.rejectInvitation("invite/1");
		expect(mocks.fetcher).toHaveBeenLastCalledWith(
			profile,
			"registry/invitation/invite%2F1/reject",
			{ method: "POST" },
			auth,
		);
	});

	test("surfaces failures so the inbox can offer a retry", async () => {
		const error = new Error("Hub unavailable");
		mocks.fetcher.mockRejectedValue(error);
		await expect(registry.listMyInvitations()).rejects.toBe(error);
		await expect(registry.acceptInvitation("invite-1")).rejects.toBe(error);
		await expect(registry.rejectInvitation("invite-1")).rejects.toBe(error);
	});

	test("requires a profile and a session before contacting the hub", async () => {
		for (const backend of [{ profile }, { auth }]) {
			const state = new RegistryState(backend as never);
			await expect(state.listMyInvitations()).rejects.toThrow("Sign in");
			await expect(state.acceptInvitation("invite-1")).rejects.toThrow(
				"Sign in",
			);
			await expect(state.rejectInvitation("invite-1")).rejects.toThrow(
				"Sign in",
			);
		}
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});
});
