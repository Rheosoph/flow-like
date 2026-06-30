import { useEffect, useMemo, useRef, useState } from "react";

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

	// Collect remote execution states from peers. Peers broadcast cursors at ~20Hz,
	// so awareness "change" fires constantly; coalesce to one frame and bail unless
	// the merged executing-node set actually changed, otherwise this re-renders the
	// whole FlowBoard ~20Hz × peerCount.
	useEffect(() => {
		if (!awareness) {
			setRemoteExecutions([]);
			return;
		}

		let rafId: number | null = null;
		let lastKey = "";

		const computeAndSet = () => {
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

			const ids = new Set<string>();
			for (const re of remote) for (const n of re.executingNodes) ids.add(n);
			const key = Array.from(ids).sort().join(",");
			if (key === lastKey) return;
			lastKey = key;
			setRemoteExecutions(remote);
		};

		const scheduleUpdate = () => {
			if (rafId !== null) return;
			rafId = requestAnimationFrame(() => {
				rafId = null;
				computeAndSet();
			});
		};

		awareness.on("change", scheduleUpdate);
		computeAndSet();

		return () => {
			if (rafId !== null) cancelAnimationFrame(rafId);
			try {
				awareness.off("change", scheduleUpdate);
			} catch {}
		};
	}, [awareness]);

	// Merged set of all remotely executing node IDs (for visual indicators).
	// Stable identity when unchanged so the consuming setNodes effect can short-circuit.
	const remoteExecutingNodeIds = useMemo(() => {
		const ids = new Set<string>();
		for (const re of remoteExecutions) {
			for (const nodeId of re.executingNodes) {
				ids.add(nodeId);
			}
		}
		return ids;
	}, [remoteExecutions]);

	return {
		remoteExecutions,
		remoteExecutingNodeIds,
	};
}
