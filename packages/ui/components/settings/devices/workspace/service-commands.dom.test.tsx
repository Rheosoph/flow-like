import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { act } from "react";
import type { GateResult } from "../../../../lib/device-management/model/types";
import {
	click,
	inPortal,
	installDom,
	queryByRole,
	settle,
	typeInto,
} from "../testing/dom-harness";
import type * as Hooks from "./index";
import type * as Harness from "./test-harness";

const dom = installDom();
const harness = await import("./test-harness");
const hooks = await import("./index");

const { Capture, TestProviders, createTestWorkspace, testPlacement } = harness;
const restoreBackend = harness.installTestBackend();

afterEach(dom.cleanup);
afterAll(() => {
	restoreBackend();
	dom.restore();
});

type Commands = Hooks.ServiceCommands;

async function mount(
	options: Parameters<typeof createTestWorkspace>[0] = {},
	unlocked = true,
) {
	const test = createTestWorkspace(options);
	const captured: Partial<Harness.Captured<Commands>> = {};
	await dom.render(
		<TestProviders test={test}>
			<Capture
				read={() => hooks.useServiceCommands("dev-1", "svc-1")}
				into={captured}
			/>
		</TestProviders>,
	);
	await test.idle();
	if (unlocked) {
		await test.unlock("dev-1");
		await test.idle();
	}
	const device = test.devices.get("dev-1") as Harness.FakeDevice;
	const commands = () => captured.current as Commands;
	const tray = () => test.workspace.activity.list();
	return { test, device, commands, tray };
}

const reason = (gate: GateResult) => (gate.ok ? "ok" : gate.copy.code);
const reasons = (commands: Commands) => ({
	start: reason(commands.start.gate),
	stop: reason(commands.stop.gate),
	restart: reason(commands.restart.gate),
	scale: reason(commands.scale(3).gate),
	remove: reason(commands.remove.gate),
});

describe("gates", () => {
	test("a running service on an unlocked, connected device: everything but Remove is allowed", async () => {
		const { commands } = await mount();
		expect(commands().service?.serviceId).toBe("svc-1");
		expect(reasons(commands())).toEqual({
			start: "ok",
			stop: "ok",
			restart: "ok",
			scale: "ok",
			remove: "service_must_be_stopped",
		});
	});

	test("while an update is in progress only Stop is allowed", async () => {
		const { test, commands } = await mount();
		await act(async () => {
			test.workspace.facts.record("dev-1", {
				rollouts: [
					{
						rollout_id: "rollout-1",
						placement_id: "svc-1",
						project_id: "app-1",
						state: "validating",
						created_at: harness.TEST_NOW_MS / 1000,
						updated_at: harness.TEST_NOW_MS / 1000,
					} as never,
				],
			});
		});
		await test.idle();
		expect(commands().service?.conv).toBe("update_in_progress");
		expect(reasons(commands())).toEqual({
			start: "rollout_in_progress",
			stop: "ok",
			restart: "rollout_in_progress",
			scale: "rollout_in_progress",
			remove: "service_must_be_stopped",
		});
		const { gate } = commands().restart;
		expect(gate.ok ? undefined : gate.kind).toBe("busy");
	});

	test("an update in progress carries the time the device ends it on its own", () => {
		const created = harness.TEST_NOW_MS / 1000;
		const updating = (rollout: Record<string, unknown>) =>
			hooks.serviceGateExtra({
				conv: "update_in_progress",
				desired: "running",
				observed: "running",
				instances: { requested: 1, ready: 1, running: 1, max: 2 },
				rollout: { created_at: created, ...rollout },
			} as never);
		expect(
			updating({ state: "validating", deadline_at: created + 120 }),
		).toMatchObject({
			activeRollout: true,
			rolloutState: "validating",
			rolloutDeadlineAt: created + 120,
		});
		expect(updating({ state: "staged" })).toMatchObject({
			activeRollout: true,
			rolloutState: "staged",
			rolloutDeadlineAt: created + 86_400,
		});
	});

	test("locked keys gate every command and a run sends nothing", async () => {
		const { device, commands } = await mount({}, false);
		const gates = reasons(commands());
		expect(new Set(Object.values(gates)).has("ok")).toBe(false);
		const { gate } = commands().stop;
		expect(gate.ok ? undefined : gate.gate).toBe("G7");
		let outcome: Hooks.DeviceActionOutcome<unknown> | undefined;
		await act(async () => {
			outcome = await commands().stop.run({ confirmed: true });
		});
		expect(outcome?.status).toBe("gated");
		expect(device.writes).toEqual([]);
	});

	test("one instance at most: changing the count is not offered", async () => {
		const { commands } = await mount({
			placements: {
				"dev-1": [
					testPlacement("svc-1", { max_replicas: 1, desired_replicas: 1 }),
				],
			},
		});
		expect(reason(commands().scale(2).gate)).toBe("single_instance_only");
	});
});

describe("commands", () => {
	test("Stop: consequence rows, one command with the settings revision, tray done when the device shows it", async () => {
		const { test, device, commands, tray } = await mount();
		const stop = commands().stop;
		expect(stop).toMatchObject({
			label: "Stop svc-1",
			title: "Stop svc-1?",
			sub: "on dev-1",
			strength: "none",
			tone: "danger",
			pending: false,
		});
		expect(stop.rows.what).toBe(
			"Its 2 instances stop. The device keeps it stopped until someone starts it.",
		);
		expect(stop.rows.stays).toContain("Settings v3");
		expect(stop.rows.undo.reversible).toBe(true);

		let outcome: Awaited<ReturnType<typeof stop.run>> | undefined;
		await act(async () => {
			outcome = await stop.run({ confirmed: true });
		});
		expect(queryByRole("alertdialog")).toBeNull();
		expect(outcome?.status).toBe("done");
		expect(device.writes).toEqual([
			{ type: "stop", placement_id: "svc-1", expected_revision: 3 },
		]);
		// The journal only says "accepted": the item waits for the services read.
		expect(tray()).toMatchObject([
			{
				kind: "command",
				state: "waiting",
				label: { code: "command", params: { command: "stop" } },
				target: { deviceId: "dev-1", serviceId: "svc-1", projectId: "app-1" },
				resume: { type: "operation", command: "stop" },
				href: { screen: "service", deviceId: "dev-1", serviceId: "svc-1" },
			},
		]);

		device.inspection = {
			...device.inspection,
			placements: [
				testPlacement("svc-1", {
					desired_state: "stopped",
					observed_state: "stopped",
					running_replicas: 0,
					ready_replicas: 0,
				}),
			],
		};
		await act(() => test.workspace.live.refreshInspection("dev-1"));
		await test.idle();
		expect(tray()).toMatchObject([{ kind: "command", state: "done" }]);
		expect(commands().service?.observed).toBe("stopped");
	});

	test("without an inline confirm the sheet opens with the same rows", async () => {
		const { device, commands } = await mount();
		let pending: ReturnType<Commands["restart"]["run"]> | undefined;
		await act(async () => {
			pending = commands().restart.run();
			await Promise.resolve();
		});
		const sheet = inPortal("alertdialog");
		expect(sheet.textContent).toContain("Restart svc-1?");
		expect(sheet.textContent).toContain("on dev-1");
		expect(sheet.textContent).toContain(
			"Restarts don't count toward the crash-loop limit.",
		);
		await click(sheet.querySelector("[data-confirm]") as HTMLElement);
		await settle();
		expect((await pending)?.status).toBe("done");
		expect(device.writes).toEqual([
			{ type: "restart", placement_id: "svc-1", expected_revision: 3 },
		]);
	});

	test("changing the instance count sends the new count and words the direction", async () => {
		const { device, commands } = await mount();
		const down = commands().scale(1);
		const up = commands().scale(4);
		expect(down.rows.what).toBe("1 instance stops. 1 keep running.");
		expect(down.label).toBe("Run 1 instance of svc-1");
		expect(down.tone).toBe("danger");
		expect(up.rows.what).toBe("2 more instances start with settings v3.");
		expect(up.tone).toBe("default");
		await act(async () => {
			await up.run({ confirmed: true });
		});
		expect(device.writes).toEqual([
			{
				type: "scale",
				placement_id: "svc-1",
				expected_revision: 3,
				replicas: 4,
			},
		]);
	});

	test("Remove needs the service ID typed, also when the screen says it confirmed", async () => {
		const { device, commands } = await mount({
			placements: {
				"dev-1": [
					testPlacement("svc-1", {
						desired_state: "stopped",
						observed_state: "stopped",
					}),
				],
			},
		});
		const remove = commands().remove;
		expect(reason(remove.gate)).toBe("ok");
		expect(remove).toMatchObject({ strength: "typed", typed: "svc-1" });
		expect(remove.rows.undo.reversible).toBe(false);

		let pending: ReturnType<typeof remove.run> | undefined;
		await act(async () => {
			pending = remove.run({ confirmed: true });
			await Promise.resolve();
		});
		const sheet = inPortal("alertdialog");
		const confirm = sheet.querySelector("[data-confirm]") as HTMLElement;
		expect(confirm.getAttribute("aria-disabled")).toBe("true");
		await typeInto(sheet.querySelector("input") as Element, "svc-1");
		await click(confirm);
		await settle();
		expect((await pending)?.status).toBe("done");
		expect(device.writes).toEqual([
			{ type: "remove", placement_id: "svc-1", expected_revision: 3 },
		]);
	});

	test("a refused command names the device's reason under the service's result key", async () => {
		const { test, device, commands } = await mount();
		const results: Partial<Harness.Captured<Hooks.InlineResultView[]>> = {};
		await dom.render(
			<TestProviders test={test}>
				<Capture
					read={() => hooks.useInlineResults(commands().resultKey)}
					into={results}
				/>
			</TestProviders>,
		);
		device.respond = () => ({
			state: "rejected",
			result: {
				code: "revision_conflict",
				error: "Placement svc-1 is at revision 4.",
				retryable: false,
			},
		});
		let outcome: Hooks.DeviceActionOutcome<unknown> | undefined;
		await act(async () => {
			outcome = await commands().start.run({ confirmed: true });
		});
		await test.idle();
		expect(outcome?.status).toBe("rejected");
		expect(commands().resultKey).toBe("service:dev-1/svc-1");
		expect(results.current?.map((row) => [row.state, row.tone])).toEqual([
			["rejected", "critical"],
		]);
		expect(results.current?.[0]?.text).toContain(
			"Placement svc-1 is at revision 4.",
		);
		expect(commands().start.pending).toBe(false);
	});
});
