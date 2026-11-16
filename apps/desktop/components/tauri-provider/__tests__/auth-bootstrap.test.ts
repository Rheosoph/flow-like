// @vitest-environment happy-dom

import {
	useAuthStatusStore,
	useSignedIn,
} from "@flow-like/flow-like-ui/state/backend-state";
import {
	QueryClient,
	QueryClientProvider,
	useQuery,
} from "@tanstack/react-query";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: vi
		.fn()
		.mockImplementation(async (command: string) =>
			command === "execution_open_auth_session" ? "native-session" : undefined,
		),
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

import { TauriBackend } from "../../tauri-provider";

const profile = { id: "profile-1", hub: "hub-1" };
const auth = {
	isLoading: false,
	isAuthenticated: true,
	user: { access_token: "token", profile: { sub: "user-1" } },
};

let root: Root;
let container: HTMLDivElement;
let queryClient: QueryClient;

function AccountInfo({ backend }: { backend: TauriBackend }) {
	const signedIn = useSignedIn();
	const info = useQuery({
		queryKey: ["getInfo"],
		queryFn: () => backend.userState.getInfo(),
		enabled: signedIn,
	});

	return createElement(
		"output",
		{ "data-signed-in": String(signedIn) },
		info.data?.name ?? info.fetchStatus,
	);
}

async function mount(backend: TauriBackend) {
	await act(async () => {
		root.render(
			createElement(
				QueryClientProvider,
				{ client: queryClient },
				createElement(AccountInfo, { backend }),
			),
		);
	});
}

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	container = document.createElement("div");
	root = createRoot(container);
	queryClient = new QueryClient({
		defaultOptions: {
			queries: { networkMode: "always", retry: 1, retryDelay: 0 },
		},
	});
});

afterEach(async () => {
	await act(async () => root.unmount());
	queryClient.clear();
	useAuthStatusStore.setState({ signedIn: undefined });
	vi.restoreAllMocks();
});

describe("native account queries during session bootstrap", () => {
	test.each([
		["unknown", undefined],
		["a previous signed-in session", true],
	] as const)(
		"keeps remote queries idle when initial host status is %s until sign-in arrives",
		async (_label, signedIn) => {
			useAuthStatusStore.setState({ signedIn });
			const backend = new TauriBackend(() => undefined, queryClient);
			const getInfo = vi
				.spyOn(backend.userState, "getInfo")
				.mockResolvedValue({ name: "Signed-in account" } as never);

			await mount(backend);

			expect(container.querySelector("output")?.dataset.signedIn).toBe("false");
			expect(container.textContent).toBe("idle");
			expect(getInfo).not.toHaveBeenCalled();

			await act(async () => backend.pushProfile(profile as never));
			expect(container.textContent).toBe("idle");
			expect(getInfo).not.toHaveBeenCalled();

			await act(async () => backend.pushAuthContext(auth as never));
			await act(async () => {
				await vi.waitFor(() => expect(getInfo).toHaveBeenCalledTimes(1));
			});
			expect(container.querySelector("output")?.dataset.signedIn).toBe("true");
			expect(queryClient.getQueryData(["getInfo"])).toEqual({
				name: "Signed-in account",
			});
		},
	);

	test("enables account queries when a usable session is supplied to the constructor", async () => {
		useAuthStatusStore.setState({ signedIn: false });
		const backend = new TauriBackend(
			() => undefined,
			queryClient,
			auth as never,
			profile as never,
		);
		const getInfo = vi
			.spyOn(backend.userState, "getInfo")
			.mockResolvedValue({ name: "Signed-in account" } as never);

		await mount(backend);
		await act(async () => {
			await vi.waitFor(() => expect(getInfo).toHaveBeenCalledTimes(1));
		});

		expect(container.querySelector("output")?.dataset.signedIn).toBe("true");
		expect(queryClient.getQueryData(["getInfo"])).toEqual({
			name: "Signed-in account",
		});
	});
});
