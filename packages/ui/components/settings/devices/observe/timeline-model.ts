import type { DeploymentRolloutStatus } from "../../../../lib/device-management/deployment";
import type {
	ArchiveRecordingStatus,
	LiveDeviceInput,
} from "../../../../lib/device-management/model/types";
import type { TelemetryRecord } from "../../../../lib/device-management/workspace/streams";
import { whole, word } from "./observe-data";

export const COMMAND_STATES = [
	"accepted",
	"completed",
	"failed",
	"unknown",
] as const;
export type CommandState = (typeof COMMAND_STATES)[number];

export const INSTANCE_STATES = [
	"starting",
	"running",
	"stopping",
	"backoff",
	"stopped",
	"failed",
	"removed",
] as const;
export type InstanceState = (typeof INSTANCE_STATES)[number];

export type HistoryPauseReason = NonNullable<ArchiveRecordingStatus["reason"]>;

interface FactBase {
	id: string;
	/** Unix seconds. */
	at: number;
}

export type TimelineFact =
	| (FactBase & {
			type: "instance";
			service: string;
			slot: number;
			state: InstanceState;
			settings?: number;
			process?: number;
	  })
	| (FactBase & {
			type: "command";
			operationId: string;
			service?: string;
			state: CommandState;
			/** When the device recorded it, if the result came later. */
			acceptedAt?: number;
	  })
	| (FactBase & {
			type: "update";
			service: string;
			rolloutId: string;
			state: DeploymentRolloutStatus["state"];
			from?: number;
			failure?: string;
	  })
	| (FactBase & { type: "boot" })
	| (FactBase & { type: "agent_start" })
	| (FactBase & {
			type: "access";
			version: number;
			applied: boolean;
			people: number;
	  })
	| (FactBase & {
			type: "history_paused";
			scope: string;
			kind: "logs" | "metrics";
			reason: HistoryPauseReason | null;
	  });

export interface TimelineSources {
	/** Activity records of the device, any order. */
	records: readonly TelemetryRecord[];
	/** Only entries about this service; `null` = the whole device. */
	serviceId: string | null;
	rollouts?: readonly DeploymentRolloutStatus[];
	host?: { booted_at: number | null; agent_started_at: number };
	history?: LiveDeviceInput["history"];
	access?: {
		version: number;
		issuedAt: number;
		applied: boolean;
		people: number;
	};
	/** Hub-corrected unix seconds. */
	now: number;
}

const AGENT_RESTART_GAP_S = 300;

const isOneOf = <T extends string>(
	values: readonly T[],
	value: unknown,
): value is T => values.includes(value as T);

type InstanceFact = Extract<TimelineFact, { type: "instance" }>;
type CommandFact = Extract<TimelineFact, { type: "command" }>;

/** A command while its records are merged: the newest state wins, the accept time is kept. */
interface CommandDraft extends CommandFact {
	sequence: number;
	accepted: number | undefined;
}

function instanceFact(
	row: TelemetryRecord,
	service: string | undefined,
): InstanceFact | undefined {
	const { data } = row;
	if (!service || !isOneOf(INSTANCE_STATES, data.state)) return undefined;
	return {
		type: "instance",
		id: `m${row.sequence}`,
		at: row.timestamp,
		service,
		slot: whole(data.replica_slot) ?? 0,
		state: data.state,
		settings: whole(data.config_revision),
		process: whole(data.process_id),
	};
}

function mergeCommand(
	commands: Map<string, CommandDraft>,
	row: TelemetryRecord,
	service: string | undefined,
): void {
	const { data } = row;
	const operationId = word(data.source_id);
	if (!operationId || !isOneOf(COMMAND_STATES, data.state)) return;
	const known = commands.get(operationId);
	const accepted = data.state === "accepted" ? row.timestamp : known?.accepted;
	if (known && known.sequence > row.sequence) {
		known.accepted = accepted;
		known.service ??= service;
		return;
	}
	commands.set(operationId, {
		type: "command",
		id: `o${operationId}`,
		at: row.timestamp,
		sequence: row.sequence,
		accepted,
		operationId,
		service: service ?? known?.service,
		state: data.state,
	});
}

function commandFact(draft: CommandDraft): CommandFact {
	const { sequence: _sequence, accepted, ...command } = draft;
	return accepted !== undefined && accepted !== command.at
		? { ...command, acceptedAt: accepted }
		: command;
}

function messageFacts(sources: TimelineSources): TimelineFact[] {
	const facts: TimelineFact[] = [];
	const commands = new Map<string, CommandDraft>();
	for (const row of sources.records) {
		const service = word(row.data.placement_id);
		if (sources.serviceId && service !== sources.serviceId) continue;
		if (row.data.kind === "operation") mergeCommand(commands, row, service);
		const instance =
			row.data.kind === "replica" ? instanceFact(row, service) : undefined;
		if (instance) facts.push(instance);
	}
	return [...facts, ...[...commands.values()].map(commandFact)];
}

function rolloutFacts(sources: TimelineSources): TimelineFact[] {
	return (sources.rollouts ?? []).flatMap((row): TimelineFact[] => {
		if (sources.serviceId && row.placement_id !== sources.serviceId) return [];
		const at = row.updated_at ?? row.created_at;
		if (at === undefined) return [];
		return [
			{
				type: "update",
				id: `r${row.rollout_id}`,
				at,
				service: row.placement_id,
				rolloutId: row.rollout_id,
				state: row.state,
				...(row.base_revision === undefined ? {} : { from: row.base_revision }),
				...(row.failure_code ? { failure: row.failure_code } : {}),
			},
		];
	});
}

function deviceFacts(sources: TimelineSources): TimelineFact[] {
	if (sources.serviceId) return [];
	const facts: TimelineFact[] = [];
	const { host, access } = sources;
	if (host) {
		const booted = host.booted_at;
		if (booted !== null) facts.push({ type: "boot", id: "boot", at: booted });
		if (booted === null || host.agent_started_at > booted + AGENT_RESTART_GAP_S)
			facts.push({
				type: "agent_start",
				id: "agent-start",
				at: host.agent_started_at,
			});
	}
	if (access && access.version > 0)
		facts.push({
			type: "access",
			id: `access-${access.version}`,
			at: access.issuedAt,
			version: access.version,
			applied: access.applied,
			people: access.people,
		});
	return facts;
}

/** A paused recording, as the device says it (BG30) or, on an older agent, from the readers list's expiry. */
export function historyPause(
	entry: NonNullable<LiveDeviceInput["history"]>[number],
	now: number,
): { since: number; reason: HistoryPauseReason | null } | undefined {
	if (entry.status)
		return entry.status.state === "paused"
			? { since: entry.status.since, reason: entry.status.reason }
			: undefined;
	return entry.expiresAt <= now
		? { since: entry.expiresAt, reason: "roster_expired" }
		: undefined;
}

function historyFacts(sources: TimelineSources): TimelineFact[] {
	return (sources.history ?? []).flatMap((entry): TimelineFact[] => {
		if (sources.serviceId && entry.scope !== sources.serviceId) return [];
		const pause = historyPause(entry, sources.now);
		if (!pause) return [];
		return [
			{
				type: "history_paused",
				id: `history-${entry.scope}-${entry.kind}`,
				at: pause.since,
				scope: entry.scope,
				kind: entry.kind,
				reason: pause.reason,
			},
		];
	});
}

/** Everything known about the device's recent past, newest first. Unknown kinds and states are left out (R3). */
export function timelineFacts(sources: TimelineSources): TimelineFact[] {
	return [
		...messageFacts(sources),
		...rolloutFacts(sources),
		...deviceFacts(sources),
		...historyFacts(sources),
	].sort((a, b) => b.at - a.at);
}
