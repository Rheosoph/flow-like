import type { ManagementCall } from "./telemetry";
import type { Inspection, PlacementStatus } from "./types";

function identifier(value: unknown): value is string {
	return typeof value === "string" && /^[A-Za-z0-9_:.-]{1,128}$/u.test(value);
}
function counter(value: unknown): value is number {
	return Number.isSafeInteger(value) && Number(value) >= 0;
}
export function placement(value: unknown): value is PlacementStatus {
	if (!value || typeof value !== "object") return false;
	const row = value as PlacementStatus;
	return (
		identifier(row.id) &&
		identifier(row.project_id) &&
		identifier(row.deployment_id) &&
		typeof row.revision === "string" &&
		row.revision.length <= 256 &&
		typeof row.desired_state === "string" &&
		typeof row.observed_state === "string" &&
		counter(row.config_revision) &&
		counter(row.intent_revision) &&
		(row.applied_revision === null || counter(row.applied_revision)) &&
		counter(row.desired_replicas) &&
		counter(row.running_replicas) &&
		counter(row.ready_replicas) &&
		counter(row.max_replicas) &&
		row.max_replicas >= 1 &&
		row.max_replicas <= 32 &&
		row.desired_replicas >= 1 &&
		row.desired_replicas <= row.max_replicas &&
		Array.isArray(row.replicas) &&
		row.replicas.length <= 32 &&
		row.replicas.every(
			(replica) =>
				counter(replica.slot) &&
				replica.slot < 32 &&
				typeof replica.observed_state === "string" &&
				(replica.applied_revision === null ||
					counter(replica.applied_revision)),
		) &&
		new Set(row.replicas.map((replica) => replica.slot)).size ===
			row.replicas.length
	);
}
/** Each page fits a management frame even when every placement has 32 slots. */
export async function readDeviceInspection(
	call: ManagementCall,
	expectedDevice: string,
): Promise<Inspection> {
	const placements: PlacementStatus[] = [];
	let after: string | null = null;
	let boot: string | null | undefined;
	for (let page = 0; page < 512; page++) {
		const response = await call({ type: "inspect_page", after, limit: 2 });
		if (response.state !== "completed")
			throw new Error("This controller cannot read device placement status.");
		const result = response.result;
		if (
			result.device_id !== expectedDevice ||
			(result.boot_id !== null && !identifier(result.boot_id)) ||
			!Array.isArray(result.placements) ||
			result.placements.length > 2 ||
			(result.next !== null && !identifier(result.next))
		)
			throw new Error("Invalid device inspection page.");
		if (page === 0) boot = result.boot_id as string | null;
		else if (result.boot_id !== boot)
			throw new Error(
				"The device rebooted while status was being read. Refresh its status.",
			);
		for (const row of result.placements) {
			if (!placement(row) || (after !== null && row.id <= after))
				throw new Error("Device inspection pages overlap or changed order.");
			placements.push(row);
			after = row.id;
		}
		if (result.next === null)
			return {
				device_id: expectedDevice,
				boot_id: boot ?? null,
				placements,
				observed_at: Date.now(),
			};
		if (!result.placements.length || result.next !== after)
			throw new Error("Device inspection returned an invalid continuation.");
	}
	throw new Error(
		"Device inspection exceeded 512 pages. Narrow the device inventory before retrying.",
	);
}
