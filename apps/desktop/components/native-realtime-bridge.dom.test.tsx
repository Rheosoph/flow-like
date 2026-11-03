// @vitest-environment happy-dom
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const state = vi.hoisted(() => ({
	ready: true,
	profileId: "profile-a",
	auth: {
		isAuthenticated: true,
		isLoading: false,
		user: {
			access_token: "token",
			profile: { sub: "user-a", iss: "https://issuer.test" },
		},
		settings: { authority: "https://issuer.test" },
	},
	watch: vi.fn(),
	close: vi.fn(),
	invalidateQueries: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("./tauri-provider", () => ({
	TauriBackend: class {
		get profile() {
			return { id: state.profileId };
		}
	},
}));
vi.mock("@flow-like/flow-like-ui", async () => {
	const { TauriBackend } = (await import("./tauri-provider")) as unknown as {
		TauriBackend: new () => object;
	};
	const backend = new TauriBackend();
	return { useBackend: () => backend, useBackendReady: () => state.ready };
});
vi.mock("react-oidc-context", () => ({ useAuth: () => state.auth }));
vi.mock("@tanstack/react-query", () => ({
	useQueryClient: () => ({ invalidateQueries: state.invalidateQueries }),
}));
vi.mock("../lib/native-realtime", () => ({ watchNativeRealtime: state.watch }));

import { NativeRealtimeBridge } from "./native-realtime-bridge";

let root: Root;
let host: HTMLDivElement;
beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	state.ready = true;
	state.profileId = "profile-a";
	state.auth.isAuthenticated = true;
	state.watch.mockReset().mockReturnValue(state.close);
	state.close.mockReset();
	state.invalidateQueries.mockReset().mockResolvedValue(undefined);
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});
afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

describe("native realtime shell bridge", () => {
	it("waits for a ready authenticated profile", async () => {
		state.ready = false;
		await act(async () => root.render(<NativeRealtimeBridge />));
		expect(state.watch).not.toHaveBeenCalled();
		state.ready = true;
		state.auth.isAuthenticated = false;
		await act(async () => root.render(<NativeRealtimeBridge />));
		expect(state.watch).not.toHaveBeenCalled();
		state.auth.isAuthenticated = true;
		await act(async () => root.render(<NativeRealtimeBridge />));
		expect(state.watch).toHaveBeenCalledWith(
			expect.objectContaining({
				profileId: "profile-a",
				subject: "user-a",
				issuer: "https://issuer.test",
			}),
		);
	});

	it("refreshes inbox queries once per revision and ignores run-only updates", async () => {
		await act(async () => root.render(<NativeRealtimeBridge />));
		const { onMessage } = state.watch.mock.calls[0][0];
		onMessage({ type: "update", response: { runs: [], errors: [] } });
		expect(state.invalidateQueries).not.toHaveBeenCalled();
		const message = {
			type: "update",
			response: { notifications: { revision: "one" } },
		};
		await onMessage(message);
		await onMessage(message);
		expect(state.invalidateQueries).toHaveBeenCalledOnce();
		const { predicate } = state.invalidateQueries.mock.calls[0][0];
		expect(predicate({ queryKey: ["getNotifications"] })).toBe(true);
		expect(predicate({ queryKey: ["listNotifications", 50] })).toBe(true);
		expect(
			predicate({
				queryKey: [
					"home",
					"hub",
					"profile",
					"subject",
					"notifications",
					false,
					"all",
					10,
				],
			}),
		).toBe(true);
		expect(
			predicate({ queryKey: ["home", "hub", "profile", "subject", "apps"] }),
		).toBe(false);
		expect(predicate({ queryKey: ["getApps"] })).toBe(false);
	});

	it("keeps a failed invalidation eligible for native redelivery", async () => {
		await act(async () => root.render(<NativeRealtimeBridge />));
		const { onMessage } = state.watch.mock.calls[0][0];
		const message = {
			type: "update",
			response: { notifications: { revision: "retry" } },
		};
		state.invalidateQueries.mockRejectedValueOnce(new Error("Fetch failed"));
		await expect(onMessage(message)).rejects.toThrow("Fetch failed");
		await onMessage(message);
		expect(state.invalidateQueries).toHaveBeenCalledTimes(2);
		await onMessage(message);
		expect(state.invalidateQueries).toHaveBeenCalledTimes(2);
	});

	it("detaches the previous native subscription on profile changes and sign-out", async () => {
		await act(async () => root.render(<NativeRealtimeBridge />));
		state.profileId = "profile-b";
		await act(async () => root.render(<NativeRealtimeBridge />));
		expect(state.close).toHaveBeenCalledOnce();
		expect(state.watch.mock.calls[1][0].profileId).toBe("profile-b");
		state.auth.isAuthenticated = false;
		await act(async () => root.render(<NativeRealtimeBridge />));
		expect(state.close).toHaveBeenCalledTimes(2);
		expect(state.watch).toHaveBeenCalledTimes(2);
	});
});
