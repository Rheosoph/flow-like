import { type UseQueryResult, useQueryClient } from "@tanstack/react-query";
import { AlertTriangleIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useRef } from "react";
import {
	BOARD_DELIVERED_EVENT,
	isBoardSyncEventFor,
} from "../lib/board-sync-events";
import { getErrorMessage } from "../lib/error-message";
import {
	type BoardEditReceiptHistoryMode,
	flowIrCommitDeliveryId,
} from "../lib/flowpilot/board-edit-job-delivery";
import type { FlowScriptApplyOrigin } from "../lib/flowscript-apply-failure";
import { toastError, toastWarning } from "../lib/messages";
import {
	LAST_EDIT_FIELD,
	sanitizeLastEdit,
} from "../lib/realtime/presence-signals";
import type { SignalFieldSetter } from "../lib/realtime/presence-signals-store";
import type { IGenericCommand } from "../lib/schema";
import type { FlowIrCommitToken } from "../lib/schema/copilot";
import type { IBoard } from "../lib/schema/flow/board";
import type { INode } from "../lib/schema/flow/node";
import { useBackendStore } from "../state/backend-state";
import type {
	IApplyFlowIrCommitResponse,
	IApplyFlowScriptResponse,
} from "../state/backend-state/board-state";
import { injectData } from "./use-invoke";

interface ExecuteCommandsOptions {
	refetch?: boolean;
	allowDeletions?: boolean;
	/** The caller renders blocked-apply diagnostics itself, so skip the generic error toast. */
	suppressBlockedToast?: boolean;
	/** Who authored the FlowScript. Defaults to the editor; FlowPilot passes "agent". */
	origin?: FlowScriptApplyOrigin;
	/** Anchors from a scoped `getFlowScriptScoped` render — limits the reconcile diff. */
	scopeAnchors?: string[];
	/** Module layer id whose file this FlowScript is; undefined for the root file (`main`). */
	module?: string;
}

interface UseCommandExecutionProps {
	appId: string;
	boardId: string;
	board: UseQueryResult<IBoard>;
	version: [number, number, number] | undefined;
	pushCommand: (command: IGenericCommand) => Promise<void>;
	pushCommands: (commands: IGenericCommand[]) => Promise<void>;
	pushCommandsOnce: (
		commands: IGenericCommand[],
		deliveryId: string,
		historyMode?: BoardEditReceiptHistoryMode,
	) => Promise<void>;
	/** Serializes each mutation with undo/redo so history always records edits in commit order. */
	withHistoryLock: <T>(operation: () => Promise<T>) => Promise<T>;
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
	withHistoryLock,
}: UseCommandExecutionProps) {
	const awarenessRef = useRef<SignalFieldSetter | undefined>(undefined);
	const queryClient = useQueryClient();
	const refetchBoard = useCallback(async () => {
		const refreshed = await board.refetch();
		if (refreshed.error) throw refreshed.error;
		return refreshed;
	}, [board.refetch]);
	/**
	 * A mutation that already returned the resulting board (the sync tail of a merged apply,
	 * applied onto the held revision) hands it straight to the query cache instead of refetching:
	 * same data, same object identities for untouched nodes, no round trip.
	 */
	const injectBoard = useCallback(
		(nextBoard: IBoard) => {
			const backend = useBackendStore.getState().backend;
			if (!backend) return undefined;
			return injectData(
				queryClient,
				backend.boardState.getBoard,
				[appId, boardId],
				nextBoard,
			);
		},
		[appId, boardId, queryClient],
	);
	// What the batch did, for peers' activity ticker — command kinds and a
	// count only, never payloads.
	const announceEdit = useCallback((edits: readonly IGenericCommand[]) => {
		const awareness = awarenessRef.current;
		if (!awareness || edits.length === 0) return;
		const payload = sanitizeLastEdit({
			kinds: [...new Set(edits.map((edit) => edit.command_type))],
			count: edits.length,
			ts: Date.now(),
		});
		if (payload) awareness.setLocalStateField(LAST_EDIT_FIELD, payload);
	}, []);

	// On desktop the local commit returns before the hub has the edit; peers are pinged again
	// once delivery lands so their refetch finds it.
	useEffect(() => {
		if (typeof window === "undefined") return;
		const handleDelivered = (event: Event) => {
			if (!isBoardSyncEventFor(event, appId, boardId)) return;
			awarenessRef.current?.setLocalStateField("boardUpdate", Date.now());
		};
		window.addEventListener(BOARD_DELIVERED_EVENT, handleDelivered);
		return () => {
			window.removeEventListener(BOARD_DELIVERED_EVENT, handleDelivered);
		};
	}, [appId, boardId]);
	const preserveApplyErrorAfterRefetch = useCallback(
		async (operation: string, error: unknown): Promise<Error> => {
			const primaryMessage = getErrorMessage(error, "Unknown error");
			try {
				await refetchBoard();
			} catch (refreshError) {
				return new Error(
					`${operation} failed: ${primaryMessage}. The board refresh/history recovery also failed: ${getErrorMessage(refreshError, "Unknown recovery error")}`,
				);
			}
			return error instanceof Error
				? error
				: new Error(`${operation} failed: ${primaryMessage}`);
		},
		[refetchBoard],
	);
	const settleCommittedMutation = useCallback(
		async (
			pushHistory: () => Promise<void>,
			shouldRefetch: boolean,
			resultingBoard?: IBoard,
		) => {
			const followupErrors: string[] = [];
			try {
				await pushHistory();
			} catch (error) {
				followupErrors.push(
					`undo/history update failed: ${getErrorMessage(error, "Unknown history error")}`,
				);
			}

			let refreshed: Awaited<ReturnType<typeof board.refetch>> | undefined;
			if (resultingBoard && shouldRefetch && followupErrors.length === 0) {
				try {
					refreshed = injectBoard(resultingBoard);
				} catch (error) {
					followupErrors.push(
						`board update failed: ${getErrorMessage(error, "Unknown board update error")}`,
					);
				}
			} else if (shouldRefetch || followupErrors.length > 0) {
				try {
					refreshed = await refetchBoard();
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
		[injectBoard, refetchBoard],
	);

	const executeCommand = useCallback(
		async (command: IGenericCommand): Promise<IGenericCommand | undefined> => {
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

			return withHistoryLock(async () => {
				console.log("[executeCommand] Executing:", command.command_type);

				let result: IGenericCommand;
				let resultingBoard: IBoard | undefined;
				try {
					result = await backend.boardState.executeCommand(
						appId,
						boardId,
						command,
						{
							onBoard: (next) => {
								resultingBoard = next;
							},
						},
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
				console.log("[executeCommand] Success:", command.command_type);
				await settleCommittedMutation(
					() => pushCommand(result),
					true,
					resultingBoard,
				);
				awarenessRef.current?.setLocalStateField("boardUpdate", Date.now());
				announceEdit([command]);
				return result;
			});
		},
		[
			appId,
			boardId,
			preserveApplyErrorAfterRefetch,
			pushCommand,
			settleCommittedMutation,
			version,
			announceEdit,
			withHistoryLock,
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

			return withHistoryLock(async () => {
				let result: IGenericCommand[];
				let resultingBoard: IBoard | undefined;
				try {
					result = await backend.boardState.executeCommands(
						appId,
						boardId,
						commands,
						{
							onBoard: (next) => {
								resultingBoard = next;
							},
						},
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
					resultingBoard,
				);
				awarenessRef.current?.setLocalStateField("boardUpdate", Date.now());
				announceEdit(commands);
				return result;
			});
		},
		[
			appId,
			boardId,
			preserveApplyErrorAfterRefetch,
			pushCommands,
			settleCommittedMutation,
			version,
			announceEdit,
			withHistoryLock,
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

			return withHistoryLock(async () => {
				let result: IApplyFlowScriptResponse;
				try {
					result = await backend.boardState.applyFlowScript(
						appId,
						boardId,
						flowscript,
						currentLayer,
						catalogNodes,
						options.allowDeletions === true,
						options.origin ?? "editor",
						options.scopeAnchors,
						options.module,
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
						await refetchBoard();
					} catch (refreshError) {
						const warning = `Board refetch after the blocked FlowScript apply failed: ${getErrorMessage(refreshError, "Unknown recovery error")}`;
						console.error("[applyFlowScript] Recovery failed:", warning);
						return {
							...result,
							diagnostics: [...result.diagnostics, warning],
						};
					}
					if (options.suppressBlockedToast === true) return result;
					toastError(
						`FlowScript apply blocked: ${result.diagnostics[0]}`,
						<XIcon />,
					);
				}

				return Number.isSafeInteger(finalBoardNodeCount)
					? { ...result, final_board_node_count: finalBoardNodeCount }
					: result;
			});
		},
		[
			appId,
			boardId,
			preserveApplyErrorAfterRefetch,
			pushCommands,
			refetchBoard,
			settleCommittedMutation,
			version,
			withHistoryLock,
		],
	);

	const applyFlowIrCommit = useCallback(
		async (
			token: FlowIrCommitToken,
			deliveryId?: string,
			historyMode: BoardEditReceiptHistoryMode = "append",
		): Promise<IApplyFlowIrCommitResponse> => {
			const boardState = useBackendStore.getState().backend?.boardState;
			const applyCommit = boardState?.applyFlowIrCommit;
			if (!boardState || !applyCommit) {
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
			return withHistoryLock(async () => {
				let result: IApplyFlowIrCommitResponse;
				try {
					result = await applyCommit.call(
						boardState,
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
					const refresh = await Promise.resolve(refetchBoard()).then(
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
							delivery_complete:
								result.delivery_complete === true && history.ok,
							diagnostics: [...result.diagnostics, warning],
						};
					}
					return {
						...result,
						delivery_complete: result.delivery_complete === true,
					};
				}
				try {
					await refetchBoard();
				} catch (refreshError) {
					const warning = `Board refetch after the compiled workflow apply did not complete: ${getErrorMessage(refreshError, "Unknown recovery error")}`;
					console.error("[applyFlowIrCommit] Recovery failed:", warning);
					return {
						...result,
						diagnostics: [...result.diagnostics, warning],
					};
				}
				return result;
			});
		},
		[
			appId,
			preserveApplyErrorAfterRefetch,
			pushCommandsOnce,
			refetchBoard,
			version,
			withHistoryLock,
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
