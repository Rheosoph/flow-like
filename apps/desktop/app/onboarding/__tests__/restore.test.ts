// @vitest-environment happy-dom
import { type ReactNode, act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => {
	const events: string[] = [];
	const saves: { id: string; resolve: () => void }[] = [];
	const completions: (() => void)[] = [];
	const getProfile = vi.fn();
	return {
		events,
		saves,
		completions,
		auth: {
			isAuthenticated: true,
			user: { access_token: "access-token" },
		},
		backend: {
			userState: { getProfile },
			capabilities: () => ({ canHostLlamaCPP: false, canHostMLX: false }),
		},
		queryClient: {
			invalidateQueries: vi.fn(async ({ queryKey }: { queryKey: string[] }) => {
				events.push(`invalidate:${queryKey.join("/")}`);
			}),
		},
		invalidate: vi.fn(async () => {
			events.push("invalidate:current-profile");
		}),
		router: {
			replace: vi.fn((path: string) => events.push(`navigate:${path}`)),
		},
		invoke: vi.fn(
			async (
				command: string,
				args: {
					profile?: { hub_profile: { id: string } };
					profileId?: string;
				},
			) => {
				if (command === "upsert_profile") {
					const id = args.profile?.hub_profile.id;
					if (!id) throw new Error("Missing profile to restore");
					events.push(`save:${id}`);
					await new Promise<void>((resolve) => saves.push({ id, resolve }));
				} else if (command === "set_current_profile") {
					events.push(`select:${args.profileId}`);
				} else if (command === "native_auth_complete_onboarding") {
					events.push("complete-onboarding");
					await new Promise<void>((resolve) => completions.push(resolve));
				}
			},
		),
		fetch: vi.fn(async () => ({
			ok: true,
			json: async () =>
				["first", "second"].map((id) => ({
					id,
					name: id,
					hub: "https://onboarding-hub.test",
					created_at: "2026-11-06T00:00:00Z",
					updated_at: "2026-11-06T00:00:00Z",
				})),
		})),
		defaultProfiles: { data: [[], {}], isLoading: false, isError: false },
	};
});

vi.mock("@flow-like/flow-like-ui", () => ({
	Button: ({ children }: { children: ReactNode }) =>
		createElement("button", { type: "button" }, children),
	Bit: {},
	isHostableLlmModel: () => true,
	isLocalLlmModel: () => false,
	useBackend: () => mocks.backend,
	useQueryClient: () => mocks.queryClient,
	useInvalidateInvoke: () => mocks.invalidate,
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
	Trans: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
	isTauri: () => true,
}));
vi.mock("@tauri-apps/plugin-http", () => ({ fetch: mocks.fetch }));
vi.mock("next/navigation", () => ({ useRouter: () => mocks.router }));
vi.mock("next/link", () => ({
	default: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("react-oidc-context", () => ({
	useAuth: () => mocks.auth,
}));
vi.mock("../../../components/useInvoke", () => ({
	useTauriInvoke: () => mocks.defaultProfiles,
}));
vi.mock("../onboarding-flow", () => ({ OnboardingFlow: () => null }));

import { bindHubAuth, setHubAuthActive } from "../../../lib/hub-auth";
import Onboarding from "../page";

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	mocks.events.length = 0;
	mocks.saves.length = 0;
	mocks.completions.length = 0;
	vi.clearAllMocks();
	mocks.auth = {
		isAuthenticated: true,
		user: { access_token: "access-token" },
	};
	const auth = mocks.auth as unknown as AuthContextProps;
	bindHubAuth(auth, "https://onboarding-hub.test");
	// The backend has no selected profile yet, but the login's hub is known.
	setHubAuthActive(auth, false);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

test("restores every profile before refreshing the native sign-in scope", async () => {
	await act(async () => root.render(createElement(Onboarding)));
	expect(mocks.fetch).toHaveBeenCalledWith(
		"https://onboarding-hub.test/api/v1/profile",
		expect.objectContaining({
			headers: expect.objectContaining({
				Authorization: "Bearer access-token",
			}),
		}),
	);
	expect(mocks.events).toEqual(["save:first"]);

	await act(async () => mocks.saves[0].resolve());
	expect(mocks.events).toEqual(["save:first", "save:second"]);
	expect(mocks.invalidate).not.toHaveBeenCalled();

	await act(async () => mocks.saves[1].resolve());
	expect(mocks.events).toEqual([
		"save:first",
		"save:second",
		"select:first",
		"complete-onboarding",
	]);
	expect(mocks.invalidate).not.toHaveBeenCalled();

	await act(async () => mocks.completions[0]());
	expect(mocks.events).toEqual([
		"save:first",
		"save:second",
		"select:first",
		"complete-onboarding",
		"invalidate:get/profiles",
		"navigate:/",
		"invalidate:current-profile",
		"invalidate:getNativeAuthContext",
	]);
	expect(mocks.invalidate).toHaveBeenCalledWith(
		mocks.backend.userState.getProfile,
		[],
	);
	expect(mocks.fetch).toHaveBeenCalledTimes(1);
});

test("does not send an unbound native session's token to the default hub", async () => {
	mocks.auth = {
		isAuthenticated: true,
		user: { access_token: "unbound-token" },
	};
	const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
	try {
		await act(async () => root.render(createElement(Onboarding)));
		expect(mocks.fetch).not.toHaveBeenCalled();
		expect(container.textContent).toContain(
			"We couldn't restore your saved profiles.",
		);
	} finally {
		warn.mockRestore();
	}
});
