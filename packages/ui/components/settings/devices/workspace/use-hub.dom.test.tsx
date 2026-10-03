import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import { installDom } from "../testing/dom-harness";
import type * as Hooks from "./index";
import type * as Harness from "./test-harness";

const dom = installDom();
const harness = await import("./test-harness");
const hooks = await import("./index");
const { ApiResponseError } = await import("../../../../lib/api-error");

const { Capture, TestProviders, createTestWorkspace } = harness;
const restoreBackend = harness.installTestBackend();

afterEach(dom.cleanup);
afterAll(() => {
	restoreBackend();
	dom.restore();
});

type Read = Hooks.HubRead<unknown>;

/** Every hook over a route this work adds (plan §3.2). */
const NEW_ROUTE_HOOKS: Record<string, () => Read> = {
	enrollments: () => hooks.useEnrollments(),
	usage: () => hooks.useDeviceUsage(),
	myAccess: () => hooks.useMyAccess("dev-1"),
	accountBackups: () => hooks.useAccountBackups(),
	fleetCertificates: () => hooks.useFleetCertificateInventory(),
	certificateNotices: () => hooks.useCertificateNotices("dev-1"),
	archiveUsage: () => hooks.useArchiveUsage(),
	resourceSummary: () => hooks.useResourceSummary(),
	appPlacements: () => hooks.useAppPlacements("app-1"),
};

function Reads({
	into,
}: Readonly<{ into: Record<string, Partial<Harness.Captured<Read>>> }>) {
	return (
		<>
			{Object.entries(NEW_ROUTE_HOOKS).map(([name, read]) => {
				into[name] ??= {};
				return <Capture key={name} read={read} into={into[name]} />;
			})}
		</>
	);
}

describe("older hub", () => {
	test("a 404 from every new route is missingOnHub, never an error", async () => {
		const test = createTestWorkspace();
		const reads: Record<string, Partial<Harness.Captured<Read>>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Reads into={reads} />
			</TestProviders>,
		);
		await test.idle();
		for (const [name, read] of Object.entries(reads)) {
			expect([name, read.current?.missingOnHub]).toEqual([name, true]);
			expect([name, read.current?.error]).toEqual([name, undefined]);
			expect([name, read.current?.data]).toEqual([name, undefined]);
			expect([name, read.current?.freshness.age]).toEqual([
				name,
				"unsupported",
			]);
			expect(read.current?.freshness.reason).toEqual({
				code: "hub_update_needed",
			});
		}
	});

	test("a 405 reads the same", async () => {
		const refuse = () =>
			new ApiResponseError({ status: 405, message: "Method not allowed" });
		const test = createTestWorkspace({
			routes: {
				"GET devices/usage": refuse,
				"GET devices/dev-1/management/my-access": refuse,
				"GET apps/app-1/device-placements": refuse,
			},
		});
		const reads: Record<string, Partial<Harness.Captured<Read>>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Reads into={reads} />
			</TestProviders>,
		);
		await test.idle();
		for (const name of ["usage", "myAccess", "appPlacements"]) {
			expect([name, reads[name]?.current?.missingOnHub]).toEqual([name, true]);
			expect([name, reads[name]?.current?.error]).toEqual([name, undefined]);
		}
	});

	test("a failing route is an error with a reason and a retry time, and keeps earlier data", async () => {
		let fail = false;
		const usage = {
			server_time: harness.TEST_NOW_MS / 1000,
			limits: {
				max_devices: 5,
				max_pending_enrollments: 2,
				enrollment_ttl_seconds: 60,
				max_enrollments_per_day: 3,
				max_account_backups: 4,
			},
			usage: {
				active_devices: 1,
				revoked_devices: 0,
				pending_enrollments: 0,
				enrollments_last_24h: 0,
				account_backups: 0,
			},
		};
		const test = createTestWorkspace({
			routes: {
				"GET devices/usage": () =>
					fail ? new ApiResponseError({ status: 503, message: "busy" }) : usage,
			},
		});
		const read: Partial<Harness.Captured<Read>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={hooks.useDeviceUsage} into={read} />
			</TestProviders>,
		);
		await test.idle();
		expect(read.current).toMatchObject({ data: usage, missingOnHub: false });
		expect(read.current?.freshness.age).toBe("current");

		fail = true;
		await act(async () => {
			await read.current?.refetch();
		});
		await test.idle();
		expect(read.current?.data).toEqual(usage);
		expect(read.current?.error?.code).toBe("server_error");
		expect(read.current?.freshness).toMatchObject({
			age: "error",
			error: { code: "server_error" },
		});
		expect(read.current?.freshness.error?.retryAt).toBeGreaterThan(0);
		expect(read.current?.freshness.dataFrom).toBeDefined();
	});
});

describe("device list and area gates", () => {
	const mountGate = async (
		prepare: (test: Harness.TestWorkspace) => void = () => undefined,
		routes: Parameters<typeof createTestWorkspace>[0] = {},
	) => {
		const test = createTestWorkspace(routes);
		prepare(test);
		const gate: Partial<Harness.Captured<Hooks.AreaGate>> = {};
		const rows: Partial<Harness.Captured<Hooks.DeviceRowsRead>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture read={hooks.useAreaGate} into={gate} />
				<Capture read={hooks.useDeviceRows} into={rows} />
			</TestProviders>,
		);
		await test.idle();
		return { test, gate, rows };
	};

	test("ready: the rows come with a Hub stamp and no gate", async () => {
		const { gate, rows } = await mountGate();
		expect(gate.current).toEqual({ kind: "ready" });
		expect(rows.current?.rows?.map((row) => row.device_id)).toEqual(["dev-1"]);
		expect(rows.current?.freshness).toMatchObject({
			src: "hub",
			age: "current",
			cadenceS: 30,
		});
	});

	test("devices off: the gate names the hub and only the first list read went out", async () => {
		const { test, gate } = await mountGate((value) => {
			value.hub.hubJson.body = { standalone: { enabled: false } };
		});
		expect(gate.current).toEqual({ kind: "hub_off", host: "hub.test" });
		// The list is read in parallel with the hub record; nothing else follows.
		expect(test.hub.calls).toEqual([["GET", "devices"]]);
	});

	test("unreachable: the gate carries the cause, the failure time and a retry that reports again", async () => {
		// A refusal is a verdict (no automatic retries), so the gate shows at once.
		const { test, gate } = await mountGate((value) => {
			value.hub.hubJson.status = 400;
		});
		const first = gate.current;
		expect(first).toMatchObject({
			kind: "hub_unreachable",
			host: "hub.test",
			error: { code: "invalid_response" },
		});
		if (first?.kind !== "hub_unreachable") throw new Error("not gated");
		expect(first.failedAt).toBeGreaterThan(0);

		await act(() => first.retry());
		await test.idle();
		expect(gate.current?.kind).toBe("hub_unreachable");

		test.hub.hubJson.status = 200;
		const again = gate.current;
		if (again?.kind !== "hub_unreachable") throw new Error("not gated");
		await act(() => again.retry());
		await test.idle();
		expect(gate.current).toEqual({ kind: "ready" });
	});

	test("a restricted access token is its own gate, not a generic error", async () => {
		const { gate, rows } = await mountGate(undefined, {
			routes: {
				"GET devices": () =>
					new ApiResponseError({
						status: 403,
						message:
							"Device management requires an unrestricted personal access token",
					}),
			},
		});
		expect(gate.current).toEqual({ kind: "token_restricted" });
		expect(rows.current?.error?.code).toBe("token_restricted");
		expect(rows.current?.rows).toBeUndefined();
	});
});
