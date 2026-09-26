import type { IBoard } from "../../../lib/schema/flow/board";
import { type ILog, ILogLevel } from "../../../lib/schema/flow/log";
import type { ILogMetadata } from "../../../lib/schema/flow/log-metadata";
import type { ILogQuery } from "../../../lib/schema/flow/log-query";
import type { INode } from "../../../lib/schema/flow/node";
import {
	type IPin,
	type IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import type { IBoardState } from "../../../state/backend-state/board-state";
import { levelIndex, logStart } from "./log-format";

export function makeLog(
	overrides: Partial<Omit<ILog, "start" | "end">> & {
		start?: number;
		end?: number;
	} = {},
): ILog {
	const { start = 1_000_000, end, ...rest } = overrides;
	const toTime = (micros: number) => ({
		secs_since_epoch: Math.floor(micros / 1_000_000),
		nanos_since_epoch: (micros % 1_000_000) * 1_000,
	});
	return {
		message: "",
		log_level: ILogLevel.Info,
		node_id: null,
		operation_id: null,
		...rest,
		start: toTime(start),
		end: toTime(end ?? start),
	};
}

export function makePin(
	id: string,
	type: IPinType,
	options: {
		exec?: boolean;
		name?: string;
		connectedTo?: string[];
		dependsOn?: string[];
		index?: number;
	} = {},
): IPin {
	return {
		id,
		name: options.name ?? id,
		friendly_name: options.name ?? id,
		description: "",
		pin_type: type,
		data_type: options.exec ? IVariableType.Execution : IVariableType.Struct,
		value_type: IValueType.Normal,
		connected_to: options.connectedTo ?? [],
		depends_on: options.dependsOn ?? [],
		index: options.index ?? 0,
	};
}

export function makeNode(
	id: string,
	name: string,
	pins: IPin[],
	layer?: string,
): INode {
	return {
		id,
		name: name.toLowerCase().replace(/\s+/g, "_"),
		friendly_name: name,
		category: "",
		description: "",
		layer: layer ?? null,
		pins: Object.fromEntries(pins.map((pin) => [pin.id, pin])),
	};
}

function matchesQuery(log: ILog, query: ILogQuery): boolean {
	const level = levelIndex(log.log_level);
	const start = logStart(log);
	const node = log.node_id ?? "";
	const message = (log.message ?? "").toLowerCase();
	const fingerprint = log.fingerprint ?? "";
	if (query.levels?.length && !query.levels.includes(level)) return false;
	if (query.exclude_levels?.includes(level)) return false;
	if (query.nodes?.length && !query.nodes.includes(node)) return false;
	if (query.exclude_nodes?.includes(node)) return false;
	if (query.text?.some((text) => !message.includes(text.toLowerCase())))
		return false;
	if (
		query.exclude_text?.some((text) => message.includes(text.toLowerCase()))
	) {
		return false;
	}
	if (query.fingerprints?.length && !query.fingerprints.includes(fingerprint)) {
		return false;
	}
	if (query.exclude_fingerprints?.includes(fingerprint)) return false;
	if (query.from !== undefined && start < query.from) return false;
	if (query.to !== undefined && start > query.to) return false;
	const fold = query.fold?.find((f) => f.fingerprint === fingerprint);
	return !fold || fold.first_start === start;
}

/** The storage contract in memory: filtered, oldest first, offset/limit paged. */
export function memoryBoardState(logs: readonly ILog[]) {
	const calls: string[] = [];
	const sorted = [...logs].sort((a, b) => logStart(a) - logStart(b));
	const state = {
		calls,
		async queryRunLogs(
			_meta: ILogMetadata,
			query: ILogQuery,
			offset: number,
			limit: number,
		): Promise<ILog[]> {
			calls.push(`query ${offset}+${limit}`);
			return sorted
				.filter((log) => matchesQuery(log, query))
				.slice(offset, offset + limit);
		},
		async countRunLogs(_meta: ILogMetadata, query: ILogQuery): Promise<number> {
			calls.push("count");
			return sorted.filter((log) => matchesQuery(log, query)).length;
		},
	};
	return state as typeof state & IBoardState;
}

export const TEST_META = {
	app_id: "app",
	board_id: "board",
	run_id: "run",
	start: 0,
	end: 0,
	event_id: "event",
	log_level: 0,
	node_id: "event",
	payload: [],
	version: "0.0.1",
} as ILogMetadata;

export function makeBoard(
	nodes: INode[],
	extra: Partial<Pick<IBoard, "layers" | "name">> = {},
): IBoard {
	return {
		id: "board",
		name: extra.name ?? "main.flow",
		description: "",
		comments: {},
		layers: extra.layers ?? {},
		nodes: Object.fromEntries(nodes.map((node) => [node.id, node])),
		refs: {},
		variables: {},
		version: [0, 0, 1],
		viewport: [0, 0, 1],
		page_ids: [],
	} as unknown as IBoard;
}
