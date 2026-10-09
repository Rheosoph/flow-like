import { z } from "zod";
import { type AgentRead, agentRead } from "./agent-reads";
import type { AgentFeatures } from "./model/types";
import type { ManagementCall } from "./telemetry";

const integer = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const identifier = z
	.string()
	.min(1)
	.max(128)
	.regex(/^[A-Za-z0-9_:.-]+$/u);
const level = z.number().int().min(0).max(4);
const runSchema = z.object({
	run_id: identifier,
	board_id: z.string().max(128),
	event_id: z.string().max(128),
	node_id: z.string().max(128),
	version: z.string().max(128),
	event_version: z.string().max(128).nullable(),
	start: integer,
	end: integer,
	log_level: level,
	logs: integer.nullable(),
});
const logSchema = z.object({
	message: z.string().max(8192),
	node_id: z.string().max(128).nullable(),
	operation_id: z.string().max(128).nullable(),
	log_level: level,
	start: integer,
	end: integer,
	truncated: z.boolean(),
});

/** Times are Unix microseconds, matching the workflow's structured log store. */
export type DeviceExecutionRun = z.infer<typeof runSchema>;
export type DeviceExecutionLog = z.infer<typeof logSchema>;
export interface ExecutionRunsPage {
	placement_id: string;
	runs: DeviceExecutionRun[];
	next_offset: number | null;
	limit_reached?: boolean;
}
export interface ExecutionLogsPage {
	placement_id: string;
	run_id: string;
	logs: DeviceExecutionLog[];
	next_offset: number | null;
	limit_reached?: boolean;
}

function pageInput(
	offset: number | undefined,
	limit: number | undefined,
	max: number,
) {
	return z
		.object({ offset: integer.max(1_000_000), limit: integer.min(1).max(max) })
		.parse({ offset: offset ?? 0, limit: limit ?? max });
}

const nextOffset = (offset: number) =>
	integer.gt(offset).max(1_000_000).nullable();

export function readExecutionRuns(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: { placementId: string; offset?: number; limit?: number },
): Promise<AgentRead<ExecutionRunsPage>> {
	const placementId = identifier.parse(input.placementId);
	const { offset, limit } = pageInput(input.offset, input.limit, 20);
	return agentRead(
		call,
		features,
		"execution_logs",
		"workflow executions",
		{ type: "execution_runs", placement_id: placementId, offset, limit },
		(result) =>
			z
				.object({
					placement_id: z.literal(placementId),
					runs: z.array(runSchema).max(limit),
					next_offset: nextOffset(offset),
					limit_reached: z.boolean().optional(),
				})
				.parse(result),
	);
}

export function readExecutionLogs(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	input: {
		placementId: string;
		runId: string;
		offset?: number;
		limit?: number;
		nodeId?: string;
		minLevel?: number;
	},
): Promise<AgentRead<ExecutionLogsPage>> {
	const placementId = identifier.parse(input.placementId);
	const runId = identifier.parse(input.runId);
	const nodeId = input.nodeId ? identifier.parse(input.nodeId) : undefined;
	const minLevel =
		input.minLevel === undefined ? undefined : level.parse(input.minLevel);
	const { offset, limit } = pageInput(input.offset, input.limit, 50);
	return agentRead(
		call,
		features,
		"execution_logs",
		"workflow execution logs",
		{
			type: "execution_logs",
			placement_id: placementId,
			run_id: runId,
			offset,
			limit,
			...(nodeId ? { node_id: nodeId } : {}),
			...(minLevel === undefined ? {} : { min_level: minLevel }),
		},
		(result) =>
			z
				.object({
					placement_id: z.literal(placementId),
					run_id: z.literal(runId),
					logs: z.array(logSchema).max(limit),
					next_offset: nextOffset(offset),
					limit_reached: z.boolean().optional(),
				})
				.parse(result),
	);
}
