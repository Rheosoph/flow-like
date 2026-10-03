import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { ReactNode } from "react";
import { installDom, settle } from "../testing/dom-harness";

const dom = installDom();
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const harness = await import("./test-harness");
const hooks = await import("./index");
const registry = await import(
	"../../../../lib/device-management/workspace/registry"
);

const { TestProviders, createTestWorkspace, TEST_AUTH, TEST_ME, TEST_PROFILE } =
	harness;
const SIGNED_OUT = { ...TEST_AUTH, signedIn: false, account: "" };
const restoreBackend = harness.installTestBackend();

afterEach(async () => {
	await dom.cleanup();
	await registry.disposeAllDeviceWorkspaces();
	registry.dismissWorkspaceSwitch();
});
afterAll(() => {
	restoreBackend();
	dom.restore();
});

function Probe({ onRender }: Readonly<{ onRender: () => ReactNode }>) {
	return <div data-probe="">{onRender()}</div>;
}

describe("provider gates", () => {
	test("signed out shows the sign-in gate and never renders the children", async () => {
		const test = createTestWorkspace();
		let rendered = 0;
		let signIns = 0;
		const signIn = () => {
			signIns++;
		};
		const view = await dom.render(
			<TestProviders test={test} auth={{ ...SIGNED_OUT, signIn }}>
				<Probe
					onRender={() => {
						rendered++;
						return null;
					}}
				/>
			</TestProviders>,
		);
		await test.idle();
		expect(rendered).toBe(0);
		expect(view.container.textContent).toContain("Sign in to manage devices");
		(view.container.querySelector("button") as HTMLElement).click();
		expect(signIns).toBe(1);
		expect(test.hub.calls).toEqual([]);
	});

	test("while the session is still loading the gate is a skeleton, not a sign-in prompt", async () => {
		const test = createTestWorkspace();
		const view = await dom.render(
			<TestProviders test={test} auth={{ ...SIGNED_OUT, loading: true }}>
				<span>inside</span>
			</TestProviders>,
		);
		expect(
			view.container.querySelector('[data-kind="loading"]'),
		).not.toBeNull();
		expect(view.container.textContent).not.toContain("Sign in");
		expect(view.container.textContent).not.toContain("inside");
	});

	test("passive skips the gates and renders the fallback in place of the children", async () => {
		const test = createTestWorkspace();
		const view = await dom.render(
			<TestProviders
				test={test}
				passive
				fallback={<span>no devices here</span>}
				auth={SIGNED_OUT}
			>
				<span>inside</span>
			</TestProviders>,
		);
		expect(view.container.textContent).toBe("no devices here");
	});

	test("a custom gate renderer replaces the built-in block and gets the reason", async () => {
		const client = new QueryClient();
		const view = await dom.render(
			<QueryClientProvider client={client}>
				<hooks.DeviceWorkspaceProvider
					overrides={{ auth: SIGNED_OUT, profile: TEST_PROFILE }}
					renderGate={(gate) => <p>custom {gate.kind}</p>}
				>
					<span>inside</span>
				</hooks.DeviceWorkspaceProvider>
			</QueryClientProvider>,
		);
		expect(view.container.textContent).toBe("custom signed_out");
	});

	test("passive with a gate renderer learns why there is no workspace", async () => {
		const reason = (auth: typeof SIGNED_OUT) =>
			dom.render(
				<QueryClientProvider client={new QueryClient()}>
					<hooks.DeviceWorkspaceProvider
						passive
						fallback={<span>no devices here</span>}
						overrides={{ auth, profile: TEST_PROFILE }}
						renderGate={(gate) => <p>because {gate.kind}</p>}
					>
						<span>inside</span>
					</hooks.DeviceWorkspaceProvider>
				</QueryClientProvider>,
			);
		const signedOut = await reason(SIGNED_OUT);
		expect(signedOut.container.textContent).toBe("because signed_out");
		await dom.cleanup();
		const loading = await reason({ ...SIGNED_OUT, loading: true });
		expect(loading.container.textContent).toBe("because loading");
	});

	test("passive without a fallback renders nothing while there is no workspace", async () => {
		const test = createTestWorkspace();
		const view = await dom.render(
			<TestProviders test={test} passive auth={SIGNED_OUT}>
				<span>inside</span>
			</TestProviders>,
		);
		expect(view.container.textContent).toBe("");
	});

	test("passive keeps the fleet-wide hub reads off: only the device list is fetched", async () => {
		const test = createTestWorkspace();
		await dom.render(
			<TestProviders test={test} passive>
				<span>cell</span>
			</TestProviders>,
		);
		await test.idle();
		expect(test.hub.calls.map(([, path]) => path)).toEqual(["devices"]);
	});
});

describe("binding", () => {
	test("signed in: children get the workspace, the rows and the scope", async () => {
		const test = createTestWorkspace();
		const seen: { rows?: number; scope?: string; passive?: boolean } = {};
		function Reader() {
			const workspace = hooks.useDeviceWorkspace();
			seen.rows = hooks.useDeviceRows().rows?.length;
			seen.scope = hooks.useDeviceScope()?.account;
			seen.passive = hooks.useWorkspacePassive();
			return (
				<span>
					{workspace.scopeKey === test.workspace.scopeKey ? "bound" : "other"}
				</span>
			);
		}
		const view = await dom.render(
			<TestProviders test={test}>
				<Reader />
			</TestProviders>,
		);
		await test.idle();
		expect(view.container.textContent).toBe("bound");
		expect(seen).toEqual({ rows: 1, scope: TEST_ME, passive: false });
		expect(test.hub.calls.map(([, path]) => path)).toContain("devices");
	});

	test("hooks keep handles out of React: a key session is a plain snapshot", async () => {
		const test = createTestWorkspace();
		let session: unknown;
		function Reader() {
			session = hooks.useKeySession("dev-1");
			return null;
		}
		await dom.render(
			<TestProviders test={test}>
				<Reader />
			</TestProviders>,
		);
		await test.idle();
		await test.unlock("dev-1", false);
		await test.idle();
		expect(session).toMatchObject({ deviceId: "dev-1", state: "unlocked" });
		expect(JSON.parse(JSON.stringify(session))).toEqual(session);
		expect(test.workspace.keys.controller("dev-1")).toBeDefined();
	});

	test("unmounting keeps the workspace; another profile locks it and reports the switch", async () => {
		const client = new QueryClient({
			defaultOptions: { queries: { retry: false } },
		});
		const fake = createTestWorkspace();
		const create = (deps: Parameters<typeof registry.getDeviceWorkspace>[0]) =>
			registry.getDeviceWorkspace(deps, {
				storage: null,
				fetch: fake.hub.fetch,
				every: () => () => undefined,
			});
		const tree = (profileId: string) => (
			<QueryClientProvider client={client}>
				<hooks.DeviceWorkspaceProvider
					overrides={{
						auth: TEST_AUTH,
						profile: { ...TEST_PROFILE, id: profileId },
						crypto: fake.deps.crypto,
						workspace: create,
					}}
				>
					<span>area</span>
				</hooks.DeviceWorkspaceProvider>
			</QueryClientProvider>
		);
		const scopeKey = JSON.stringify([
			TEST_AUTH.issuer,
			TEST_AUTH.account,
			"https://hub.test",
			"profile-1",
		]);
		const view = await dom.render(tree("profile-1"));
		await settle();
		const workspace = registry.openDeviceWorkspace(scopeKey);
		expect(workspace).toBeDefined();

		await view.unmount();
		expect(registry.openDeviceWorkspace(scopeKey)).toBe(workspace);

		const again = await dom.render(tree("profile-1"));
		await settle();
		expect(registry.openDeviceWorkspace(scopeKey)).toBe(workspace);
		expect(registry.lastWorkspaceSwitch()).toBeUndefined();

		await again.rerender(tree("profile-2"));
		await settle();
		expect(registry.openDeviceWorkspace(scopeKey)).toBeUndefined();
		expect(registry.lastWorkspaceSwitch()).toMatchObject({
			changed: ["profile"],
			lockedSessions: 0,
		});
	});

	test("a sign-out while mounted disposes the workspace and shows the gate", async () => {
		const test = createTestWorkspace();
		const view = await dom.render(
			<TestProviders test={test}>
				<span>area</span>
			</TestProviders>,
		);
		await test.idle();
		await test.unlock("dev-1", false);
		expect(test.workspace.keys.snapshot("dev-1").state).toBe("unlocked");

		await view.rerender(
			<TestProviders test={test} auth={SIGNED_OUT}>
				<span>area</span>
			</TestProviders>,
		);
		await test.idle();
		expect(test.workspace.disposed).toBe(true);
		expect(test.workspace.keys.snapshot("dev-1").state).toBe("locked");
		expect(view.container.textContent).toContain("Sign in to manage devices");
	});
});
