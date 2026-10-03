import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import {
	type FlowPublishRefusal,
	type FlowVersionPublished,
	type FlowVersionState,
	type HubResult,
	parseFlowVersionPublished,
	parseFlowVersionState,
	publishFlowVersion,
	readFlowVersion,
} from "./hub/endpoints";

/*
 * The flow of an event that follows Latest, as a version. A device can only
 * address a flow id and a concrete version, so a deploy publishes a version of
 * the current edits when none equals them and ships the event pinned to it.
 */

export type { FlowVersionPublished, FlowVersionState } from "./hub/endpoints";

export interface LatestFlows {
	/** Which published version equals the flow as saved, and the newest one. */
	read(appId: string, boardId: string): Promise<HubResult<FlowVersionState>>;
	/** Publishes a Patch version only when no published version equals the flow. */
	publish(
		appId: string,
		boardId: string,
	): Promise<HubResult<FlowVersionPublished> | FlowPublishRefusal>;
}

/**
 * Online apps: the hub publishes. Never the desktop's local-first
 * `createBoardVersion`: it publishes in the background and returns the next
 * draft number, not the published one.
 */
export function hubLatestFlows(api: IApiState, profile: IProfile): LatestFlows {
	return {
		read: (appId, boardId) => readFlowVersion(api, profile, appId, boardId),
		publish: (appId, boardId) =>
			publishFlowVersion(api, profile, appId, boardId),
	};
}

/** The desktop commands, replaceable in tests. */
export interface FlowVersionCommands {
	read(appId: string, boardId: string): Promise<unknown>;
	publish(
		appId: string,
		boardId: string,
		publishedBy: string | undefined,
	): Promise<unknown>;
}

export async function desktopFlowVersionCommands(): Promise<FlowVersionCommands> {
	const { invoke } = await import("@tauri-apps/api/core");
	return {
		read: (appId, boardId) =>
			invoke("get_board_version_current", { appId, boardId }),
		publish: (appId, boardId, publishedBy) =>
			invoke("publish_board_version_current", { appId, boardId, publishedBy }),
	};
}

/** Local-only apps: this computer holds the flow and publishes it. */
export function desktopLatestFlows(
	account: string | undefined,
	commands: () => Promise<FlowVersionCommands> = desktopFlowVersionCommands,
): LatestFlows {
	return {
		read: async (appId, boardId) => ({
			kind: "ok",
			data: parseFlowVersionState(
				await (await commands()).read(appId, boardId),
			),
		}),
		publish: async (appId, boardId) => ({
			kind: "ok",
			data: parseFlowVersionPublished(
				await (await commands()).publish(appId, boardId, account),
			),
		}),
	};
}

export type FlowTriple = [number, number, number];

export type FlowPublishOutcome =
	/** A version of the current edits was created. */
	| { kind: "created"; version: FlowTriple }
	/** A published version already equals the flow. */
	| { kind: "unchanged"; version: FlowTriple }
	/** The hub can't deploy events that follow Latest. */
	| { kind: "missing_on_hub" }
	/** The flow has edits no version holds, and this person can't publish one. */
	| { kind: "role" }
	/** Someone holds the flow's edit lock. */
	| { kind: "busy" }
	/** The hub's own sentence: the flow never compares equal to a version. */
	| { kind: "incomparable"; message?: string };

export interface PublishFlowOptions {
	/** Further attempts while the flow is being edited. */
	retries?: number;
	delayMs?: number;
	sleep?(ms: number): Promise<void>;
	signal?: AbortSignal;
}

const wait = (ms: number) =>
	new Promise<void>((resolve) => setTimeout(resolve, ms));

/** A person who can't publish can still deploy a flow that already has a version equal to it. */
async function withoutPublishing(
	flows: LatestFlows,
	appId: string,
	boardId: string,
): Promise<FlowPublishOutcome> {
	const read = await flows.read(appId, boardId);
	if (read.kind !== "ok") return { kind: "missing_on_hub" };
	return read.data.current
		? { kind: "unchanged", version: read.data.current }
		: { kind: "role" };
}

/** One event that follows Latest, with the flow it follows. */
export interface LatestEvent {
	eventId: string;
	boardId: string;
}

/** What a deploy did for one flow before it prepared the copy. */
export interface PreparedFlow {
	boardId: string;
	version: FlowTriple;
	/** False: a published version already equalled the flow. */
	created: boolean;
}

/**
 * Why an event that follows Latest can't be shipped now. `moved`: the flow was
 * edited between publishing and preparing, so preparing again helps.
 * `target`: a version equals the flow, and the event's Page or start node is
 * no longer in it, so preparing again does not.
 */
export type LatestProblem =
	| "busy"
	| "role"
	| "incomparable"
	| "hub"
	| "moved"
	| "target";

export class LatestFlowError extends Error {
	constructor(
		readonly problem: LatestProblem,
		readonly at: { boardId: string; eventId?: string },
		/** `incomparable`: the hub's own sentence. */
		readonly detail = "",
	) {
		super(
			detail ||
				`Flow ${at.boardId} can't be deployed as a version (${problem}).`,
		);
		this.name = "LatestFlowError";
	}
}

const REFUSALS: Record<
	Exclude<FlowPublishOutcome["kind"], "created" | "unchanged">,
	LatestProblem
> = {
	missing_on_hub: "hub",
	role: "role",
	busy: "busy",
	incomparable: "incomparable",
};

/**
 * Publish-if-changed for every flow of the named events, in a stable order.
 * Throws a `LatestFlowError` at the first flow that can't become a version.
 */
export async function publishLatestFlows(
	flows: LatestFlows,
	appId: string,
	latest: readonly LatestEvent[],
	options: PublishFlowOptions = {},
): Promise<PreparedFlow[]> {
	const done: PreparedFlow[] = [];
	for (const boardId of [...new Set(latest.map((row) => row.boardId))].sort()) {
		const outcome = await publishFlow(flows, appId, boardId, options);
		if (outcome.kind === "created" || outcome.kind === "unchanged") {
			done.push({
				boardId,
				version: outcome.version,
				created: outcome.kind === "created",
			});
			continue;
		}
		const eventId = latest.find((row) => row.boardId === boardId)?.eventId;
		throw new LatestFlowError(
			REFUSALS[outcome.kind],
			{ boardId, ...(eventId ? { eventId } : {}) },
			outcome.kind === "incomparable" ? (outcome.message ?? "") : "",
		);
	}
	return done;
}

/**
 * Online apps, once the bundle exists: the pin of every named event. An event
 * the bundle left out is asked about once more: no version equals its flow any
 * more (`moved`), or one does and the event does not fit it (`target`).
 */
export async function latestPinsOnline(
	flows: LatestFlows,
	appId: string,
	latest: readonly LatestEvent[],
	pinOf: (eventId: string) => FlowTriple | null | undefined,
): Promise<Record<string, FlowTriple>> {
	const pins: Record<string, FlowTriple> = {};
	for (const { eventId, boardId } of latest) {
		const pin = pinOf(eventId);
		if (pin) {
			pins[eventId] = pin;
			continue;
		}
		const read = await flows.read(appId, boardId);
		const current = read.kind === "ok" ? read.data.current : null;
		throw new LatestFlowError(current ? "target" : "moved", {
			boardId,
			eventId,
		});
	}
	return pins;
}

/** Local-only apps: the export says which version each event was pinned to, or why it was left out. */
export function latestPinsOffline(
	latest: readonly LatestEvent[],
	exported:
		| readonly {
				event_id: string;
				board_version: FlowTriple | null;
				problem: "edited" | "target_missing" | null;
		  }[]
		| null,
): Record<string, FlowTriple> {
	const pins: Record<string, FlowTriple> = {};
	for (const { eventId, boardId } of latest) {
		const entry = exported?.find((row) => row.event_id === eventId);
		if (entry?.board_version) {
			pins[eventId] = entry.board_version;
			continue;
		}
		throw new LatestFlowError(
			entry?.problem === "target_missing" ? "target" : "moved",
			{ boardId, eventId },
		);
	}
	return pins;
}

/** Publish-if-changed for one flow, with what a deploy does about each refusal. */
export async function publishFlow(
	flows: LatestFlows,
	appId: string,
	boardId: string,
	options: PublishFlowOptions = {},
): Promise<FlowPublishOutcome> {
	const { retries = 3, delayMs = 2_000, sleep = wait, signal } = options;
	for (let attempt = 0; ; attempt++) {
		signal?.throwIfAborted();
		const result = await flows.publish(appId, boardId);
		if (result.kind === "ok")
			return {
				kind: result.data.created ? "created" : "unchanged",
				version: result.data.version,
			};
		if (result.kind === "missing_on_hub") return { kind: "missing_on_hub" };
		if (result.kind === "flow_role")
			return withoutPublishing(flows, appId, boardId);
		if (result.kind === "flow_incomparable")
			return {
				kind: "incomparable",
				...(result.message ? { message: result.message } : {}),
			};
		if (attempt >= retries) return { kind: "busy" };
		await sleep(delayMs);
	}
}
