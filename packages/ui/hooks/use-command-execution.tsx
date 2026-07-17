import type { UseQueryResult } from "@tanstack/react-query";
import { AlertTriangleIcon, XIcon } from "lucide-react";
import { useCallback, useRef } from "react";
import { getErrorMessage } from "../lib/error-message";
import { boardFingerprint } from "../lib/flow-history-stacks";
import { toastError, toastWarning } from "../lib/messages";
import type { IGenericCommand } from "../lib/schema";
import type { FlowIrCommitToken } from "../lib/schema/copilot";
import type { IBoard } from "../lib/schema/flow/board";
import type { INode } from "../lib/schema/flow/node";
import { useBackendStore } from "../state/backend-state";
import type { IApplyFlowIrCommitResponse } from "../state/backend-state/board-state";

interface ExecuteCommandsOptions {
	refetch?: boolean;
	allowDeletions?: boolean;
	suppressBlockedToast?: boolean;
}

interface UseCommandExecutionProps {
	appId: string;
	boardId: string;
	board: UseQueryResult<IBoard>;
	version: [number, number, number] | undefined;
	pushCommand: (command: any, append?: boolean) => Promise<void>;
	pushCommands: (commands: any[]) => Promise<void>;
	stampHistory: (stamp?: string) => Promise<void>;
}

function totalBoardNodeCount(board: IBoard): number {
	const nodeIds = new Set(Object.keys(board.nodes ?? {}));
	for (const layer of Object.values(board.layers ?? {})) {
		for (const nodeId of Object.keys(layer?.nodes ?? {})) nodeIds.add(nodeId);
	}
	return nodeIds.size;
}

export function useCommandExecution({
	appId,
	boardId,
	board,
	version,
	pushCommand,
	pushCommands,
	stampHistory,
}: UseCommandExecutionProps) {
	const awarenessRef = useRef<any | undefined>(undefined);

	const executeCommand = useCallback(
		async (command: IGenericCommand, append = false): Promise<any> => {
			const backend = useBackendStore.getState().backend;
			if (!backend) {
				console.error("[executeCommand] No backend available");
				toastError("Backend not initialized", <XIcon />);
				return;
			}
			if (typeof version !== "undefined") {
				console.error("[executeCommand] Cannot modify old version:", version);
				toastError("Cannot change old version", <XIcon />);
				return;
			}

			console.log("[executeCommand] Executing:", command.command_type, command);

			try {
				const result = await backend.boardState.executeCommand(
					appId,
					boardId,
					command,
				);
				console.log("[executeCommand] Success:", command.command_type, result);
				await pushCommand(result, append);
				const refreshed = await board.refetch();
				await stampHistory(boardFingerprint(refreshed.data));

				if (awarenessRef.current) {
					awarenessRef.current.setLocalStateField("boardUpdate", Date.now());
				}

				return result;
			} catch (error) {
				console.error("[executeCommand] Failed:", command.command_type, error);
				toastError(
					`Command failed: ${getErrorMessage(error, "Unknown error")}`,
					<XIcon />,
				);
				throw error;
			}
		},
		[board.refetch, appId, boardId, pushCommand, stampHistory, version],
	);

	const executeCommands = useCallback(
		async (
			commands: IGenericCommand[],
			options: ExecuteCommandsOptions = {},
		) => {
			const backend = useBackendStore.getState().backend;
			if (!backend) {
				console.error("[executeCommands] No backend available");
				toastError("Backend not initialized", <XIcon />);
				return;
			}
			if (typeof version !== "undefined") {
				console.error("[executeCommands] Cannot modify old version:", version);
				toastError("Cannot change old version", <XIcon />);
				return;
			}
			if (commands.length === 0) return;

			try {
				const result = await backend.boardState.executeCommands(
					appId,
					boardId,
					commands,
				);
				await pushCommands(result);
				if (options.refetch !== false) {
					const refreshed = await board.refetch();
					await stampHistory(boardFingerprint(refreshed.data));
				}

				if (awarenessRef.current) {
					awarenessRef.current.setLocalStateField("boardUpdate", Date.now());
				}

				return result;
			} catch (error) {
				console.error("[executeCommands] Failed:", error);
				toastError(
					`Commands failed: ${getErrorMessage(error, "Unknown error")}`,
					<XIcon />,
				);
				throw error;
			}
		},
		[board.refetch, appId, boardId, pushCommands, stampHistory, version],
	);

	const applyFlowScript = useCallback(
		async (
			flowscript: string,
			currentLayer?: string,
			catalogNodes?: INode[],
			options: ExecuteCommandsOptions = {},
		) => {
			const backend = useBackendStore.getState().backend;
			if (!backend) {
				console.error("[applyFlowScript] No backend available");
				toastError("Backend not initialized", <XIcon />);
				return;
			}
			if (typeof version !== "undefined") {
				console.error("[applyFlowScript] Cannot modify old version:", version);
				toastError("Cannot change old version", <XIcon />);
				return;
			}
			if (!flowscript.trim()) return;

			try {
				const result = await backend.boardState.applyFlowScript(
					appId,
					boardId,
					flowscript,
					currentLayer,
					catalogNodes,
					options.allowDeletions === true,
				);

				let finalBoardNodeCount: number | undefined;
				if (result.commands.length > 0) {
					await pushCommands(result.commands);
					if (options.refetch !== false) {
						const refreshed = await board.refetch();
						await stampHistory(boardFingerprint(refreshed.data));
						if (refreshed.data) {
							finalBoardNodeCount = totalBoardNodeCount(refreshed.data);
						}
					}

					if (awarenessRef.current) {
						awarenessRef.current.setLocalStateField("boardUpdate", Date.now());
					}

					// Partial apply: the derivable changes were applied, but some arguments/
					// connections were skipped. Surface them without blocking.
					if (result.diagnostics.length > 0) {
						toastWarning(
							`Applied with ${result.diagnostics.length} warning${
								result.diagnostics.length === 1 ? "" : "s"
							}: ${result.diagnostics[0]}`,
							<AlertTriangleIcon />,
						);
					}
				} else if (result.diagnostics.length > 0) {
					const suppressToast =
						options.suppressBlockedToast === true &&
						result.diagnostics[0]?.startsWith("FlowScript edit would delete ");
					if (suppressToast) return result;
					toastError(
						`FlowScript apply blocked: ${result.diagnostics[0]}`,
						<XIcon />,
					);
				}

				return Number.isSafeInteger(finalBoardNodeCount)
					? { ...result, final_board_node_count: finalBoardNodeCount }
					: result;
			} catch (error) {
				console.error("[applyFlowScript] Failed:", error);
				toastError(
					`FlowScript apply failed: ${getErrorMessage(error, "Unknown error")}`,
					<XIcon />,
				);
				throw error;
			}
		},
		[board.refetch, appId, boardId, pushCommands, stampHistory, version],
	);

	const applyFlowIrCommit = useCallback(
		async (token: FlowIrCommitToken): Promise<IApplyFlowIrCommitResponse> => {
			const backend = useBackendStore.getState().backend;
			if (!backend?.boardState.applyFlowIrCommit) {
				throw new Error(
					"Atomic compiled workflow apply is unavailable on this backend",
				);
			}
			if (typeof version !== "undefined") {
				throw new Error("Cannot change an old board version");
			}
			const result = await backend.boardState.applyFlowIrCommit(appId, token);
			if (result.status === "applied" && result.commands.length > 0) {
				// The native transaction has already persisted and acknowledged the exact
				// retained compiled workflow batch. Renderer bookkeeping must never turn that
				// success into a retry/dismiss path: collect refresh/history failures as
				// recoverable warnings.
				const followups = await Promise.allSettled([
					pushCommands(result.commands),
					board.refetch(),
				]);
				awarenessRef.current?.setLocalStateField("boardUpdate", Date.now());
				const followupErrors = followups.flatMap((followup) =>
					followup.status === "rejected"
						? [getErrorMessage(followup.reason, "Unknown renderer error")]
						: [],
				);
				const [pushResult, refetchResult] = followups;
				if (
					pushResult.status === "fulfilled" &&
					refetchResult.status === "fulfilled"
				) {
					try {
						await stampHistory(boardFingerprint(refetchResult.value.data));
					} catch (error) {
						followupErrors.push(
							getErrorMessage(error, "Unknown renderer error"),
						);
					}
				}
				if (followupErrors.length > 0) {
					const warning = `The workflow was applied, but local history or refresh bookkeeping needs recovery: ${followupErrors.join("; ")}`;
					console.error(
						"[applyFlowIrCommit] Post-apply recovery needed:",
						warning,
					);
					toastWarning(warning, <AlertTriangleIcon />);
					return {
						...result,
						diagnostics: [...result.diagnostics, warning],
					};
				}
			}
			return result;
		},
		[appId, board.refetch, pushCommands, stampHistory, version],
	);

	return {
		executeCommand,
		executeCommands,
		applyFlowScript,
		applyFlowIrCommit,
		awarenessRef,
	};
}
