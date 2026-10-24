// @vitest-environment happy-dom

import type { IProfile } from "@flow-like/flow-like-ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	backend: undefined as unknown as TauriBackend,
	invoke: vi.fn(),
	fetch: vi.fn(),
	translate: (_key: string, fallback: string) => fallback,
}));

vi.mock("@flow-like/flow-like-ui", async (importOriginal) => ({
	...(await importOriginal<typeof import("@flow-like/flow-like-ui")>()),
	useBackend: () => mocks.backend,
}));
vi.mock("@flow-like/locales", async (importOriginal) => ({
	...(await importOriginal<typeof import("@flow-like/locales")>()),
	useTranslation: () => ({ t: mocks.translate }),
}));
vi.mock("@tauri-apps/plugin-http", () => ({ fetch: mocks.fetch }));
vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
	...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
}));
vi.mock("../apps-db", () => ({
	appsDB: {
		visibility: { toArray: async () => [] },
		shortcuts: {
			where: () => ({ equals: () => ({ toArray: async () => [] }) }),
		},
	},
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

import { ProfileSyncer, TauriBackend } from "../../components/tauri-provider";
import { bindHubAuth } from "../hub-auth";

const hub = "https://profile-sync.test";
let root: Root;
let container: HTMLDivElement;
let queryClient: QueryClient;
let profile: IProfile;
let serverProfile: IProfile;
let signature: number;
let accessToken: string;
let signedIn: boolean;
let missingBitReferences: Set<string>;
let remoteCustomBits: unknown;
let remoteCustomBitsStatus: number;

function render() {
	root.render(
		createElement(
			QueryClientProvider,
			{ client: queryClient },
			createElement(ProfileSyncer, {
				auth: { isAuthenticated: signedIn, accessToken },
				hub,
			}),
		),
	);
}

function pushes() {
	return mocks.fetch.mock.calls.filter(([, init]) => init.method === "POST");
}

async function settle() {
	await act(async () => {
		await vi.advanceTimersByTimeAsync(10);
	});
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.stubEnv("NEXT_PUBLIC_API_URL", hub);
	vi.spyOn(console, "log").mockImplementation(() => {});
	vi.clearAllMocks();
	signature = 0;
	accessToken = "token-a";
	signedIn = true;
	missingBitReferences = new Set();
	remoteCustomBits = [];
	remoteCustomBitsStatus = 200;
	profile = {
		id: "personal",
		name: "Personal",
		hub,
		bits: [],
		icon: "https://media.test/icon.webp?signature=initial",
		created: "2026-10-01T10:00:00.000Z",
		updated: "2026-10-01T10:00:00.000Z",
	};
	serverProfile = structuredClone(profile);
	queryClient = new QueryClient({
		defaultOptions: {
			queries: { retry: false, staleTime: Number.POSITIVE_INFINITY },
		},
	});
	const session = {
		isAuthenticated: true,
		isLoading: false,
		user: { access_token: accessToken, profile: { sub: "user-a" } },
	};
	bindHubAuth(session as never, hub);
	mocks.backend = Object.assign(Object.create(TauriBackend.prototype), {
		queryClient,
		userState: {
			getProfile: async function getProfile() {
				return structuredClone(profile);
			},
		},
		pushProfile: (next: IProfile) => {
			mocks.backend.profile = next;
		},
	});
	Object.defineProperty(mocks.backend, "auth", {
		value: session,
		configurable: true,
	});
	mocks.invoke.mockImplementation(
		async (
			command: string,
			args?: {
				profile?: { hub_profile: IProfile };
				remoteCustomBitIds?: string[];
			},
		) => {
			if (command === "get_profiles_raw") {
				if (!profile.id) throw new Error("The profile fixture needs an ID.");
				return {
					[profile.id]: {
						hub_profile: structuredClone(profile),
						updated: profile.updated,
					},
				};
			}
			if (command === "merge_synced_profile" && args?.profile) {
				profile = structuredClone(args.profile.hub_profile);
				return true;
			}
			if (command === "prune_missing_profile_bits") {
				const bits = (profile.bits ?? []).filter(
					(reference) =>
						!missingBitReferences.has(reference) ||
						args?.remoteCustomBitIds?.includes(
							reference.split(":").at(-1) ?? "",
						),
				);
				if (bits.length === profile.bits?.length) return false;
				profile = {
					...profile,
					bits,
					updated: new Date(Date.parse(profile.updated) + 1).toISOString(),
				};
				return true;
			}
			throw new Error(`Unexpected native command: ${command}`);
		},
	);
	mocks.fetch.mockImplementation(async (url: string, init: RequestInit) => {
		if (url.includes("/api/v1/user/bits?")) {
			return Response.json(remoteCustomBits, {
				status: remoteCustomBitsStatus,
			});
		}
		if (init.method === "POST") {
			const [uploaded] = JSON.parse(init.body as string);
			if (Date.parse(uploaded.updatedAt) > Date.parse(serverProfile.updated)) {
				serverProfile = {
					...serverProfile,
					name: uploaded.name,
					bits: uploaded.bit_ids,
					updated: new Date(Date.parse(uploaded.updatedAt) + 1).toISOString(),
				};
			}
			return Response.json({ created: [], updated: [], skipped: [profile.id] });
		}
		return Response.json([
			{
				...serverProfile,
				bit_ids: serverProfile.bits,
				icon: `https://media.test/icon.webp?signature=${++signature}`,
				created_at: serverProfile.created,
				updated_at: serverProfile.updated,
			},
		]);
	});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	queryClient.clear();
	container.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
	vi.unstubAllEnvs();
	vi.useRealTimers();
});

test("refreshing signed images does not trigger another sync or reset the minute timer", async () => {
	await act(async () => render());
	await settle();
	expect(pushes()).toHaveLength(1);
	expect(queryClient.getQueryData<IProfile>(["getProfile"])?.icon).toContain(
		"signature=1",
	);
	await act(async () => {
		await vi.advanceTimersByTimeAsync(59_000);
	});
	expect(pushes()).toHaveLength(1);
	await act(async () => {
		await vi.advanceTimersByTimeAsync(1_000);
	});
	await settle();
	expect(pushes()).toHaveLength(2);
	expect(queryClient.getQueryData<IProfile>(["getProfile"])?.icon).toContain(
		"signature=2",
	);
});

test("local revisions upload immediately and settle after the server acknowledgement", async () => {
	await act(async () => render());
	await settle();
	profile = {
		...profile,
		name: "Renamed",
		updated: "2026-10-01T10:00:01.000Z",
	};
	await act(async () => {
		await queryClient.invalidateQueries({ queryKey: ["getProfile"] });
	});
	await settle();
	expect(pushes()).toHaveLength(3);
	expect(JSON.parse(pushes()[1][1].body)[0].name).toBe("Renamed");
	expect(profile.updated).toBe("2026-10-01T10:00:01.001Z");
	await act(async () => {
		await vi.advanceTimersByTimeAsync(10_000);
	});
	expect(pushes()).toHaveLength(3);
});

test("bits removed after a remote pull upload on the next sync without waiting for the timer", async () => {
	missingBitReferences.add("https://profile-sync.test:deleted-bit");
	serverProfile = {
		...serverProfile,
		bits: ["https://profile-sync.test:deleted-bit", "live-bit"],
		updated: "2026-10-01T10:00:01.000Z",
	};

	await act(async () => render());
	await settle();

	expect(JSON.parse(pushes()[0][1].body)[0].bit_ids).toEqual([]);
	expect(JSON.parse(pushes()[1][1].body)[0].bit_ids).toEqual(["live-bit"]);
	expect(profile.bits).toEqual(["live-bit"]);
	expect(serverProfile.bits).toEqual(["live-bit"]);
	expect(queryClient.getQueryData<IProfile>(["getProfile"])?.bits).toEqual([
		"live-bit",
	]);
	const pushCount = pushes().length;
	await act(async () => {
		await vi.advanceTimersByTimeAsync(10_000);
	});
	expect(pushes()).toHaveLength(pushCount);
});

test("cleanup requests another sync when the active profile revision is unchanged", async () => {
	const nativeCommand = mocks.invoke.getMockImplementation();
	let cleanupChanged = true;
	mocks.invoke.mockImplementation(async (command, args) => {
		if (command === "prune_missing_profile_bits") {
			const changed = cleanupChanged;
			cleanupChanged = false;
			return changed;
		}
		return nativeCommand?.(command, args);
	});
	const originalRevision = profile.updated;

	await act(async () => render());
	await settle();

	expect(profile.updated).toBe(originalRevision);
	expect(pushes()).toHaveLength(2);
});

test("a fresh profile preserves private bits before the local custom library is hydrated", async () => {
	const privateReferences = [
		"private-bit",
		":private-bit",
		"https://profile-sync.test:private-bit",
	];
	remoteCustomBits = [{ id: "private-bit" }];
	serverProfile = {
		...serverProfile,
		bits: [...privateReferences, "deleted-bit"],
		updated: "2026-10-01T10:00:01.000Z",
	};
	missingBitReferences = new Set(serverProfile.bits);

	await act(async () => render());
	await settle();

	expect(mocks.invoke).toHaveBeenCalledWith("prune_missing_profile_bits", {
		hub,
		remoteCustomBitIds: ["private-bit"],
	});
	expect(profile.bits).toEqual(privateReferences);
	expect(serverProfile.bits).toEqual(privateReferences);
	expect(mocks.fetch).toHaveBeenCalledWith(
		`${hub}/api/v1/user/bits?device_models=true`,
		expect.objectContaining({
			headers: expect.objectContaining({ Authorization: "Bearer token-a" }),
		}),
	);
});

test.each([
	[503, { error: "Temporarily unavailable" }],
	[200, { error: "Invalid library" }],
	[200, [{ id: 12 }]],
	[200, [{ id: " " }]],
])(
	"cleanup skips an unreadable custom library (%i, %j)",
	async (status, library) => {
		vi.spyOn(console, "warn").mockImplementation(() => {});
		remoteCustomBits = library;
		remoteCustomBitsStatus = status;
		missingBitReferences.add("private-bit");
		serverProfile = {
			...serverProfile,
			name: "Renamed on the hub",
			bits: ["private-bit"],
			updated: "2026-10-01T10:00:01.000Z",
		};

		await act(async () => render());
		await settle();

		expect(
			mocks.invoke.mock.calls.some(
				([command]) => command === "prune_missing_profile_bits",
			),
		).toBe(false);
		expect(profile.bits).toEqual(["private-bit"]);
		expect(queryClient.getQueryData<IProfile>(["getProfile"])?.name).toBe(
			"Renamed on the hub",
		);
	},
);

test("a cleanup failure still refreshes the profile after a successful pull", async () => {
	const nativeCommand = mocks.invoke.getMockImplementation();
	const failure = new Error("Could not save settings");
	const warning = vi.spyOn(console, "warn").mockImplementation(() => {});
	mocks.invoke.mockImplementation(async (command, args) => {
		if (command === "prune_missing_profile_bits") throw failure;
		return nativeCommand?.(command, args);
	});
	serverProfile = {
		...serverProfile,
		name: "Renamed on the hub",
		updated: "2026-10-01T10:00:01.000Z",
	};

	await act(async () => render());
	await settle();

	expect(queryClient.getQueryData<IProfile>(["getProfile"])?.name).toBe(
		"Renamed on the hub",
	);
	expect(warning).toHaveBeenCalledWith(
		"Failed to remove missing profile bits:",
		failure,
	);
});

test("manual saves, profile switches and authentication changes still request synchronization", async () => {
	await act(async () => render());
	await settle();
	await act(async () => {
		window.dispatchEvent(new CustomEvent("flow-like:profile-sync"));
	});
	await settle();
	expect(pushes()).toHaveLength(2);
	profile = { ...profile, id: "work" };
	serverProfile = { ...serverProfile, id: "work" };
	await act(async () => {
		await queryClient.invalidateQueries({ queryKey: ["getProfile"] });
	});
	await settle();
	expect(pushes()).toHaveLength(3);
	accessToken = "token-b";
	await act(async () => render());
	await settle();
	expect(pushes()).toHaveLength(4);
	signedIn = false;
	await act(async () => render());
	await act(async () => {
		await vi.advanceTimersByTimeAsync(60_000);
	});
	expect(pushes()).toHaveLength(4);
});
