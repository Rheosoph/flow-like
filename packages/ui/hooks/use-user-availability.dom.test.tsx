// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import {
	DEFAULT_AVAILABILITY,
	readLocalAvailability,
} from "../lib/availability";

const state = vi.hoisted(() => ({
	auth: {
		isAuthenticated: true,
		user: { profile: { iss: "issuer", sub: "person_a" } },
	},
	profile: { id: "profile_a", hub: "https://hub-a.test" },
	getAvailability: vi.fn(),
	updateAvailability: vi.fn(),
	clearAvailability: vi.fn(),
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => state.auth }));
vi.mock("./use-invoke", () => ({ useInvoke: () => ({ data: state.profile }) }));
const backend = {
	userState: {
		getProfile: vi.fn(),
		getAvailability: state.getAvailability,
		updateAvailability: state.updateAvailability,
		clearAvailability: state.clearAvailability,
	},
};
vi.mock("../state/backend-state", () => ({ useBackend: () => backend }));
const { useUserAvailability } = await import("./use-user-availability");
let current: ReturnType<typeof useUserAvailability>;
let client: QueryClient;
let root: ReturnType<typeof createRoot>;
let container: HTMLDivElement;
function Probe({
	local = false,
	enabled = true,
}: { local?: boolean; enabled?: boolean }) {
	current = useUserAvailability({ local, enabled });
	return null;
}
async function show(props: { local?: boolean; enabled?: boolean } = {}) {
	await act(async () => {
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement(Probe, props),
			),
		);
	});
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}
beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	state.auth.isAuthenticated = true;
	state.auth.user.profile.sub = "person_a";
	state.profile = { id: "profile_a", hub: "https://hub-a.test" };
	state.getAvailability.mockReset().mockResolvedValue(DEFAULT_AVAILABILITY);
	state.updateAvailability.mockReset().mockResolvedValue(DEFAULT_AVAILABILITY);
	client = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	container.remove();
});

test("disabled context loading reads nothing until explicitly requested", async () => {
	await show({ enabled: false });
	expect(state.getAvailability).not.toHaveBeenCalled();
	await current.load();
	expect(state.getAvailability).toHaveBeenCalledOnce();
});
test("same identity on different hubs has separate availability cache", async () => {
	state.getAvailability.mockResolvedValue({
		...DEFAULT_AVAILABILITY,
		status: "focus",
	});
	await show();
	const before = current.scope;
	state.profile = { id: "profile_b", hub: "https://hub-b.test" };
	state.getAvailability.mockResolvedValue({
		...DEFAULT_AVAILABILITY,
		status: "busy",
	});
	await show();
	expect(current.scope).not.toBe(before);
	expect(current.data?.status).toBe("busy");
});
test("a late mutation cannot update the next account's cached status", async () => {
	await show({ enabled: false });
	let finish!: (value: unknown) => void;
	state.updateAvailability.mockImplementationOnce(
		() =>
			new Promise((resolve) => {
				finish = resolve;
			}),
	);
	const pending = current.update({ status: "focus" });
	state.auth.user.profile.sub = "person_b";
	await show({ enabled: false });
	await act(async () => {
		finish({ ...DEFAULT_AVAILABILITY, status: "focus" });
		await pending;
	});
	expect(current.data).toBeUndefined();
});
test("an accepted local response updates its original account scope after a switch", async () => {
	await show({ local: true, enabled: false });
	const approved = current.applyLocalStatus;
	const original = current.scope;
	state.auth.user.profile.sub = "person_b";
	await show({ local: true, enabled: false });
	await act(async () => {
		approved("focus", Date.now() + 60_000);
	});
	expect(readLocalAvailability(original).status).toBe("focus");
	expect(readLocalAvailability(current.scope).status).toBe("available");
});
