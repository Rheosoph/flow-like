"use client";

import { useId } from "react";
import type { PlacementResources } from "../../../lib/device-management/deployment";
import { Input } from "../../ui/input";

export function DeviceIsolationFields({
	value,
	onChange,
}: {
	value: PlacementResources | null;
	onChange: (value: PlacementResources | null) => void;
}) {
	const id = useId();
	const enabled = value?.profile === "linux_sandbox";
	return (
		<fieldset className="space-y-3 rounded border p-3">
			<legend className="px-1 text-sm font-medium">Placement isolation</legend>
			<label className="flex items-center gap-2 text-sm">
				<input
					type="checkbox"
					checked={enabled}
					onChange={(event) =>
						onChange(
							event.target.checked
								? {
										profile: "linux_sandbox",
										cpu_millis: 1000,
										memory_bytes: 1024 ** 3,
										max_processes: 256,
										disk_bytes: 4 * 1024 ** 3,
									}
								: null,
						)
					}
				/>
				Require Linux sandbox and resource limits
			</label>
			<p className="text-xs text-muted-foreground">
				The default uses the device agent’s account and is suitable for trusted
				projects on a dedicated device. Isolation requires a prepared Linux
				host; startup fails if the requested boundary or limits cannot be
				enforced.
			</p>
			{enabled && value && (
				<>
					<div className="grid gap-3 sm:grid-cols-2">
						{(
							[
								["cpu_millis", "CPU per instance (millicores)", 1, 1, 1024000],
								[
									"memory_bytes",
									"Memory per instance (MiB)",
									1024 ** 2,
									64,
									16 * 1024 ** 2,
								],
								[
									"max_processes",
									"Processes and threads per instance",
									1,
									16,
									65536,
								],
								[
									"disk_bytes",
									"Placement disk limit (MiB)",
									1024 ** 2,
									16,
									1024 ** 3,
								],
							] as const
						).map(([field, label, scale, minimum, maximum]) => (
							<label
								key={field}
								htmlFor={`${id}-${field}`}
								className="space-y-1 text-sm"
							>
								<span>{label}</span>
								<Input
									id={`${id}-${field}`}
									type="number"
									min={minimum}
									max={maximum}
									step={1}
									value={value[field] == null ? "" : value[field] / scale}
									onChange={(event) =>
										onChange({
											...value,
											[field]:
												event.target.value === ""
													? null
													: Number(event.target.value) * scale,
										})
									}
								/>
							</label>
						))}
					</div>
					<p className="text-xs text-muted-foreground">
						1000 millicores is one CPU. Instances share the placement’s disk
						quota. The host needs Bubblewrap, Linux 6.12 or newer with Landlock,
						delegated cgroup v2 budgets, and an enforced ext4 project quota.
						Network access is shared with the host; local services must
						authenticate callers.
					</p>
				</>
			)}
		</fieldset>
	);
}
