import type { UseQueryResult } from "@tanstack/react-query";
import { AlertTriangleIcon, XIcon } from "lucide-react";
import { useCallback, useRef } from "react";
import { getErrorMessage } from "../lib/error-message";
import { boardFingerprint } from "../lib/flow-history-stacks";
import {
	type BoardEditReceiptHistoryMode,
	flowIrCommitDeliveryId,
} from "../lib/flowpilot/board-edit-job-delivery";
import { toastError, toastWarning } from "../lib/messages";
import type { IGenericCommand } from "../lib/schema";
import type { FlowIrCommitToken } from "../lib/schema/copilot";
import type { IBoard } from "../lib/schema/flow/board";
import type { INode } from "../lib/schema/flow/node";
import { useBackendStore } from "../state/backend-state";
import type {
	IApplyFlowIrCommitResponse,
	IApplyFlowScriptResponse,
} from "../state/backend-state/board-state";

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
	pushCommand: (command: IGenericCommand, append?: boolean) => Promise<void>;
	pushCommands: (commands: IGenericCommand[]) => Promise<void>;
	pushCommandsOnce: (
		commands: IGenericCommand[],
		deliveryId: string,
		historyMode?: BoardEditReceiptHistoryMode,
	) => Promise<void>;
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
	pushCommandsOnce,
	stampHistory,
}: UseCommandExecutionProps) {
	const awarenessRef = useRef<any | undefined>(undefined);
	const refetchBoardAndStampHistory = useCallback(async () => {
		const refreshed = await board.refetch();
		if (refreshed.error) throw refreshed.error;
		await stampHistory(boardFingerprint(refreshed.data));
		return refreshed;
	}, [board.refetch, stampHistory]);
	const preserveApplyErrorAfterRefetch = useCallback(
		async (operation: string, error: unknown): Promise<Error> => {
			const primaryMessage = getErrorMessage(error, "Unknown error");
			try {
				await refetchBoardAndStampHistory();
			} catch (refreshError) {
				return new Error(
					`${operation} failed: ${primaryMessage}. The board refresh/history recovery also failed: ${getErrorMessage(refreshError, "Unknown recovery error")}`,
				);
			}
			return error instanceof Error
				? error
				: new Error(`${operation} failed: ${primaryMessage}`);
		},
		[refetchBoardAndStampHistory],
	);
	const settleCommittedMutation = useCallback(
		async (pushHistory: () => Promise<void>, shouldRefetch: boolean) => {
			const followupErrors: string[] = [];
			try {
				await pushHistory();
			} catch (error) {
				followupErrors.push(
					`undo/history update failed: ${getErrorMessage(error, "Unknown history error")}`,
				);
			}

			let refreshed: Awaited<ReturnType<typeof board.refetch>> | undefined;
			if (shouldRefetch || followupErrors.length > 0) {
				try {
					refreshed = await refetchBoardAndStampHistory();
				} catch (error) {
					followupErrors.push(
						`board refetch failed: ${getErrorMessage(error, "Unknown board refetch error")}`,
					);
				}
			}

			if (followupErrors.length > 0) {
				const warning = `The board mutation was applied, but local bookkeeping needs recovery: ${followupErrors.join("; ")}`;
				console.error(
					"[commandExecution] Post-apply recovery needed:",
					warning,
				);
				toastWarning(warning, <AlertTriangleIcon />);
			}
			return refreshed;
		},
		[refetchBoardAndStampHistory],
	);

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

			let result: IGenericCommand;
			try {
				result = await backend.boardState.executeCommand(
					appId,
					boardId,
					command,
				);
			} catch (error) {
				const recoveredError = await preserveApplyErrorAfterRefetch(
					`Command ${command.command_type}`,
					error,
				);
				console.error(
					"[executeCommand] Failed:",
					command.command_type,
					recoveredError,
				);
				toastError(`Command failed: ${recoveredError.message}`, <XIcon />);
				throw recoveredError;
			}
			console.log("[executeCommand] Success:", command.command_type, result);
			await settleCommittedMutation(() => pushCommand(result, append), true);
			awarenessRef.current?.setLocalStateField("boardUpdate", Date.now());
			return result;
		},
		[
			appId,
			boardId,
			preserveApplyErrorAfterRefetch,
			pushCommand,
			settleCommittedMutation,
			version,
		],
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

			let result: IGenericCommand[];
			try {
				result = await backend.boardState.executeCommands(
					appId,
					boardId,
					commands,
				);
			} catch (error) {
				const recoveredError = await preserveApplyErrorAfterRefetch(
					"Command batch",
					error,
				);
				console.error("[executeCommands] Failed:", recoveredError);
				toastError(`Commands failed: ${recoveredError.message}`, <XIcon />);
				throw recoveredError;
			}
			await settleCommittedMutation(
				() => pushCommands(result),
				options.refetch !== false,
			);
			awarenessRef.current?.setLocalStateField("boardUpdate", Date.now());
			return result;
		},
		[
			appId,
			boardId,
			preserveApplyErrorAfterRefetch,
			pushCommands,
			settleCommittedMutation,
			version,
		],
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

			let result: IApplyFlowScriptResponse;
			try {
				result = await backend.boardState.applyFlowScript(
					appId,
					boardId,
					flowscript,
					currentLayer,
					catalogNodes,
					options.allowDeletions === true,
				);
			} catch (error) {
				const recoveredError = await preserveApplyErrorAfterRefetch(
					"FlowScript apply",
					error,
				);
				console.error("[applyFlowScript] Failed:", recoveredError);
				toastError(
					`FlowScript apply failed: ${recoveredError.message}`,
					<XIcon />,
				);
				throw recoveredError;
			}

			let finalBoardNodeCount: number | undefined;
			if (result.commands.length > 0) {
				const refreshed = await settleCommittedMutation(
					() => pushCommands(result.commands),
					options.refetch !== false,
				);
				if (refreshed?.data) {
					finalBoardNodeCount = totalBoardNodeCount(refreshed.data);
				}
				awarenessRef.current?.setLocalStateField("boardUpdate", Date.now());

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
				try {
					await refetchBoardAndStampHistory();
				} catch (refreshError) {
					const warning = `Board refetch after the blocked FlowScript apply failed: ${getErrorMessage(refreshError, "Unknown recovery error")}`;
					console.error("[applyFlowScript] Recovery failed:", warning);
					return {
						...result,
						diagnostics: [...result.diagnostics, warning],
					};
				}
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
		},
		[
			appId,
			boardId,
			preserveApplyErrorAfterRefetch,
			pushCommands,
			refetchBoardAndStampHistory,
			settleCommittedMutation,
			version,
		],
	);

	const applyFlowIrCommit = useCallback(
		async (
			token: FlowIrCommitToken,
			deliveryId?: string,
			historyMode: BoardEditReceiptHistoryMode = "append",
		): Promise<IApplyFlowIrCommitResponse> => {
			const backend = useBackendStore.getState().backend;
			if (!backend?.boardState.applyFlowIrCommit) {
				throw new Error(
					"Atomic compiled workflow apply is unavailable on this backend",
				);
			}
			if (typeof version !== "undefined") {
				throw new Error("Cannot change an old board version");
			}
			const effectiveDeliveryId = flowIrCommitDeliveryId(token);
			if (deliveryId && deliveryId !== effectiveDeliveryId) {
				throw new Error(
					"Compiled workflow delivery identity must match its immutable claim id",
				);
			}
			let result: IApplyFlowIrCommitResponse;
			try {
				result = await backend.boardState.applyFlowIrCommit(
					appId,
					token,
					effectiveDeliveryId,
				);
			} catch (error) {
				throw await preserveApplyErrorAfterRefetch(
					"Compiled workflow apply",
					error,
				);
			}
			if (result.status === "applied" && result.commands.length > 0) {
				const effectiveHistoryMode = result.replayed
					? "invalidate"
					: historyMode;
				// The native mutation is already committed, but its delivery job must remain
				// retryable until both remote/outbox handoff and the idempotent history marker
				// are durable. Refresh failure is visible but does not invalidate those writes.
				const history = await Promise.resolve(
					pushCommandsOnce(
						result.commands,
						effectiveDeliveryId,
						effectiveHistoryMode,
					),
				).then(
					() => ({ ok: true as const }),
					(error) => ({ ok: false as const, error }),
				);
				const refresh = await Promise.resolve(
					refetchBoardAndStampHistory(),
				).then(
					() => ({ ok: true as const }),
					(error) => ({ ok: false as const, error }),
				);
				awarenessRef.current?.setLocalStateField("boardUpdate", Date.now());
				const followupErrors = [history, refresh].flatMap((followup) =>
					followup.ok
						? []
						: [getErrorMessage(followup.error, "Unknown renderer error")],
				);
				if (followupErrors.length > 0) {
					const warning = `The workflow was applied, but local history or refresh bookkeeping needs recovery: ${followupErrors.join("; ")}`;
					console.error(
						"[applyFlowIrCommit] Post-apply recovery needed:",
						warning,
					);
					toastWarning(warning, <AlertTriangleIcon />);
					return {
						...result,
						delivery_complete: result.delivery_complete === true && history.ok,
						diagnostics: [...result.diagnostics, warning],
					};
				}
				return {
					...result,
					delivery_complete: result.delivery_complete === true,
				};
			} else {
				try {
					await refetchBoardAndStampHistory();
				} catch (refreshError) {
					const warning = `Board refetch after the compiled workflow apply did not complete: ${getErrorMessage(refreshError, "Unknown recovery error")}`;
					console.error("[applyFlowIrCommit] Recovery failed:", warning);
					return {
						...result,
						diagnostics: [...result.diagnostics, warning],
					};
				}
			}
			return result;
		},
		[
			appId,
			preserveApplyErrorAfterRefetch,
			pushCommandsOnce,
			refetchBoardAndStampHistory,
			version,
		],
	);

	return {
		executeCommand,
		executeCommands,
		applyFlowScript,
		applyFlowIrCommit,
		awarenessRef,
	};
}
