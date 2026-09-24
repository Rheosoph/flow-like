import { expect, test } from "bun:test";
import { readDeviceInspection } from "./inspection";
import type { ManagementCall } from "./telemetry";
function row(id: string) {
	return {
		id,
		project_id: "project",
		deployment_id: "deploy",
		revision: "v1",
		desired_state: "running",
		observed_state: "running",
		config_revision: 1,
		intent_revision: 1,
		applied_revision: 1,
		desired_replicas: 1,
		running_replicas: 1,
		ready_replicas: 1,
		max_replicas: 32,
		replicas: [{ slot: 0, observed_state: "running", applied_revision: 1 }],
	};
}
test("paged inspection preserves exact order and pins one boot/device", async () => {
	const requests: unknown[] = [];
	const call: ManagementCall = async (command) => {
		requests.push(command);
		return {
			operation_id: "op",
			state: "completed",
			result: {
				device_id: "device",
				boot_id: "boot",
				placements: command.after ? [row("c")] : [row("a"), row("b")],
				next: command.after ? null : "b",
			},
		};
	};
	const result = await readDeviceInspection(call, "device");
	expect(result.placements.map((row) => row.id)).toEqual(["a", "b", "c"]);
	expect(result.boot_id).toBe("boot");
	expect(requests).toEqual([
		{ type: "inspect_page", after: null, limit: 2 },
		{ type: "inspect_page", after: "b", limit: 2 },
	]);
});
test("fresh placements and starting replica slots preserve their unapplied revision", async () => {
	const fresh = {
		...row("fresh"),
		desired_state: "stopped",
		observed_state: "unknown",
		applied_revision: null,
		running_replicas: 0,
		ready_replicas: 0,
		replicas: [],
	};
	const starting = {
		...row("starting"),
		observed_state: "starting",
		applied_revision: null,
		ready_replicas: 0,
		replicas: [{ slot: 0, observed_state: "starting", applied_revision: null }],
	};
	const call: ManagementCall = async () => ({
		operation_id: "op",
		state: "completed",
		result: {
			device_id: "device",
			boot_id: null,
			placements: [fresh, starting],
			next: null,
		},
	});
	const result = await readDeviceInspection(call, "device");
	expect(result.placements).toEqual([fresh, starting]);
	expect(result.boot_id).toBeNull();
});
test("inspection rejects reboot, duplicate cursors, wrong audiences and oversized pages", async () => {
	for (const changed of [
		{ boot_id: "other" },
		{ device_id: "other" },
		{ placements: [row("a")] },
		{ next: "bad" },
		{ placements: [], next: "a" },
		{ placements: [row("b"), row("c"), row("d")] },
	]) {
		let page = 0;
		const call: ManagementCall = async () => ({
			operation_id: "op",
			state: "completed",
			result: {
				device_id: "device",
				boot_id: "boot",
				placements: [row(page === 0 ? "a" : "b")],
				next: page++ === 0 ? "a" : null,
				...(page > 1 ? changed : {}),
			},
		});
		await expect(readDeviceInspection(call, "device")).rejects.toThrow();
	}
	const malformed: ManagementCall = async () => ({
		operation_id: "op",
		state: "completed",
		result: {
			device_id: "device",
			boot_id: "boot",
			placements: [
				{
					...row("a"),
					replicas: [
						{ slot: 0, observed_state: "running", applied_revision: 1 },
						{ slot: 0, observed_state: "running", applied_revision: 1 },
					],
				},
			],
			next: null,
		},
	});
	await expect(readDeviceInspection(malformed, "device")).rejects.toThrow();
});
