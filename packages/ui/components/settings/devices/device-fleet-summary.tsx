"use client";
import { useEffect, useState } from "react";
import type { OpenFleet } from "../../../lib/device-management/fleet";
import { visibleInventory } from "../../../lib/device-management/inventory";

/** Compare authorized observations without treating locked or stale devices as zero. */
export function DeviceFleetSummary({
	projectId,
	fleet,
	deviceCount,
}: {
	projectId: string;
	fleet: Record<string, OpenFleet>;
	deviceCount: number;
}) {
	const [now, setNow] = useState(Date.now());
	useEffect(() => {
		const timer = setInterval(() => setNow(Date.now()), 5000);
		return () => clearInterval(timer);
	}, []);
	const rows = Object.entries(fleet);
	return (
		<div className="space-y-2 rounded border p-3">
			<p className="font-medium">Project across devices</p>
			<p className="text-sm text-muted-foreground">
				{rows.length} of {deviceCount} visible devices unlocked for monitoring.
				Locked devices and missing samples are excluded. Each row shows the
				device's last encrypted observation.
			</p>
			{rows.length > 0 && (
				<table className="w-full text-left text-sm">
					<thead>
						<tr>
							<th>Device</th>
							<th>Placements</th>
							<th>Ready / requested</th>
							<th>Observation</th>
						</tr>
					</thead>
					<tbody>
						{rows.map(([id, value]) => {
							const visible = visibleInventory(value.observations, projectId);
							const applicable = value.observations.filter(
								(o) =>
									o.scope.kind === "device" || o.scope.project_id === projectId,
							);
							const at = Math.min(...applicable.map((o) => o.observed_at ?? 0));
							const complete = applicable.some(
								(o) => o.scope.kind !== "placement",
							);
							return (
								<tr key={id}>
									<td>{id}</td>
									<td>
										{applicable.length
											? `${visible.length}${complete ? "" : " (partial access)"}`
											: "Unavailable"}
									</td>
									<td>
										{applicable.length
											? `${visible.reduce((n, v) => n + (v.row.ready_replicas ?? 0), 0)} / ${visible.reduce((n, v) => n + (v.row.desired_replicas ?? 0), 0)}`
											: "Unavailable"}
									</td>
									<td>
										{Number.isFinite(at) && at > 0
											? `${new Date(at).toLocaleTimeString()}${now - at > 90_000 ? " (stale)" : ""}`
											: "No snapshot"}
									</td>
								</tr>
							);
						})}
					</tbody>
				</table>
			)}
		</div>
	);
}
