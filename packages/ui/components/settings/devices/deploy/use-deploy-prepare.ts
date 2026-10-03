"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type ProjectArtifactAssets,
	parseProjectArtifactAssets,
	prepareProjectArtifact,
	projectFilesFromSelection,
	selectedProjectAssetFiles,
} from "../../../../lib/device-management/artifacts";
import {
	type LatestEvent,
	LatestFlowError,
	type LatestFlows,
	type LatestProblem,
	type PreparedFlow,
	type PublishFlowOptions,
	latestPinsOffline,
	latestPinsOnline,
	publishLatestFlows,
} from "../../../../lib/device-management/latest-flows";
import {
	type DeployPlan,
	planLatestFlows,
} from "../../../../lib/device-management/model/deploy-plan";
import { prepareOnlineDependencies } from "../../../../lib/device-management/online-dependencies";
import {
	type ApprovedOnlineMetadata,
	prepareOnlineMetadata,
} from "../../../../lib/device-management/online-metadata";
import {
	desktopExportCommands,
	prepareDesktopProject,
} from "../../../../lib/device-management/project-export";
import {
	type IBackendState,
	useBackend,
} from "../../../../state/backend-state";
import type { IProfile } from "../../../../types";
import {
	useDeviceAuth,
	useDeviceWorkspace,
} from "../workspace/device-workspace-provider";
import { useLatestFlows } from "../workspace/use-latest-flows";
import type { DeployPrepared } from "./step-props";
import { planExportTypes } from "./update-path";

/*
 * Step 2 "Preparing" (APP §3.6): one bundle per choice of app, version and
 * events, prepared on this computer and handed to every later step.
 */

/** First of all: every flow a chosen Latest event follows becomes a version when no published one equals it. */
const PUBLISH_FLOWS = "publish_flows";

export const ONLINE_CHECKS = [
	PUBLISH_FLOWS,
	"read_hub",
	"check_events",
	"collect",
	"approved",
] as const;
export const OFFLINE_CHECKS = [
	PUBLISH_FLOWS,
	"read_app",
	"no_secrets",
	"no_history",
	"no_writers",
	"limits",
] as const;
export type PrepareCheckId =
	| (typeof ONLINE_CHECKS)[number]
	| (typeof OFFLINE_CHECKS)[number];

export interface PrepareCheck {
	id: PrepareCheckId;
	/** `skip` on `publish_flows`: no chosen event follows Latest. */
	state: "pending" | "active" | "pass" | "fail" | "skip";
	/** `publish_flows`, once it passed: what was done per flow. */
	flows?: readonly PreparedFlow[];
}

export interface PrepareFailure {
	check: PrepareCheckId;
	/**
	 * `event`: a chosen event isn't in the approved definitions; `flow`: an
	 * event that follows Latest can't be shipped; `error`: what the hub or this
	 * computer answered.
	 */
	kind: "event" | "flow" | "error";
	eventId?: string;
	detail: string;
	/** `flow`: why. Only `moved` is cured by preparing again. */
	flow?: LatestProblem;
}

/** A copy prepared elsewhere (advanced, offline, desktop): the app folder, the pins file's text and the assets folder. */
export interface ImportedCopy {
	files: File[];
	pins: string;
	assets: File[];
}

export interface DeployPrepareState {
	state: "idle" | "running" | "ready" | "blocked";
	checks: PrepareCheck[];
	prepared: DeployPrepared | null;
	failure: PrepareFailure | null;
	imported: boolean;
	again(): void;
	importCopy(copy: ImportedCopy | null): void;
}

/** Test seams: the native export is only reachable inside the desktop app, and a busy flow is retried after a pause. */
export const deployPrepareSeams: {
	exportCommands: typeof desktopExportCommands;
	publish: PublishFlowOptions;
} = {
	exportCommands: desktopExportCommands,
	publish: {},
};

interface PrepareContext {
	appId: string;
	events: readonly string[];
	/** The plan's events that follow Latest, each with its flow. */
	latest: readonly LatestEvent[];
	/** The event types the hub exports only when asked (`exportTypes`). */
	types: readonly string[];
	/** Where a flow is read and published as a version; null where nothing can. */
	flows: LatestFlows | null;
	backend: IBackendState;
	profile: IProfile;
	desktop: boolean;
	account: string;
	signal: AbortSignal;
	/** Called as each check starts. */
	enter(check: PrepareCheckId): void;
	/** Called once `publish_flows` is over. */
	published(flows: PreparedFlow[]): void;
}

interface PrepareResult {
	prepared: DeployPrepared;
	release?(): Promise<void>;
}

class PrepareBlocked extends Error {
	constructor(readonly failure: PrepareFailure) {
		super(failure.detail);
	}
}

/** The hub's own sentence when it sent one: its error's `message` also carries the code and a reference. */
const errorText = (error: unknown) => {
	const server = (error as { serverMessage?: unknown } | null)?.serverMessage;
	if (typeof server === "string" && server.trim()) return server;
	return error instanceof Error ? error.message : String(error);
};

const OFFLINE_REFUSALS: readonly [RegExp, PrepareCheckId][] = [
	[/secret/i, "no_secrets"],
	[/histor|index|version/i, "no_history"],
	[/writ|lock|in use|busy/i, "no_writers"],
	[/exceed|limit|too (many|large)/i, "limits"],
];

/** Which offline check the export's refusal belongs to; the first one when it doesn't say. */
function offlineCheckOf(message: string): PrepareCheckId {
	return (
		OFFLINE_REFUSALS.find(([pattern]) => pattern.test(message))?.[1] ??
		"read_app"
	);
}

function assertEventsApproved(
	approved: ApprovedOnlineMetadata,
	events: readonly string[],
) {
	for (const eventId of events) {
		const event = approved.catalog.events.find((row) => row.id === eventId);
		if (event?.eligible) continue;
		throw new PrepareBlocked({
			check: "check_events",
			kind: "event",
			eventId,
			detail: event?.ineligible_reason ?? "",
		});
	}
}

/** A Latest event that can't be shipped stops the check it was found in, with its own cause. */
async function latestChecked<T>(
	check: PrepareCheckId,
	run: () => Promise<T> | T,
): Promise<T> {
	try {
		return await run();
	} catch (error) {
		if (!(error instanceof LatestFlowError)) throw error;
		throw new PrepareBlocked({
			check,
			kind: "flow",
			flow: error.problem,
			...(error.at.eventId ? { eventId: error.at.eventId } : {}),
			detail: error.detail,
		});
	}
}

/** Publish-if-changed for the flows of the plan's Latest events; nothing when there are none. */
async function publishFlows(context: PrepareContext): Promise<PreparedFlow[]> {
	const { latest, flows } = context;
	if (!latest.length || !flows) return [];
	context.enter(PUBLISH_FLOWS);
	const done = await latestChecked(PUBLISH_FLOWS, () =>
		publishLatestFlows(flows, context.appId, latest, {
			signal: context.signal,
			...deployPrepareSeams.publish,
		}),
	);
	context.published(done);
	return done;
}

/** The parts of the bundle that come from its Latest events: what was published, and each event's pin. */
function latestFacts(
	flows: readonly PreparedFlow[],
	pins: Readonly<Record<string, [number, number, number]>>,
): Pick<DeployPrepared, "flows" | "latest"> {
	return {
		...(flows.length ? { flows } : {}),
		...(Object.keys(pins).length ? { latest: pins } : {}),
	};
}

async function prepareOnline(context: PrepareContext): Promise<PrepareResult> {
	const { appId, backend, profile, signal, latest } = context;
	const flows = await publishFlows(context);
	context.enter("read_hub");
	const approved = await prepareOnlineMetadata(
		appId,
		backend,
		profile,
		signal,
		latest.map((row) => row.eventId),
		context.types,
	);
	context.enter("check_events");
	const hub = context.flows;
	const pinOf = (eventId: string) =>
		approved.catalog.events.find((row) => row.id === eventId)?.board_version;
	const pins = hub
		? await latestChecked("check_events", () =>
				latestPinsOnline(hub, appId, latest, pinOf),
			)
		: {};
	assertEventsApproved(approved, context.events);
	context.enter("collect");
	const facts = latestFacts(flows, pins);
	const needsNative =
		context.desktop &&
		(approved.app.bits.length > 0 ||
			Object.keys(approved.app.packages ?? {}).length > 0);
	if (!needsNative) {
		const { artifact } = await prepareOnlineDependencies(
			approved.app,
			backend,
			profile,
			signal,
			approved,
		);
		context.enter("approved");
		return {
			prepared: { artifact, approved, preparedAt: Date.now(), ...facts },
		};
	}
	const exported = await prepareDesktopProject(
		appId,
		await deployPrepareSeams.exportCommands(approved.app),
		signal,
		approved,
	);
	context.enter("approved");
	return {
		prepared: {
			artifact: exported.artifact,
			approved,
			preparedAt: Date.now(),
			...facts,
		},
		release: exported.release,
	};
}

async function prepareOffline(context: PrepareContext): Promise<PrepareResult> {
	const flows = await publishFlows(context);
	context.enter("read_app");
	const exported = await prepareDesktopProject(
		context.appId,
		await deployPrepareSeams.exportCommands(undefined, context.account),
		context.signal,
	);
	try {
		const pins = await latestChecked("read_app", () =>
			latestPinsOffline(context.latest, exported.latestEvents),
		);
		return {
			prepared: {
				artifact: exported.artifact,
				preparedAt: Date.now(),
				...latestFacts(flows, pins),
			},
			release: exported.release,
		};
	} catch (error) {
		await exported.release().catch(() => undefined);
		throw error;
	}
}

async function prepareImported(
	context: PrepareContext,
	copy: ImportedCopy,
): Promise<PrepareResult> {
	context.enter("read_app");
	const assets: ProjectArtifactAssets = copy.pins.trim()
		? parseProjectArtifactAssets(copy.pins)
		: { bit_pins: [], package_pins: [] };
	const pinned =
		assets.bit_pins.length || assets.package_pins.length
			? await selectedProjectAssetFiles(copy.assets, assets, context.signal)
			: [];
	const artifact = await prepareProjectArtifact(
		context.appId,
		[...projectFilesFromSelection(context.appId, copy.files), ...pinned],
		context.signal,
		assets,
	);
	return { prepared: { artifact, preparedAt: Date.now() } };
}

function failureOf(
	error: unknown,
	active: PrepareCheckId,
	offline: boolean,
): PrepareFailure {
	if (error instanceof PrepareBlocked) return error.failure;
	const detail = errorText(error);
	return {
		check: offline ? offlineCheckOf(detail) : active,
		kind: "error",
		detail,
	};
}

/** A check's state from its place against the one that runs or failed: before it passed, after it wasn't reached. */
function relativeState(
	index: number,
	at: number,
	here: PrepareCheck["state"],
	after: PrepareCheck["state"],
): PrepareCheck["state"] {
	if (index < at) return "pass";
	return index === at ? here : after;
}

function checksOf(
	ids: readonly PrepareCheckId[],
	run: Pick<RunState, "status" | "active" | "failure" | "flows">,
): PrepareCheck[] {
	const failed = run.failure ? ids.indexOf(run.failure.check) : -1;
	const active = run.active ? ids.indexOf(run.active) : -1;
	const states: Record<
		RunState["status"],
		(index: number) => PrepareCheck["state"]
	> = {
		idle: () => "pending",
		ready: () => "pass",
		blocked: (index) => relativeState(index, failed, "fail", "skip"),
		running: (index) => relativeState(index, active, "active", "pending"),
	};
	return ids.map((id, index) => {
		const state = states[run.status](index);
		if (id !== PUBLISH_FLOWS) return { id, state };
		// Passed without a flow to publish: nothing was checked, and the list says so.
		if (state === "pass" && !run.flows?.length) return { id, state: "skip" };
		return { id, state, ...(run.flows?.length ? { flows: run.flows } : {}) };
	});
}

interface RunState {
	key: string;
	status: "idle" | "running" | "ready" | "blocked";
	active: PrepareCheckId | null;
	failure: PrepareFailure | null;
	prepared: DeployPrepared | null;
	/** What `publish_flows` did, once it is over. */
	flows: readonly PreparedFlow[] | null;
}

const IDLE: Omit<RunState, "key"> = {
	status: "idle",
	active: null,
	failure: null,
	prepared: null,
	flows: null,
};

const NO_LATEST: readonly LatestEvent[] = [];

/**
 * What to prepare: the app, its mode, the chosen events and the types the
 * export is asked for; nothing for `version: "keep"` or a local-only app on
 * web. `hubTypes` is the hub's `event_types` (undefined while unknown).
 */
function prepareTarget(
	plan: DeployPlan | null,
	enabled: boolean,
	desktop: boolean,
	hubTypes: readonly string[] | undefined,
) {
	if (!plan?.app)
		return {
			appId: "",
			offline: false,
			eventKey: "",
			typeKey: "",
			latest: NO_LATEST,
			wanted: false,
		};
	const offline = plan.mode === "offline";
	const events = plan.services.flatMap((service) => service.events);
	const eventKey = [...new Set(events)].sort().join(",");
	const skipped = plan.draft.version === "keep" || (offline && !desktop);
	return {
		appId: plan.app.id,
		offline,
		eventKey,
		typeKey: offline ? "" : planExportTypes(plan, hubTypes).join(","),
		latest: planLatestFlows(plan),
		wanted: enabled && !skipped && eventKey !== "",
	};
}

interface PrepareJob {
	appId: string;
	events: readonly string[];
	latest: readonly LatestEvent[];
	types: readonly string[];
	flows: LatestFlows | null;
	offline: boolean;
	desktop: boolean;
	backend: IBackendState;
	profile: IProfile;
	account: string;
	imported: ImportedCopy | null;
}

/** Runs one preparation and reports each state; the returned cleanup cancels it and gives the native snapshot back. */
function startPrepare(
	job: PrepareJob,
	report: (next: Omit<RunState, "key">) => void,
): () => void {
	const controller = new AbortController();
	let alive = true;
	let release: PrepareResult["release"];
	let active: PrepareCheckId = job.offline ? "read_app" : "read_hub";
	let flows: RunState["flows"] = null;
	const apply = (next: Omit<RunState, "key" | "flows">) => {
		if (alive) report({ ...next, flows });
	};
	const context: PrepareContext = {
		...job,
		signal: controller.signal,
		enter(check) {
			active = check;
			apply({ ...IDLE, status: "running", active: check });
		},
		published(done) {
			flows = done;
		},
	};
	const run = job.imported
		? prepareImported(context, job.imported)
		: job.offline
			? prepareOffline(context)
			: prepareOnline(context);
	run.then(
		(result) => {
			if (!alive) {
				result.release?.().catch(() => undefined);
				return;
			}
			release = result.release;
			apply({ ...IDLE, status: "ready", prepared: result.prepared });
		},
		(error) =>
			apply({
				...IDLE,
				status: "blocked",
				failure: failureOf(error, active, job.offline),
			}),
	);
	return () => {
		alive = false;
		controller.abort();
		release?.().catch(() => undefined);
	};
}

/**
 * Prepares the bundle for the plan's app, version and events once `enabled`
 * (step 2 or later), again whenever that choice changes or `again()` is
 * called. Skipped for `version: "keep"` and for a local-only app on web.
 * `hubTypes` (the hub's `event_types`) decides which types the export names.
 */
export function useDeployPrepare(
	plan: DeployPlan | null,
	options: { enabled?: boolean; hubTypes?: readonly string[] } = {},
): DeployPrepareState {
	const backend = useBackend();
	const workspace = useDeviceWorkspace();
	const { account } = useDeviceAuth();
	const desktop = workspace.deps.platform === "desktop";
	const { profile } = workspace.deps;
	const [attempt, setAttempt] = useState(0);
	const [imported, setImported] = useState<ImportedCopy | null>(null);
	const [run, setRun] = useState<RunState>({ key: "", ...IDLE });

	const target = prepareTarget(
		plan,
		options.enabled !== false,
		desktop,
		options.hubTypes,
	);
	const { appId, offline, eventKey, typeKey } = target;
	const flows = useLatestFlows(plan?.app ? (plan.mode ?? null) : null);
	// The names of the Latest events and the exported types are part of the choice: another set is another bundle.
	const latestKey = target.latest
		.map((row) => `${row.eventId}@${row.boardId}`)
		.join(",");
	const key = target.wanted
		? `${appId}|${offline ? "offline" : "online"}|${eventKey}|${latestKey}|${typeKey}|${attempt}|${imported ? "import" : "app"}`
		: "";
	const newest = useRef({
		backend,
		profile,
		account,
		imported,
		flows,
		latest: target.latest,
	});
	newest.current = {
		backend,
		profile,
		account,
		imported,
		flows,
		latest: target.latest,
	};

	useEffect(() => {
		if (!key) return;
		return startPrepare(
			{
				appId,
				events: eventKey.split(","),
				types: typeKey ? typeKey.split(",") : [],
				offline,
				desktop,
				...newest.current,
			},
			(next) => setRun({ key, ...next }),
		);
	}, [key, appId, eventKey, typeKey, offline, desktop]);

	const current = run.key === key && key ? run : { key, ...IDLE };
	const again = useCallback(() => setAttempt((value) => value + 1), []);
	const importCopy = useCallback((copy: ImportedCopy | null) => {
		setImported(copy);
		setAttempt((value) => value + 1);
	}, []);
	const ids = offline ? OFFLINE_CHECKS : ONLINE_CHECKS;
	const { status, active, failure, prepared } = current;
	const published = current.flows;
	return useMemo(
		() => ({
			state: status,
			checks: checksOf(ids, { status, active, failure, flows: published }),
			prepared,
			failure,
			imported: imported !== null,
			again,
			importCopy,
		}),
		[
			status,
			active,
			failure,
			published,
			prepared,
			ids,
			imported,
			again,
			importCopy,
		],
	);
}
