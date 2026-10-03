"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import type { DeploymentCatalog } from "../../../../lib/device-management/deployment";
import {
	type AppHubFacts,
	type AppInput,
	type AppMode,
	appMode,
} from "../../../../lib/device-management/model/app-plan";
import { versionLabel as versionLabelOf } from "../../../../lib/device-management/model/app-versions";
import {
	type DeployDraft,
	type DeployPlan,
	type DeployResult,
	type DeployTargetDraft,
	type PlanApp,
	type PlanCheck,
	type PlanFacts,
	checkPlan,
	draftWithoutSecrets,
	makePlan,
	resolvePlan,
	withBotTokens,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	DeployRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import { useBackend } from "../../../../state/backend-state";
import { type AppViewRead, useAppView } from "../workspace/use-app";
import { useAttentionState } from "../workspace/use-attention";
import {
	type DeployDevice,
	type UnresolvedOverride,
	buildDeployDevices,
	buildPlanFacts,
	mergeEventVariables,
	runsNewest,
	unresolvedOverrides,
	withSavedBotTokens,
} from "./deploy-facts";
import type { DeployDeviceCheck } from "./step-props";
import { exportTypesOf } from "./update-path";
import {
	type Configurations,
	type DefinitionsRead,
	useDeployRole,
	useDeviceConfigurations,
	useInstalledVariables,
	useKeepCatalog,
	useLiveDemand,
	useLocalTriggers,
	usePreviousSecrets,
	useSharedAccess,
} from "./use-deploy-reads";

/* The wizard's one draft (APP §3.1, §3.3): choices in `sessionStorage` without secrets, the plan and its checks derived from `deploy-plan.ts`. */

const STORAGE_PREFIX = "flow-like:devices:deploy";
const SAVED_VERSION = 1;

interface SavedDraft {
	v: typeof SAVED_VERSION;
	draft: DeployDraft;
	/** Index of the furthest step visited. */
	reached: number;
	/** The plan's run ended; the next change starts a new plan. */
	deployed: boolean;
	/** How that run ended. */
	outcome?: DeployResult["outcome"];
	/** A one-service update took its events from the service. */
	seeded: boolean;
	/** The user changed something: only then "Picked up where you left off" is true. */
	touched: boolean;
}

export interface DeployDraftState {
	/** The saved-progress key (APP §3.1). */
	key: string;
	loading: boolean;
	error?: Error;
	app: AppInput | undefined;
	appRead: AppViewRead;
	/** The app's own version text ("1.5.0"), when it has one. */
	versionLabel?: string;
	/** False when the viewer's role on the app lacks Read boards: nothing can be deployed. */
	canReadFlows: boolean;
	mode: AppMode | null;
	draft: DeployDraft;
	facts: PlanFacts;
	plan: DeployPlan;
	check: PlanCheck;
	/** Every device that isn't revoked, with card facts and its Where gate. */
	devices: DeployDevice[];
	revoked: number;
	/** Live-read configurations of the selected devices' services. */
	configurations: Configurations;
	/**
	 * Whether variable definitions outside the preparation are still being
	 * read, or why they couldn't be. `known`: the flows' own settings are in
	 * (a bot's token setting is there before them).
	 */
	definitions: Pick<DefinitionsRead, "loading" | "error"> & { known: boolean };
	/** Stored values of updated services that the update can't carry over as they are. */
	unresolved: UnresolvedOverride[];
	resumed: boolean;
	reached: number;
	deployed: boolean;
	/** How the plan's last run ended; undefined until one did. */
	outcome?: DeployResult["outcome"];
	update(patch: Partial<DeployDraft>): void;
	updateTarget(
		deviceId: string,
		change: (target: DeployTargetDraft) => DeployTargetDraft,
	): void;
	toggleDevice(deviceId: string, on: boolean): void;
	setReached(index: number): void;
	/** The definitions of the bundle step 2 prepared. */
	setCatalog(catalog: DeploymentCatalog | null): void;
	reportDeviceCheck(deviceId: string, check: DeployDeviceCheck): void;
	dismissResumed(): void;
	/** Forgets the saved progress and starts over from the entry. */
	discard(): void;
	markDeployed(outcome?: DeployResult["outcome"]): void;
}

/** `app|<appId>|<new|update>|<eventId or all>` or `dev|<deviceId>|<appId>|<new or serviceId>`. */
export function deployDraftKey(
	route: DeployRoute,
	scope: DevicesScope,
): string {
	const [deviceId = ""] = route.deviceIds;
	if (scope.kind !== "app")
		return `dev|${deviceId}|${route.appId ?? ""}|${route.serviceId ?? "new"}`;
	const mode = route.serviceId || route.mode === "update" ? "update" : "new";
	const base = `app|${scope.appId}|${mode}|${route.eventId ?? "all"}`;
	return route.serviceId ? `${base}|${deviceId}|${route.serviceId}` : base;
}

/** Where the draft of one entry is kept in `sessionStorage`, per account and hub. */
export function deployStorageKey(scopeKey: string, draftKey: string): string {
	return `${STORAGE_PREFIX}:${scopeKey}:${draftKey}`;
}

function readSaved(storageKey: string): SavedDraft | null {
	try {
		const raw = globalThis.sessionStorage?.getItem(storageKey);
		if (!raw) return null;
		const saved = JSON.parse(raw) as SavedDraft;
		return saved?.v === SAVED_VERSION && Array.isArray(saved.draft?.targets)
			? saved
			: null;
	} catch {
		return null;
	}
}

function writeSaved(storageKey: string, saved: SavedDraft | null): void {
	try {
		if (!saved) globalThis.sessionStorage?.removeItem(storageKey);
		else
			globalThis.sessionStorage?.setItem(
				storageKey,
				JSON.stringify({ ...saved, draft: draftWithoutSecrets(saved.draft) }),
			);
	} catch {
		// Saved progress is a convenience; the draft still lives in memory.
	}
}

const newTarget = (deviceId: string): DeployTargetDraft => ({
	deviceId,
	choices: {},
	serveBoth: [],
	over: {},
});

/** A saved draft also takes the devices the entry names now. */
function withRouteDevices(saved: SavedDraft, route: DeployRoute): SavedDraft {
	const known = new Set(saved.draft.targets.map((target) => target.deviceId));
	const added = [...new Set(route.deviceIds)]
		.filter((deviceId) => !known.has(deviceId))
		.map(newTarget);
	return added.length
		? {
				...saved,
				draft: { ...saved.draft, targets: [...saved.draft.targets, ...added] },
			}
		: saved;
}

interface EntryInput {
	route: DeployRoute;
	scope: DevicesScope;
	app: PlanApp | null;
	now: number;
	/** The one service this entry updates already runs what the app publishes now. */
	runsNewest: boolean;
	/** What the app's hub can do for its events, as far as it is known when the draft starts. */
	hub?: AppHubFacts;
}

function freshDraft({
	route,
	scope,
	app,
	now,
	runsNewest: upToDate,
	hub,
}: EntryInput): SavedDraft {
	const draft = makePlan({
		scope,
		route,
		app,
		deploymentId: globalThis.crypto.randomUUID(),
		now,
		...(hub ? { hub } : {}),
	});
	// "Change settings…" and an update of a service that is already newest keep the version: nothing is uploaded.
	const keep =
		draft.entry === "update" && (route.step === "settings" || upToDate);
	return {
		v: SAVED_VERSION,
		draft: keep ? { ...draft, version: "keep" } : draft,
		reached: 0,
		deployed: false,
		seeded: false,
		touched: false,
	};
}

function loadDraft(
	storageKey: string,
	entry: EntryInput,
): { saved: SavedDraft; resumed: boolean } {
	const stored = readSaved(storageKey);
	return stored && stored.draft.appId === (entry.app?.id ?? null)
		? {
				saved: withRouteDevices(stored, entry.route),
				resumed: stored.touched === true,
			}
		: { saved: freshDraft(entry), resumed: false };
}

function appServicesOn(device: DeployDevice | undefined, appId: string | null) {
	return (device?.services ?? []).filter(
		(service) => service.projectId === appId,
	);
}

/** The target with its device's own service of the app as the one to update; unchanged when it has a choice or none is readable. */
function withUpdateChoice(
	target: DeployTargetDraft,
	devices: readonly DeployDevice[],
	appId: string | null,
): DeployTargetDraft {
	if (target.choices.main) return target;
	const [service] = appServicesOn(
		devices.find((device) => device.id === target.deviceId),
		appId,
	);
	if (!service) return target;
	const main = { kind: "update" as const, serviceId: service.serviceId };
	return { ...target, choices: { ...target.choices, main } };
}

/** Update entries without `service=`: each device updates its own service of the app once that is readable. */
function missingUpdateChoices(
	draft: DeployDraft,
	devices: readonly DeployDevice[],
): DeployTargetDraft[] | null {
	if (draft.entry !== "update" || !draft.keepEvents) return null;
	const targets = draft.targets.map((target) =>
		withUpdateChoice(target, devices, draft.appId),
	);
	const changed = targets.some(
		(target, index) => target !== draft.targets[index],
	);
	return changed ? targets : null;
}

/** A one-service update plans that service's own events once the device says which they are. */
function seededEvents(
	saved: SavedDraft,
	route: DeployRoute,
	devices: readonly DeployDevice[],
): string[] | null {
	const { draft } = saved;
	if (saved.seeded || draft.entry !== "update" || !route.serviceId) return null;
	if (route.eventId) return null;
	const [target] = draft.targets;
	const service = appServicesOn(
		devices.find((device) => device.id === target?.deviceId),
		draft.appId,
	).find((row) => row.serviceId === route.serviceId);
	return service?.events?.map((event) => event.event_id) ?? null;
}

interface DraftStore {
	saved: SavedDraft;
	resumed: boolean;
	change(next: (saved: SavedDraft) => SavedDraft): void;
	dismissResumed(): void;
	discard(): void;
}

interface StoreEntry {
	key: string;
	saved: SavedDraft;
	resumed: boolean;
}

/** The draft's state for one entry: loaded once the app is known, saved on every change. */
function useDraftStore(
	storageKey: string,
	ready: boolean,
	entry: EntryInput,
): DraftStore {
	const entryRef = useRef(entry);
	entryRef.current = entry;
	// The entry is read once per key, when the app is known.
	const initial = useMemo<StoreEntry | null>(
		() =>
			ready
				? { key: storageKey, ...loadDraft(storageKey, entryRef.current) }
				: null,
		[storageKey, ready],
	);
	const fallback = useMemo(
		() => freshDraft({ ...entryRef.current, app: null }),
		[],
	);
	const [edit, setEdit] = useState<StoreEntry | null>(null);
	const current = edit?.key === storageKey ? edit : initial;
	const currentRef = useRef(current);
	currentRef.current = current;

	const saved = current?.saved;
	useEffect(() => {
		if (saved) writeSaved(storageKey, saved);
	}, [storageKey, saved]);

	const mutate = useCallback(
		(next: (entry: StoreEntry) => StoreEntry) =>
			setEdit((previous) => {
				const base =
					previous?.key === storageKey ? previous : currentRef.current;
				return base ? next(base) : previous;
			}),
		[storageKey],
	);
	const change = useCallback(
		(next: (saved: SavedDraft) => SavedDraft) =>
			mutate((base) => ({ ...base, saved: next(base.saved) })),
		[mutate],
	);
	const dismissResumed = useCallback(
		() => mutate((base) => ({ ...base, resumed: false })),
		[mutate],
	);
	const discard = useCallback(() => {
		writeSaved(storageKey, null);
		setEdit({
			key: storageKey,
			saved: freshDraft(entryRef.current),
			resumed: false,
		});
	}, [storageKey]);

	return {
		saved: saved ?? fallback,
		resumed: current?.resumed ?? false,
		change,
		dismissResumed,
		discard,
	};
}

/** Changing a deployed plan starts a new one from the same choices (APP §3.13). */
function patched(saved: SavedDraft, patch: Partial<DeployDraft>): SavedDraft {
	const { outcome: _ended, ...rest } = saved;
	return {
		...rest,
		deployed: false,
		touched: true,
		draft: {
			...saved.draft,
			...patch,
			...(saved.deployed
				? { deploymentId: globalThis.crypto.randomUUID() }
				: {}),
		},
	};
}

function toggled(
	draft: DeployDraft,
	deviceId: string,
	on: boolean,
): DeployTargetDraft[] {
	const rest = draft.targets.filter((target) => target.deviceId !== deviceId);
	if (!on) return rest;
	return rest.length === draft.targets.length
		? [...draft.targets, newTarget(deviceId)]
		: draft.targets;
}

interface Keyed<T> {
	key: string;
	value: T;
}

/** State that belongs to one draft: it reads as `initial` again when the entry changes. */
function useKeyed<T>(key: string, initial: T) {
	const [state, setState] = useState<Keyed<T>>({ key, value: initial });
	const value = state.key === key ? state.value : initial;
	const set = useCallback(
		(next: (value: T) => T) =>
			setState((previous) => {
				const base = previous.key === key ? previous.value : initial;
				const changed = next(base);
				return changed === base && previous.key === key
					? previous
					: { key, value: changed };
			}),
		[key, initial],
	);
	return [value, set] as const;
}

const NO_CHECKS: Record<string, DeployDeviceCheck> = {};

/** The draft's writers, each one a patch on the newest saved state. */
function useDraftActions(change: DraftStore["change"]) {
	const update = useCallback(
		(patch: Partial<DeployDraft>) => change((value) => patched(value, patch)),
		[change],
	);
	const updateTarget = useCallback(
		(
			deviceId: string,
			next: (target: DeployTargetDraft) => DeployTargetDraft,
		) =>
			change((value) =>
				patched(value, {
					targets: value.draft.targets.map((target) =>
						target.deviceId === deviceId ? next(target) : target,
					),
				}),
			),
		[change],
	);
	const toggleDevice = useCallback(
		(deviceId: string, on: boolean) =>
			change((value) =>
				patched(value, { targets: toggled(value.draft, deviceId, on) }),
			),
		[change],
	);
	const setReached = useCallback(
		(index: number) =>
			change((value) =>
				index > value.reached
					? { ...value, reached: index, touched: true }
					: value,
			),
		[change],
	);
	const markDeployed = useCallback(
		(outcome?: DeployResult["outcome"]) =>
			change((value) => ({
				...value,
				deployed: true,
				...(outcome ? { outcome } : {}),
			})),
		[change],
	);
	return { update, updateTarget, toggleDevice, setReached, markDeployed };
}

export function useDeployDraft(
	route: DeployRoute,
	scope: DevicesScope,
): DeployDraftState {
	const backend = useBackend();
	const { workspace, input, tokenScopeAll, items } = useAttentionState();
	const appId = route.appId ?? (scope.kind === "app" ? scope.appId : undefined);
	const appRead = useAppView(appId);
	const role = useDeployRole(appId);
	const appRecord = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[appId ?? ""],
		!!appId,
	);
	const key = deployDraftKey(route, scope);
	const storageKey = deployStorageKey(workspace.scopeKey, key);
	const { app } = appRead;
	const baseApp = useMemo<PlanApp | null>(
		() =>
			app
				? {
						id: app.id,
						name: app.name,
						visibility: app.visibility,
						events: app.events,
					}
				: null,
		[app],
	);
	const servedNow = appRead.view?.services.find(
		(row) =>
			row.serviceId === route.serviceId && row.deviceId === route.deviceIds[0],
	)?.events;
	const upToDate =
		!!route.serviceId &&
		!!app &&
		appMode(app.visibility) === "online" &&
		runsNewest(app.events, servedNow);
	// What the hub can do for the app's events, and whether this person may create a flow version.
	const noFlowEdits = role.canEditFlows === false;
	const hub = useMemo<AppHubFacts>(
		() => (noFlowEdits ? { ...appRead.hub, canEditFlows: false } : appRead.hub),
		[appRead.hub, noFlowEdits],
	);
	const store = useDraftStore(storageKey, !appId || !!app, {
		route,
		scope,
		app: baseApp,
		now: input.now,
		runsNewest: upToDate,
		hub,
	});
	const { saved, change } = store;
	const { draft } = saved;
	const mode = baseApp ? appMode(baseApp.visibility) : null;
	const updating = draft.entry === "update";

	const [catalog, setCatalogState] = useKeyed<DeploymentCatalog | null>(
		storageKey,
		null,
	);
	const [checks, setChecks] = useKeyed(storageKey, NO_CHECKS);

	const selected = useMemo(
		() => draft.targets.map((target) => target.deviceId),
		[draft.targets],
	);
	const gateInput = useSharedAccess(workspace, input);
	const { devices, revoked } = useMemo(
		() =>
			buildDeployDevices(
				{ workspace, input: gateInput, tokenScopeAll },
				items,
				{
					appId: draft.appId,
					mode,
					update: updating,
					...(route.serviceId ? { serviceId: route.serviceId } : {}),
				},
			),
		[
			workspace,
			gateInput,
			tokenScopeAll,
			items,
			draft.appId,
			updating,
			mode,
			route.serviceId,
		],
	);
	useLiveDemand(workspace, devices, selected);
	const configurations = useDeviceConfigurations(workspace, devices, selected);
	// The export names the types of the chosen events and of those the picked devices' services of the app keep.
	const keepTypes = useMemo(() => {
		const kept = devices
			.filter((device) => selected.includes(device.id))
			.flatMap((device) => appServicesOn(device, draft.appId))
			.flatMap((service) => (service.events ?? []).map((row) => row.event_id));
		return exportTypesOf(baseApp, [...draft.events, ...kept], hub.hubTypes);
	}, [devices, selected, draft.appId, draft.events, baseApp, hub.hubTypes]);
	const keepCatalog = useKeepCatalog(
		workspace,
		draft.appId,
		mode === "online" && draft.version === "keep",
		keepTypes,
	);
	const installed = useInstalledVariables(
		workspace,
		mode === "offline" && updating,
		draft.appId,
		configurations,
	);
	const approved = catalog ?? keepCatalog.catalog ?? null;
	const previousOnline = usePreviousSecrets(
		draft.appId,
		updating ? approved : null,
		configurations,
	);
	const previous = mode === "offline" ? installed.byService : previousOnline;

	const merged = useMemo(
		() => mergeEventVariables(installed.variables, checks, approved?.variables),
		[installed.variables, checks, approved],
	);
	const planApp = useMemo<PlanApp | null>(() => {
		if (!baseApp) return null;
		// Every bot brings its token as a setting of its own, before the flows' own settings are known; a flow variable with a token key's id never is one.
		const variables = withBotTokens(baseApp.events, merged);
		return variables ? { ...baseApp, variables } : baseApp;
	}, [baseApp, merged]);
	const definitionsError = keepCatalog.error ?? installed.error;
	const known = merged !== undefined;
	const definitions = useMemo(
		() => ({
			loading: keepCatalog.loading || installed.loading,
			known,
			...(definitionsError ? { error: definitionsError } : {}),
		}),
		[keepCatalog.loading, installed.loading, known, definitionsError],
	);

	// A role without the Owner permission (an Admin included) can't approve the app's files; with it the hub still decides.
	const notOwner = role.isOwner === false;
	const { platform } = workspace.deps;
	const localTriggers = useLocalTriggers(baseApp, platform);
	const schedules = appRead.view?.schedules;
	const { canEditEvents } = role;
	const facts = useMemo(
		() =>
			buildPlanFacts({
				app: planApp,
				devices,
				configurations,
				checks,
				platform,
				now: input.now,
				...(notOwner ? { isAppOwner: false } : {}),
				hub,
				...(schedules === undefined ? {} : { schedules }),
				...(canEditEvents === undefined ? {} : { canEditEvents }),
				localTriggers,
			}),
		[
			planApp,
			devices,
			configurations,
			checks,
			platform,
			input.now,
			notOwner,
			hub,
			schedules,
			canEditEvents,
			localTriggers,
		],
	);
	const plan = useMemo(
		() => resolvePlan(withSavedBotTokens(draft, facts), facts),
		[draft, facts],
	);
	const check = useMemo(() => checkPlan(plan, facts), [plan, facts]);
	const unresolved = useMemo(
		() => unresolvedOverrides(plan, { configurations, previous }),
		[plan, configurations, previous],
	);

	const actions = useDraftActions(change);
	const setCatalog = useCallback(
		(next: DeploymentCatalog | null) => setCatalogState(() => next),
		[setCatalogState],
	);
	const reportDeviceCheck = useCallback(
		(deviceId: string, value: DeployDeviceCheck) =>
			setChecks((known) => ({ ...known, [deviceId]: value })),
		[setChecks],
	);

	useEffect(() => {
		const targets = missingUpdateChoices(draft, devices);
		if (targets)
			change((value) => ({ ...value, draft: { ...value.draft, targets } }));
	}, [draft, devices, change]);
	useEffect(() => {
		const events = seededEvents(saved, route, devices);
		if (events)
			change((value) => ({
				...value,
				seeded: true,
				draft: { ...value.draft, events },
			}));
	}, [saved, route, devices, change]);

	const error = appRead.error;
	const versionLabel = versionLabelOf(appRecord.data?.version) ?? undefined;
	return {
		key,
		loading: !!appId && !app && !error,
		...(error ? { error } : {}),
		app,
		appRead,
		...(versionLabel ? { versionLabel } : {}),
		canReadFlows: role.canReadFlows,
		mode,
		draft,
		facts,
		plan,
		check,
		devices,
		revoked,
		configurations,
		definitions,
		unresolved,
		resumed: store.resumed,
		reached: saved.reached,
		deployed: saved.deployed,
		...(saved.outcome ? { outcome: saved.outcome } : {}),
		...actions,
		setCatalog,
		reportDeviceCheck,
		dismissResumed: store.dismissResumed,
		discard: store.discard,
	};
}
