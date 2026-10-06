interface BoardRunProgress {
	stage:
		| "preparing"
		| "generating"
		| "explaining"
		| "settling"
		| "applying"
		| "verifying";
	last_tool?: string;
	last_tool_status?: "running" | "returned";
	workspace_status?: string;
	draft_id?: string;
	revision?: number | string;
	diagnostic_count?: number;
	commit_state:
		| "not_observed"
		| "running"
		| "returned"
		| "queued"
		| "applying"
		| "applied";
}

export interface FlowPilotBoardRunActivity
	extends Omit<BoardRunProgress, "stage"> {
	request_id: string;
	app_id: string;
	board_id: string;
	mode: "edit" | "explain";
	stage: BoardRunProgress["stage"] | "cancelling";
	started_at_ms: number;
	last_activity_at_ms: number;
}

/** Metadata only. Draft source and tool arguments never enter the status response. */
export class FlowPilotBoardActivity {
	private readonly runs = new Map<
		string,
		{
			token: symbol;
			signal?: AbortSignal;
			progress: FlowPilotBoardRunActivity;
		}
	>();

	constructor(private readonly now = () => Date.now()) {}

	begin(
		appId: string,
		boardId: string,
		requestId: string,
		options: {
			mode: "edit" | "explain";
			signal?: AbortSignal;
		},
	) {
		const key = JSON.stringify([appId, boardId]);
		const token = Symbol(requestId);
		const now = this.now();
		this.runs.set(key, {
			token,
			signal: options.signal,
			progress: {
				request_id: requestId,
				app_id: appId,
				board_id: boardId,
				mode: options.mode,
				stage: "preparing",
				commit_state: "not_observed",
				started_at_ms: now,
				last_activity_at_ms: now,
			},
		});
		const update = (progress: Partial<BoardRunProgress> = {}) => {
			const run = this.runs.get(key);
			if (run?.token !== token) return;
			Object.assign(run.progress, progress, {
				last_activity_at_ms: this.now(),
			});
		};
		return {
			update,
			tool: (name: string, status: "running" | "returned") => {
				if (!name) return;
				const commit = name.endsWith("commit_flowscript");
				const commitState = this.runs.get(key)?.progress.commit_state;
				const confirmed =
					commitState === "queued" ||
					commitState === "applying" ||
					commitState === "applied";
				update({
					last_tool: name.slice(0, 160),
					last_tool_status: status,
					// Tool frames can arrive after the workspace receipt or host apply result.
					...(commit && !confirmed ? { commit_state: status } : {}),
				});
			},
			finish: () => {
				if (this.runs.get(key)?.token === token) this.runs.delete(key);
			},
		};
	}

	snapshot(
		appId: string,
		boardId: string,
	): FlowPilotBoardRunActivity | undefined {
		const run = this.runs.get(JSON.stringify([appId, boardId]));
		if (!run) return undefined;
		return {
			...run.progress,
			...(run.signal?.aborted ? { stage: "cancelling" as const } : {}),
		};
	}
}

export const flowPilotBoardActivity = new FlowPilotBoardActivity();
