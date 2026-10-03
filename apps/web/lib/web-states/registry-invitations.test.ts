import { beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ apiGet: vi.fn(), apiPost: vi.fn() }));
vi.mock("./api-utils", () => ({
	apiGet: mocks.apiGet,
	apiPost: mocks.apiPost,
	apiPut: vi.fn(),
	apiDelete: vi.fn(),
}));
vi.mock(
	"@flow-like/flow-like-ui/components/a2ui/use-micro-widget-grant",
	() => ({ forgetMicroWidgetGrants: vi.fn() }),
);

import { WebRegistryState } from "./registry-state";

const auth = { isAuthenticated: true, user: { access_token: "token" } };
const registry = new WebRegistryState({ auth } as never);

describe("web package invitations", () => {
	beforeEach(() => vi.resetAllMocks());

	test("loads invitations for the signed-in recipient", async () => {
		const invitations = [{ id: "invite-1", packageId: "com.example.sales" }];
		mocks.apiGet.mockResolvedValue(invitations);
		await expect(registry.listMyInvitations()).resolves.toEqual(invitations);
		expect(mocks.apiGet).toHaveBeenCalledWith("registry/invitations/me", auth);
	});

	test("accepts and declines through authenticated package endpoints", async () => {
		const membership = { userId: "recipient", permission: 4 };
		mocks.apiPost.mockResolvedValueOnce(membership);
		await expect(registry.acceptInvitation("invite/1")).resolves.toEqual(
			membership,
		);
		expect(mocks.apiPost).toHaveBeenLastCalledWith(
			"registry/invitation/invite%2F1/accept",
			undefined,
			auth,
		);
		await registry.rejectInvitation("invite/1");
		expect(mocks.apiPost).toHaveBeenLastCalledWith(
			"registry/invitation/invite%2F1/reject",
			undefined,
			auth,
		);
	});

	test("surfaces failures so the inbox can offer a retry", async () => {
		const error = new Error("Hub unavailable");
		mocks.apiGet.mockRejectedValue(error);
		mocks.apiPost.mockRejectedValue(error);
		await expect(registry.listMyInvitations()).rejects.toBe(error);
		await expect(registry.acceptInvitation("invite-1")).rejects.toBe(error);
		await expect(registry.rejectInvitation("invite-1")).rejects.toBe(error);
	});
});
