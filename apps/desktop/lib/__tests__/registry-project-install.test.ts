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

const PACKAGE = "com.example.sales";
const PINNED = "1.2.0";
const HASH = "ab".repeat(32);
const DIGEST = `sha256:${"cd".repeat(32)}`;
const installed = (version = PINNED) => ({
	id: PACKAGE,
	version,
	manifest: { widgets: [{ id: "chart" }] },
});

/** The install queue and its failure memory are module state, so every test uses its own project. */
let projects = 0;
const nextProject = () => `app-${++projects}`;

function backend({
	signedIn = true,
	offline = false,
	pins = { [PACKAGE]: PINNED } as Record<string, string>,
} = {}) {
	return {
		profile: signedIn ? { hub: "hub.example" } : undefined,
		auth: signedIn
			? { isAuthenticated: true, user: { access_token: "token" } }
			: undefined,
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
const installArgs = (appId: string, version = PINNED) => [
	"registry_install_package",
	{ packageId: PACKAGE, version, token: "token", appId },
];

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

beforeEach(() => {
	mocks.invoke.mockReset();
	mocks.fetcher.mockReset();
});

afterEach(() => {
	vi.restoreAllMocks();
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
		const state = new RegistryState(backend({ pins: {} }) as never);
		await expect(state.getPackage(PACKAGE, nextProject())).resolves.toEqual(
			installed("1.0.0"),
		);
		expect(installs()).toEqual([]);
	});

	test("without a project, a sign-in or an online project nothing is installed", async () => {
		nativeCommands(localFollowsInstalls());
		await expect(
			new RegistryState(backend() as never).getPackage(PACKAGE),
		).resolves.toBeNull();
		await expect(
			new RegistryState(backend({ signedIn: false }) as never).getPackage(
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
	});

	test("a refused install resolves to nothing and is not retried for five minutes", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		const now = vi.spyOn(Date, "now").mockReturnValue(1_000_000);
		nativeCommands({
			registry_get_package: () => null,
			registry_install_package: () => {
				throw { error: "403 Forbidden" };
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
	});

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

describe("desktop widget describe for a missing bundle", () => {
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
	const describes = () => callsOf("registry_describe_widget_policy");

	test("installs the pinned version through the project and describes once more", async () => {
		nativeCommands({
			registry_describe_widget_policy: (call) => {
				if (call === 1) throw notInstalled;
				return descriptor;
			},
		});
		const state = new RegistryState(backend() as never);
		const appId = nextProject();
		await expect(
			state.describeWidgetPolicy({ ...request, appId }),
		).resolves.toMatchObject({ policyDigest: DIGEST, status: "ok" });
		expect(installs()).toEqual([installArgs(appId)]);
		expect(describes()).toHaveLength(2);
	});

	test("a widget placed with another version than the pin keeps the original error", async () => {
		nativeCommands({
			registry_describe_widget_policy: () => {
				throw notInstalled;
			},
		});
		const state = new RegistryState(
			backend({ pins: { [PACKAGE]: "1.3.0" } }) as never,
		);
		await expect(
			state.describeWidgetPolicy({ ...request, appId: nextProject() }),
		).rejects.toBe(notInstalled);
		expect(installs()).toEqual([]);
		expect(describes()).toHaveLength(1);
	});

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
