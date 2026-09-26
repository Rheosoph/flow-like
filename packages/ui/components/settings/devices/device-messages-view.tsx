"use client";

function object(value: unknown): Record<string, unknown> | undefined {
	return value !== null && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;
}

const states = new Set([
	"accepted",
	"completed",
	"failed",
	"unknown",
	"starting",
	"running",
	"stopping",
	"backoff",
	"stopped",
	"removed",
]);

export function DeviceMessagesView({
	sample,
}: { sample: Record<string, unknown> | undefined }) {
	const records = Array.isArray(sample?.records)
		? sample.records.slice(0, 100)
		: [];
	return (
		<div className="space-y-2 text-sm">
			<p className="text-xs text-muted-foreground">
				Command and replica transitions recorded on this device. Messages share
				the local telemetry retention limit.
			</p>
			{typeof sample?.outbox_dropped === "number" &&
				sample.outbox_dropped > 0 && (
					<p className="text-xs text-amber-700">
						{sample.outbox_dropped.toLocaleString()} transitions expired from
						the pending queue before they could be retained.
					</p>
				)}
			{records.length === 0 && (
				<p className="text-muted-foreground">
					No recorded transitions in this page.
				</p>
			)}
			<ol className="space-y-2">
				{records.map((value) => {
					const record = object(value);
					const data = object(record?.data);
					if (
						!data ||
						typeof record?.sequence !== "number" ||
						!Number.isSafeInteger(record.sequence) ||
						typeof data.state !== "string" ||
						!states.has(data.state) ||
						(data.kind !== "operation" && data.kind !== "replica")
					)
						return null;
					const timestamp =
						typeof record.timestamp === "number" &&
						Number.isFinite(record.timestamp)
							? new Date(record.timestamp * 1000).toLocaleString()
							: "Time unavailable";
					return (
						<li key={record.sequence} className="rounded border p-3">
							<p className="font-medium">
								{data.kind === "operation" ? "Command" : "Replica"}:{" "}
								{data.state}
							</p>
							<p className="text-xs text-muted-foreground">
								{timestamp}
								{typeof data.placement_id === "string" &&
									` · ${data.placement_id}`}
								{typeof data.replica_slot === "number" &&
									Number.isSafeInteger(data.replica_slot) &&
									` · slot ${data.replica_slot}`}
							</p>
							{typeof data.source_id === "string" && (
								<p className="break-all font-mono text-xs">{data.source_id}</p>
							)}
						</li>
					);
				})}
			</ol>
		</div>
	);
}
