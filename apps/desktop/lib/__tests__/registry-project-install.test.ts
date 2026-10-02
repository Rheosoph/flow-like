import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
	invoke: mocks.invoke,
}));

vi.mock("../api", () => ({
	fetcher: mocks.fetcher,
}));

vi.mock(
	"@flow-like/flow-like-ui/components/a2ui/use-micro-widget-grant",
	() => ({ forgetMicroWidgetGrants: vi.fn() }),
);

import { RegistryState } from "../../components/tauri-provider/registry-state";
import { ApiResponseError } from "../api-error";
import {
	HUB_REFRESH_TIMEOUT_MS,
	RequestTimeoutError,
} from "../request-deadline";

const PACKAGE = "com.example.sales";
const PINNED = "1.2.0";
const HASH = "ab".repeat(32);
const DIGEST = `sha256:${"cd".repeat(32)}`;
const HUB_REGISTRY = "https://hub.example/api/v1/registry";
const OTHER_REGISTRY = "https://other.example/api/v1/registry";
const SIGNED_IN = { isAuthenticated: true, user: { access_token: "token" } };
const SIGNED_OUT = { isAuthenticated: false, user: null };

const installed = (version = PINNED) => ({
	id: PACKAGE,
	version,
	manifest: { widgets: [{ id: "chart" }] },
});

/** A registry install as the native client records it. */
const installedFrom = (registryUrl: string, version = PINNED) => ({
	...installed(version),
	source: { type: "remote", registry_url: registryUrl, download_url: "" },
});

/** The hub's package entry read through a project; only a member's carries the pin. */
const hubEntry = (pinnedVersion?: string) => ({
	id: PACKAGE,
	manifest: { id: PACKAGE, version: "9.9.9" },
	...(pinnedVersion ? { pinnedVersion } : {}),
});

/** The install queue and its memories are module state, so every test uses its own project. */
let projects = 0;
const nextProject = () => `app-${++projects}`;

function backend({
	auth = SIGNED_IN as unknown,
	profile = { hub: "hub.example" } as
		| { hub?: string; secure?: boolean }
		| undefined,
	offline = false,
	pins = { [PACKAGE]: PINNED } as Record<string, string>,
} = {}) {
	return {
		profile,
		auth,
		isOffline: vi.fn().mockResolvedValue(offline),
		appState: { listPackages: vi.fn().mockResolvedValue(pins) },
	};
}

/** Answers native commands from a table; a function value sees its call count. */
function nativeCommands(
	answers: Record<string, (call: number, args: unknown) => unknown>,
) {
	const calls = new Map<string, number>();
	mocks.invoke.mockImplementation(async (command: string, args: unknown) => {
		const call = (calls.get(command) ?? 0) + 1;
		calls.set(command, call);
		const answer = answers[command];
		return answer ? answer(call, args) : undefined;
	});
}

const callsOf = (command: string) =>
	mocks.invoke.mock.calls.filter(([name]) => name === command);
const installs = () => callsOf("registry_install_package");
const installArgs = (appId: string, version = PINNED, token = "token") => [
	"registry_install_package",
	{ packageId: PACKAGE, version, token, appId },
];
const describes = () => callsOf("registry_describe_widget_policy");

/** The local copy becomes the version of the last finished native install. */
function localFollowsInstalls(initial: string | null = null) {
	let version = initial;
	return {
		registry_get_package: () => (version ? installed(version) : null),
		registry_install_package: (_call: number, args: unknown) => {
			version = (args as { version: string }).version;
		},
	};
}

/** The native client answers an install with a copy from the profile's registry. */
function localReplacedByInstalls(initial: unknown) {
	let local = initial;
	return {
		registry_get_package: () => local,
		registry_install_package: (_call: number, args: unknown) => {
			local = installedFrom(
				HUB_REGISTRY,
				(args as { version: string }).version,
			);
		},
	};
}

const request = {
	packageId: PACKAGE,
	packageVersion: PINNED,
	bundleHash: HASH,
	widgetId: "chart",
	preview: false,
};
const descriptor = {
	source: "registry:hub.example",
	packageId: PACKAGE,
	bundleHash: HASH,
	widgetId: "chart",
	preview: false,
	status: "ok",
	policy: {},
	policyDigest: DIGEST,
};
const notInstalled = {
	error: `Widget bundle ${HASH} of package '${PACKAGE}' is not installed`,
};

/** The bundle is missing until a native install brought it. */
function bundleFollowsInstalls() {
	let bundle = false;
	return {
		registry_install_package: () => {
			bundle = true;
		},
		registry_describe_widget_policy: () => {
			if (!bundle) throw notInstalled;
			return descriptor;
		},
	};
}

beforeEach(() => {
	mocks.invoke.mockReset();
	mocks.fetcher.mockReset();
});

afterEach(() => {
	vi.restoreAllMocks();
	vi.unstubAllEnvs();
});

describe("desktop packages a project pins", () => {
	test("the pinned version installed locally answers without an install", async () => {
		nativeCommands(localFollowsInstalls(PINNED));
		const state = new RegistryState(backend() as never);
		await expect(state.getPackage(PACKAGE, nextProject())).resolves.toEqual(
			installed(),
		);
		expect(installs()).toEqual([]);
	});

	test("a missing package installs the pinned version once, however many ask", async () => {
		nativeCommands(localFollowsInstalls());
		const state = new RegistryState(backend() as never);
		const appId = nextProject();
		const results = await Promise.all([
			state.getPackage(PACKAGE, appId),
			state.getPackage(PACKAGE, appId),
		]);
		expect(results).toEqual([installed(), installed()]);
		expect(installs()).toEqual([installArgs(appId)]);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("another local version is replaced by the pinned one", async () => {
		nativeCommands(localFollowsInstalls("1.0.0"));
		const state = new RegistryState(backend() as never);
		const appId = nextProject();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installed(),
		);
		expect(installs()).toEqual([installArgs(appId)]);
	});

	test("a package the project does not pin stays as installed", async () => {
		nativeCommands(localFollowsInstalls("1.0.0"));
		mocks.fetcher.mockResolvedValue(hubEntry());
		const state = new RegistryState(backend({ pins: {} }) as never);
		await expect(state.getPackage(PACKAGE, nextProject())).resolves.toEqual(
			installed("1.0.0"),
		);
		expect(installs()).toEqual([]);
	});

	test("without a project, a profile or an online project nothing is installed", async () => {
		nativeCommands(localFollowsInstalls());
		await expect(
			new RegistryState(backend() as never).getPackage(PACKAGE),
		).resolves.toBeNull();
		const withoutProfile = backend();
		withoutProfile.profile = undefined;
		await expect(
			new RegistryState(withoutProfile as never).getPackage(
				PACKAGE,
				nextProject(),
			),
		).resolves.toBeNull();
		await expect(
			new RegistryState(backend({ offline: true }) as never).getPackage(
				PACKAGE,
				nextProject(),
			),
		).resolves.toBeNull();
		expect(installs()).toEqual([]);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test.each([
		"403 Forbidden",
		'Failed to download package: 403 Forbidden: {"code":"PACKAGE_NODES_WITHHELD"}',
		"Failed to download package: 402 Payment Required: Purchase required to download this package",
		"Failed to download package: 404 Not Found: Package not found",
	])(
		"an install refused with `%s` resolves to nothing and is not retried for five minutes",
		async (refusal) => {
			const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
			const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
			nativeCommands({
				registry_get_package: () => null,
				registry_install_package: () => {
					throw { error: refusal };
				},
			});
			const state = new RegistryState(backend() as never);
			const appId = nextProject();

			await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
			await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
			expect(installs()).toHaveLength(1);
			expect(warn).toHaveBeenCalledTimes(2);

			now.mockReturnValue(1_000_000 + 5 * 60_000 - 1);
			await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
			expect(installs()).toHaveLength(1);

			now.mockReturnValue(1_000_000 + 5 * 60_000);
			await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
			expect(installs()).toHaveLength(2);
		},
	);

	test.each([
		"error sending request for url (https://hub.example/api/v1/registry/download)",
		"Failed to download package: 503 Service Unavailable",
		"Failed to download package: 502 Bad Gateway: upstream said: 404 Not Found",
		"Failed to download package: 429 Too Many Requests",
		"Failed to download package: 408 Request Timeout",
		"Failed to download WASM from CDN: 403 Forbidden",
		"Failed to download widget bundle: 404 Not Found",
	])(
		"an install that failed with `%s` is tried again after five seconds",
		async (failure) => {
			vi.spyOn(console, "warn").mockImplementation(() => {});
			const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
			let version: string | null = null;
			nativeCommands({
				registry_get_package: () => (version ? installed(version) : null),
				registry_install_package: (call, args) => {
					if (call === 1) throw { error: failure };
					version = (args as { version: string }).version;
				},
			});
			const state = new RegistryState(backend() as never);
			const appId = nextProject();

			await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
			now.mockReturnValue(1_000_000 + 5_000 - 1);
			await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
			expect(installs()).toHaveLength(1);

			now.mockReturnValue(1_000_000 + 5_000);
			await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
				installed(),
			);
			expect(installs()).toHaveLength(2);
		},
	);

	test("installs of one package run one after another", async () => {
		let version: string | null = null;
		let finishFirst: () => void = () => {};
		nativeCommands({
			registry_get_package: () => (version ? installed(version) : null),
			registry_install_package: (call, args) => {
				const requested = (args as { version: string }).version;
				if (call > 1) {
					version = requested;
					return undefined;
				}
				return new Promise<void>((resolve) => {
					finishFirst = () => {
						version = requested;
						resolve();
					};
				});
			},
		});
		const state = new RegistryState(backend() as never);
		const appId = nextProject();

		const other = state.installPackage(PACKAGE, "1.0.0");
		const pinned = state.getPackage(PACKAGE, appId);
		await vi.waitFor(() => expect(installs()).toHaveLength(1));
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(installs()).toHaveLength(1);

		finishFirst();
		await other;
		await expect(pinned).resolves.toEqual(installed());
		expect(installs().map(([, args]) => args.version)).toEqual([
			"1.0.0",
			PINNED,
		]);
	});

	test("an install that waited for one of the pinned version does not repeat it", async () => {
		let version: string | null = null;
		let finishFirst: () => void = () => {};
		nativeCommands({
			registry_get_package: () => (version ? installed(version) : null),
			registry_install_package: () =>
				new Promise<void>((resolve) => {
					finishFirst = () => {
						version = PINNED;
						resolve();
					};
				}),
		});
		const state = new RegistryState(backend() as never);
		const appId = nextProject();

		const runInstall = state.installPackage(PACKAGE, PINNED, undefined, appId);
		const listed = state.getPackage(PACKAGE, appId);
		await vi.waitFor(() => expect(installs()).toHaveLength(1));

		finishFirst();
		await runInstall;
		await expect(listed).resolves.toEqual(installed());
		expect(installs()).toHaveLength(1);
	});
});

describe("desktop installed versions of a pinned package", () => {
	const pinnedManifest = { widgets: [{ id: "chart" }, { id: "table" }] };
	const holdingBoth = (pinnedMetadata?: { name: string }) => ({
		...installed("1.0.0"),
		metadata: { name: "Sales" },
		versions: {
			"1.0.0": { version: "1.0.0", manifest: installed("1.0.0").manifest },
			[PINNED]: {
				version: PINNED,
				manifest: pinnedManifest,
				...(pinnedMetadata ? { metadata: pinnedMetadata } : {}),
			},
		},
	});

	test("an installed version other than the active one answers without an install", async () => {
		const local = holdingBoth();
		nativeCommands({
			registry_get_package: () => local,
			registry_install_package: () => {
				throw { error: "403 Forbidden" };
			},
		});
		const state = new RegistryState(backend() as never);
		await expect(state.getPackage(PACKAGE, nextProject())).resolves.toEqual({
			...local,
			version: PINNED,
			manifest: pinnedManifest,
		});
		expect(installs()).toEqual([]);
		expect(local.version).toBe("1.0.0");
	});

	test("the installed version answers with its own name when it has one", async () => {
		nativeCommands({
			registry_get_package: () => holdingBoth({ name: "Sales 1.2" }),
		});
		const state = new RegistryState(backend() as never);
		await expect(
			state.getPackage(PACKAGE, nextProject()),
		).resolves.toMatchObject({
			version: PINNED,
			metadata: { name: "Sales 1.2" },
		});
	});

	test("a pinned version that is not among the installed ones is installed", async () => {
		let local: unknown = {
			...installed("1.0.0"),
			versions: {
				"1.0.0": { version: "1.0.0", manifest: installed("1.0.0").manifest },
			},
		};
		nativeCommands({
			registry_get_package: () => local,
			registry_install_package: () => {
				local = installed();
			},
		});
		const state = new RegistryState(backend() as never);
		const appId = nextProject();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installed(),
		);
		expect(installs()).toEqual([installArgs(appId)]);
	});

	test("a pin naming an inherited property is not an installed version", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		nativeCommands({
			registry_get_package: () => holdingBoth(),
			registry_install_package: () => {
				throw { error: "404 Not Found" };
			},
		});
		const state = new RegistryState(
			backend({ pins: { [PACKAGE]: "constructor" } }) as never,
		);
		await expect(state.getPackage(PACKAGE, nextProject())).resolves.toBeNull();
		expect(installs()).toHaveLength(1);
		expect(warn).toHaveBeenCalledTimes(1);
	});
});

describe("desktop copies from another registry", () => {
	test("the pinned version another registry installed is installed again through the project", async () => {
		nativeCommands(localReplacedByInstalls(installedFrom(OTHER_REGISTRY)));
		const state = new RegistryState(backend() as never);
		const appId = nextProject();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installedFrom(HUB_REGISTRY),
		);
		expect(installs()).toEqual([installArgs(appId)]);
	});

	test("a version another registry installed beside the active one is installed again", async () => {
		const foreign = {
			...installedFrom(OTHER_REGISTRY, "1.0.0"),
			versions: {
				[PINNED]: { version: PINNED, manifest: installed().manifest },
			},
		};
		nativeCommands(localReplacedByInstalls(foreign));
		const state = new RegistryState(backend() as never);
		const appId = nextProject();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installedFrom(HUB_REGISTRY),
		);
		expect(installs()).toEqual([installArgs(appId)]);
	});

	test("a refused install of such a copy resolves to nothing", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		nativeCommands({
			registry_get_package: () => installedFrom(OTHER_REGISTRY),
			registry_install_package: () => {
				throw { error: "403 Forbidden" };
			},
		});
		const state = new RegistryState(backend() as never);
		await expect(state.getPackage(PACKAGE, nextProject())).resolves.toBeNull();
		expect(installs()).toHaveLength(1);
		expect(warn).toHaveBeenCalledTimes(1);
	});

	test("the copy the native client keeps after its install is the answer", async () => {
		nativeCommands({
			registry_get_package: () => installedFrom(OTHER_REGISTRY),
		});
		const state = new RegistryState(backend() as never);
		const appId = nextProject();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installedFrom(OTHER_REGISTRY),
		);
		expect(installs()).toEqual([installArgs(appId)]);
	});

	test.each([
		[{ hub: "hub.example" }, HUB_REGISTRY],
		[{ hub: "hub.example" }, `${HUB_REGISTRY}/`],
		[{ hub: " https://hub.example/ " }, HUB_REGISTRY],
		[{ hub: "HUB.example:443" }, HUB_REGISTRY],
		[{ hub: "https://hub.example/api/v1" }, HUB_REGISTRY],
		[
			{ hub: "hub.example", secure: false },
			"http://hub.example/api/v1/registry",
		],
		[
			{ hub: "http://localhost:8080", secure: true },
			"http://localhost:8080/api/v1/registry",
		],
		[{}, "https://api.flow-like.com/api/v1/registry"],
		[{ hub: " " }, "https://api.flow-like.com/api/v1/registry"],
	])("the registry of profile %j is %s", async (profile, registryUrl) => {
		nativeCommands({ registry_get_package: () => installedFrom(registryUrl) });
		const state = new RegistryState(backend({ profile }) as never);
		await expect(state.getPackage(PACKAGE, nextProject())).resolves.toEqual(
			installedFrom(registryUrl),
		);
		expect(installs()).toEqual([]);
	});

	test.each([
		[{ hub: "hub.example" }, "http://hub.example/api/v1/registry"],
		[{ hub: "hub.example" }, "https://hub.example:8443/api/v1/registry"],
		[{ hub: "hub.example" }, "https://hub.example/registry"],
		[{ hub: "hub.example" }, "https://api.flow-like.com/api/v1/registry"],
		[{ hub: "hub.example", secure: false }, HUB_REGISTRY],
		[{}, HUB_REGISTRY],
	])(
		"a copy of profile %j recorded from %s is another registry's",
		async (profile, registryUrl) => {
			nativeCommands(localReplacedByInstalls(installedFrom(registryUrl)));
			const state = new RegistryState(backend({ profile }) as never);
			await state.getPackage(PACKAGE, nextProject());
			expect(installs()).toHaveLength(1);
		},
	);

	test("the API override of a build does not move the registry", async () => {
		vi.stubEnv("NEXT_PUBLIC_API_URL", "https://other.example/");
		nativeCommands({ registry_get_package: () => installedFrom(HUB_REGISTRY) });
		const state = new RegistryState(backend() as never);
		await expect(state.getPackage(PACKAGE, nextProject())).resolves.toEqual(
			installedFrom(HUB_REGISTRY),
		);
		expect(installs()).toEqual([]);
	});

	test.each([
		["a developer package", { type: "local", path: "/dev/sales" }],
		["an embedded package", { type: "embedded", data: [] }],
		["a copy without a recorded registry", { type: "remote" }],
		["a copy without a source", undefined],
	])("%s keeps answering", async (_name, source) => {
		const local = { ...installed(), source };
		nativeCommands({ registry_get_package: () => local });
		const state = new RegistryState(backend() as never);
		await expect(state.getPackage(PACKAGE, nextProject())).resolves.toEqual(
			local,
		);
		expect(installs()).toEqual([]);
	});

	test("without a pin, a session or an online project the copy answers as it is", async () => {
		nativeCommands({
			registry_get_package: () => installedFrom(OTHER_REGISTRY),
		});
		mocks.fetcher.mockResolvedValue(hubEntry());
		for (const host of [
			backend({ pins: {} }),
			backend({ auth: SIGNED_OUT }),
			backend({ offline: true }),
		]) {
			await expect(
				new RegistryState(host as never).getPackage(PACKAGE, nextProject()),
			).resolves.toEqual(installedFrom(OTHER_REGISTRY));
		}
		await expect(
			new RegistryState(backend() as never).getPackage(PACKAGE),
		).resolves.toEqual(installedFrom(OTHER_REGISTRY));
		expect(installs()).toEqual([]);
	});

	test("an install that waited behind a failed one still replaces such a copy", async () => {
		const failure = { error: "connection reset" };
		let local: unknown = installedFrom(OTHER_REGISTRY);
		let failFirst: () => void = () => {};
		nativeCommands({
			registry_get_package: () => local,
			registry_install_package: (call, args) => {
				if (call > 1) {
					local = installedFrom(
						HUB_REGISTRY,
						(args as { version: string }).version,
					);
					return undefined;
				}
				return new Promise<void>((_, reject) => {
					failFirst = () => reject(failure);
				});
			},
		});
		const state = new RegistryState(backend() as never);
		const appId = nextProject();

		const other = state.installPackage(PACKAGE, "1.0.0");
		const listed = state.getPackage(PACKAGE, appId);
		await vi.waitFor(() => expect(installs()).toHaveLength(1));
		await new Promise((resolve) => setTimeout(resolve, 0));

		failFirst();
		await expect(other).rejects.toBe(failure);
		await expect(listed).resolves.toEqual(installedFrom(HUB_REGISTRY));
		expect(installs()).toHaveLength(2);
	});
});

describe("desktop project pin of a member who cannot list the pins", () => {
	const lookup = (appId: string) => [
		{ hub: "hub.example" },
		`registry/package/${PACKAGE}?app_id=${appId}`,
		{ method: "GET", timeoutMs: HUB_REFRESH_TIMEOUT_MS },
		SIGNED_IN,
	];

	test("the version the hub names as pinned is installed through the project", async () => {
		nativeCommands(localFollowsInstalls());
		mocks.fetcher.mockResolvedValue(hubEntry(PINNED));
		const state = new RegistryState(backend({ pins: {} }) as never);
		const appId = nextProject();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installed(),
		);
		expect(installs()).toEqual([installArgs(appId)]);
		expect(mocks.fetcher.mock.calls).toEqual([lookup(appId)]);
	});

	test("ids are escaped in the hub request", async () => {
		nativeCommands(localFollowsInstalls());
		mocks.fetcher.mockResolvedValue(hubEntry());
		const state = new RegistryState(backend({ pins: {} }) as never);
		await state.getPackage("a/b?c", "app&1");
		expect(mocks.fetcher.mock.calls[0][1]).toBe(
			"registry/package/a%2Fb%3Fc?app_id=app%261",
		);
	});

	test("a pin listing that fails falls back to the hub too", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		nativeCommands(localFollowsInstalls());
		mocks.fetcher.mockResolvedValue(hubEntry(PINNED));
		const host = backend();
		host.appState.listPackages.mockRejectedValue(new Error("no manifest"));
		const state = new RegistryState(host as never);
		const appId = nextProject();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installed(),
		);
		expect(installs()).toEqual([installArgs(appId)]);
		expect(warn).toHaveBeenCalledTimes(1);
	});

	test("a widget of the version the hub names is described after its install", async () => {
		nativeCommands(bundleFollowsInstalls());
		mocks.fetcher.mockResolvedValue(hubEntry(PINNED));
		const state = new RegistryState(backend({ pins: {} }) as never);
		const appId = nextProject();
		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).resolves.toMatchObject({ policyDigest: DIGEST, status: "ok" });
		expect(installs()).toEqual([installArgs(appId)]);
		expect(describes()).toHaveLength(2);
	});

	test("a widget of another version than the hub names keeps the original error", async () => {
		nativeCommands(bundleFollowsInstalls());
		mocks.fetcher.mockResolvedValue(hubEntry("1.3.0"));
		const state = new RegistryState(backend({ pins: {} }) as never);
		await expect(
			state.describeWidgetPolicy({ ...request, appId: nextProject() }),
		).rejects.toBe(notInstalled);
		expect(installs()).toEqual([]);
		expect(describes()).toHaveLength(1);
	});

	test("the manifest version of an entry that names no pin is never the pin", async () => {
		nativeCommands({
			...bundleFollowsInstalls(),
			registry_get_package: () => installed("1.0.0"),
		});
		mocks.fetcher.mockResolvedValue({
			id: PACKAGE,
			manifest: { id: PACKAGE, version: PINNED },
		});
		const state = new RegistryState(backend({ pins: {} }) as never);
		const appId = nextProject();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installed("1.0.0"),
		);
		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).rejects.toBe(notInstalled);
		expect(installs()).toEqual([]);
	});

	test("the widgets of one page ask the hub once", async () => {
		nativeCommands(bundleFollowsInstalls());
		mocks.fetcher.mockResolvedValue(hubEntry(PINNED));
		const state = new RegistryState(backend({ pins: {} }) as never);
		const appId = nextProject();
		await Promise.all(
			Array.from({ length: 3 }, () =>
				state.describeWidgetPolicy({ ...request, appId }),
			),
		);
		expect(describes()).toHaveLength(6);
		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
		expect(installs()).toEqual([installArgs(appId)]);
	});

	test("the hub's answer is reused for thirty seconds", async () => {
		const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
		nativeCommands(localFollowsInstalls("1.0.0"));
		mocks.fetcher.mockResolvedValue(hubEntry());
		const state = new RegistryState(backend({ pins: {} }) as never);
		const appId = nextProject();

		await state.getPackage(PACKAGE, appId);
		now.mockReturnValue(1_000_000 + 30_000 - 1);
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installed("1.0.0"),
		);
		expect(mocks.fetcher).toHaveBeenCalledTimes(1);

		now.mockReturnValue(1_000_000 + 30_000);
		mocks.fetcher.mockResolvedValue(hubEntry(PINNED));
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installed(),
		);
		expect(mocks.fetcher).toHaveBeenCalledTimes(2);
	});

	test.each([
		["is offline", new Error("Network unavailable: GET registry/package")],
		[
			"timed out",
			new RequestTimeoutError("GET registry/package", HUB_REFRESH_TIMEOUT_MS),
		],
		[
			"is overloaded",
			new ApiResponseError({ status: 503, message: "Service Unavailable" }),
		],
	])(
		"a hub that %s pins nothing and is asked again after five seconds",
		async (_name, failure) => {
			const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
			const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
			nativeCommands(localFollowsInstalls("1.0.0"));
			mocks.fetcher.mockRejectedValue(failure);
			const state = new RegistryState(backend({ pins: {} }) as never);
			const appId = nextProject();

			await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
				installed("1.0.0"),
			);
			now.mockReturnValue(1_000_000 + 5_000 - 1);
			await state.getPackage(PACKAGE, appId);
			expect(mocks.fetcher).toHaveBeenCalledTimes(1);
			expect(warn).toHaveBeenCalledTimes(1);
			expect(installs()).toEqual([]);

			now.mockReturnValue(1_000_000 + 5_000);
			mocks.fetcher.mockResolvedValue(hubEntry(PINNED));
			await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
				installed(),
			);
			expect(mocks.fetcher).toHaveBeenCalledTimes(2);
			expect(installs()).toEqual([installArgs(appId)]);
		},
	);

	test.each([403, 404])(
		"a %i from the hub pins nothing for thirty seconds",
		async (status) => {
			vi.spyOn(console, "warn").mockImplementation(() => {});
			const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
			nativeCommands(localFollowsInstalls("1.0.0"));
			mocks.fetcher.mockRejectedValue(
				new ApiResponseError({ status, message: "refused" }),
			);
			const state = new RegistryState(backend({ pins: {} }) as never);
			const appId = nextProject();

			await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
				installed("1.0.0"),
			);
			now.mockReturnValue(1_000_000 + 30_000 - 1);
			await state.getPackage(PACKAGE, appId);
			expect(mocks.fetcher).toHaveBeenCalledTimes(1);

			now.mockReturnValue(1_000_000 + 30_000);
			await state.getPackage(PACKAGE, appId);
			expect(mocks.fetcher).toHaveBeenCalledTimes(2);
			expect(installs()).toEqual([]);
		},
	);

	test("another session asks the hub again", async () => {
		nativeCommands(localFollowsInstalls());
		mocks.fetcher.mockResolvedValue(hubEntry());
		const host = backend({ pins: {} });
		const state = new RegistryState(host as never);
		const appId = nextProject();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();

		host.auth = { isAuthenticated: true, user: { access_token: "member" } };
		mocks.fetcher.mockResolvedValue(hubEntry(PINNED));
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installed(),
		);
		expect(mocks.fetcher).toHaveBeenCalledTimes(2);
		expect(installs()).toEqual([installArgs(appId, PINNED, "member")]);
	});
});

describe("desktop project installs and the session", () => {
	test.each([
		["signed-out", SIGNED_OUT],
		["restoring", { isAuthenticated: false, isLoading: true }],
		[
			"not authenticated",
			{ isAuthenticated: false, user: { access_token: "stale" } },
		],
		[
			"since expired",
			{ isAuthenticated: true, user: { access_token: "stale", expired: true } },
		],
		["tokenless", { isAuthenticated: true, user: { access_token: "" } }],
	])(
		"a %s session tries nothing, and the sign-in installs at once",
		async (_name, auth) => {
			nativeCommands({
				...localFollowsInstalls(),
				...bundleFollowsInstalls(),
				registry_install_package: () => {
					throw { error: "401 Unauthorized" };
				},
			});
			const host = backend({ auth, pins: {} });
			const state = new RegistryState(host as never);
			const appId = nextProject();

			await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
			await expect(
				state.describeWidgetPolicy({ ...request, appId }),
			).rejects.toBe(notInstalled);
			expect(installs()).toEqual([]);
			expect(mocks.fetcher).not.toHaveBeenCalled();
			expect(host.appState.listPackages).not.toHaveBeenCalled();

			nativeCommands(localFollowsInstalls());
			mocks.fetcher.mockResolvedValue(hubEntry(PINNED));
			host.auth = SIGNED_IN;
			await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
				installed(),
			);
			expect(installs()).toEqual([installArgs(appId)]);
		},
	);

	test("a failed install is tried again at once with another token", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		nativeCommands({
			registry_get_package: () => null,
			registry_install_package: () => {
				throw { error: "401 Unauthorized" };
			},
		});
		const host = backend();
		const state = new RegistryState(host as never);
		const appId = nextProject();

		await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
		expect(installs()).toEqual([installArgs(appId)]);

		host.auth = { isAuthenticated: true, user: { access_token: "renewed" } };
		await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
		await expect(state.getPackage(PACKAGE, appId)).resolves.toBeNull();
		expect(installs()).toEqual([
			installArgs(appId),
			installArgs(appId, PINNED, "renewed"),
		]);
		expect(warn).toHaveBeenCalledTimes(4);
	});

	test("a failed describe install does not block the describe after another sign-in", async () => {
		vi.spyOn(console, "warn").mockImplementation(() => {});
		nativeCommands({
			registry_describe_widget_policy: () => {
				throw notInstalled;
			},
			registry_install_package: () => {
				throw { error: "401 Unauthorized" };
			},
		});
		const host = backend();
		const state = new RegistryState(host as never);
		const appId = nextProject();
		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).rejects.toBe(notInstalled);

		nativeCommands(bundleFollowsInstalls());
		host.auth = { isAuthenticated: true, user: { access_token: "renewed" } };
		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).resolves.toMatchObject({ policyDigest: DIGEST, status: "ok" });
	});

	test("a session that ended while the install waited tries nothing and blocks nothing", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		let version: string | null = null;
		let finishFirst: () => void = () => {};
		nativeCommands({
			registry_get_package: () => (version ? installed(version) : null),
			registry_install_package: (call, args) => {
				const requested = (args as { version: string }).version;
				if (call > 1) {
					version = requested;
					return undefined;
				}
				return new Promise<void>((resolve) => {
					finishFirst = () => {
						version = requested;
						resolve();
					};
				});
			},
		});
		const host = backend();
		const state = new RegistryState(host as never);
		const appId = nextProject();

		const other = state.installPackage(PACKAGE, "1.0.0");
		const listed = state.getPackage(PACKAGE, appId);
		await vi.waitFor(() => expect(installs()).toHaveLength(1));
		await new Promise((resolve) => setTimeout(resolve, 0));

		host.auth = SIGNED_OUT;
		finishFirst();
		await other;
		await expect(listed).resolves.toBeNull();
		expect(installs()).toHaveLength(1);
		expect(warn).toHaveBeenCalledTimes(1);

		host.auth = SIGNED_IN;
		await expect(state.getPackage(PACKAGE, appId)).resolves.toEqual(
			installed(),
		);
		expect(installs().map(([, args]) => args.version)).toEqual([
			"1.0.0",
			PINNED,
		]);
	});
});

describe("desktop widget describe for a missing bundle", () => {
	test("installs the pinned version through the project and describes once more", async () => {
		nativeCommands(bundleFollowsInstalls());
		const state = new RegistryState(backend() as never);
		const appId = nextProject();
		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).resolves.toMatchObject({ policyDigest: DIGEST, status: "ok" });
		expect(installs()).toEqual([installArgs(appId)]);
		expect(describes()).toHaveLength(2);
		expect(mocks.fetcher).not.toHaveBeenCalled();
	});

	test("a widget placed with another version than the listed pin and the hub's keeps the original error", async () => {
		nativeCommands({
			registry_describe_widget_policy: () => {
				throw notInstalled;
			},
		});
		mocks.fetcher.mockResolvedValue(hubEntry("1.3.0"));
		const state = new RegistryState(
			backend({ pins: { [PACKAGE]: "1.3.0" } }) as never,
		);
		await expect(
			state.describeWidgetPolicy({ ...request, appId: nextProject() }),
		).rejects.toBe(notInstalled);
		expect(installs()).toEqual([]);
		expect(describes()).toHaveLength(1);
		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
	});

	test("a widget of the version the hub pins is installed although the listed pin is an older one", async () => {
		nativeCommands(bundleFollowsInstalls());
		mocks.fetcher.mockResolvedValue(hubEntry(PINNED));
		const state = new RegistryState(
			backend({ pins: { [PACKAGE]: "1.0.0" } }) as never,
		);
		const appId = nextProject();
		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).resolves.toMatchObject({ policyDigest: DIGEST, status: "ok" });
		expect(installs()).toEqual([installArgs(appId)]);
		expect(describes()).toHaveLength(2);
		expect(mocks.fetcher).toHaveBeenCalledTimes(1);
	});

	test.each([
		["no version", undefined],
		["an empty version", ""],
	])(
		"a placement with %s keeps the original error and installs nothing",
		async (_name, packageVersion) => {
			nativeCommands({
				registry_describe_widget_policy: () => {
					throw notInstalled;
				},
			});
			mocks.fetcher.mockResolvedValue(hubEntry());
			const state = new RegistryState(backend({ pins: {} }) as never);
			await expect(
				state.describeWidgetPolicy({
					...request,
					packageVersion: packageVersion as string,
					appId: nextProject(),
				}),
			).rejects.toBe(notInstalled);
			expect(installs()).toEqual([]);
			expect(describes()).toHaveLength(1);
			expect(mocks.fetcher).not.toHaveBeenCalled();
		},
	);

	test("other failures and widgets outside a project keep the original error", async () => {
		nativeCommands({
			registry_describe_widget_policy: () => {
				throw notInstalled;
			},
		});
		const state = new RegistryState(backend() as never);
		await expect(state.describeWidgetPolicy(request)).rejects.toBe(
			notInstalled,
		);

		const other = { error: 'Invalid widget id "x"' };
		nativeCommands({
			registry_describe_widget_policy: () => {
				throw other;
			},
		});
		await expect(
			state.describeWidgetPolicy({ ...request, appId: nextProject() }),
		).rejects.toBe(other);
		expect(installs()).toEqual([]);
	});

	test("a refused install keeps the original error", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		nativeCommands({
			registry_describe_widget_policy: () => {
				throw notInstalled;
			},
			registry_install_package: () => {
				throw { error: "403 Forbidden" };
			},
		});
		const state = new RegistryState(backend() as never);
		await expect(
			state.describeWidgetPolicy({ ...request, appId: nextProject() }),
		).rejects.toBe(notInstalled);
		expect(warn).toHaveBeenCalled();
		expect(describes()).toHaveLength(1);
	});

	test("a describe whose install failed in transit installs again five seconds later", async () => {
		vi.spyOn(console, "warn").mockImplementation(() => {});
		const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
		const bundle = bundleFollowsInstalls();
		nativeCommands({
			...bundle,
			registry_install_package: (call) => {
				if (call === 1) throw { error: "error sending request" };
				bundle.registry_install_package();
			},
		});
		const state = new RegistryState(backend() as never);
		const appId = nextProject();

		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).rejects.toBe(notInstalled);
		now.mockReturnValue(1_000_000 + 5_000 - 1);
		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).rejects.toBe(notInstalled);
		expect(installs()).toHaveLength(1);

		now.mockReturnValue(1_000_000 + 5_000);
		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).resolves.toMatchObject({ policyDigest: DIGEST, status: "ok" });
		expect(installs()).toHaveLength(2);
	});

	test("a bundle still missing after the install surfaces the second failure", async () => {
		nativeCommands({
			registry_describe_widget_policy: () => {
				throw notInstalled;
			},
		});
		const state = new RegistryState(backend() as never);
		await expect(
			state.describeWidgetPolicy({ ...request, appId: nextProject() }),
		).rejects.toBe(notInstalled);
		expect(installs()).toHaveLength(1);
		expect(describes()).toHaveLength(2);
	});
});
