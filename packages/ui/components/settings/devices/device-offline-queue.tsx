"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	type OfflineQueueStatus,
	readOfflineQueues,
	retryOfflineQueue,
	skipOfflineQueue,
} from "../../../lib/device-management/offline-queue";
import type { ManagementCall } from "../../../lib/device-management/telemetry";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

function QueueStatus({
	queue,
	busy,
	action,
}: {
	queue: OfflineQueueStatus;
	busy: boolean;
	action: (
		kind: "retry" | "skip",
		queue: OfflineQueueStatus,
		reason?: string,
		acknowledged?: boolean,
	) => Promise<void>;
}) {
	const reasonId = useId();
	const [reason, setReason] = useState("");
	const [acknowledged, setAcknowledged] = useState(false);
	const head = queue.head;
	const blocked =
		head && ["blocked", "conflict", "outcome_unknown"].includes(head.state);
	return (
		<div className="space-y-2 rounded border p-3">
			<p className="text-sm font-medium">
				{queue.quarantined
					? "Authorization blocked"
					: (head?.state.replaceAll("_", " ") ?? "Up to date")}
			</p>
			<p className="text-xs text-muted-foreground">
				{queue.pending_count} pending ·{" "}
				{(queue.pending_bytes / 1048576).toFixed(2)} MiB
				{queue.oldest_at !== null &&
					` · oldest ${new Date(queue.oldest_at * 1000).toLocaleString()}`}
			</p>
			<p className="break-all text-xs text-muted-foreground">
				Authorization scope: {queue.scope}
			</p>
			{queue.quarantined && (
				<p className="text-sm">
					This authorization no longer permits replay. Queued data remains on
					the device for review.
				</p>
			)}
			{queue.mirror_error && (
				<output className="block text-sm">
					Local table refresh: {queue.mirror_error}
				</output>
			)}
			{head && (
				<>
					<p className="break-all text-xs">
						Operation {head.operation_id} · {head.attempts} dispatch attempts
					</p>
					{head.error && (
						<output className="block text-sm">{head.error}</output>
					)}
					{!queue.quarantined && (
						<div className="space-y-2">
							{blocked && (
								<>
									<p className="text-xs text-muted-foreground">
										Retry checks the same write and cloud revision. A conflict
										needs review; retry does not replace newer cloud data.
									</p>
									<Button
										type="button"
										variant="outline"
										size="sm"
										disabled={busy}
										onClick={() => void action("retry", queue)}
									>
										Retry queued write
									</Button>
								</>
							)}
							<details>
								<summary className="cursor-pointer text-sm">
									Skip this queued write
								</summary>
								<form
									className="mt-2 space-y-2"
									onSubmit={(event) => {
										event.preventDefault();
										void action("skip", queue, reason, acknowledged);
									}}
								>
									<p className="text-xs text-muted-foreground">
										Skip removes this write from the local view so later writes
										can proceed. It cannot undo changes already applied in the
										cloud.
									</p>
									<label className="block text-sm" htmlFor={reasonId}>
										Skip reason
										<Input
											id={reasonId}
											value={reason}
											onChange={(event) => setReason(event.target.value)}
											maxLength={512}
											disabled={busy}
											required
										/>
									</label>
									{head.attempts > 0 && (
										<label className="flex items-start gap-2 text-sm">
											<input
												type="checkbox"
												checked={acknowledged}
												onChange={(event) =>
													setAcknowledged(event.target.checked)
												}
												disabled={busy}
											/>
											I understand this write may already have reached the cloud
											and skipping cannot undo it.
										</label>
									)}
									<Button
										type="submit"
										variant="outline"
										size="sm"
										disabled={
											busy ||
											!reason.trim() ||
											(head.attempts > 0 && !acknowledged)
										}
									>
										Skip queued write
									</Button>
								</form>
							</details>
						</div>
					)}
				</>
			)}
		</div>
	);
}

export function DeviceOfflineQueue({
	placement,
	busy,
	run,
}: {
	placement: string;
	busy: boolean;
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
}) {
	const [queues, setQueues] = useState<OfflineQueueStatus[]>();
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string>();
	const [notice, setNotice] = useState<string>();
	const alive = useRef(true);
	const working = useRef(false);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	async function execute(operation: (call: ManagementCall) => Promise<void>) {
		if (working.current || busy) return;
		working.current = true;
		setLoading(true);
		setError(undefined);
		setNotice(undefined);
		try {
			await run(operation);
		} catch (error) {
			if (alive.current)
				setError(
					error instanceof Error
						? error.message
						: "Offline queue request failed.",
				);
		} finally {
			working.current = false;
			if (alive.current) setLoading(false);
		}
	}
	async function refresh(call: ManagementCall) {
		const result = await readOfflineQueues(call, placement);
		if (alive.current) setQueues(result);
	}
	async function action(
		kind: "retry" | "skip",
		queue: OfflineQueueStatus,
		reason = "",
		acknowledged = false,
	) {
		await execute(async (call) => {
			const command =
				kind === "retry"
					? retryOfflineQueue(placement, queue)
					: skipOfflineQueue(placement, queue, reason, acknowledged);
			const response = await call(command);
			if (response.state !== "completed")
				throw new Error(
					"The device did not confirm this queue action. Refresh its status before trying again.",
				);
			if (alive.current)
				setNotice(
					kind === "skip"
						? "Skip requested. The running service rebuilds the local view before continuing. Refresh to check completion."
						: "Retry requested. Refresh to check replay progress.",
				);
			await refresh(call);
		});
	}
	return (
		<details className="w-full space-y-2 border-t pt-2">
			<summary className="cursor-pointer text-sm">Offline write queues</summary>
			<p className="text-xs text-muted-foreground">
				Writes stay on this device until replay succeeds or you skip them.
				Status includes counts and errors without file contents or table rows.
			</p>
			<Button
				type="button"
				variant="outline"
				size="sm"
				disabled={busy || loading}
				onClick={() => void execute(refresh)}
			>
				Refresh offline queues
			</Button>
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			{notice && <output className="block text-sm">{notice}</output>}
			{queues?.length === 0 && (
				<p className="text-sm text-muted-foreground">
					No offline write queue exists for this placement.
				</p>
			)}
			{queues?.map((queue) => (
				<QueueStatus
					key={`${queue.scope}:${queue.head?.operation_id ?? "empty"}`}
					queue={queue}
					busy={busy || loading}
					action={action}
				/>
			))}
		</details>
	);
}
