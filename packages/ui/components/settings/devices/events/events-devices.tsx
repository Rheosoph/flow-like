"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueries } from "@tanstack/react-query";
import {
	type MouseEvent,
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { AuthContext, type AuthContextProps } from "react-oidc-context";
import { useInvoke } from "../../../../hooks/use-invoke";
import {
	type EventEligibility,
	eventEligibility,
} from "../../../../lib/device-management/deployment";
import { queries } from "../../../../lib/device-management/hub/queries";
import type { AppView } from "../../../../lib/device-management/model/app-plan";
import { evaluateGate } from "../../../../lib/device-management/model/gates";
import type {
	DevicesRoute,
	DevicesScope,
	FixAction,
	Freshness,
	GateResult,
	HubErrorCode,
} from "../../../../lib/device-management/model/types";
import type { IEvent } from "../../../../lib/schema/flow/event";
import type { IHub } from "../../../../lib/schema/hub/hub";
import { useBackend, useBackendReady } from "../../../../state/backend-state";
import { withoutAccess } from "../app/app-view-local";
import { appCopy } from "../copy/app-copy";
import { AreaOverlays } from "../overlays/area-overlays";
import { DvSheet } from "../primitives/dv-sheet";
import { ModeExplainer } from "../primitives/how-runs";
import { devicesHref } from "../routing/devices-href";
import { useDevicesRoute } from "../routing/use-devices-route";
import {
	type Coverage,
	type DeviceWorkspaceOverrides,
	DeviceWorkspaceProvider,
	type FixOutcome,
	buildGateContext,
	useAppView,
	useAreaGate,
	useAttentionState,
	useCoverage,
	useDeviceRows,
	useDeviceViews,
	useDeviceWorkspace,
	useFixAction,
} from "../workspace";
import { type RunsOnRow, runsOnDeviceNames, runsOnRows } from "./runs-on-model";

/* The Events page's view of the device area (APP §4): one read for every row. */

export type EventsDevicesStatus =
	| "signed_out"
	| "token"
	| "hub_off"
	| "blind"
	| "loading"
	| "error"
	| "ready";

/** Why Run on a device… is off for every row; shown once above the sections (APP §4.5). */
export type EventsDevicesBlock =
	| "signed_out"
	| "token"
	| "hub_off"
	| "blind"
	| "error"
	| "no_target";

export interface EventsDevicesLive {
	view: AppView;
	rows: ReadonlyMap<string, RunsOnRow>;
	names: ReadonlyMap<string, string>;
	coverage: Coverage;
	/** At least one device can take a deploy of this app, at most an unlock or a connect away. */
	canDeploy: boolean;
	/** The device list read, for the strip's stamp (R5). */
	hub: Freshness;
	/** Runs a gate's fix: overlays open here, places come back as a route. */
	runFix(fix: FixAction): FixOutcome;
}

export interface EventsDevicesProblem {
	code?: HubErrorCode;
	retry?(): void;
}

interface LiveReport {
	status: EventsDevicesStatus;
	live: EventsDevicesLive | null;
	problem: EventsDevicesProblem | null;
}

export interface EventsDevicesLink {
	href: string;
	onClick(event: MouseEvent<HTMLAnchorElement>): void;
}

export interface EventsDevicesValue extends LiveReport {
	appId: string;
	scope: DevicesScope;
	block: EventsDevicesBlock | null;
	events: ReadonlyMap<string, IEvent>;
	/** The device's event rule per event, from the events the page lists. */
	eligibility: ReadonlyMap<string, EventEligibility>;
	/** The event whose popover a deep link or the row menu asked for. */
	focus: { eventId: string; token: number } | null;
	/** True for the first visible cell that asks: only one popover opens per request. */
	claimFocus(token: number): boolean;
	showOnDevices(eventId: string): void;
	/** Opens "Online or offline?" for this app. */
	explainMode(): void;
	/** A real link into the device area that navigates in the app on a plain click. */
	link(route: DevicesRoute, scope?: DevicesScope): EventsDevicesLink;
	/** Opens a device area route from a button (a fix that is a place). */
	go(route: DevicesRoute, scope?: DevicesScope): void;
	signIn?(): void;
	openEvent?(eventId: string): void;
	renderTile?(eventId: string): ReactNode;
}

const EventsDevicesContext = createContext<EventsDevicesValue | null>(null);

export function useEventsDevices(): EventsDevicesValue {
	const value = useContext(EventsDevicesContext);
	if (!value)
		throw new Error(
			"The Events device column needs an EventsDevicesProvider above it.",
		);
	return value;
}

/** `null` where neither the Events page nor a list in the Devices area provides the column's data. */
export function useOptionalEventsDevices(): EventsDevicesValue | null {
	return useContext(EventsDevicesContext);
}

/** For tests and galleries: the column over a hand-made value. */
export const EventsDevicesValueProvider = EventsDevicesContext.Provider;

export interface EventsDevicesHarness {
	overrides?: DeviceWorkspaceOverrides;
}

export interface EventsDevicesProviderProps {
	appId: string;
	events: readonly IEvent[];
	/** The host's hub record; with device support off no workspace is created. */
	hub?: IHub;
	canReadBoards: boolean;
	onNavigate?(href: string): void;
	/** Opens the event editor (the fix for an event that can't run on devices). */
	onOpenEvent?(eventId: string): void;
	renderTile?(eventId: string): ReactNode;
	harness?: EventsDevicesHarness;
	children: ReactNode;
}

/*
 * A passive workspace provider renders its fallback for every state before a
 * workspace exists, so who is signed in is read here the way the provider does.
 */
function useHostSignIn(overrides: DeviceWorkspaceOverrides | undefined) {
	const oidc = useContext(AuthContext);
	const ready = useBackendReady();
	const auth = overrides?.auth;
	return useMemo(() => {
		if (auth)
			return {
				signedOut: !auth.loading && !auth.signedIn,
				signIn: auth.signIn,
			};
		if (!oidc) return { signedOut: ready, signIn: undefined };
		const signedIn = Boolean(oidc.isAuthenticated && oidc.user?.profile.sub);
		return {
			signedOut: ready && !oidc.isLoading && !signedIn,
			signIn: () => signInAndReturn(oidc),
		};
	}, [auth, oidc, ready]);
}

/** Starts the host's sign-in and comes back to this page. */
function signInAndReturn(oidc: AuthContextProps) {
	const { pathname, search } = globalThis.location ?? {};
	void oidc.signinRedirect({
		url_state: pathname === undefined ? undefined : pathname + search,
	});
}

const LOADING: LiveReport = { status: "loading", live: null, problem: null };

function isPlainClick(event: MouseEvent<HTMLAnchorElement>) {
	return (
		!event.defaultPrevented &&
		event.button === 0 &&
		!(event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
	);
}

/** Steps the deploy wizard takes on this computer: a device waiting for one of them can still be a target. */
const DEPLOY_STEPS: readonly FixAction["kind"][] = [
	"unlock",
	"connect",
	"take_over",
];

/** Offline, revoked, shared for another app or without keys here: those can't take a deploy (APP §4.5). */
function takesDeploy(gate: GateResult): boolean {
	return gate.ok || (!!gate.fix && DEPLOY_STEPS.includes(gate.fix.kind));
}

/** What the device workspace says about one app's events: the hub's gates first, then the fleet. */
function useLiveReport(appId: string): LiveReport {
	const gate = useAreaGate();
	const list = useDeviceRows();
	const state = useAttentionState();
	const { hub } = useDeviceWorkspace();
	const { devices } = state.input;
	const extras = useMemo(
		() => ({ focusDeviceIds: devices.map((row) => row.device_id) }),
		[devices],
	);
	const app = useAppView(appId, extras);
	// What a shared device's access covers tells "no access" from "unknown until unlocked": the hub says it per device.
	useQueries({
		queries: devices
			.filter((row) => row.status === "active" && row.relationship === "shared")
			.map((row) => queries.myAccess(hub, row.device_id)),
	});
	const coverage = useCoverage(appId);
	const view = useMemo(
		() => (app.view ? withoutAccess(app.view, coverage.noAccess) : undefined),
		[app.view, coverage.noAccess],
	);
	const runFix = useFixAction();
	const canDeploy = useMemo(
		() =>
			devices.some((row) =>
				takesDeploy(
					evaluateGate(
						"create_service",
						buildGateContext(state, row.device_id, { projectId: appId }),
					),
				),
			),
		[devices, state, appId],
	);

	return useMemo<LiveReport>(() => {
		const failed = (problem: EventsDevicesProblem): LiveReport => ({
			status: "error",
			live: null,
			problem,
		});
		if (gate.kind === "token_restricted")
			return { status: "token", live: null, problem: null };
		if (gate.kind === "session_expired")
			return { status: "signed_out", live: null, problem: null };
		if (gate.kind === "hub_off")
			return { status: "hub_off", live: null, problem: null };
		if (gate.kind === "hub_checking") return LOADING;
		if (gate.kind === "hub_unreachable")
			return failed({
				...(gate.error ? { code: gate.error.code } : {}),
				retry: () => void gate.retry(),
			});
		if (list.rows === undefined)
			return list.error
				? failed({
						code: list.error.code,
						retry: () => void list.refetch(),
					})
				: LOADING;
		if (app.error) return failed({});
		if (!view) return LOADING;
		return {
			status: "ready",
			live: {
				view,
				rows: runsOnRows(view),
				names: runsOnDeviceNames(view),
				coverage,
				canDeploy,
				hub: list.freshness,
				runFix,
			},
			problem: null,
		};
	}, [gate, list, app.error, view, coverage, canDeploy, runFix]);
}

/** Reads the fleet for one app under the passive workspace and reports it to the provider above. */
function LiveProbe({
	appId,
	onReport,
}: Readonly<{ appId: string; onReport(report: LiveReport): void }>) {
	useDeviceViews({ watch: true });
	const report = useLiveReport(appId);
	useEffect(() => onReport(report), [report, onReport]);
	useEffect(() => () => onReport(LOADING), [onReport]);
	return null;
}

/** APP §7.2: the online vs offline comparison, opened from the strip and the popover. */
function ModeSheet({
	view,
	open,
	onOpenChange,
}: Readonly<{
	view: AppView;
	open: boolean;
	onOpenChange(open: boolean): void;
}>) {
	const { t } = useTranslation("devices");
	const copy = appCopy(t);
	const { name, mode } = view.app;
	return (
		<DvSheet
			open={open}
			onOpenChange={onOpenChange}
			wide
			title={copy.explainerTitle()}
			sub={copy.explainerSub(mode, name)}
		>
			<ModeExplainer app={name} mode={mode} />
		</DvSheet>
	);
}

/** The first cause that turns Run on a device… off for every row, if one holds. */
export function blockOf(
	report: Pick<LiveReport, "status" | "live">,
): EventsDevicesBlock | null {
	if (report.status === "loading") return null;
	if (report.status !== "ready") return report.status;
	return report.live?.canDeploy ? null : "no_target";
}

/** The states in which no workspace is created and no device read is sent, first cause first. */
function idleStatus(
	signedOut: boolean,
	hub: IHub | undefined,
	canReadBoards: boolean,
): EventsDevicesStatus | null {
	if (signedOut) return "signed_out";
	if (hub !== undefined && hub.standalone?.enabled !== true) return "hub_off";
	return canReadBoards ? null : "blind";
}

/** The page's events by id, each with the device's event rule. */
function useEventRules(events: readonly IEvent[]) {
	return useMemo(
		() => ({
			events: new Map(events.map((event) => [event.id, event])),
			eligibility: new Map(
				events.map((event) => [event.id, eventEligibility(event)]),
			),
		}),
		[events],
	);
}

/** Requests to open one event's popover (a deep link, the row menu): each opens exactly one. */
function usePopoverRequests() {
	const [focus, setFocus] = useState<EventsDevicesValue["focus"]>(null);
	const claimed = useRef(0);
	const claimFocus = useCallback((token: number) => {
		if (claimed.current === token) return false;
		claimed.current = token;
		return true;
	}, []);
	const showOnDevices = useCallback(
		(eventId: string) =>
			setFocus((previous) => ({
				eventId,
				token: (previous?.token ?? 0) + 1,
			})),
		[],
	);
	return useMemo(
		() => ({ focus, claimFocus, showOnDevices }),
		[focus, claimFocus, showOnDevices],
	);
}

/** Links and button navigation into the device area, through the page's router when it has one. */
function useDeviceLinks(
	appId: string,
	open: ((route: DevicesRoute, scope: DevicesScope) => void) | undefined,
) {
	return useMemo(() => {
		const scope: DevicesScope = { kind: "app", appId };
		return {
			scope,
			link: (route: DevicesRoute, target: DevicesScope = scope) => ({
				href: devicesHref(route, target),
				onClick: (event: MouseEvent<HTMLAnchorElement>) => {
					if (!open || !isPlainClick(event)) return;
					event.preventDefault();
					open(route, target);
				},
			}),
			go: (route: DevicesRoute, target: DevicesScope = scope) => {
				if (open) open(route, target);
				else globalThis.location?.assign(devicesHref(route, target));
			},
		};
	}, [appId, open]);
}

const NO_EVENTS: readonly IEvent[] = [];

/**
 * The same context for a cell inside the Devices area (App › Devices, list
 * mode), where there is no Events page around it: it reads the workspace the
 * area already has, takes the events from the backend and navigates through
 * the area's route. Wrap a whole list in it to read the fleet once.
 */
export function AreaEventsDevices({
	appId,
	children,
}: Readonly<{ appId: string; children: ReactNode }>) {
	const backend = useBackend();
	const { navigate } = useDevicesRoute();
	const report = useLiveReport(appId);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId],
		appId !== "",
	);
	const rules = useEventRules(events.data ?? NO_EVENTS);
	const [explaining, setExplaining] = useState(false);
	const open = useCallback(
		(route: DevicesRoute, scope: DevicesScope) => navigate(route, { scope }),
		[navigate],
	);
	const links = useDeviceLinks(appId, open);
	const value = useMemo<EventsDevicesValue>(
		() => ({
			...report,
			...rules,
			...links,
			appId,
			block: blockOf(report),
			focus: null,
			claimFocus: () => false,
			showOnDevices: () => undefined,
			explainMode: () => setExplaining(true),
		}),
		[report, rules, links, appId],
	);
	return (
		<EventsDevicesContext.Provider value={value}>
			{children}
			{report.live ? (
				<ModeSheet
					view={report.live.view}
					open={explaining}
					onOpenChange={setExplaining}
				/>
			) : null}
		</EventsDevicesContext.Provider>
	);
}

/**
 * Wraps the Events list. With devices on, a signed-in account and a role that
 * reads flows, it mounts the passive device workspace next to the list (so
 * Unlock… works from here) and shares what it reads through one context; in
 * every other case no workspace is created and no device read is sent.
 */
export function EventsDevicesProvider(
	props: Readonly<EventsDevicesProviderProps>,
) {
	const { appId, harness, onNavigate, onOpenEvent, renderTile } = props;
	const { signedOut, signIn } = useHostSignIn(harness?.overrides);
	const [reported, setReported] = useState<LiveReport>(LOADING);
	const [explaining, setExplaining] = useState(false);
	const idle = idleStatus(signedOut, props.hub, props.canReadBoards);
	const rules = useEventRules(props.events);
	const requests = usePopoverRequests();
	const open = useMemo(
		() =>
			onNavigate
				? (route: DevicesRoute, scope: DevicesScope) =>
						onNavigate(devicesHref(route, scope))
				: undefined,
		[onNavigate],
	);
	const links = useDeviceLinks(appId, open);
	const explainMode = useCallback(() => setExplaining(true), []);

	const value = useMemo<EventsDevicesValue>(() => {
		const report: LiveReport = idle
			? { status: idle, live: null, problem: null }
			: reported;
		return {
			...report,
			...rules,
			...requests,
			...links,
			appId,
			block: blockOf(report),
			explainMode,
			...(signIn ? { signIn } : {}),
			...(onOpenEvent ? { openEvent: onOpenEvent } : {}),
			...(renderTile ? { renderTile } : {}),
		};
	}, [
		idle,
		reported,
		rules,
		requests,
		links,
		appId,
		explainMode,
		signIn,
		onOpenEvent,
		renderTile,
	]);

	return (
		<EventsDevicesContext.Provider value={value}>
			{props.children}
			{value.live ? (
				<ModeSheet
					view={value.live.view}
					open={explaining}
					onOpenChange={setExplaining}
				/>
			) : null}
			{idle ? null : (
				<DeviceWorkspaceProvider passive overrides={harness?.overrides}>
					<LiveProbe appId={appId} onReport={setReported} />
					<AreaOverlays scope={links.scope} onNavigate={onNavigate} />
				</DeviceWorkspaceProvider>
			)}
		</EventsDevicesContext.Provider>
	);
}
