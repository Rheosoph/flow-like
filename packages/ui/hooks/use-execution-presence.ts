import { useEffect, useRef, useState } from "react";

export interface ExecutionPresenceState {
	/** Nodes currently being executed by this peer */
	executingNodes: string[];
	/** Run ID if any */
	runId?: string;
	/** Sub of the peer who started the execution */
	sub?: string;
}

interface UseExecutionPresenceProps {
	// biome-ignore lint/suspicious/noExplicitAny: Yjs awareness is untyped
	awareness: any | undefined;
	sub?: string;
	/** Local execution store to watch */
	runs: Map<
		string,
		{
			boardId: string;
			nodes: Set<string>;
			already_executed: Set<string>;
		}
	>;
	boardId: string;
}

export interface RemoteExecution {
	sub: string;
	runId?: string;
	executingNodes: string[];
}

export function useExecutionPresence({
	awareness,
	sub,
	runs,
	boardId,
}: UseExecutionPresenceProps) {
	const [remoteExecutions, setRemoteExecutions] = useState<RemoteExecution[]>(
		[],
	);
	const lastBroadcastRef = useRef<string>("");

	// Broadcast local execution state to peers
	useEffect(() => {
		if (!awareness || !sub) return;

		const executingNodes: string[] = [];
		let activeRunId: string | undefined;

		for (const [runId, run] of runs) {
			if (run.boardId !== boardId) continue;
			if (run.nodes.size > 0) {
				activeRunId = runId;
				executingNodes.push(...Array.from(run.nodes));
			}
		}

		const key = executingNodes.sort().join(",") + (activeRunId ?? "");
		if (key === lastBroadcastRef.current) return;
		lastBroadcastRef.current = key;

		awareness.setLocalStateField("executionPresence", {
			executingNodes,
			runId: activeRunId,
			sub,
		} satisfies ExecutionPresenceState);
	}, [awareness, sub, runs, boardId]);

	// Collect remote execution states from peers
	useEffect(() => {
		if (!awareness) {
			setRemoteExecutions([]);
			return;
		}

		const handleChange = () => {
			const states = awareness.getStates() as Map<
				number,
				Record<string, unknown>
			>;
			const remote: RemoteExecution[] = [];

			for (const [clientId, state] of states) {
				if (clientId === awareness.clientID) continue;

				const ep = state?.executionPresence as
					| ExecutionPresenceState
					| undefined;
				if (!ep?.executingNodes?.length) continue;

				remote.push({
					sub: ep.sub ?? "unknown",
					runId: ep.runId,
					executingNodes: ep.executingNodes,
				});
			}

			setRemoteExecutions(remote);
		};

		awareness.on("change", handleChange);
		handleChange();

		return () => {
			try {
				awareness.off("change", handleChange);
			} catch {}
		};
	}, [awareness]);

	// Merged set of all remotely executing node IDs (for visual indicators)
	const remoteExecutingNodeIds = new Set<string>();
	for (const re of remoteExecutions) {
		for (const nodeId of re.executingNodes) {
			remoteExecutingNodeIds.add(nodeId);
		}
	}

	return {
		remoteExecutions,
		remoteExecutingNodeIds,
	};
}
