"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type ProjectArtifactAssets,
	parseProjectArtifactAssets,
	prepareProjectArtifact,
	projectFilesFromSelection,
	selectedProjectAssetFiles,
} from "../../../../lib/device-management/artifacts";
import type { DeployPlan } from "../../../../lib/device-management/model/deploy-plan";
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
import type { DeployPrepared } from "./step-props";

/*
 * Step 2 "Preparing" (APP §3.6): one bundle per choice of app, version and
 * events, prepared on this computer and handed to every later step.
 */

export const ONLINE_CHECKS = [
	"read_hub",
	"check_events",
	"collect",
	"approved",
] as const;
export const OFFLINE_CHECKS = [
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
	state: "pending" | "active" | "pass" | "fail" | "skip";
}

export interface PrepareFailure {
	check: PrepareCheckId;
	/** `event`: a chosen event isn't in the approved definitions; `error`: what the hub or this computer answered. */
	kind: "event" | "error";
	eventId?: string;
	detail: string;
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

/** Test seam: the native export is only reachable inside the desktop app. */
export const deployPrepareSeams = {
	exportCommands: desktopExportCommands,
};

interface PrepareContext {
	appId: string;
	events: readonly string[];
	backend: IBackendState;
	profile: IProfile;
	desktop: boolean;
	account: string;
	signal: AbortSignal;
	/** Called as each check starts. */
	enter(check: PrepareCheckId): void;
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

async function prepareOnline(context: PrepareContext): Promise<PrepareResult> {
	const { appId, backend, profile, signal } = context;
	context.enter("read_hub");
	const approved = await prepareOnlineMetadata(appId, backend, profile, signal);
	context.enter("check_events");
	assertEventsApproved(approved, context.events);
	context.enter("collect");
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
		return { prepared: { artifact, approved, preparedAt: Date.now() } };
	}
	const exported = await prepareDesktopProject(
		appId,
		await deployPrepareSeams.exportCommands(approved.app),
		signal,
		approved,
	);
	context.enter("approved");
	return {
		prepared: { artifact: exported.artifact, approved, preparedAt: Date.now() },
		release: exported.release,
	};
}

async function prepareOffline(context: PrepareContext): Promise<PrepareResult> {
	context.enter("read_app");
	const exported = await prepareDesktopProject(
		context.appId,
		await deployPrepareSeams.exportCommands(undefined, context.account),
		context.signal,
	);
	return {
		prepared: { artifact: exported.artifact, preparedAt: Date.now() },
		release: exported.release,
	};
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
	run: Pick<RunState, "status" | "active" | "failure">,
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
	return ids.map((id, index) => ({ id, state: states[run.status](index) }));
}

interface RunState {
	key: string;
	status: "idle" | "running" | "ready" | "blocked";
	active: PrepareCheckId | null;
	failure: PrepareFailure | null;
	prepared: DeployPrepared | null;
}

const IDLE: Omit<RunState, "key"> = {
	status: "idle",
	active: null,
	failure: null,
	prepared: null,
};

/** What to prepare: the app, its mode and the chosen events; nothing for `version: "keep"` or a local-only app on web. */
function prepareTarget(
	plan: DeployPlan | null,
	enabled: boolean,
	desktop: boolean,
) {
	if (!plan?.app)
		return { appId: "", offline: false, eventKey: "", wanted: false };
	const offline = plan.mode === "offline";
	const events = plan.services.flatMap((service) => service.events);
	const eventKey = [...new Set(events)].sort().join(",");
	const skipped = plan.draft.version === "keep" || (offline && !desktop);
	return {
		appId: plan.app.id,
		offline,
		eventKey,
		wanted: enabled && !skipped && eventKey !== "",
	};
}

interface PrepareJob {
	appId: string;
	events: readonly string[];
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
	const apply = (next: Omit<RunState, "key">) => {
		if (alive) report(next);
	};
	const context: PrepareContext = {
		...job,
		signal: controller.signal,
		enter(check) {
			active = check;
			apply({ ...IDLE, status: "running", active: check });
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
 */
export function useDeployPrepare(
	plan: DeployPlan | null,
	options: { enabled?: boolean } = {},
): DeployPrepareState {
	const backend = useBackend();
	const workspace = useDeviceWorkspace();
	const { account } = useDeviceAuth();
	const desktop = workspace.deps.platform === "desktop";
	const { profile } = workspace.deps;
	const [attempt, setAttempt] = useState(0);
	const [imported, setImported] = useState<ImportedCopy | null>(null);
	const [run, setRun] = useState<RunState>({ key: "", ...IDLE });

	const target = prepareTarget(plan, options.enabled !== false, desktop);
	const { appId, offline, eventKey } = target;
	const key = target.wanted
		? `${appId}|${offline ? "offline" : "online"}|${eventKey}|${attempt}|${imported ? "import" : "app"}`
		: "";
	const latest = useRef({ backend, profile, account, imported });
	latest.current = { backend, profile, account, imported };

	useEffect(() => {
		if (!key) return;
		return startPrepare(
			{
				appId,
				events: eventKey.split(","),
				offline,
				desktop,
				...latest.current,
			},
			(next) => setRun({ key, ...next }),
		);
	}, [key, appId, eventKey, offline, desktop]);

	const current = run.key === key && key ? run : { key, ...IDLE };
	const again = useCallback(() => setAttempt((value) => value + 1), []);
	const importCopy = useCallback((copy: ImportedCopy | null) => {
		setImported(copy);
		setAttempt((value) => value + 1);
	}, []);
	const ids = offline ? OFFLINE_CHECKS : ONLINE_CHECKS;
	const { status, active, failure, prepared } = current;
	return useMemo(
		() => ({
			state: status,
			checks: checksOf(ids, { status, active, failure }),
			prepared,
			failure,
			imported: imported !== null,
			again,
			importCopy,
		}),
		[status, active, failure, prepared, ids, imported, again, importCopy],
	);
}
