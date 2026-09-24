"use client";

import { useEffect, useId, useRef, useState } from "react";
import {
	type DeploymentEvent,
	type DeploymentPlan,
	DeploymentPublicationFailedError,
	DeploymentReviewRequiredError,
	DeploymentRolloutEndedError,
	type DeploymentRolloutStatus,
	type DeploymentVariable,
	type InstalledProject,
	type OfflineWritesConfig,
	type PlacementConfiguration,
	type PlacementResources,
	StaleDeploymentRevisionError,
	canCheckDeploymentStartup,
	cancelDeploymentRollout,
	createDeploymentPlan,
	discoverOfflineEvents,
	discoverOfflineVariables,
	discoverOnlineEvents,
	discoverOnlineVariables,
	executeDeploymentPlan,
	mergeVariables,
	readExistingDeployment,
	validateVariableValue,
	variableText,
	waitForDeploymentRollout,
} from "../../../lib/device-management/deployment";
import type { ManagementCall } from "../../../lib/device-management/telemetry";
import type { PlacementStatus } from "../../../lib/device-management/types";
import {
	type DeviceResources,
	isActiveGrant,
	loadDeviceResources,
} from "../../../lib/device-resources";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { DeviceIsolationFields } from "./device-isolation-fields";
import { DeviceOfflineWritesFields } from "./device-offline-writes-fields";

export function DeviceDeploymentForm({
	installed,
	connected = true,
	placements = [],
	deviceId,
	profile,
	run,
	onApplied,
}: {
	installed: InstalledProject;
	connected?: boolean;
	placements?: PlacementStatus[];
	deviceId: string;
	profile: IProfile;
	run: <T>(operation: (call: ManagementCall) => Promise<T>) => Promise<T>;
	onApplied: () => Promise<void>;
}) {
	const backend = useBackend();
	const formId = useId();
	const [target, setTarget] = useState("");
	const [existing, setExisting] = useState<PlacementConfiguration>();
	const [removeOverrides, setRemoveOverrides] = useState<string[]>([]);
	const [removedEvents, setRemovedEvents] = useState<string[]>([]);
	const [acceptRemovedEvents, setAcceptRemovedEvents] = useState(false);
	const [needsReload, setNeedsReload] = useState(false);
	const generation = useRef(0);
	const [events, setEvents] = useState<DeploymentEvent[]>([]);
	const [selected, setSelected] = useState<string[]>([]);
	const [definitions, setDefinitions] = useState<
		Record<string, DeploymentVariable[]>
	>({});
	const [overrides, setOverrides] = useState<Record<string, string>>({});
	const [editedOverrides, setEditedOverrides] = useState<string[]>([]);
	const [placement, setPlacement] = useState(
		() => `service-${crypto.randomUUID()}`,
	);
	const [deployment, setDeployment] = useState<string>(() =>
		crypto.randomUUID(),
	);
	const [host, setHost] = useState("127.0.0.1");
	const [port, setPort] = useState("8080");
	const [replicas, setReplicas] = useState("1");
	const [serviceToken, setServiceToken] = useState("");
	const [offlineWrites, setOfflineWrites] =
		useState<OfflineWritesConfig | null>(null);
	const [resourceLimits, setResourceLimits] =
		useState<PlacementResources | null>(null);
	const [resources, setResources] = useState<DeviceResources>();
	const [grantId, setGrantId] = useState("");
	const [billingId, setBillingId] = useState("");
	const [busy, setBusy] = useState(false);
	const [loaded, setLoaded] = useState(false);
	const [error, setError] = useState<string>();
	const [done, setDone] = useState(false);
	const [retry, setRetry] = useState(false);
	const [healthChecked, setHealthChecked] = useState(true);
	const [rollout, setRollout] = useState<DeploymentRolloutStatus>();
	const plan = useRef<DeploymentPlan | undefined>(undefined);
	const alive = useRef(true);
	const abort = useRef(new AbortController());
	useEffect(() => {
		alive.current = true;
		abort.current = new AbortController();
		return () => {
			alive.current = false;
			generation.current++;
			abort.current.abort();
			plan.current = undefined;
		};
	}, []);
	const selectedEvents = events.filter((event) => selected.includes(event.id));
	let variables: DeploymentVariable[] = [];
	let conflict: string | undefined;
	try {
		variables = mergeVariables(selected.map((id) => definitions[id] ?? []));
	} catch (error) {
		conflict = (error as Error).message;
	}
	const hosted = selectedEvents.some((event) => event.hosted);
	const canCheckStartup = canCheckDeploymentStartup(
		installed,
		existing,
		selectedEvents,
	);
	const automaticRollout = canCheckStartup && healthChecked;
	const activeRollout =
		rollout &&
		["staged", "validating", "activating", "rolling_back"].includes(
			rollout.state,
		);
	const grants =
		resources?.grants.filter(
			(grant) =>
				grant.project_id === installed.project_id &&
				grant.device_id === deviceId &&
				isActiveGrant(grant) &&
				(installed.source === "offline"
					? !grant.app_id
					: grant.app_id === installed.project_id && grant.online_access),
		) ?? [];
	const grant = grants.find((grant) => grant.grant_id === grantId);
	const matchingPlacements = placements.filter(
		(value) => value.project_id === installed.project_id,
	);
	const unresolvedOverrides = existing
		? [
				...Object.keys(existing.config.variables),
				...Object.keys(existing.config.secret_overrides),
			].filter((id) => {
				if (removeOverrides.includes(id)) return false;
				const definition = variables.find((value) => value.id === id);
				if (!definition) return true;
				const wasSecret = Object.hasOwn(existing.config.secret_overrides, id);
				const hasReplacement =
					editedOverrides.includes(id) &&
					Object.hasOwn(overrides, id) &&
					(!definition.secret || overrides[id] !== "");
				if (wasSecret !== definition.secret) return !hasReplacement;
				if (!wasSecret && !hasReplacement) {
					try {
						validateVariableValue(definition, existing.config.variables[id]);
					} catch {
						return true;
					}
				}
				return false;
			})
		: [];
	function resetTarget(value: string) {
		generation.current++;
		setTarget(value);
		setExisting(undefined);
		setOfflineWrites(null);
		setResourceLimits(null);
		setEvents([]);
		setSelected([]);
		setDefinitions({});
		setOverrides({});
		setEditedOverrides([]);
		setRemoveOverrides([]);
		setRemovedEvents([]);
		setAcceptRemovedEvents(false);
		setServiceToken("");
		setLoaded(false);
		setNeedsReload(false);
		setError(undefined);
		setGrantId("");
		setBillingId("");
		setPlacement(value || `service-${crypto.randomUUID()}`);
		setDeployment(crypto.randomUUID());
		setHost("127.0.0.1");
		setPort("8080");
		setReplicas("1");
		setHealthChecked(true);
		setRollout(undefined);
	}
	async function load() {
		if (busy || !connected || plan.current) return;
		const sequence = ++generation.current;
		setBusy(true);
		setError(undefined);
		try {
			const snapshot = target
				? await run((call) =>
						readExistingDeployment(call, target, installed.project_id),
					)
				: undefined;
			if (!alive.current || sequence !== generation.current) return;
			if (snapshot && snapshot.config.source !== installed.source)
				throw new Error(
					"The existing placement uses another project source. Create a new placement instead.",
				);
			const discovery = async (call?: ManagementCall) => {
				const rows = call
					? await discoverOfflineEvents(call, installed)
					: await discoverOnlineEvents(
							backend.eventState,
							installed.project_id,
						);
				const previousIds =
					snapshot?.config.events.map((event) => event.event_id) ?? [];
				const chosen = rows.filter(
					(event) => event.eligible && previousIds.includes(event.id),
				);
				const vars: Record<string, DeploymentVariable[]> = {};
				for (const event of chosen) {
					if (!alive.current || sequence !== generation.current)
						throw new Error("Project discovery was cancelled.");
					vars[event.id] = call
						? await discoverOfflineVariables(call, installed, event.id)
						: await discoverOnlineVariables(
								backend.eventState,
								backend.boardState,
								installed.project_id,
								event,
							);
				}
				return { rows, chosen, vars, previousIds };
			};
			const { rows, chosen, vars, previousIds } =
				installed.source === "offline"
					? await run(discovery)
					: await discovery();
			if (!alive.current || sequence !== generation.current) return;
			const values: Record<string, string> = {};
			for (const variable of mergeVariables(Object.values(vars))) {
				if (
					!variable.secret &&
					snapshot &&
					Object.hasOwn(snapshot.config.variables, variable.id)
				) {
					try {
						validateVariableValue(
							variable,
							snapshot.config.variables[variable.id],
						);
						values[variable.id] = variableText(
							variable,
							snapshot.config.variables[variable.id],
						);
					} catch {
						// Keep the stored value unresolved until the user replaces or removes it.
					}
				}
			}
			setExisting(snapshot);
			setOfflineWrites(snapshot?.config.offline_writes ?? null);
			setResourceLimits(snapshot?.config.resources ?? null);
			setRollout(snapshot?.rollout ?? undefined);
			setEvents(rows);
			setLoaded(true);
			setSelected(chosen.map((event) => event.id));
			setDefinitions(vars);
			setOverrides(values);
			setEditedOverrides([]);
			setRemoveOverrides([]);
			setRemovedEvents(
				previousIds.filter((id) => !chosen.some((event) => event.id === id)),
			);
			setAcceptRemovedEvents(false);
			setNeedsReload(false);
			setServiceToken("");
			setGrantId("");
			setBillingId("");
			if (snapshot) {
				setPlacement(snapshot.placement_id);
				setDeployment(snapshot.deployment_id);
				setHost(snapshot.config.hosting?.host ?? "127.0.0.1");
				setPort(String(snapshot.config.hosting?.port ?? 8080));
				setReplicas(String(snapshot.config.max_replicas));
			}
		} catch (error) {
			if (alive.current && sequence === generation.current)
				setError(
					error instanceof Error ? error.message : "Project discovery failed.",
				);
		} finally {
			if (alive.current && sequence === generation.current) setBusy(false);
		}
	}
	async function select(event: DeploymentEvent, checked: boolean) {
		if (busy || !connected) return;
		if (!checked) {
			setSelected((previous) => previous.filter((id) => id !== event.id));
			return;
		}
		setBusy(true);
		setError(undefined);
		try {
			const vars =
				installed.source === "offline"
					? await run((call) =>
							discoverOfflineVariables(call, installed, event.id),
						)
					: await discoverOnlineVariables(
							backend.eventState,
							backend.boardState,
							installed.project_id,
							event,
						);
			if (alive.current) {
				setDefinitions((previous) => ({ ...previous, [event.id]: vars }));
				setSelected((previous) => [...previous, event.id]);
			}
		} catch (error) {
			if (alive.current)
				setError(
					error instanceof Error ? error.message : "Variable discovery failed.",
				);
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	async function provision() {
		if (
			busy ||
			conflict ||
			!connected ||
			needsReload ||
			(target && !existing) ||
			unresolvedOverrides.length ||
			(removedEvents.length && !acceptRemovedEvents)
		)
			return;
		setBusy(true);
		setError(undefined);
		try {
			if (!plan.current) {
				const billing = resources?.billing.find(
					(value) =>
						value.billing_grant_id === billingId &&
						value.grant_id === grantId &&
						isActiveGrant(value),
				);
				if (grantId && !grant)
					throw new Error(
						"Reload resource approvals; the selected approval has expired.",
					);
				if (
					grant &&
					(grant.placement_id !== placement ||
						grant.deployment_id !== deployment ||
						Number(replicas) > grant.max_instances)
				)
					throw new Error(
						"Resource approval does not match the deployment or replica limit.",
					);
				if (billingId && !billing)
					throw new Error("Select a current model allowance.");
				if (grant?.model_ids.length && !billing)
					throw new Error(
						"Select an active model allowance for this resource approval.",
					);
				plan.current = createDeploymentPlan({
					installed,
					existing,
					healthChecked: automaticRollout,
					removeOverrides,
					placement,
					deployment,
					events: selectedEvents,
					variables,
					overrides: Object.fromEntries(
						Object.entries(overrides).filter(
							([id, value]) =>
								(!existing || editedOverrides.includes(id)) &&
								(value !== "" ||
									!variables.find((variable) => variable.id === id)?.secret),
						),
					),
					host,
					port: Number(port),
					replicas: Number(replicas),
					serviceToken,
					offlineWrites,
					resourceLimits,
					resourceGrant: grant
						? {
								grant_id: grant.grant_id,
								authz_version: grant.authz_version,
								...(billing
									? {
											billing_grant_id: billing.billing_grant_id,
											billing_authz_version: billing.authz_version,
										}
									: {}),
							}
						: undefined,
				});
			}
			const prepared = plan.current;
			if (!prepared) throw new Error("Deployment preparation was cancelled.");
			await run((call) =>
				executeDeploymentPlan(
					call,
					prepared,
					abort.current.signal,
					(status) => {
						if (alive.current) setRollout(status);
					},
				),
			);
			if (alive.current) {
				setDone(true);
				setRetry(false);
				setOverrides({});
				setServiceToken("");
				plan.current = undefined;
			}
			if (alive.current) {
				try {
					await onApplied();
				} catch {
					if (alive.current)
						setError("Placement installed. Reconnect to refresh its status.");
				}
			}
		} catch (error) {
			if (alive.current) {
				if (error instanceof DeploymentRolloutEndedError) {
					setRollout(error.status);
					plan.current = undefined;
					setNeedsReload(true);
					setOverrides({});
					setServiceToken("");
					try {
						await onApplied();
					} catch {
						/* A later read can refresh the placement. */
					}
				} else if (error instanceof DeploymentPublicationFailedError) {
					const placementId = String(plan.current?.config.id ?? placement);
					plan.current = undefined;
					setTarget(placementId);
					setExisting(undefined);
					setNeedsReload(true);
					setOverrides({});
					setEditedOverrides([]);
					setServiceToken("");
					try {
						await onApplied();
					} catch {
						// The next explicit configuration read can recover without inspection.
					}
				} else if (
					error instanceof StaleDeploymentRevisionError ||
					error instanceof DeploymentReviewRequiredError
				) {
					plan.current = undefined;
					setNeedsReload(true);
					setOverrides({});
					setEditedOverrides([]);
					setServiceToken("");
				}
				if (!alive.current) return;
				setRetry(Boolean(plan.current));
				setError(
					error instanceof Error
						? error.message
						: "Deployment could not be confirmed.",
				);
			}
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	async function observeRollout() {
		if (!rollout || busy || !connected || rollout.state === "staged") return;
		setBusy(true);
		setError(undefined);
		try {
			await run((call) =>
				waitForDeploymentRollout(
					call,
					rollout,
					abort.current.signal,
					(status) => {
						if (alive.current) setRollout(status);
					},
				),
			);
			if (alive.current) {
				plan.current = undefined;
				setRetry(false);
				setOverrides({});
				setEditedOverrides([]);
				setServiceToken("");
				setDone(true);
				try {
					await onApplied();
				} catch {
					if (alive.current)
						setError(
							"Rollout completed. Reconnect to refresh the placement status.",
						);
				}
			}
		} catch (error) {
			if (alive.current) {
				if (error instanceof DeploymentRolloutEndedError) {
					setRollout(error.status);
					plan.current = undefined;
					setRetry(false);
					setOverrides({});
					setEditedOverrides([]);
					setServiceToken("");
					setNeedsReload(true);
					try {
						await onApplied();
					} catch {
						/* The explicit reload can refresh later. */
					}
				}
				if (!alive.current) return;
				setError(
					error instanceof Error
						? error.message
						: "Rollout status could not be confirmed.",
				);
			}
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	async function discardRollout() {
		if (!rollout || busy || !connected) return;
		setBusy(true);
		setError(undefined);
		try {
			const status = await run((call) =>
				cancelDeploymentRollout(call, rollout, abort.current.signal),
			);
			if (!alive.current) return;
			setRollout(status);
			if (
				["cancelled", "failed", "rolled_back", "healthy"].includes(status.state)
			) {
				plan.current = undefined;
				setRetry(false);
				setOverrides({});
				setEditedOverrides([]);
				setServiceToken("");
				setNeedsReload(status.state !== "healthy");
				setDone(status.state === "healthy");
				const snapshot = await run((call) =>
					readExistingDeployment(call, status.placement_id, status.project_id),
				);
				if (!alive.current) return;
				setExisting(snapshot);
				setRollout(snapshot.rollout ?? status);
				await onApplied();
			} else
				setError(
					"Activation already began. Follow rollout status to see whether it starts successfully or rolls back.",
				);
		} catch (error) {
			if (alive.current)
				setError(
					error instanceof Error
						? error.message
						: "The staged update could not be discarded.",
				);
		} finally {
			if (alive.current) setBusy(false);
		}
	}
	return (
		<details open className="rounded border p-3">
			<summary className="cursor-pointer font-medium">
				Deploy {installed.project_id}
			</summary>
			<form
				className="mt-3 space-y-3"
				onSubmit={(event) => {
					event.preventDefault();
					void provision();
				}}
			>
				<p className="text-sm text-muted-foreground">
					{target
						? automaticRollout
							? "Stage this project revision and its secrets, then restart the services. The device checks listener startup for 10 seconds and restores the previous configuration if startup fails. Mutable project data is preserved."
							: "Update an existing placement using the selected project snapshot. Applying the update stops its running services. Review the new configuration and start the placement from its controls when ready."
						: "Create a placement on this device. Select published events, configure this device's variables, then start the placement from its controls."}
				</p>
				<p className="break-all text-xs text-muted-foreground">
					{installed.source === "offline"
						? `Snapshot ${installed.revision}`
						: "Online project with pinned event and board versions"}
				</p>
				<fieldset
					disabled={busy || done || retry || !connected}
					className="space-y-3"
				>
					<label className="block text-sm">
						Deployment action
						<select
							className="block w-full rounded border bg-background p-2"
							value={target}
							onChange={(event) => resetTarget(event.target.value)}
						>
							<option value="">Create a new placement</option>
							{target &&
								!matchingPlacements.some((value) => value.id === target) && (
									<option value={target}>Update {target}</option>
								)}
							{matchingPlacements.map((value) => (
								<option key={value.id} value={value.id}>
									Update {value.id} · revision {value.config_revision} ·{" "}
									{value.desired_state}
								</option>
							))}
						</select>
					</label>
					<Button type="button" variant="outline" onClick={() => void load()}>
						{target
							? existing
								? "Reload current placement and published events"
								: "Read current placement and published events"
							: loaded
								? "Reload published events"
								: "Read published events"}
					</Button>
					{existing && (
						<p className="text-xs text-muted-foreground">
							Updating configuration revision {existing.config_revision}.
							Placement identity and source remain fixed. Listener request
							limits, restart policy and cloud approvals are retained.
						</p>
					)}
					{canCheckStartup && (
						<label className="flex gap-2 text-sm">
							<input
								type="checkbox"
								checked={healthChecked}
								onChange={(event) => setHealthChecked(event.target.checked)}
							/>
							Check startup and roll back automatically
						</label>
					)}
					{existing && !canCheckStartup && (
						<p className="text-xs text-muted-foreground">
							Automatic startup checks require a running HTTP, chat or Page
							placement and device support for its project source. This update
							uses manual review and Start.
						</p>
					)}
					{removedEvents.length > 0 && (
						<label className="flex gap-2 text-sm">
							<input
								type="checkbox"
								checked={acceptRemovedEvents}
								onChange={(event) =>
									setAcceptRemovedEvents(event.target.checked)
								}
							/>
							<span>
								The new snapshot cannot run these previously selected events:{" "}
								{removedEvents.join(", ")}. Remove them from this placement.
							</span>
						</label>
					)}
					{unresolvedOverrides.length > 0 && (
						<div role="alert" className="space-y-2 rounded border p-2 text-sm">
							<p>
								Resolve stored overrides that are outside the selected events or
								no longer match the variable definition. Restore the event,
								provide a replacement, or explicitly remove each override.
							</p>
							{unresolvedOverrides.map((id) => (
								<div key={id} className="flex items-center gap-2">
									<span>
										{id}
										{Object.hasOwn(existing?.config.secret_overrides ?? {}, id)
											? " · stored secret reference"
											: " · stored public override"}
									</span>
									<Button
										type="button"
										variant="outline"
										onClick={() => {
											setRemoveOverrides((previous) => [...previous, id]);
											setOverrides((previous) => {
												const next = { ...previous };
												delete next[id];
												return next;
											});
										}}
									>
										Remove override {id}
									</Button>
								</div>
							))}
						</div>
					)}
					{removeOverrides.length > 0 && (
						<p className="text-xs text-muted-foreground">
							Overrides removed by this update: {removeOverrides.join(", ")}.
							Their published defaults will apply where the variables still
							exist.
						</p>
					)}
					{loaded && !events.length && (
						<p className="text-sm">This project has no published events.</p>
					)}
					{events.map((event) => (
						<label className="flex items-start gap-2 text-sm" key={event.id}>
							<input
								type="checkbox"
								disabled={!event.eligible}
								checked={selected.includes(event.id)}
								onChange={(value) => void select(event, value.target.checked)}
							/>
							<span>
								{event.name || event.id} · {event.event_type} ·{" "}
								{event.event_version?.join(".")}
								<span className="block text-xs text-muted-foreground">
									{event.eligible
										? `Board ${event.board_version?.join(".")}`
										: "Requires an active supported event, concrete board version, and no traffic variants."}
								</span>
							</span>
						</label>
					))}
					<label htmlFor={`${formId}-placement`} className="block text-sm">
						Placement ID
						<Input
							id={`${formId}-placement`}
							value={placement}
							maxLength={128}
							disabled={Boolean(grant) || Boolean(target)}
							onChange={(event) => setPlacement(event.target.value)}
						/>
					</label>
					<label htmlFor={`${formId}-deployment`} className="block text-sm">
						Deployment ID
						<Input
							id={`${formId}-deployment`}
							value={deployment}
							maxLength={128}
							disabled={Boolean(grant) || Boolean(target)}
							onChange={(event) => setDeployment(event.target.value)}
						/>
					</label>
					{existing ? (
						<div className="space-y-1 rounded border p-2 text-sm">
							<p>Cloud resource and billing approvals are retained.</p>
							<p className="break-all text-xs text-muted-foreground">
								{existing.config.resource_grant
									? `Resource approval ${existing.config.resource_grant.grant_id}${existing.config.resource_grant.billing_grant_id ? ` · allowance ${existing.config.resource_grant.billing_grant_id}` : ""}`
									: "Local resources only"}
							</p>
						</div>
					) : (
						<div className="space-y-2 rounded border p-2">
							<p className="text-sm">Cloud resources</p>
							<p className="text-xs text-muted-foreground">
								Online projects require a resource approval for this placement.
								Model access also requires an allowance. Create these in the
								device's Resources dialog using the IDs above.
							</p>
							<Button
								type="button"
								variant="outline"
								onClick={async () => {
									setBusy(true);
									setError(undefined);
									try {
										const value = await loadDeviceResources(
											backend.apiState,
											profile,
											deviceId,
										);
										if (alive.current) setResources(value);
									} catch {
										if (alive.current)
											setError("Resource approvals could not be loaded.");
									} finally {
										if (alive.current) setBusy(false);
									}
								}}
							>
								Load resource approvals
							</Button>
							<label className="block text-sm">
								Resource approval
								<select
									className="block w-full rounded border bg-background p-2"
									value={grantId}
									onChange={(event) => {
										setGrantId(event.target.value);
										setBillingId("");
										const next = grants.find(
											(grant) => grant.grant_id === event.target.value,
										);
										if (next) {
											setPlacement(next.placement_id);
											setDeployment(next.deployment_id);
										}
									}}
								>
									<option value="">
										{installed.source === "offline"
											? "Local resources only"
											: "Select an approval"}
									</option>
									{grants.map((grant) => (
										<option key={grant.grant_id} value={grant.grant_id}>
											{grant.placement_id} · {grant.grant_id}
										</option>
									))}
								</select>
							</label>
							{grant && (
								<label className="block text-sm">
									Model allowance
									<select
										className="block w-full rounded border bg-background p-2"
										value={billingId}
										onChange={(event) => setBillingId(event.target.value)}
									>
										<option value="">No hosted models</option>
										{resources?.billing
											.filter(
												(value) =>
													value.grant_id === grantId && isActiveGrant(value),
											)
											.map((value) => (
												<option
													key={value.billing_grant_id}
													value={value.billing_grant_id}
												>
													{value.billing_grant_id}
												</option>
											))}
									</select>
								</label>
							)}
						</div>
					)}
					<DeviceIsolationFields
						value={resourceLimits}
						onChange={setResourceLimits}
					/>
					{installed.source === "online" && (
						<DeviceOfflineWritesFields
							value={offlineWrites}
							onChange={setOfflineWrites}
						/>
					)}
					{hosted && (
						<div className="space-y-2">
							<label htmlFor={`${formId}-host`} className="block text-sm">
								Listener IP
								<Input
									id={`${formId}-host`}
									value={host}
									onChange={(event) => setHost(event.target.value)}
								/>
							</label>
							<label htmlFor={`${formId}-port`} className="block text-sm">
								Port
								<Input
									id={`${formId}-port`}
									type="number"
									min={1}
									max={65535}
									value={port}
									onChange={(event) => setPort(event.target.value)}
								/>
							</label>
							<label htmlFor={`${formId}-token`} className="block text-sm">
								Service access token
								<Input
									id={`${formId}-token`}
									type="password"
									autoComplete="new-password"
									value={serviceToken}
									onChange={(event) => setServiceToken(event.target.value)}
									minLength={32}
									maxLength={4096}
								/>
							</label>
							<p className="text-xs text-muted-foreground">
								{existing?.config.hosting && (
									<span className="block">
										Stored token reference:{" "}
										{existing.config.hosting.auth_secret}. Leave the token blank
										to keep it. Enter a replacement to change client access.
									</span>
								)}
								Use a token from your password manager. Clients need this token
								to access HTTP, chat and Page services. It is sent only over the
								encrypted device connection.
							</p>
						</div>
					)}
					{selectedEvents.some((event) => !event.hosted) && (
						<p className="text-sm text-muted-foreground">
							REST, MCP and daemon events use their workflow listener settings.
							Configure exposed host and port variables below.
						</p>
					)}
					<label htmlFor={`${formId}-replicas`} className="block text-sm">
						Maximum replicas
						<Input
							id={`${formId}-replicas`}
							type="number"
							min={1}
							max={selectedEvents.some((event) => !event.hosted) ? 1 : 32}
							value={replicas}
							disabled={Boolean(existing)}
							onChange={(event) => setReplicas(event.target.value)}
						/>
					</label>
					{variables.length > 0 && (
						<p className="text-sm">
							Variable overrides apply only to this placement. Public overrides
							can be edited or unchecked to use published defaults. Stored
							secret references remain unchanged unless you replace or
							explicitly remove them.
						</p>
					)}
					{variables.map((variable) => (
						<div key={variable.id} className="space-y-1">
							<label className="flex gap-2 text-sm">
								<input
									type="checkbox"
									checked={Object.hasOwn(overrides, variable.id)}
									onChange={(event) => {
										const checked = event.target.checked;
										setEditedOverrides((previous) =>
											checked
												? [...new Set([...previous, variable.id])]
												: previous.filter((id) => id !== variable.id),
										);
										setRemoveOverrides((previous) =>
											checked
												? previous.filter((id) => id !== variable.id)
												: !variable.secret &&
														Object.hasOwn(
															existing?.config.variables ?? {},
															variable.id,
														)
													? [...new Set([...previous, variable.id])]
													: previous,
										);
										setOverrides((previous) => {
											const next = { ...previous };
											if (checked) next[variable.id] = "";
											else delete next[variable.id];
											return next;
										});
									}}
								/>
								{variable.name} · {variable.data_type}/{variable.value_type}
								{variable.secret ? " · secret" : ""}
							</label>
							{variable.secret &&
								existing?.config.secret_overrides[variable.id] &&
								!removeOverrides.includes(variable.id) && (
									<div className="text-xs text-muted-foreground">
										<p>
											Stored secret reference:{" "}
											{existing.config.secret_overrides[variable.id]}. Blank
											replacement preserves it. The runtime validates its value
											against the new workflow before starting; replace it if
											the type changed.
										</p>
										<Button
											type="button"
											variant="outline"
											onClick={() => {
												setRemoveOverrides((previous) => [
													...previous,
													variable.id,
												]);
												setOverrides((previous) => {
													const next = { ...previous };
													delete next[variable.id];
													return next;
												});
											}}
										>
											Remove override {variable.id}
										</Button>
									</div>
								)}
							{Object.hasOwn(overrides, variable.id) && (
								<Input
									aria-label={`${variable.name} value`}
									type={variable.secret ? "password" : "text"}
									autoComplete="off"
									value={overrides[variable.id]}
									maxLength={8192}
									onChange={(event) => {
										setEditedOverrides((previous) => [
											...new Set([...previous, variable.id]),
										]);
										setOverrides((previous) => ({
											...previous,
											[variable.id]: event.target.value,
										}));
									}}
								/>
							)}
						</div>
					))}
					<p className="text-xs text-muted-foreground">
						Enter text for scalar strings, paths and dates. Other values use
						JSON. The runtime validates values against the pinned workflow
						schema before starting.
					</p>
				</fieldset>
				{!connected && (
					<output className="text-sm">
						Reconnect to continue this deployment. Pending operations remain
						available while the device is unlocked.
					</output>
				)}
				{needsReload && (
					<p role="alert" className="text-sm">
						The configuration changed or the update was rejected. Reload the
						current placement and review your changes before trying again. No
						newer revision will be applied automatically.
					</p>
				)}
				{conflict && (
					<p role="alert" className="text-sm text-destructive">
						{conflict}
					</p>
				)}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				{rollout && (
					<div className="space-y-2 text-sm">
						<output>{rolloutMessage(rollout.state)}</output>
						{["staged", "validating"].includes(rollout.state) && !busy && (
							<Button
								type="button"
								variant="outline"
								disabled={!connected}
								onClick={() => void discardRollout()}
							>
								Discard staged update
							</Button>
						)}
						{activeRollout && rollout.state !== "staged" && !busy && (
							<Button
								type="button"
								variant="outline"
								disabled={!connected}
								onClick={() => void observeRollout()}
							>
								Follow rollout status
							</Button>
						)}
					</div>
				)}
				{done ? (
					<output className="text-sm">
						{rollout?.state === "healthy"
							? "Updated services started and passed the listener startup check."
							: existing
								? "Placement update applied and secrets confirmed. It is stopped and ready for review. Start it from its controls when ready."
								: "Placement installed and secrets confirmed. It is stopped and ready for review."}
					</output>
				) : (
					<Button
						type="submit"
						disabled={
							busy ||
							!connected ||
							!selected.length ||
							Boolean(conflict) ||
							needsReload ||
							Boolean(activeRollout && !plan.current) ||
							Boolean(target && !existing) ||
							unresolvedOverrides.length > 0 ||
							(removedEvents.length > 0 && !acceptRemovedEvents)
						}
					>
						{busy
							? "Working…"
							: retry
								? "Retry the same provisioning operations"
								: existing
									? automaticRollout
										? "Update with startup checks"
										: "Apply placement update"
									: "Create stopped placement"}
					</Button>
				)}
			</form>
		</details>
	);
}

function rolloutMessage(state: DeploymentRolloutStatus["state"]): string {
	return {
		staged:
			"Update prepared on the device. Resume it from the same unlocked session, or discard it while the current services keep running.",
		validating:
			"Validating the staged project and secrets. Current services remain active during preflight.",
		activating: "Starting the new revision and observing listener readiness.",
		healthy: "The new revision passed the listener startup check.",
		rolling_back:
			"Startup failed. Restoring the previous configuration and secret references.",
		rolled_back:
			"The previous configuration was restored. Review the placement status before another update.",
		failed: "The rollout failed. Review the current placement status.",
		cancelled: "The rollout was cancelled.",
	}[state];
}
