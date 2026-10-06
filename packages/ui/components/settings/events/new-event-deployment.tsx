"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	ChevronRight,
	LoaderCircle,
	Radio,
	Search,
	Server,
	Wifi,
	WifiOff,
	X,
} from "lucide-react";
import {
	type CSSProperties,
	type ReactNode,
	memo,
	useCallback,
	useDeferredValue,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
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
import type { DeployDevice } from "../devices/deploy/deploy-facts";
import type { PlanStepProps } from "../devices/deploy/step-props";
import { AccessCostStep } from "../devices/deploy/steps/access-cost-step";
import { CopyUploadStep } from "../devices/deploy/steps/copy-upload-step";
import { EndpointLimitsStep } from "../devices/deploy/steps/endpoint-limits-step";
import { ReviewStep } from "../devices/deploy/steps/review-step";
import { RolloutStep } from "../devices/deploy/steps/rollout-step";
import { SettingsStep } from "../devices/deploy/steps/settings-step";
import { WhereStep } from "../devices/deploy/steps/where-step";
import { GateFixes } from "../devices/deploy/target-card";
import { newTarget, useDeployDraft } from "../devices/deploy/use-deploy-draft";
import { useDeployPrepare } from "../devices/deploy/use-deploy-prepare";
import { useDeployRunState } from "../devices/deploy/use-deploy-run";
import { AreaOverlays } from "../devices/overlays/area-overlays";
import type { DevicesT } from "../devices/primitives/area-context";
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
	CHIP_LIMIT,
	type CreateBlockCode,
	DEVICE_PAGE,
	type DeviceFilterCounts,
	type NewEventDeviceFilter,
	createBlock,
	defaultDeviceFilter,
	deploymentIsBusy,
	deviceFilterCounts,
	deviceRunsApp,
	indexDevices,
	listDevices,
	pickableDevices,
	resultFooterAction,
	stepTab,
	unavailableSelectedDevice,
} from "./new-event-device-choice";

/** Test seams: a save that outlives `slowSaveMs` releases the dialog (the event id is reserved, so closing is safe); `rowRendered` counts device row renders. */
export const newEventSeams: {
	slowSaveMs: number;
	rowRendered?: (deviceId: string) => void;
} = { slowSaveMs: 20_000 };

export interface NewEventDeploymentProps {
	appId: string;
	draftEvent?: Partial<IEvent>;
	/** Saves the definition with source activation suppressed. Called once per dialog. */
	onCreate(): Promise<IEvent>;
	disabled?: boolean;
	/** Protects dismissal during creation and active deployment work. */
	onBusyChange?(busy: boolean): void;
	/** The saved event once it exists (locks source fields while dismissal stays possible), null on reset and unmount. */
	onSavedChange?(event: IEvent | null): void;
	/** True once a deployment placed the event on at least one device. */
	onDeployedChange?(deployed: boolean): void;
	/** Called when the user closes the deployment result. */
	onComplete?(event: IEvent): void;
	/** Keeps the creation actions reachable in the dialog's fixed footer. */
	footerContainer?: HTMLElement | null;
	/** Follows explicit links to device setup, services, or the Events page. */
	onNavigate?(href: string): void;
	/** Account and transport substitutes for component tests and previews. */
	overrides?: DeviceWorkspaceOverrides;
}

function InFooter({
	container,
	children,
}: Readonly<{ container?: HTMLElement | null; children: ReactNode }>) {
	return container ? createPortal(children, container) : children;
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
					<>
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
											className="mt-3 min-h-11"
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
										className="mt-3 min-h-11"
										onClick={gate.retry}
									>
										{t("retry", "Retry")}
									</Button>
								</>
							)}
						</div>
						{props.footerContainer && gate.kind !== "loading"
							? createPortal(
									<output className="block min-w-0 flex-1 text-xs text-warning">
										{gate.kind === "signed_out"
											? t(
													"signInToCreateDeviceEvents",
													"Sign in to create events for devices.",
												)
											: t(
													"deviceProfileRequiredToCreate",
													"Your device profile could not be loaded, so events cannot be created for devices yet.",
												)}
									</output>,
									props.footerContainer,
								)
							: null}
					</>
				)}
			>
				<ConfirmProvider>
					<DeploymentPanel {...props} scope={scope} restart={restart} />
					<AreaOverlays scope={scope} onNavigate={props.onNavigate} />
				</ConfirmProvider>
			</DeviceWorkspaceProvider>
		</MemoryDevicesRoute>
	);
}

const ROW_STYLE: CSSProperties = {
	contentVisibility: "auto",
	containIntrinsicSize: "auto 4rem",
};

function ConnectionIndicator({ device }: Readonly<{ device: DeployDevice }>) {
	const { t } = useTranslation("devices");
	const { Icon, good, text } = connectionOf(t, device);
	return (
		<span
			className={cn(
				"inline-flex shrink-0 items-center gap-1 text-xs",
				good ? "text-good" : "text-muted-foreground",
			)}
		>
			<Icon aria-hidden className="size-3.5" />
			{text}
		</span>
	);
}

function connectionOf(t: DevicesT, device: DeployDevice) {
	const { live, presence } = device;
	if (live.kind === "live" || live.kind === "renewing")
		return {
			Icon: Radio,
			good: true,
			text:
				live.transport === "websocket"
					? t("deploy.card.liveRelayed", "Live · relayed")
					: t("deploy.card.liveDirect", "Live · direct"),
		};
	switch (presence.kind) {
		case "online":
			return {
				Icon: Wifi,
				good: true,
				text: t("newEvent.presence.online", "Online"),
			};
		case "late":
			return {
				Icon: WifiOff,
				good: false,
				text: t("newEvent.presence.late", "Late"),
			};
		case "never":
			return {
				Icon: WifiOff,
				good: false,
				text: t("newEvent.presence.never", "Not connected yet"),
			};
		default:
			return {
				Icon: WifiOff,
				good: false,
				text: t("newEvent.presence.offline", "Offline"),
			};
	}
}

function GateReason({
	device,
	appName,
}: Readonly<{ device: DeployDevice; appName: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const [fixing, setFixing] = useState(false);
	const { gate } = device;
	if (!gate) return null;
	return (
		<div className="mt-1 space-y-1 pl-7 text-xs text-muted-foreground">
			<div className="flex flex-wrap items-center gap-x-3">
				<p className="min-w-0 flex-1">
					{whereGateText(t, gate, device.name, time)}
				</p>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="min-h-11"
					aria-expanded={fixing}
					aria-label={t("newEvent.fixDevice", "Fix {{device}}", {
						device: device.name,
					})}
					onClick={() => setFixing((value) => !value)}
				>
					{t("newEvent.fix", "Fix")}
				</Button>
			</div>
			{fixing && (
				<div className="flex flex-wrap gap-2">
					<GateFixes device={device} gate={gate} appName={appName} size="sm" />
				</div>
			)}
		</div>
	);
}

interface DeviceRowProps {
	device: DeployDevice;
	checked: boolean;
	disabled: boolean;
	single: boolean;
	appId: string;
	appName: string;
	onToggle(deviceId: string, on: boolean): void;
}

/** One device of the picker; it only re-renders when its own facts or selection change. */
export const NewEventDeviceRow = memo(function NewEventDeviceRow({
	device,
	checked,
	disabled,
	single,
	appId,
	appName,
	onToggle,
}: Readonly<DeviceRowProps>) {
	const { t } = useTranslation("devices");
	newEventSeams.rowRendered?.(device.id);
	const facts = [
		device.platform ?? t("newEvent.device", "Device"),
		device.agent
			? t("newEvent.agent", "agent {{version}}", { version: device.agent })
			: null,
		deviceRunsApp(device, appId)
			? t("newEvent.runsThisApp", "Runs this app")
			: null,
		device.locked ? t("newEvent.locked", "Locked") : null,
	].filter(Boolean);
	return (
		<div
			className={cn("px-3 py-1", checked && "bg-primary/5")}
			style={ROW_STYLE}
		>
			<label className="flex min-h-11 cursor-pointer items-center gap-3">
				<input
					type={single ? "radio" : "checkbox"}
					name={single ? "new-event-device" : undefined}
					checked={checked}
					disabled={disabled || !!device.gate}
					onChange={(event) => onToggle(device.id, event.target.checked)}
					className="size-4 accent-primary"
				/>
				<Server aria-hidden className="size-4 shrink-0 text-muted-foreground" />
				<span className="min-w-0 flex-1">
					<span className="block truncate text-sm font-medium">
						{device.name}
					</span>
					<span className="block truncate text-xs text-muted-foreground">
						{facts.join(" · ")}
					</span>
				</span>
				<ConnectionIndicator device={device} />
			</label>
			{device.gate && <GateReason device={device} appName={appName} />}
		</div>
	);
});

interface Selection {
	selected: ReadonlySet<string>;
	index: ReadonlyMap<string, DeployDevice>;
	single: boolean;
	locked: boolean;
	onToggle(deviceId: string, on: boolean): void;
}

function FilterGroup({
	counts,
	active,
	onPick,
}: Readonly<{
	counts: DeviceFilterCounts;
	active: NewEventDeviceFilter;
	onPick(filter: NewEventDeviceFilter): void;
}>) {
	const { t } = useTranslation("devices");
	const filters: { id: NewEventDeviceFilter; label: string }[] = [
		{ id: "ready", label: t("newEvent.ready", "Ready") },
		{ id: "app", label: t("newEvent.runsThisApp", "Runs this app") },
		{ id: "all", label: t("newEvent.allDevices", "All") },
	];
	return (
		<fieldset
			aria-label={t("newEvent.filterDevices", "Filter devices")}
			className="m-0 flex min-w-0 gap-1 rounded-lg border-0 bg-muted/60 p-1"
		>
			{filters.map((item) => (
				<button
					key={item.id}
					type="button"
					aria-pressed={active === item.id}
					onClick={() => onPick(item.id)}
					className={cn(
						"min-h-11 min-w-0 flex-1 rounded-md px-2 text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
						active === item.id
							? "bg-background font-medium shadow-sm"
							: "text-muted-foreground hover:text-foreground",
					)}
				>
					{item.label}
					<span className="ml-1.5 tabular-nums text-muted-foreground">
						{counts[item.id]}
					</span>
				</button>
			))}
		</fieldset>
	);
}

function SelectedChips({
	selection: { selected, index, locked, onToggle },
	onMore,
}: Readonly<{ selection: Selection; onMore(): void }>) {
	const { t } = useTranslation("devices");
	const chips = useMemo(() => [...selected].slice(0, CHIP_LIMIT), [selected]);
	const hidden = selected.size - chips.length;
	if (selected.size === 0) return null;
	return (
		<fieldset
			className="m-0 flex min-w-0 flex-wrap gap-1.5 border-0 p-0"
			aria-label={t("newEvent.selectedDevices", "Selected devices")}
		>
			{chips.map((id) => {
				const name = index.get(id)?.name;
				return (
					<button
						key={id}
						type="button"
						disabled={locked}
						onClick={() => onToggle(id, false)}
						aria-label={t("newEvent.removeDevice", "Remove {{device}}", {
							device:
								name ?? t("newEvent.unavailableDevice", "unavailable device"),
						})}
						className="inline-flex min-h-11 max-w-full items-center gap-1.5 rounded-md border border-primary/15 bg-primary/5 px-3 text-xs text-foreground"
					>
						<span className="truncate">
							{name ?? t("newEvent.unavailableDevice", "Unavailable device")}
						</span>
						<X aria-hidden className="size-3 shrink-0" />
					</button>
				);
			})}
			{hidden > 0 && (
				<button
					type="button"
					onClick={onMore}
					className="inline-flex min-h-11 items-center rounded-md border px-3 text-xs text-muted-foreground"
				>
					{t("newEvent.moreSelected", "+{{count}} more", { count: hidden })}
				</button>
			)}
		</fieldset>
	);
}

function DeviceList({
	shown,
	remaining,
	empty,
	selection: { selected, single, locked, onToggle },
	appId,
	appName,
	onMore,
}: Readonly<{
	shown: readonly DeployDevice[];
	remaining: number;
	empty: boolean;
	selection: Selection;
	appId: string;
	appName: string;
	onMore(): void;
}>) {
	const { t } = useTranslation("devices");
	return (
		<div className="max-h-80 overflow-y-auto rounded-xl border divide-y">
			{shown.length === 0 && (
				<p className="px-4 py-7 text-center text-sm text-muted-foreground">
					{empty
						? t("newEvent.noMatchingDevices", "No matching devices.")
						: t(
								"newEvent.noDevices",
								"No devices are connected to this account yet.",
							)}
				</p>
			)}
			{shown.map((device) => (
				<NewEventDeviceRow
					key={device.id}
					device={device}
					checked={selected.has(device.id)}
					disabled={locked}
					single={single}
					appId={appId}
					appName={appName}
					onToggle={onToggle}
				/>
			))}
			{remaining > 0 && (
				<div className="p-2 text-center">
					<Button
						type="button"
						variant="ghost"
						className="min-h-11"
						onClick={onMore}
					>
						{t("newEvent.showMore", "Show {{count}} more", {
							count: Math.min(DEVICE_PAGE, remaining),
						})}
					</Button>
				</div>
			)}
		</div>
	);
}

/** True once `value` has been true; only an unmount resets it. */
export function useLatched(value: boolean): boolean {
	const [latched, setLatched] = useState(value);
	useEffect(() => {
		if (value) setLatched(true);
	}, [value]);
	return latched || value;
}

const PICK_ALL_CONFIRM_ABOVE = 50;

function PickAll({
	count,
	disabled,
	onPick,
}: Readonly<{ count: number; disabled: boolean; onPick(): void }>) {
	const { t } = useTranslation("devices");
	const [confirming, setConfirming] = useState(false);
	if (confirming)
		return (
			<span className="flex flex-wrap items-center gap-1">
				<output className="text-xs">
					{t("newEvent.pickAllConfirm", "Pick {{count}} devices?", { count })}
				</output>
				<Button
					type="button"
					size="sm"
					className="min-h-11"
					onClick={() => {
						setConfirming(false);
						onPick();
					}}
				>
					{t("newEvent.pickAllConfirmYes", "Pick {{count}}", { count })}
				</Button>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="min-h-11"
					onClick={() => setConfirming(false)}
				>
					{t("newEvent.pickAllConfirmNo", "Cancel")}
				</Button>
			</span>
		);
	return (
		<Button
			type="button"
			variant="ghost"
			size="sm"
			className="min-h-11"
			disabled={disabled}
			onClick={() =>
				count > PICK_ALL_CONFIRM_ABOVE ? setConfirming(true) : onPick()
			}
		>
			{t("newEvent.pickAll", "Pick all {{count}} ready devices", { count })}
		</Button>
	);
}

interface DeviceChoiceProps {
	appId: string;
	appName: string;
	devices: readonly DeployDevice[];
	selection: Selection;
	status: { loading: boolean; failed: boolean; onRetry(): void };
	onPickAll(ids: string[]): void;
}

function DeviceChoice({
	appId,
	appName,
	devices,
	selection,
	status,
	onPickAll,
}: Readonly<DeviceChoiceProps>) {
	const { t } = useTranslation("devices");
	const { selected, single, locked } = selection;
	const [search, setSearch] = useState("");
	const [selectedOnly, setSelectedOnly] = useState(false);
	const [filter, setFilter] = useState<NewEventDeviceFilter | null>(null);
	const [limit, setLimit] = useState(DEVICE_PAGE);
	const counts = useMemo(
		() => deviceFilterCounts(devices, appId),
		[devices, appId],
	);
	const activeFilter = filter ?? defaultDeviceFilter(counts);
	const query = useDeferredValue(search.trim().toLowerCase());
	const listed = useMemo(
		() =>
			listDevices(devices, {
				filter: activeFilter,
				appId,
				search: query,
				selectedOnly,
				selected,
			}),
		[devices, activeFilter, appId, query, selectedOnly, selected],
	);
	const pickable = useMemo(
		() => (single ? [] : pickableDevices(listed, selected)),
		[single, listed, selected],
	);
	const shown = listed.slice(0, limit);
	const showSelectedOnly = (on: boolean) => {
		setSelectedOnly(on);
		setLimit(DEVICE_PAGE);
	};
	return (
		<>
			<div className="flex flex-wrap items-center justify-between gap-2 text-xs">
				<span className="font-medium">
					{t("newEvent.selectDevices", "Select devices")}
				</span>
				<div className="flex flex-wrap items-center gap-1">
					{pickable.length > 0 && (
						<PickAll
							key={pickable.length}
							count={pickable.length}
							disabled={locked}
							onPick={() => onPickAll(pickable)}
						/>
					)}
					<Button
						type="button"
						variant="ghost"
						size="sm"
						className="min-h-11 text-muted-foreground"
						aria-pressed={selectedOnly}
						onClick={() => showSelectedOnly(!selectedOnly)}
					>
						{t("newEvent.selectedCount", "{{count}} selected", {
							count: selected.size,
						})}
						<span className="sr-only">
							{t("newEvent.showSelectedOnly", ", show only the selected")}
						</span>
					</Button>
				</div>
			</div>
			<div className="relative">
				<Search
					aria-hidden
					className="pointer-events-none absolute left-3 top-4 size-4 text-muted-foreground"
				/>
				<Input
					aria-label={t("newEvent.searchDevices", "Search devices")}
					placeholder={t("newEvent.searchDevices", "Search devices")}
					value={search}
					onChange={(event) => {
						setSearch(event.target.value);
						setLimit(DEVICE_PAGE);
					}}
					className="h-11 pl-9"
				/>
			</div>
			<FilterGroup
				counts={counts}
				active={activeFilter}
				onPick={(next) => {
					setFilter(next);
					showSelectedOnly(false);
				}}
			/>
			<SelectedChips
				selection={selection}
				onMore={() => showSelectedOnly(true)}
			/>
			{single && (
				<p className="text-xs text-muted-foreground">
					{t(
						"newEvent.oneDeviceForTrigger",
						"Schedules and bots run on one device at a time.",
					)}
				</p>
			)}
			{status.loading ? (
				<p
					aria-live="polite"
					className="py-5 text-center text-sm text-muted-foreground"
				>
					{t("newEvent.loadingDevices", "Loading devices…")}
				</p>
			) : status.failed ? (
				<div role="alert" className="space-y-2 text-sm">
					<p>
						{t("newEvent.devicesUnavailable", "Devices could not be loaded.")}
					</p>
					<Button
						type="button"
						variant="outline"
						className="min-h-11"
						onClick={status.onRetry}
					>
						{t("action.gate.retry", "Retry")}
					</Button>
				</div>
			) : (
				<DeviceList
					shown={shown}
					remaining={listed.length - shown.length}
					empty={!!query || selectedOnly || activeFilter !== "all"}
					selection={selection}
					appId={appId}
					appName={appName}
					onMore={() => setLimit((value) => value + DEVICE_PAGE)}
				/>
			)}
		</>
	);
}

function DeploymentPanel({
	appId,
	draftEvent,
	onCreate,
	disabled,
	onBusyChange,
	onSavedChange,
	onDeployedChange,
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
	const [slow, setSlow] = useState(false);
	const creatingRef = useRef(false);
	const [error, setError] = useState<string | null>(null);
	const [tab, setTab] = useState<DeployStepId>("where");
	const [landed, setLanded] = useState(false);
	const [settled, setSettled] = useState(false);
	const [result, setResult] = useState<DeployResult | null>(null);
	const [footerSlot, setFooterSlot] = useState<HTMLElement | null>(null);
	const headingRef = useRef<HTMLHeadingElement>(null);
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
	const state = useDeployDraft(route, scope, { persist: !!saved });
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
	const prepareOver = prepare.state === "ready" || prepare.state === "blocked";
	useEffect(() => {
		setCatalog(approved);
		setSettled(prepareOver);
	}, [approved, prepareOver, setCatalog]);
	const run = useDeployRunState(draft.deploymentId);
	const running = !!run && run.status !== "idle";
	const names = planNames(t, plan, state.facts, time.abs);
	const blocker = check.firstBlocking;
	const blockingText = blocker ? issueText(t, blocker, names) : undefined;
	const selected = useMemo(
		() => new Set(draft.targets.map((target) => target.deviceId)),
		[draft.targets],
	);
	const index = useMemo(() => indexDevices(state.devices), [state.devices]);
	const unavailable = unavailableSelectedDevice(selected, index);
	const hub = state.facts.hub;
	const preview = draftEvent?.event_type
		? eventEligibility(
				{
					...draftEvent,
					id: draftId,
					active: true,
					event_type: draftEvent.event_type,
					event_version: [0, 0, 0],
				},
				{
					...(hub?.hubTypes ? { hubTypes: hub.hubTypes } : {}),
					...(hub?.hubSchedules === undefined
						? {}
						: { hubSchedules: hub.hubSchedules }),
				},
			)
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

	const hostRef = useRef({ onBusyChange, onSavedChange, onDeployedChange });
	hostRef.current = { onBusyChange, onSavedChange, onDeployedChange };
	useEffect(
		() => () => {
			hostRef.current.onBusyChange?.(false);
			hostRef.current.onSavedChange?.(null);
			hostRef.current.onDeployedChange?.(false);
		},
		[],
	);
	useEffect(() => {
		if (!creating) {
			setSlow(false);
			return;
		}
		const timer = setTimeout(() => setSlow(true), newEventSeams.slowSaveMs);
		return () => clearTimeout(timer);
	}, [creating]);
	const busy = (creating && !slow) || deploymentIsBusy(run);
	useEffect(() => {
		onBusyChange?.(busy);
	}, [busy, onBusyChange]);
	useEffect(() => {
		onSavedChange?.(saved);
	}, [saved, onSavedChange]);
	const deployed = useLatched((result?.done.length ?? 0) > 0);
	useEffect(() => {
		onDeployedChange?.(deployed);
	}, [deployed, onDeployedChange]);
	useEffect(() => {
		if (saved) headingRef.current?.focus();
	}, [saved]);
	useEffect(() => {
		if (!saved || !savedIsListed || !settled || landed) return;
		setLanded(true);
		setTab(stepTab(check.firstBlocking?.step));
	}, [saved, savedIsListed, settled, landed, check.firstBlocking]);

	const finished = useCallback(
		(next: DeployResult) => {
			setResult(next);
			state.markDeployed(next.outcome);
		},
		[state.markDeployed],
	);
	const refreshEvents = () => invalidate(backend.eventState.getEvents, [appId]);
	const goTo = (step: DeployStepId) => setTab(stepTab(step));
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
		embedded: true,
		deployMore: startOver,
		footerContainer: footerContainer ? footerSlot : undefined,
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

	const toggleRef = useRef<(deviceId: string, on: boolean) => void>(() => {});
	toggleRef.current = (deviceId, on) => {
		if (singleDevice && on) state.update({ targets: [newTarget(deviceId)] });
		else state.toggleDevice(deviceId, on);
	};
	const onToggle = useCallback(
		(deviceId: string, on: boolean) => toggleRef.current(deviceId, on),
		[],
	);
	const onPickAll = (ids: string[]) =>
		state.update({ targets: [...draft.targets, ...ids.map(newTarget)] });

	const deviceCount = (count: number) =>
		t("newEvent.deviceCount", "{{count, number}} devices", { count });
	const [firstSelected] = selected;
	const target =
		selected.size === 1
			? (index.get(firstSelected ?? "")?.name ?? null)
			: selected.size > 1
				? deviceCount(selected.size)
				: null;
	const blockedDevice = unavailable ? index.get(unavailable) : undefined;
	const reasons: Record<CreateBlockCode, () => string> = {
		form_incomplete: () =>
			t("newEvent.completeDetails", "Complete the event details first."),
		unsupported: () => unsupported ?? "",
		devices_loading: () => t("newEvent.loadingDevices", "Loading devices…"),
		devices_failed: () =>
			t("newEvent.devicesUnavailable", "Devices could not be loaded."),
		hub_loading: () =>
			t("newEvent.checkingHub", "Checking device creation support…"),
		hub_failed: () =>
			t(
				"newEvent.hubCheckFailed",
				"Device creation support could not be checked. Retry to continue.",
			),
		hub_outdated: () =>
			t(
				"newEvent.hubUpdateRequired",
				"Update your hub to create events directly on devices.",
			),
		too_many: () =>
			t(
				"newEvent.chooseOneDevice",
				"Choose one device for this schedule or bot.",
			),
		device_blocked: () =>
			blockedDevice?.gate
				? t("newEvent.deviceNotReady", "{{device}} isn't ready: {{reason}}", {
						device: blockedDevice.name,
						reason: whereGateText(
							t,
							blockedDevice.gate,
							blockedDevice.name,
							time,
						),
					})
				: t(
						"newEvent.selectedDeviceMissing",
						"A selected device is no longer available. Remove it from the selection.",
					),
		no_device: () => t("newEvent.pickDevice", "Pick a device to deploy to."),
	};
	const block = createBlock({
		hostDisabled: !!disabled && !creating,
		hubBlocked: state.mode === "online" && !canCreateOnHub,
		hubLoading: placements.loading,
		hubFailed: !!placements.error,
		devicesLoading: rows.loading || state.loading,
		devicesFailed: !!(rows.error || state.error),
		unsupported: !!unsupported,
		selected: selected.size,
		singleDevice,
		unavailable,
	});
	const hubRetry = block?.startsWith("hub_") && block !== "hub_loading";
	const createOnlyDisabled =
		disabled ||
		!canCreateOnHub ||
		creating ||
		state.loading ||
		!!state.error ||
		!!unsupported;
	const create = async (deploy: boolean) => {
		if (creatingRef.current || createOnlyDisabled || (deploy && block)) return;
		creatingRef.current = true;
		setCreating(true);
		setError(null);
		try {
			// Retain the saved record before refreshing. A failed refresh must not create another event.
			const event = savedRef.current ?? (await onCreate());
			savedRef.current = event;
			if (deploy) {
				setInitialDevices([...selected]);
				setSaved(event);
			}
			await refreshEvents();
			if (!deploy) {
				onComplete?.(event);
				return;
			}
			await state.appRead.placements.refetch();
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			creatingRef.current = false;
			setCreating(false);
		}
	};
	const announce = (
		<output className="sr-only">
			{saved ? t("newEvent.announceSaved", "Event saved") : ""}
		</output>
	);

	if (saved) {
		const savedTarget =
			initialDevices.length === 1
				? (index.get(initialDevices[0] ?? "")?.name ??
					deviceCount(initialDevices.length))
				: deviceCount(initialDevices.length);
		return (
			<>
				{announce}
				<div className="min-w-0 space-y-4" data-new-event-deployment="saved">
					<output className="flex items-start gap-3 rounded-xl border border-good-line bg-good-bg p-4">
						<Check aria-hidden className="mt-0.5 size-4 shrink-0 text-good" />
						<div>
							<h3
								ref={headingRef}
								tabIndex={-1}
								className="text-sm font-medium outline-none"
							>
								{t("newEvent.eventSaved", "{{name}} saved", {
									name: saved.name,
								})}
							</h3>
							<p className="mt-1 text-xs text-muted-foreground">
								{t(
									"newEvent.savedNext",
									"Deploy it to {{target}} now, or close and do it later from its Runs on column.",
									{ target: savedTarget },
								)}
							</p>
						</div>
					</output>
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
								className="min-h-11"
								onClick={() => void refreshEvents()}
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
							<Button
								type="button"
								variant="outline"
								className="min-h-11"
								onClick={prepare.again}
							>
								{t("newEvent.retryPreparation", "Retry preparation")}
							</Button>
						</div>
					)}
					{savedIsListed && landed && !running && (
						<Tabs
							value={tab}
							onValueChange={(value) => setTab(value as DeployStepId)}
							className="gap-3"
						>
							<TabsList
								aria-label={t(
									"newEvent.deploymentSettings",
									"Deployment settings",
								)}
								className="h-auto w-full flex-wrap justify-start"
							>
								{tabs.map((item) => (
									<TabsTrigger
										key={item.id}
										value={item.id}
										className="min-h-11 flex-none px-3 text-xs"
									>
										{item.name}
									</TabsTrigger>
								))}
							</TabsList>
							{blockingText && (
								<div className="rounded-lg border border-warning-line bg-warning-bg p-3 text-xs text-warning">
									<p>{blockingText}</p>
									{blocker && blocker.step !== tab && (
										<button
											type="button"
											className="mt-1 min-h-11 font-medium underline underline-offset-4"
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
							<TabsContent value="where" className="min-w-0">
								<WhereStep {...stepProps} />
							</TabsContent>
							<TabsContent value="settings" className="min-w-0">
								<SettingsStep {...stepProps} />
							</TabsContent>
							<TabsContent value="endpoint" className="min-w-0">
								<EndpointLimitsStep {...stepProps} />
							</TabsContent>
							<TabsContent value="access_cost" className="min-w-0">
								<AccessCostStep {...stepProps} />
							</TabsContent>
							<TabsContent value="copy_upload" className="min-w-0">
								<CopyUploadStep {...stepProps} />
							</TabsContent>
							<TabsContent value="review" className="min-w-0">
								{prepare.state === "ready" && <ReviewStep {...stepProps} />}
							</TabsContent>
						</Tabs>
					)}
					{running && <RolloutStep {...stepProps} />}
					<InFooter container={footerContainer}>
						<div
							className="flex min-w-0 flex-wrap items-center justify-end gap-2"
							data-new-event-footer="saved"
						>
							{result ? (
								resultFooterAction(result) === "done" && (
									<Button
										type="button"
										className="h-11"
										disabled={busy}
										onClick={() => onComplete?.(saved)}
									>
										{t("newEvent.done", "Done")}
									</Button>
								)
							) : running ? (
								<Button type="button" className="h-11" disabled>
									<LoaderCircle aria-hidden className="size-4 animate-spin" />
									{t("newEvent.deploying", "Deploying…")}
								</Button>
							) : (
								savedIsListed &&
								landed &&
								tab !== "review" && (
									<Button
										type="button"
										className="h-11"
										onClick={() => setTab("review")}
									>
										{t("newEvent.reviewDeployment", "Review deployment")}
										<ChevronRight aria-hidden className="size-4" />
									</Button>
								)
							)}
							<div ref={setFooterSlot} className="contents" />
						</div>
					</InFooter>
				</div>
			</>
		);
	}

	return (
		<>
			{announce}
			<div className="space-y-3" data-new-event-deployment="draft">
				<DeviceChoice
					appId={appId}
					appName={state.app?.name ?? ""}
					devices={state.devices}
					selection={{
						selected,
						index,
						single: singleDevice,
						locked: creating,
						onToggle,
					}}
					status={{
						loading: rows.loading || state.loading,
						failed: !!(rows.error || state.error),
						onRetry: () => void Promise.all([rows.refetch(), refreshEvents()]),
					}}
					onPickAll={onPickAll}
				/>
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
				{hubRetry && (
					<Button
						type="button"
						variant="outline"
						className="min-h-11"
						onClick={() => void placements.refetch()}
					>
						{t("newEvent.checkAgain", "Check again")}
					</Button>
				)}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				<InFooter container={footerContainer}>
					<div
						className="flex min-w-0 flex-wrap items-center justify-end gap-2"
						data-new-event-footer="draft"
					>
						{creating && slow ? (
							<output className="block min-w-0 flex-1 text-xs">
								{t(
									"newEvent.slowSave",
									"Taking longer than usual. You can close this; the event id is reserved, so nothing is lost.",
								)}
							</output>
						) : (
							block && (
								<output
									className={cn(
										"block min-w-0 flex-1 text-xs",
										block === "no_device" ||
											block === "devices_loading" ||
											block === "form_incomplete"
											? "text-muted-foreground"
											: "text-warning",
									)}
								>
									{reasons[block]()}
								</output>
							)
						)}
						<Button
							type="button"
							variant="outline"
							className="h-11"
							disabled={createOnlyDisabled}
							onClick={() => void create(false)}
						>
							{t("newEvent.createOnly", "Create only")}
						</Button>
						<Button
							type="button"
							className="h-11 max-w-full"
							disabled={createOnlyDisabled || block !== null}
							onClick={() => void create(true)}
						>
							{creating ? (
								<LoaderCircle aria-hidden className="size-4 animate-spin" />
							) : (
								<Server aria-hidden className="size-4" />
							)}
							<span className="truncate">
								{creating
									? t("newEvent.creating", "Creating event…")
									: target
										? t(
												"newEvent.createAndDeployTo",
												"Create & deploy to {{target}}",
												{ target },
											)
										: t("newEvent.createAndDeploy", "Create & deploy")}
							</span>
						</Button>
					</div>
				</InFooter>
			</div>
		</>
	);
}
