"use client";
import { useEffect, useId, useState } from "react";
import type { PlacementStatus } from "../../../lib/device-management/types";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

export function DeviceReplicaControl({
	placement,
	busy,
	request,
}: {
	placement: PlacementStatus;
	busy: boolean;
	request: (command: Record<string, unknown>) => Promise<void>;
}) {
	const id = useId();
	const [desired, setDesired] = useState(
		String(placement.desired_replicas ?? 1),
	);
	useEffect(() => {
		setDesired(String(placement.desired_replicas ?? 1));
	}, [placement.desired_replicas]);
	if (
		placement.max_replicas === undefined ||
		placement.desired_replicas === undefined ||
		placement.ready_replicas === undefined ||
		placement.running_replicas === undefined
	)
		return null;
	const count = Number(desired);
	const valid =
		Number.isInteger(count) && count >= 1 && count <= placement.max_replicas;
	return (
		<div className="w-full space-y-2 border-t pt-2">
			<p className="text-xs text-muted-foreground">
				{placement.ready_replicas} ready · {placement.running_replicas} running
				· {placement.desired_replicas} requested
			</p>
			{placement.max_replicas > 1 && (
				<form
					className="flex items-center gap-2"
					onSubmit={(event) => {
						event.preventDefault();
						if (valid)
							void request({
								type: "scale",
								placement_id: placement.id,
								expected_revision: placement.config_revision,
								replicas: count,
							});
					}}
				>
					<label htmlFor={id} className="text-xs">
						Instances (maximum {placement.max_replicas})
					</label>
					<Input
						id={id}
						type="number"
						min={1}
						max={placement.max_replicas}
						value={desired}
						onChange={(event) => setDesired(event.target.value)}
						className="w-24"
						disabled={busy}
					/>
					<Button
						type="submit"
						variant="outline"
						size="sm"
						disabled={busy || !valid || count === placement.desired_replicas}
					>
						Scale
					</Button>
				</form>
			)}
			{placement.replicas?.length ? (
				<details>
					<summary className="cursor-pointer text-xs">Instance status</summary>
					<ul className="mt-1 text-xs">
						{placement.replicas.map((replica) => (
							<li key={replica.slot}>
								Slot {replica.slot}: {replica.observed_state} · applied revision{" "}
								{replica.applied_revision}
							</li>
						))}
					</ul>
				</details>
			) : null}
		</div>
	);
}
