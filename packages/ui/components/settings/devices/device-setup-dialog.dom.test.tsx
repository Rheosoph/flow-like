import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { ReleaseConfig } from "../../../lib/device-management/package";
import type { DeviceSetupReadiness } from "../../../lib/device-management/readiness";
import { standalonePackageModes } from "../../../lib/device-package";
import type { IProfile } from "../../../types";

const window = new Window({ url: "https://app.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	Element: window.Element,
	DocumentFragment: window.DocumentFragment,
	CustomEvent: window.CustomEvent,
	NodeFilter: window.NodeFilter,
	getComputedStyle: window.getComputedStyle.bind(window),
	requestAnimationFrame: (callback: FrameRequestCallback) =>
		setTimeout(() => callback(0), 0),
	cancelAnimationFrame: clearTimeout,
	Node: window.Node,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (error: Error) => void;
	const promise = new Promise<T>((yes, no) => {
		resolve = yes;
		reject = no;
	});
	return { promise, resolve, reject };
}
const checks: {
	request: ReturnType<typeof deferred<DeviceSetupReadiness>>;
	signal: AbortSignal;
}[] = [];
const releases: {
	request: ReturnType<typeof deferred<unknown>>;
	signal: AbortSignal;
}[] = [];
const enrollments: unknown[] = [];
const api = {};
// Bun module mocks also affect later test files. Retain actual functions before
// replacing the dialog's network boundary, then restore them for package tests.
const actualReadiness = {
	...(await import("../../../lib/device-management/readiness")),
};
const actualPackage = {
	...(await import("../../../lib/device-management/package")),
};
const actualSetup = {
	...(await import("../../../lib/device-management/setup")),
};
mock.module("../../../state/backend-state", () => ({
	useBackendReady: () => true,
	useBackend: () => ({ apiState: api }),
}));
mock.module("../../../lib/device-management/readiness", () => ({
	...actualReadiness,
	checkDeviceSetup: (_api: unknown, _profile: unknown, signal: AbortSignal) => {
		const request = deferred<DeviceSetupReadiness>();
		checks.push({ request, signal });
		return request.promise;
	},
}));
mock.module("../../../lib/device-management/package", () => ({
	...actualPackage,
	standalonePackageModes,
	fetchVerifiedRelease: (_release: unknown, signal: AbortSignal) => {
		const request = deferred<unknown>();
		releases.push({ request, signal });
		return request.promise;
	},
}));
mock.module("../../../lib/device-management/setup", () => ({
	...actualSetup,
	prepareDevicePackage: async (options: unknown) => {
		enrollments.push(options);
		throw new Error("Enrollment reached");
	},
}));
const { createRoot } = await import("react-dom/client");
const { DeviceSetupDialog } = await import("./device-setup-dialog");
const profile = { id: "profile" } as IProfile;
const scope = {
	issuer: "issuer",
	account: "account",
	apiOrigin: "https://api.test",
	profileId: "profile",
};
const release = {} as ReleaseConfig;
const ready = (value: boolean): DeviceSetupReadiness => ({
	version: 1,
	ready: value,
	checks: ["policy", "signing", "api", "signaling", "release", "database"].map(
		(id) => ({
			id: id as DeviceSetupReadiness["checks"][number]["id"],
			ready: value,
			message: value ? `${id} ready` : `${id} blocked`,
		}),
	),
});
const verified = {
	targets: ["x86_64-unknown-linux-gnu"],
	manifest: {
		release_version: "1.0.0",
		artifacts: [{ target: "x86_64-unknown-linux-gnu", size: 128 }],
		container: { platforms: ["linux/amd64"] },
	},
};
let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
	checks.length = 0;
	releases.length = 0;
	enrollments.length = 0;
});
afterAll(() => {
	mock.module(
		"../../../lib/device-management/readiness",
		() => actualReadiness,
	);
	mock.module("../../../lib/device-management/package", () => actualPackage);
	mock.module("../../../lib/device-management/setup", () => actualSetup);
});
async function fixture() {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const render = async (nextRelease = release) => {
		await act(async () =>
			root.render(
				<DeviceSetupDialog
					profile={profile}
					scope={scope}
					release={nextRelease}
					onClose={() => {}}
				/>,
			),
		);
	};
	await render();
	cleanup = async () => {
		await act(async () => root.unmount());
		container.remove();
	};
	const button = (text: string) => {
		const found = [...document.body.querySelectorAll("button")].find(
			(item) => item.textContent === text,
		);
		if (!found) throw new Error(`Missing button ${text}`);
		return found;
	};
	const submit = async () => {
		await act(async () => {
			document.body
				.querySelector("form")
				?.dispatchEvent(
					new Event("submit", { bubbles: true, cancelable: true }),
				);
		});
	};
	return { container: document.body, render, button, submit };
}
test("API readiness gates release fetching and enrollment, including direct form submission", async () => {
	const f = await fixture();
	expect(checks).toHaveLength(1);
	expect(releases).toHaveLength(0);
	expect(f.button("Create deployment package").disabled).toBe(true);
	await f.submit();
	expect(enrollments).toHaveLength(0);
	await act(async () => checks[0].request.resolve(ready(false)));
	expect(f.container.textContent).toContain("database blocked");
	expect(releases).toHaveLength(0);
	await f.submit();
	expect(enrollments).toHaveLength(0);
	await act(async () => f.button("Check again").click());
	await act(async () => checks[1].request.resolve(ready(true)));
	expect(releases).toHaveLength(1);
	expect(f.button("Create deployment package").disabled).toBe(true);
	await f.submit();
	expect(enrollments).toHaveLength(0);
	await act(async () => releases[0].request.resolve(verified));
	expect(f.button("Create deployment package").disabled).toBe(false);
	await f.submit();
	expect(enrollments).toHaveLength(1);
});
test("retry ignores stale readiness and stale release responses", async () => {
	const f = await fixture();
	await act(async () => f.button("Check again").click());
	expect(checks[0].signal.aborted).toBe(true);
	await act(async () => checks[0].request.resolve(ready(true)));
	expect(releases).toHaveLength(0);
	await act(async () => checks[1].request.resolve(ready(true)));
	expect(releases).toHaveLength(1);
	await act(async () => f.button("Check again").click());
	expect(releases[0].signal.aborted).toBe(true);
	await act(async () => releases[0].request.resolve(verified));
	expect(f.button("Create deployment package").disabled).toBe(true);
	await f.submit();
	expect(enrollments).toHaveLength(0);
	await act(async () => checks[2].request.resolve(ready(false)));
	expect(releases).toHaveLength(1);
});
test("failed checks show a retryable error without exposing backend details", async () => {
	const f = await fixture();
	await act(async () =>
		checks[0].request.reject(new Error("private backend detail")),
	);
	expect(f.container.textContent).toContain("No enrollment was created");
	expect(f.container.textContent).not.toContain("private backend detail");
	await act(async () => f.button("Check again").click());
	expect(f.container.textContent).not.toContain("No enrollment was created");
	await act(async () => checks[1].request.resolve(ready(true)));
	await act(async () => releases[0].request.resolve(verified));
	expect(f.button("Create deployment package").disabled).toBe(false);
});

test("oversized Linux release selects Docker before enrollment and disables unusable native targets", async () => {
	const f = await fixture();
	await act(async () => checks[0].request.resolve(ready(true)));
	await act(async () =>
		releases[0].request.resolve({
			...verified,
			targets: ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"],
			manifest: {
				...verified.manifest,
				artifacts: [
					{ target: "x86_64-unknown-linux-gnu", size: 512 * 1024 * 1024 },
					{ target: "aarch64-apple-darwin", size: 512 * 1024 * 1024 },
				],
			},
		}),
	);
	const selects = f.container.querySelectorAll<HTMLSelectElement>("select");
	const mode = selects[1];
	expect(mode.value).toBe("docker");
	expect(
		mode.querySelector<HTMLOptionElement>('[value="binary"]')?.disabled,
	).toBe(true);
	expect(
		selects[0].querySelector<HTMLOptionElement>(
			'[value="aarch64-apple-darwin"]',
		)?.disabled,
	).toBe(true);
	expect(f.container.textContent).toContain("browser package limit of 256 MiB");
	await act(async () => {
		mode.value = "binary";
		mode.dispatchEvent(new Event("change", { bubbles: true }));
	});
	await f.submit();
	expect(enrollments).toHaveLength(0);
	await act(async () => {
		mode.value = "docker";
		mode.dispatchEvent(new Event("change", { bubbles: true }));
	});
	await f.submit();
	expect(enrollments).toHaveLength(1);
	expect((enrollments[0] as { mode: string }).mode).toBe("docker");
});

test("a release with no browser-compatible packaging mode cannot create enrollment", async () => {
	const f = await fixture();
	await act(async () => checks[0].request.resolve(ready(true)));
	await act(async () =>
		releases[0].request.resolve({
			...verified,
			targets: ["aarch64-apple-darwin"],
			manifest: {
				...verified.manifest,
				artifacts: [
					{ target: "aarch64-apple-darwin", size: 512 * 1024 * 1024 },
				],
			},
		}),
	);
	expect(f.container.textContent).toContain("no browser package");
	expect(f.button("Create deployment package").disabled).toBe(true);
	await f.submit();
	expect(enrollments).toHaveLength(0);
});
