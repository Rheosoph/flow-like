"use client";
import { useEffect, useId, useRef, useState } from "react";
import {
	type ReleaseConfig,
	type VerifiedRelease,
	fetchVerifiedRelease,
} from "../../../lib/device-management/package";
import type { ManagementCall } from "../../../lib/device-management/telemetry";
import type { PlacementStatus } from "../../../lib/device-management/types";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

export function DeviceHostOperations({
	bootId,
	release,
	placements,
	run,
	pending,
	setPending,
}: {
	bootId: string | null;
	release?: ReleaseConfig;
	placements: PlacementStatus[];
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
	pending?: { placement: string; operationId: string };
	setPending: (
		pending: { placement: string; operationId: string } | undefined,
	) => void;
}) {
	const id = useId();
	const [placement, setPlacement] = useState("");
	const [name, setName] = useState("listener");
	const [secret, setSecret] = useState("");
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [result, setResult] = useState<string>();
	const [verified, setVerified] = useState<VerifiedRelease>();
	const [update, setUpdate] = useState(false);
	const [reboot, setReboot] = useState(false);
	const alive = useRef(true);
	const abort = useRef(new AbortController());
	useEffect(() => {
		alive.current = true;
		abort.current = new AbortController();
		return () => {
			alive.current = false;
			abort.current.abort();
		};
	}, []);
	async function execute(operation: () => Promise<void>) {
		if (busy) return;
		setBusy(true);
		setError(undefined);
		try {
			await operation();
		} catch (error) {
			if (alive.current)
				setError(
					error instanceof Error ? error.message : "Device operation failed.",
				);
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	async function saveSecret() {
		const value = secret;
		setSecret("");
		await execute(() =>
			run(async (call) => {
				const target = placements.find((row) => row.id === placement);
				if (!target || target.desired_state !== "stopped")
					throw new Error(
						"Stop this placement before setting its listener secret.",
					);
				if (
					!/^[A-Za-z0-9_.-]{1,128}$/u.test(name) ||
					name === "." ||
					name === ".."
				)
					throw new Error("Use a valid secret name.");
				if (!value || new TextEncoder().encode(value).length > 4096)
					throw new Error("The secret must contain 1 to 4096 UTF-8 bytes.");
				const operationId = crypto.randomUUID();
				if (alive.current) {
					setPending({ placement, operationId });
				}
				const response = await call(
					{
						type: "set_secret",
						placement_id: placement,
						expected_revision: target.config_revision,
						name,
						value,
					},
					operationId,
				);
				if (response.state === "rejected") {
					if (alive.current) {
						setPending(undefined);
					}
					throw new Error("The device rejected this secret update.");
				}
				if (alive.current)
					setResult(
						`Secret operation ${operationId}: ${response.state}. Check completion before starting this placement.`,
					);
			}),
		);
	}
	return (
		<details className="rounded border p-3">
			<summary className="cursor-pointer font-medium">
				Service secrets and host operations
			</summary>
			<div className="mt-3 space-y-4">
				<form
					className="space-y-3"
					onSubmit={(event) => {
						event.preventDefault();
						void saveSecret();
					}}
				>
					<p className="text-sm text-muted-foreground">
						Apply the placement stopped, then set the secret named by its
						listener's auth_secret field. Start it after the secret operation
						completes.
					</p>
					<label className="block text-sm">
						Stopped placement
						<select
							className="ml-2 rounded border bg-background p-2"
							value={placement}
							onChange={(event) => setPlacement(event.target.value)}
							disabled={busy || Boolean(pending)}
							required
						>
							<option value="">Choose placement</option>
							{placements
								.filter((row) => row.desired_state === "stopped")
								.map((row) => (
									<option value={row.id} key={row.id}>
										{row.project_id} / {row.id}
									</option>
								))}
						</select>
					</label>
					<label htmlFor={`${id}-name`} className="block space-y-1 text-sm">
						Secret name
						<Input
							id={`${id}-name`}
							value={name}
							onChange={(event) => setName(event.target.value)}
							maxLength={128}
							disabled={busy || Boolean(pending)}
							required
						/>
					</label>
					<label htmlFor={`${id}-secret`} className="block space-y-1 text-sm">
						Secret value
						<Input
							id={`${id}-secret`}
							type="password"
							autoComplete="off"
							value={secret}
							onChange={(event) => setSecret(event.target.value)}
							maxLength={4096}
							disabled={busy || Boolean(pending)}
							required
						/>
					</label>
					<Button
						type="submit"
						disabled={busy || Boolean(pending) || !placement}
					>
						Send secret to device
					</Button>
				</form>
				{pending && (
					<Button
						variant="outline"
						disabled={busy}
						onClick={() =>
							void execute(() =>
								run(async (call) => {
									const response = await call({
										type: "operation",
										operation_id: pending.operationId,
									});
									const state = String(response.result.state ?? response.state);
									if (alive.current) {
										setResult(
											`Secret operation ${pending.operationId}: ${state}`,
										);
										if (
											state === "completed" ||
											state === "failed" ||
											state === "rejected"
										) {
											setPending(undefined);
										}
									}
								}),
							)
						}
					>
						Check secret completion
					</Button>
				)}
				<div className="space-y-2">
					<p className="font-medium">Agent update</p>
					{!release ? (
						<p className="text-sm text-muted-foreground">
							This hub has no configured release signing keys.
						</p>
					) : (
						<>
							<Button
								variant="outline"
								disabled={busy}
								onClick={() =>
									void execute(async () => {
										const value = await fetchVerifiedRelease(
											release,
											abort.current.signal,
										);
										if (alive.current) {
											setVerified(value);
											setUpdate(false);
										}
									})
								}
							>
								Check signed release
							</Button>
							{verified && (
								<>
									<p className="text-sm">
										Verified release {verified.manifest.release_version}
									</p>
									<label className="flex items-start gap-2 text-sm">
										<input
											type="checkbox"
											checked={update}
											onChange={(event) => setUpdate(event.target.checked)}
											disabled={busy}
										/>
										Drain running services and restart this agent with the
										verified release.
									</label>
									<Button
										disabled={busy || !update || !bootId}
										onClick={() =>
											void execute(() =>
												run(async (call) => {
													if (verified.manifestJws.length > 13_000)
														throw new Error(
															"The release manifest exceeds the management message limit.",
														);
													const response = await call({
														type: "update_agent",
														expected_boot_id: bootId,
														release_jws: verified.manifestJws,
													});
													if (alive.current) {
														setResult(
															`Update operation ${response.operation_id}: ${response.state}. Reconnect to check its status.`,
														);
														setUpdate(false);
													}
												}),
											)
										}
									>
										Install verified update
									</Button>
								</>
							)}
						</>
					)}
				</div>
				<div className="space-y-2">
					<label className="flex items-start gap-2 text-sm">
						<input
							type="checkbox"
							checked={reboot}
							onChange={(event) => setReboot(event.target.checked)}
							disabled={busy}
						/>
						Reboot this device and interrupt its running services.
					</label>
					<Button
						variant="outline"
						disabled={busy || !reboot || !bootId}
						onClick={() =>
							void execute(() =>
								run(async (call) => {
									const response = await call({
										type: "reboot",
										expected_boot_id: bootId,
									});
									if (alive.current) {
										setResult(
											`Reboot operation ${response.operation_id}: ${response.state}. Reconnect after the device returns.`,
										);
										setReboot(false);
									}
								}),
							)
						}
					>
						Reboot device
					</Button>
				</div>
				{result && <output className="block text-sm">{result}</output>}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
			</div>
		</details>
	);
}
