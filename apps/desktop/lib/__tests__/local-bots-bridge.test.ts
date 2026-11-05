// @vitest-environment happy-dom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { StrictMode, act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	type MockInstance,
	afterEach,
	beforeEach,
	describe,
	expect,
	test,
	vi,
} from "vitest";

interface Registration {
	event: string;
	handler: (event: { payload: unknown }) => void;
}

interface SessionAuth {
	isLoading: boolean;
	isAuthenticated: boolean;
	user?: { access_token?: string; profile: { sub: string } };
}

const mocks = vi.hoisted(() => {
	const session: { auth?: unknown; profile?: unknown } = {
		profile: { id: "profile-1" },
	};
	class TauriBackend {
		get auth() {
			return session.auth;
		}
		get profile() {
			return session.profile;
		}
	}
	return {
		session,
		TauriBackend,
		backend: new TauriBackend(),
		contextAuth: { isLoading: true, isAuthenticated: false } as unknown,
		registrations: new Set<Registration>(),
		invoke: vi.fn(),
		toastInfo: vi.fn(),
		push: vi.fn(),
	};
});

vi.mock("../../components/tauri-provider", () => ({
	TauriBackend: mocks.TauriBackend,
}));
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => ({
	useBackend: () => mocks.backend,
}));
vi.mock("@flow-like/flow-like-ui/lib/client-navigation", () => ({
	useClientRouter: () => ({ push: mocks.push }),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, options?: string | Record<string, unknown>) => {
			if (typeof options === "string") return options;
			const count = Number(options?.count);
			const template =
				count === 1 ? options?.defaultValue_one : options?.defaultValue_other;
			return String(template ?? key).replace("{{count}}", String(count));
		},
	}),
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => mocks.contextAuth }));
vi.mock("sonner", () => ({ toast: { info: mocks.toastInfo } }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
	listen: async (event: string, handler: Registration["handler"]) => {
		const registration = { event, handler };
		mocks.registrations.add(registration);
		return () => mocks.registrations.delete(registration);
	},
}));

import { LocalBotsBridge } from "../../components/local-bots-bridge";
import { SessionWatch } from "../local-bots/session-watch";

let root: Root;
let container: HTMLDivElement;
let queryClient: QueryClient;
let invalidate: MockInstance<QueryClient["invalidateQueries"]>;
let localBotCount: number;
/** Assistant's and Flux's rows on this computer, listed before the person's own bots. */
let localBuiltIns: Record<string, unknown>[];

function builtIn(system: "general" | "flux", updatedAt: string) {
	return {
		id: `bot_local_${system}`,
		system,
		created_at: "2026-11-02T09:00:00Z",
		updated_at: updatedAt,
		local: true,
	};
}

async function flush() {
	await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
}

async function render(strict = false) {
	const bridge = createElement(LocalBotsBridge);
	await act(async () =>
		root.render(
			createElement(
				QueryClientProvider,
				{ client: queryClient },
				strict ? createElement(StrictMode, null, bridge) : bridge,
			),
		),
	);
	await flush();
}

/** What the backend holds after AuthInner pushed it; the context copy is what re-renders the bridge. */
async function authAs(auth: SessionAuth | undefined, context = auth) {
	mocks.session.auth = auth;
	mocks.contextAuth = context
		? { ...context }
		: { isLoading: true, isAuthenticated: false };
	await render();
}

const loading: SessionAuth = { isLoading: true, isAuthenticated: false };
const signedOut: SessionAuth = { isLoading: false, isAuthenticated: false };
function signedIn(sub: string, expired = false): SessionAuth {
	return {
		isLoading: false,
		isAuthenticated: !expired,
		user: { access_token: "token", profile: { sub } },
	};
}

function listeners(event: string): number {
	return [...mocks.registrations].filter((item) => item.event === event).length;
}

async function emit(event: string) {
	await act(async () => {
		for (const registration of mocks.registrations) {
			if (registration.event === event) registration.handler({ payload: {} });
		}
	});
}

function invalidatedKeys(): unknown[] {
	return invalidate.mock.calls.map(([filters]) => filters?.queryKey);
}

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	mocks.registrations.clear();
	mocks.invoke.mockReset();
	mocks.toastInfo.mockReset();
	mocks.push.mockReset();
	mocks.session.auth = undefined;
	mocks.contextAuth = { isLoading: true, isAuthenticated: false };
	localBotCount = 0;
	localBuiltIns = [];
	mocks.invoke.mockImplementation(async (command: string) => {
		if (command !== "local_bots_list") throw `Command ${command} not found`;
		return [
			...localBuiltIns,
			...Array.from({ length: localBotCount }, (_, index) => ({
				id: `bot_${index}`,
				system: null,
				local: true,
			})),
		];
	});
	queryClient = new QueryClient();
	invalidate = vi.spyOn(queryClient, "invalidateQueries");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	await flush();
	queryClient.clear();
	container.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

describe("local store events", () => {
	test("one listener per event survives strict mode, and none outlive the bridge", async () => {
		await render(true);
		expect(listeners("local-bots-changed")).toBe(1);
		expect(listeners("local-inbox-changed")).toBe(1);

		await act(async () => root.unmount());
		await flush();
		expect(mocks.registrations.size).toBe(0);
		root = createRoot(container);
	});

	test("refresh bot and inbox queries", async () => {
		await render();
		await emit("local-bots-changed");
		expect(invalidatedKeys()).toEqual([["bots"]]);
		await emit("local-inbox-changed");
		expect(invalidatedKeys()).toEqual([["bots"], ["inbox"]]);
	});
});

describe("account changes", () => {
	test("refresh both query families whenever the routing account changes", async () => {
		await authAs(signedOut);
		expect(invalidate).not.toHaveBeenCalled();

		await authAs(signedIn("user-1"));
		expect(invalidatedKeys()).toEqual([["bots"], ["inbox"]]);

		await authAs(signedIn("user-1", true));
		await authAs(signedIn("user-1"));
		await authAs(signedOut);
		expect(invalidatedKeys()).toEqual([
			["bots"],
			["inbox"],
			["bots"],
			["inbox"],
			["bots"],
			["inbox"],
			["bots"],
			["inbox"],
		]);
	});

	test("right after a sign-in, local bots are offered for the move", async () => {
		localBotCount = 2;
		await authAs(signedOut);
		await authAs(loading);
		await authAs(signedIn("user-1"));

		expect(mocks.toastInfo).toHaveBeenCalledTimes(1);
		const [title, options] = mocks.toastInfo.mock.calls[0];
		expect(title).toBe("2 bots live only on this computer");
		expect(options).toMatchObject({
			id: "local-bots-move",
			action: { label: "Review" },
		});
		options.action.onClick();
		expect(mocks.push).toHaveBeenCalledWith("/bots?move=local");
	});

	test("one local bot reads in the singular", async () => {
		localBotCount = 1;
		await authAs(signedOut);
		await authAs(signedIn("user-1"));
		expect(mocks.toastInfo.mock.calls[0][0]).toBe(
			"1 bot lives only on this computer",
		);
	});

	test("a restored session and a token renewal are not a sign-in", async () => {
		localBotCount = 2;
		// The placeholder provider settles signed out before the backend has any auth.
		await authAs(undefined, signedOut);
		await authAs(loading);
		await authAs(signedIn("user-1", true));
		await authAs(signedIn("user-1"));

		expect(mocks.toastInfo).not.toHaveBeenCalled();
		expect(mocks.invoke).not.toHaveBeenCalled();
	});

	test("nothing is offered without local bots", async () => {
		await authAs(signedOut);
		await authAs(signedIn("user-1"));
		expect(mocks.invoke).toHaveBeenCalledWith("local_bots_list", undefined);
		expect(mocks.toastInfo).not.toHaveBeenCalled();
	});

	test("Assistant and Flux as this computer made them are nothing to move; edited, they are", async () => {
		localBuiltIns = [
			builtIn("general", "2026-11-02T09:00:00Z"),
			builtIn("flux", "2026-11-02T09:00:00Z"),
		];
		await authAs(signedOut);
		await authAs(signedIn("user-1"));
		expect(mocks.toastInfo).not.toHaveBeenCalled();

		localBuiltIns = [
			builtIn("general", "2026-11-02T10:00:00Z"),
			builtIn("flux", "2026-11-02T09:00:00Z"),
		];
		await authAs(signedOut);
		await authAs(signedIn("user-1"));
		expect(mocks.toastInfo.mock.calls[0][0]).toBe(
			"1 bot lives only on this computer",
		);
	});
});

describe("session watch", () => {
	test("the first observation is the baseline", () => {
		const watch = new SessionWatch();
		expect(watch.observe({ routing: "user-1", person: "user-1" })).toEqual({
			refetch: false,
			signedIn: undefined,
		});
	});

	test("unknown states keep the last known person", () => {
		const watch = new SessionWatch();
		watch.observe({ routing: null, person: null });
		expect(watch.observe({ routing: null, person: undefined })).toEqual({
			refetch: false,
		});
		expect(watch.observe({ routing: "user-1", person: "user-1" })).toEqual({
			refetch: true,
			signedIn: "user-1",
		});
	});

	test("switching accounts is a sign-in to the new one", () => {
		const watch = new SessionWatch();
		watch.observe({ routing: "user-1", person: "user-1" });
		expect(watch.observe({ routing: "user-2", person: "user-2" })).toEqual({
			refetch: true,
			signedIn: "user-2",
		});
		expect(watch.observe({ routing: null, person: null })).toEqual({
			refetch: true,
			signedIn: undefined,
		});
	});
});
