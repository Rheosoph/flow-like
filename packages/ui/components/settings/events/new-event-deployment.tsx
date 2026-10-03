"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	ChevronRight,
	LoaderCircle,
	Search,
	Server,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useInvalidateInvoke } from "../../../hooks/use-invoke";
import { eventEligibility } from "../../../lib/device-management/deployment";
import { isClaimedKind } from "../../../lib/device-management/model/app-plan";
import type { DeployResult } from "../../../lib/device-management/model/deploy-plan";
import type {
	DeployRoute,
	DeployStepId,
	DevicesScope,
} from "../../../lib/device-management/model/types";
import type { IEvent } from "../../../lib/schema/flow/event";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import {
	eligibilityCopy,
	eligibilityInput,
} from "../devices/copy/eligibility-copy";
import {
	issueText,
	planNames,
	prepareFailureText,
	whereGateText,
} from "../devices/deploy/deploy-copy";
import type { PlanStepProps } from "../devices/deploy/step-props";
import { AccessCostStep } from "../devices/deploy/steps/access-cost-step";
import { CopyUploadStep } from "../devices/deploy/steps/copy-upload-step";
import { EndpointLimitsStep } from "../devices/deploy/steps/endpoint-limits-step";
import { ReviewStep } from "../devices/deploy/steps/review-step";
import { RolloutStep } from "../devices/deploy/steps/rollout-step";
import { SettingsStep } from "../devices/deploy/steps/settings-step";
import { WhereStep } from "../devices/deploy/steps/where-step";
import { GateFixes } from "../devices/deploy/target-card";
import { useDeployDraft } from "../devices/deploy/use-deploy-draft";
import { useDeployPrepare } from "../devices/deploy/use-deploy-prepare";
import { useDeployRunState } from "../devices/deploy/use-deploy-run";
import { AreaOverlays } from "../devices/overlays/area-overlays";
import { useAreaTime } from "../devices/primitives/area-context";
import { ConfirmProvider } from "../devices/primitives/confirm-sheet";
import { parseDevicesRoute } from "../devices/routing/devices-route";
import { MemoryDevicesRoute } from "../devices/routing/use-devices-route";
import {
	type DeviceWorkspaceOverrides,
	DeviceWorkspaceProvider,
} from "../devices/workspace/device-workspace-provider";
import { useDeviceRows } from "../devices/workspace/use-hub";
import {
	type NewEventDeviceFilter,
	deploymentIsBusy,
	matchesDeviceFilter,
	unavailableSelectedDevice,
} from "./new-event-device-choice";

export interface NewEventDeploymentProps {
	appId: string;
	draftEvent?: Partial<IEvent>;
	/** Saves the definition with source activation suppressed. Called once per dialog. */
	onCreate(): Promise<IEvent>;
	disabled?: boolean;
	/** Protects dismissal during creation and active deployment work. */
	onBusyChange?(busy: boolean): void;
	/** Locks source fields after creation while allowing dismissal during setup or a failed deploy. */
	onSavedChange?(saved: boolean): void;
	/** Called when the user closes the deployment result. */
	onComplete?(event: IEvent): void;
	/** Keeps the creation actions reachable in the dialog's fixed footer. */
	footerContainer?: HTMLElement | null;
	/** Follows explicit links to device setup, services, or the Events page. */
	onNavigate?(href: string): void;
	/** Account and transport substitutes for component tests and previews. */
	overrides?: DeviceWorkspaceOverrides;
}

/** Creation and deployment share a dialog and retain the saved event across retries. */
export function NewEventDeployment(props: Readonly<NewEventDeploymentProps>) {
	const { t } = useTranslation("common");
	const [restart, setRestart] = useState(0);
	const scope = useMemo<DevicesScope>(
		() => ({ kind: "app", appId: props.appId }),
		[props.appId],
	);
	return (
		<MemoryDevicesRoute
			host="app"
			initialSearch={`id=${encodeURIComponent(props.appId)}`}
			onNavigate={({ href }) => {
				const route = parseDevicesRoute(href.split("?")[1] ?? "", "app").route;
				if (route.screen === "deploy") {
					if (route.step === "where") {
						setRestart((value) => value + 1);
						return;
					}
					if (route.step === "rollout") return;
				}
				props.onNavigate?.(href);
			}}
		>
			<DeviceWorkspaceProvider
				passive
				overrides={props.overrides}
				renderGate={(gate) => (
					<div className="rounded-xl border border-dashed p-5 text-sm text-muted-foreground">
						{gate.kind === "loading" ? (
							<output>{t("loadingDevices", "Loading devices…")}</output>
						) : gate.kind === "signed_out" ? (
							<>
								<p>
									{t(
										"signInToDeployDevices",
										"Sign in to deploy to your devices.",
									)}
								</p>
								{gate.signIn && (
									<Button
										type="button"
										variant="outline"
										className="mt-3"
										onClick={gate.signIn}
									>
										{t("signIn", "Sign in")}
									</Button>
								)}
							</>
						) : (
							<>
								<p>
									{t(
										"deviceProfileUnavailable",
										"Your device profile could not be loaded.",
									)}
								</p>
								<Button
									type="button"
									variant="outline"
									className="mt-3"
									onClick={gate.retry}
								>
									{t("retry", "Retry")}
								</Button>
							</>
						)}
					</div>
				)}
			>
				<ConfirmProvider>
					<DeploymentPanel {...props} scope={scope} restart={restart} />
					<AreaOverlays scope={scope} />
				</ConfirmProvider>
			</DeviceWorkspaceProvider>
		</MemoryDevicesRoute>
	);
}

function DeploymentPanel({
	appId,
	draftEvent,
	onCreate,
	disabled,
	onBusyChange,
	onSavedChange,
	onComplete,
	footerContainer,
	scope,
	restart,
}: Readonly<
	NewEventDeploymentProps & { scope: DevicesScope; restart: number }
>) {
	const { t } = useTranslation("devices");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const time = useAreaTime();
	const [draftId] = useState(() => `new-event-${crypto.randomUUID()}`);
	const [saved, setSaved] = useState<IEvent | null>(null);
	const savedRef = useRef<IEvent | null>(null);
	const [initialDevices, setInitialDevices] = useState<string[]>([]);
	const [creating, setCreating] = useState(false);
	const creatingRef = useRef(false);
	const [error, setError] = useState<string | null>(null);
	const [search, setSearch] = useState("");
	const [selectedOnly, setSelectedOnly] = useState(false);
	const [filter, setFilter] = useState<NewEventDeviceFilter>("all");
	const [tab, setTab] = useState<DeployStepId>("where");
	const [result, setResult] = useState<DeployResult | null>(null);
	const route = useMemo<DeployRoute>(
		() => ({
			screen: "deploy",
			appId,
			eventId: saved?.id ?? draftId,
			deviceIds: initialDevices,
			mode: "new",
			from: "events",
		}),
		[appId, saved?.id, draftId, initialDevices],
	);
	const state = useDeployDraft(route, scope);
	const startOver = useCallback(() => {
		state.discard();
		state.update({ targets: [] });
		setInitialDevices([]);
		setResult(null);
		setTab("where");
	}, [state.discard, state.update]);
	const seenRestart = useRef(restart);
	useEffect(() => {
		if (seenRestart.current === restart) return;
		seenRestart.current = restart;
		startOver();
	}, [restart, startOver]);
	const rows = useDeviceRows();
	const { plan, draft, check, setCatalog } = state;
	const placements = state.appRead.placements;
	const canCreateOnHub =
		state.mode === "offline" ||
		(state.mode === "online" &&
			!placements.error &&
			!placements.missingOnHub &&
			placements.data?.device_event_creation === true);
	const savedIsListed =
		!!saved && plan.app?.events.some((event) => event.id === saved.id);
	const prepare = useDeployPrepare(plan, {
		enabled: savedIsListed,
		hubTypes: state.facts.hub?.hubTypes,
	});
	const approved = prepare.prepared?.approved?.catalog ?? null;
	useEffect(() => setCatalog(approved), [approved, setCatalog]);
	const run = useDeployRunState(draft.deploymentId);
	const running = !!run && run.status !== "idle";
	const names = planNames(t, plan, state.facts, time.abs);
	const blocker = check.firstBlocking;
	const blockingText = blocker ? issueText(t, blocker, names) : undefined;
	const selected = new Set(draft.targets.map((target) => target.deviceId));
	const unavailable = unavailableSelectedDevice(selected, state.devices);
	const unavailableName = state.devices.find(
		(device) => device.id === unavailable,
	)?.name;
	const query = search.trim().toLowerCase();
	const devices = state.devices.filter(
		(device) =>
			matchesDeviceFilter(device, filter, appId) &&
			(!selectedOnly || selected.has(device.id)) &&
			(!query ||
				`${device.name} ${device.platform ?? ""}`
					.toLowerCase()
					.includes(query)),
	);
	const preview = draftEvent?.event_type
		? eventEligibility({
				...draftEvent,
				id: draftId,
				active: true,
				event_type: draftEvent.event_type,
				event_version: [0, 0, 0],
			})
		: null;
	const unsupported = preview?.code
		? eligibilityCopy(
				t,
				eligibilityInput(
					{ ...preview, code: preview.code },
					draftEvent?.event_type ?? "",
				),
			).long
		: undefined;
	const singleDevice = isClaimedKind(preview?.kind);
	const tooManyDevices = singleDevice && selected.size > 1;
	const deviceFilters: { id: NewEventDeviceFilter; label: string }[] = [
		{ id: "ready", label: t("newEvent.ready", "Ready") },
		{ id: "app", label: t("newEvent.runsThisApp", "Runs this app") },
		{ id: "all", label: t("newEvent.allDevices", "All") },
	];
	const selectDevice = (deviceId: string, on: boolean) => {
		if (singleDevice && on) {
			state.update({
				targets: [{ deviceId, choices: {}, serveBoth: [], over: {} }],
			});
			return;
		}
		state.toggleDevice(deviceId, on);
	};

	const busy = creating || deploymentIsBusy(run);
	useEffect(() => {
		onBusyChange?.(busy);
	}, [busy, onBusyChange]);
	useEffect(() => {
		onSavedChange?.(!!saved);
	}, [saved, onSavedChange]);
	const finished = useCallback(
		(next: DeployResult) => {
			setResult(next);
			state.markDeployed(next.outcome);
		},
		[state.markDeployed],
	);
	const create = async (deploy: boolean) => {
		if (
			disabled ||
			!canCreateOnHub ||
			unsupported ||
			creatingRef.current ||
			(deploy &&
				(selected.size === 0 || unavailable || tooManyDevices || rows.error)) ||
			state.error
		)
			return;
		creatingRef.current = true;
		setCreating(true);
		setError(null);
		try {
			// Retain the saved record before refreshing. A failed refresh must not create another event.
			const event = savedRef.current ?? (await onCreate());
			savedRef.current = event;
			setInitialDevices([...selected]);
			setSaved(event);
			await invalidate(backend.eventState.getEvents, [appId]);
			if (!deploy) {
				onComplete?.(event);
				return;
			}
			await state.appRead.placements.refetch();
			setTab("settings");
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			creatingRef.current = false;
			setCreating(false);
		}
	};
	const goTo = (step: DeployStepId) =>
		setTab(step === "what" || step === "how" ? "where" : step);
	const stepProps: PlanStepProps = {
		scope,
		route,
		state,
		prepare,
		plan,
		draft,
		check,
		update: state.update,
		goTo,
		onFinished: finished,
		startOver,
		prepared: prepare.prepared,
		blockingText,
		reportDeviceCheck: state.reportDeviceCheck,
	};
	const tabs: { id: DeployStepId; name: string }[] = [
		{ id: "where", name: t("newEvent.devices", "Devices") },
		{ id: "settings", name: t("newEvent.settings", "Settings") },
		{ id: "endpoint", name: t("newEvent.endpoint", "Endpoint & limits") },
		state.mode === "offline"
			? { id: "copy_upload", name: t("newEvent.copy", "Copy & upload") }
			: { id: "access_cost", name: t("newEvent.access", "Access & cost") },
		{ id: "review", name: t("newEvent.review", "Review") },
	];
	const creationDisabled =
		disabled ||
		!canCreateOnHub ||
		creating ||
		state.loading ||
		!!state.error ||
		!!unsupported;
	const creationActions = (
		<div className="flex flex-wrap items-center justify-end gap-2">
			<Button
				type="button"
				variant="outline"
				disabled={creationDisabled}
				onClick={() => void create(false)}
			>
				{t("newEvent.createOnly", "Create only")}
			</Button>
			<Button
				type="button"
				className="h-10"
				disabled={
					creationDisabled ||
					!!rows.error ||
					selected.size === 0 ||
					!!unavailable ||
					tooManyDevices
				}
				onClick={() => void create(true)}
			>
				{creating ? (
					<LoaderCircle aria-hidden className="size-4 animate-spin" />
				) : (
					<Server aria-hidden className="size-4" />
				)}
				{creating
					? t("newEvent.creating", "Creating event…")
					: t("newEvent.createAndDeploy", "Create & deploy")}
			</Button>
		</div>
	);

	if (saved)
		return (
			<div className="min-w-0 space-y-4" data-new-event-deployment="saved">
				<div className="flex items-start gap-3 rounded-xl border bg-primary/5 p-4">
					<Check aria-hidden className="mt-0.5 size-4 shrink-0 text-primary" />
					<div>
						<p className="text-sm font-medium">{saved.name}</p>
						<p className="mt-1 text-xs text-muted-foreground">
							{t(
								"newEvent.savedForDevices",
								"Event saved for device deployment. Review its settings and deploy below.",
							)}
						</p>
					</div>
				</div>
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				{!savedIsListed && (
					<div
						aria-live="polite"
						className="space-y-2 text-sm text-muted-foreground"
					>
						<p>
							{state.error
								? t(
										"newEvent.savedReadFailed",
										"The event was saved, but its settings could not be loaded. Retry to continue.",
									)
								: t("newEvent.readingDefinition", "Loading the saved event…")}
						</p>
						<Button
							type="button"
							variant="outline"
							onClick={() =>
								void invalidate(backend.eventState.getEvents, [appId])
							}
						>
							{t("action.gate.retry", "Retry")}
						</Button>
					</div>
				)}
				{prepare.state === "running" && (
					<p
						aria-live="polite"
						className="flex items-center gap-2 text-sm text-muted-foreground"
					>
						<LoaderCircle aria-hidden className="size-4 animate-spin" />
						{t(
							"newEvent.preparing",
							"Preparing the flow and checking its settings…",
						)}
					</p>
				)}
				{prepare.failure && (
					<div role="alert" className="space-y-2 text-sm text-destructive">
						<p>
							{prepareFailureText(
								t,
								prepare.failure,
								plan,
								state.facts.hub?.hubTypes,
							)}
						</p>
						<Button type="button" variant="outline" onClick={prepare.again}>
							{t("newEvent.retryPreparation", "Retry preparation")}
						</Button>
					</div>
				)}
				{savedIsListed && !running && (
					<>
						<div
							role="tablist"
							aria-label={t(
								"newEvent.deploymentSettings",
								"Deployment settings",
							)}
							className="flex flex-wrap gap-1 rounded-lg bg-muted/60 p-1"
						>
							{tabs.map((item) => (
								<button
									key={item.id}
									type="button"
									role="tab"
									aria-selected={tab === item.id}
									onClick={() => setTab(item.id)}
									className={cn(
										"rounded-md px-2.5 py-2 text-xs transition-colors",
										tab === item.id
											? "bg-background font-medium shadow-sm"
											: "text-muted-foreground hover:text-foreground",
									)}
								>
									{item.name}
								</button>
							))}
						</div>
						{blockingText && (
							<div className="rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 text-xs">
								<p>{blockingText}</p>
								{blocker && blocker.step !== tab && (
									<button
										type="button"
										className="mt-1 font-medium underline underline-offset-4"
										onClick={() => goTo(blocker.step)}
									>
										{t(
											"newEvent.openRequiredSettings",
											"Open required settings",
										)}
									</button>
								)}
							</div>
						)}
						<div role="tabpanel" className="min-w-0">
							{tab === "where" && <WhereStep {...stepProps} />}
							{tab === "settings" && <SettingsStep {...stepProps} />}
							{tab === "endpoint" && <EndpointLimitsStep {...stepProps} />}
							{tab === "access_cost" && <AccessCostStep {...stepProps} />}
							{tab === "copy_upload" && <CopyUploadStep {...stepProps} />}
							{tab === "review" && prepare.state === "ready" && (
								<ReviewStep {...stepProps} />
							)}
						</div>
						{tab !== "review" && (
							<Button
								type="button"
								variant="outline"
								className="w-full"
								onClick={() => setTab("review")}
							>
								{t("newEvent.reviewDeployment", "Review deployment")}
								<ChevronRight aria-hidden className="size-4" />
							</Button>
						)}
					</>
				)}
				{running && <RolloutStep {...stepProps} />}
				{result && (
					<Button
						type="button"
						className="w-full"
						disabled={busy}
						onClick={() => onComplete?.(saved)}
					>
						{t("newEvent.done", "Done")}
					</Button>
				)}
			</div>
		);

	return (
		<div className="space-y-3" data-new-event-deployment="draft">
			<div className="flex items-center justify-between gap-2 text-xs">
				<span className="font-medium">
					{t("newEvent.selectDevices", "Select devices")}
				</span>
				<button
					type="button"
					className="text-muted-foreground hover:text-foreground"
					aria-pressed={selectedOnly}
					onClick={() => setSelectedOnly((value) => !value)}
				>
					{t("newEvent.selectedCount", "{{count}} selected", {
						count: selected.size,
					})}
				</button>
			</div>
			<div className="relative">
				<Search
					aria-hidden
					className="pointer-events-none absolute left-3 top-3 size-4 text-muted-foreground"
				/>
				<Input
					aria-label={t("newEvent.searchDevices", "Search devices")}
					placeholder={t("newEvent.searchDevices", "Search devices")}
					value={search}
					onChange={(event) => setSearch(event.target.value)}
					className="h-10 pl-9"
				/>
			</div>
			<div
				className="flex gap-1 rounded-lg bg-muted/60 p-1"
				aria-label={t("newEvent.filterDevices", "Filter devices")}
			>
				{deviceFilters.map((item) => (
					<button
						key={item.id}
						type="button"
						aria-pressed={filter === item.id}
						onClick={() => {
							setFilter(item.id);
							setSelectedOnly(false);
						}}
						className={cn(
							"min-w-0 flex-1 rounded-md px-2 py-1.5 text-xs transition-colors",
							filter === item.id
								? "bg-background font-medium shadow-sm"
								: "text-muted-foreground hover:text-foreground",
						)}
					>
						{item.label}
						<span className="ml-1.5 tabular-nums text-muted-foreground">
							{
								state.devices.filter((device) =>
									matchesDeviceFilter(device, item.id, appId),
								).length
							}
						</span>
					</button>
				))}
			</div>
			{selected.size > 0 && (
				<div
					className="flex flex-wrap gap-1.5"
					aria-label={t("newEvent.selectedDevices", "Selected devices")}
				>
					{[...selected].map((id) => (
						<button
							key={id}
							type="button"
							disabled={creating}
							onClick={() => selectDevice(id, false)}
							aria-label={t("newEvent.removeDevice", "Remove {{device}}", {
								device:
									state.devices.find((device) => device.id === id)?.name ??
									t("newEvent.unavailableDevice", "unavailable device"),
							})}
							className="inline-flex max-w-full items-center gap-1.5 rounded-md border border-primary/15 bg-primary/5 px-2 py-1 text-xs text-primary"
						>
							<span className="truncate">
								{state.devices.find((device) => device.id === id)?.name ??
									t("newEvent.unavailableDevice", "Unavailable device")}
							</span>
							<X aria-hidden className="size-3 shrink-0" />
						</button>
					))}
				</div>
			)}
			{singleDevice && (
				<p className="text-xs text-muted-foreground">
					{t(
						"newEvent.oneDeviceForTrigger",
						"Schedules and bots run on one device at a time.",
					)}
				</p>
			)}
			{rows.loading || state.loading ? (
				<p
					aria-live="polite"
					className="py-5 text-center text-sm text-muted-foreground"
				>
					{t("newEvent.loadingDevices", "Loading devices…")}
				</p>
			) : rows.error || state.error ? (
				<div role="alert" className="space-y-2 text-sm">
					<p>
						{t("newEvent.devicesUnavailable", "Devices could not be loaded.")}
					</p>
					<Button
						type="button"
						variant="outline"
						onClick={() =>
							void Promise.all([
								rows.refetch(),
								invalidate(backend.eventState.getEvents, [appId]),
							])
						}
					>
						{t("action.gate.retry", "Retry")}
					</Button>
				</div>
			) : (
				<div className="max-h-64 overflow-y-auto rounded-xl border divide-y">
					{devices.length === 0 && (
						<p className="px-4 py-7 text-center text-sm text-muted-foreground">
							{query || selectedOnly || filter !== "all"
								? t("newEvent.noMatchingDevices", "No matching devices.")
								: t(
										"newEvent.noDevices",
										"No devices are connected to this account yet.",
									)}
						</p>
					)}
					{devices.map((device) => (
						<div
							key={device.id}
							className={cn("p-3", selected.has(device.id) && "bg-primary/5")}
						>
							<label className="flex cursor-pointer items-center gap-3">
								<input
									type={singleDevice ? "radio" : "checkbox"}
									name={singleDevice ? "new-event-device" : undefined}
									checked={selected.has(device.id)}
									disabled={creating || !!device.gate}
									onChange={(event) =>
										selectDevice(device.id, event.target.checked)
									}
									className="size-4 accent-primary"
								/>
								<Server
									aria-hidden
									className="size-4 shrink-0 text-muted-foreground"
								/>
								<span className="min-w-0 flex-1">
									<span className="block truncate text-sm font-medium">
										{device.name}
									</span>
									<span className="block text-xs text-muted-foreground">
										{device.platform ?? t("newEvent.device", "Device")}
										{device.locked
											? ` · ${t("newEvent.locked", "locked")}`
											: ""}
									</span>
								</span>
								<span
									className={cn(
										"size-1.5 shrink-0 rounded-full",
										device.presence.kind === "online"
											? "bg-emerald-500"
											: "bg-muted-foreground/40",
									)}
									aria-hidden
								/>
							</label>
							{device.gate && (
								<div className="mt-2 space-y-1 pl-7 text-xs text-muted-foreground">
									<p>{whereGateText(t, device.gate, device.name, time)}</p>
									<GateFixes
										device={device}
										gate={device.gate}
										appName={state.app?.name ?? ""}
										size="sm"
									/>
								</div>
							)}
						</div>
					))}
				</div>
			)}
			<div className="space-y-2 rounded-lg bg-muted/40 p-3 text-xs leading-relaxed text-muted-foreground">
				<p className="font-medium text-foreground">
					{t("newEvent.creationSummary", "What happens when you create")}
				</p>
				<p>
					{t(
						"newEvent.deviceCopyHint",
						"The event is saved in this app for device deployment. After you review settings, each selected device runs the flow snapshot you deploy.",
					)}
					{preview?.route && (
						<code className="mt-1 block font-mono text-foreground">
							{preview.route.method} {preview.route.path}
						</code>
					)}
				</p>
				<p>
					{t(
						"newEvent.removeDeviceService",
						"You can stop or remove a device service later. The event stays in this app.",
					)}
				</p>
			</div>
			{unsupported && (
				<p role="alert" className="text-xs text-destructive">
					{unsupported}
				</p>
			)}
			{state.mode === "online" && !canCreateOnHub && (
				<div
					aria-live="polite"
					className="space-y-2 text-xs text-muted-foreground"
				>
					<p>
						{placements.loading
							? t("newEvent.checkingHub", "Checking device creation support…")
							: placements.error
								? t(
										"newEvent.hubCheckFailed",
										"Device creation support could not be checked. Retry to continue.",
									)
								: t(
										"newEvent.hubUpdateRequired",
										"Update your hub to create events directly on devices.",
									)}
					</p>
					{!placements.loading && (
						<Button
							type="button"
							variant="outline"
							size="sm"
							onClick={() => void placements.refetch()}
						>
							{t("action.gate.retry", "Retry")}
						</Button>
					)}
				</div>
			)}
			{unavailable && (
				<p role="alert" className="text-xs text-destructive">
					{unavailableName
						? t(
								"newEvent.selectedDeviceBlocked",
								"{{device}} is no longer ready for deployment. Resolve its status or remove it from the selection.",
								{ device: unavailableName },
							)
						: t(
								"newEvent.selectedDeviceMissing",
								"A selected device is no longer available. Remove it from the selection.",
							)}
				</p>
			)}
			{tooManyDevices && (
				<p role="alert" className="text-xs text-destructive">
					{t(
						"newEvent.chooseOneDevice",
						"Choose one device for this schedule or bot.",
					)}
				</p>
			)}
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			{footerContainer
				? createPortal(creationActions, footerContainer)
				: creationActions}
		</div>
	);
}
