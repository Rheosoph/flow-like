import type { UseQueryResult } from "@tanstack/react-query";
import { XIcon } from "lucide-react";
import { useCallback, useRef } from "react";
import { getErrorMessage } from "../lib/error-message";
import { toastError } from "../lib/messages";
import type { IGenericCommand } from "../lib/schema";
import type { IBoard } from "../lib/schema/flow/board";
import type { INode } from "../lib/schema/flow/node";
import { useBackendStore } from "../state/backend-state";

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
}

export function useCommandExecution({
	appId,
	boardId,
	board,
	version,
	pushCommand,
	pushCommands,
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
				await board.refetch();

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
		[board.refetch, appId, boardId, pushCommand, version],
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
					await board.refetch();
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
		[board.refetch, appId, boardId, pushCommands, version],
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

				if (result.commands.length > 0) {
					await pushCommands(result.commands);
					if (options.refetch !== false) {
						await board.refetch();
					}

					if (awarenessRef.current) {
						awarenessRef.current.setLocalStateField("boardUpdate", Date.now());
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

				return result;
			} catch (error) {
				console.error("[applyFlowScript] Failed:", error);
				toastError(
					`FlowScript apply failed: ${getErrorMessage(error, "Unknown error")}`,
					<XIcon />,
				);
				throw error;
			}
		},
		[board.refetch, appId, boardId, pushCommands, version],
	);

	return {
		executeCommand,
		executeCommands,
		applyFlowScript,
		awarenessRef,
	};
}
