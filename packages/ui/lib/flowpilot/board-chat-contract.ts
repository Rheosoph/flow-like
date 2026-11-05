import type { FlowIrCommitToken } from "../schema/copilot";
import {
	type IBoard,
	IExecutionMode,
	IExecutionStage,
	ILogLevel,
} from "../schema/flow/board";

/** Older API versions deserialize Board before reloading its authorized saved state by id. */
export function compactCopilotBoardReference(boardId: string): IBoard {
	return {
		id: boardId,
		name: "",
		description: "",
		nodes: {},
		variables: {},
		comments: {},
		layers: {},
		refs: {},
		viewport: [0, 0, 1],
		version: [0, 0, 0],
		stage: IExecutionStage.Dev,
		log_level: ILogLevel.Debug,
		execution_mode: IExecutionMode.Hybrid,
		page_ids: [],
		hash: null,
		created_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
	};
}

function record(value: unknown): Record<string, unknown> | undefined {
	return value !== null && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;
}

function reviewToken(value: unknown): FlowIrCommitToken | undefined {
	const token = record(value);
	if (
		!token ||
		!["board_id", "draft_id", "base_fingerprint", "claim_id"].every(
			(key) => typeof token[key] === "string" && token[key].trim().length > 0,
		) ||
		!Number.isSafeInteger(token.revision) ||
		(token.revision as number) < 0
	)
		return undefined;
	return token as unknown as FlowIrCommitToken;
}

export function assertBoardSpecialistResponse(value: unknown): void {
	const response = record(value);
	if (
		!response ||
		typeof response.message !== "string" ||
		response.active_scope !== "Board" ||
		(response.components != null &&
			(!Array.isArray(response.components) ||
				response.components.length > 0)) ||
		(response.commands != null && !Array.isArray(response.commands)) ||
		(response.suggestions != null && !Array.isArray(response.suggestions)) ||
		(response.flow_ir_commit != null &&
			!reviewToken(response.flow_ir_commit)) ||
		(response.root_component_id != null && response.root_component_id !== "") ||
		response.canvas_settings != null
	) {
		throw new BoardSpecialistScopeError(value);
	}
}

export class BoardSpecialistScopeError extends Error {
	readonly code = "FLOWPILOT_BOARD_SCOPE_MISMATCH";
	readonly flowIrCommit;
	readonly actualScope;

	constructor(value: unknown) {
		const response = record(value);
		const scope =
			typeof response?.active_scope === "string" ? response.active_scope : null;
		super(
			scope && scope !== "Board"
				? `The board request was handled by the ${scope} specialist. No board result was accepted.`
				: "The board specialist returned an invalid response or a page draft. No board result was accepted.",
		);
		this.flowIrCommit = reviewToken(response?.flow_ir_commit);
		this.actualScope = scope;
	}
}

export function boardSpecialistRoutingFailure(
	error: BoardSpecialistScopeError,
	appId: string,
	boardId: string,
	createdBoard: boolean,
) {
	return {
		status: "error" as const,
		code: error.code,
		app_id: appId,
		board_id: boardId,
		...(createdBoard ? { created_board_id: boardId } : {}),
		actual_scope: error.actualScope,
		expected_scope: "Board",
		applied: false,
		retryable: false,
		next_action: "repair_board_routing_then_reuse_existing_board",
		message: `${error.message} Do not repeat the delegation in this turn or create another board. Repair the specialist routing, then continue with board ${boardId}.`,
	};
}

/** A backend routing failure cannot be repaired by asking another model in the same turn. */
export class BoardSpecialistRoutingFailures {
	private readonly failures = new Map<
		string,
		ReturnType<typeof boardSpecialistRoutingFailure>
	>();

	get(ownerId: string) {
		return this.failures.get(ownerId);
	}

	record(
		ownerId: string,
		failure: ReturnType<typeof boardSpecialistRoutingFailure>,
	) {
		this.failures.set(ownerId, failure);
		if (this.failures.size > 64) {
			const oldest = this.failures.keys().next().value;
			if (oldest) this.failures.delete(oldest);
		}
	}
}
