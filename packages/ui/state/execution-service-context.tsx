"use client";

import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useMemo,
	useState,
} from "react";
import { RuntimeVariablesPrompt } from "../components/flow/runtime-variables-prompt";
import { WasmSandboxWarningDialog } from "../components/flow/wasm-sandbox-warning-dialog";
import type { IIntercomEvent, ILogMetadata, IRunPayload } from "../lib";
import type { IBoard, IVariable } from "../lib/schema/flow/board";
import { IExecutionMode } from "../lib/schema/flow/board";
import { useBackend } from "./backend-state";
import type { IRuntimeVariable } from "./backend-state/types";
import {
	type RuntimeVariableValue,
	useRuntimeVariables,
} from "./runtime-variables-context";

interface PendingExecution {
	appId: string;
	boardId: string;
	payload: IRunPayload;
	streamState?: boolean;
	eventId?: (id: string) => void;
	cb?: (event: IIntercomEvent[]) => void;
	skipConsentCheck?: boolean;
	isRemote: boolean;
	isEvent: boolean;
	eventIdStr?: string;
	resolve: (result: ILogMetadata | undefined) => void;
	reject: (error: Error) => void;
}

// ---- WASM consent helpers (localStorage-backed) ----

function wasmConsentKey(
	scope: "board" | "event" | "package",
	id: string,
): string {
	return `wasm-consent-${scope}-${id}`;
}

function hasWasmConsent(
	scope: "board" | "event" | "package",
	id: string,
): boolean {
	try {
		return localStorage.getItem(wasmConsentKey(scope, id)) === "1";
	} catch {
		return false;
	}
}

function saveWasmConsent(
	scope: "board" | "event" | "package",
	id: string,
): void {
	try {
		localStorage.setItem(wasmConsentKey(scope, id), "1");
	} catch {
		// Ignore storage errors
	}
}

function allPackagesTrusted(packageIds: string[]): boolean {
	return packageIds.every((id) => hasWasmConsent("package", id));
}

interface ExecutionServiceContextValue {
	/**
	 * Execute a board with runtime variables check.
	 * If runtime-configured variables are missing, shows a prompt.
	 */
	executeBoard: (
		appId: string,
		boardId: string,
		payload: IRunPayload,
		streamState?: boolean,
		eventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
	) => Promise<ILogMetadata | undefined>;

	/**
	 * Execute a board remotely with runtime variables check.
	 */
	executeBoardRemote: (
		appId: string,
		boardId: string,
		payload: IRunPayload,
		streamState?: boolean,
		eventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
	) => Promise<ILogMetadata | undefined>;

	/**
	 * Execute an event with runtime variables check.
	 * If runtime-configured variables are missing, shows a prompt.
	 */
	executeEvent: (
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
	) => Promise<ILogMetadata | undefined>;

	/**
	 * Execute without runtime variables check (for internal use).
	 * Use this when you've already validated runtime variables.
	 */
	executeBoardDirect: (
		appId: string,
		boardId: string,
		payload: IRunPayload,
		streamState?: boolean,
		eventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
	) => Promise<ILogMetadata | undefined>;

	/**
	 * Execute event without runtime variables check (for internal use).
	 */
	executeEventDirect: (
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
	) => Promise<ILogMetadata | undefined>;
}

const ExecutionServiceContext = createContext<
	ExecutionServiceContextValue | undefined
>(undefined);

export function useExecutionService(): ExecutionServiceContextValue {
	const ctx = useContext(ExecutionServiceContext);
	if (!ctx) {
		throw new Error(
			"useExecutionService must be used within ExecutionServiceProvider",
		);
	}
	return ctx;
}

export function useExecutionServiceOptional():
	| ExecutionServiceContextValue
	| undefined {
	return useContext(ExecutionServiceContext);
}

interface ExecutionServiceProviderProps {
	children: ReactNode;
}

export function ExecutionServiceProvider({
	children,
}: ExecutionServiceProviderProps) {
	const backend = useBackend();
	const runtimeVarsContext = useRuntimeVariables();

	const [promptOpen, setPromptOpen] = useState(false);
	const [pendingExecution, setPendingExecution] =
		useState<PendingExecution | null>(null);
	const [runtimeConfiguredVars, setRuntimeConfiguredVars] = useState<
		IVariable[]
	>([]);
	const [existingRuntimeVars, setExistingRuntimeVars] = useState<
		Map<string, RuntimeVariableValue>
	>(new Map());

	// WASM consent dialog state
	const [wasmDialogOpen, setWasmDialogOpen] = useState(false);
	const [wasmPackageIds, setWasmPackageIds] = useState<string[]>([]);
	const [wasmPackagePermissions, setWasmPackagePermissions] = useState<
		Record<string, string[]>
	>({});
	const [wasmConsentResolve, setWasmConsentResolve] = useState<
		((granted: boolean) => void) | null
	>(null);
	const [pendingWasmBoardId, setPendingWasmBoardId] = useState<string>("");
	const [pendingWasmEventId, setPendingWasmEventId] = useState<string>("");

	const convertToRuntimeVariablesMap = useCallback(
		async (
			appId: string,
			runtimeVars: IVariable[],
			includeSecrets: boolean,
		): Promise<Record<string, IVariable> | undefined> => {
			if (!runtimeVarsContext || runtimeVars.length === 0) return undefined;

			const storedValues = await runtimeVarsContext.getValues(appId);
			const result: Record<string, IVariable> = {};

			for (const variable of runtimeVars) {
				// For remote execution, skip secrets
				if (!includeSecrets && variable.secret) continue;

				const storedValue = storedValues.get(variable.id);
				if (storedValue?.value !== undefined) {
					// storedValue.value is already in the correct format (number[] representing JSON-encoded bytes)
					result[variable.id] = {
						...variable,
						default_value: storedValue.value,
					};
				}
			}

			return Object.keys(result).length > 0 ? result : undefined;
		},
		[runtimeVarsContext],
	);

	/**
	 * Get variables that need to be prompted based on execution context.
	 * For local execution: prompt for missing runtime_configured vars AND missing secrets
	 * For remote execution: only prompt for missing runtime_configured vars (secrets never sent)
	 */
	const getVariablesNeedingPrompt = useCallback(
		(board: IBoard, isRemote: boolean): IVariable[] => {
			const executionMode = board.execution_mode ?? IExecutionMode.Hybrid;
			const isLocalExecution =
				!isRemote && executionMode !== IExecutionMode.Remote;

			return Object.values(board.variables).filter((v) => {
				if (v.runtime_configured) return true;
				// Only include secrets for local execution
				if (v.secret && isLocalExecution) return true;
				return false;
			});
		},
		[],
	);

	/**
	 * Convert IRuntimeVariable from prerun endpoint to IVariable format for the prompt.
	 */
	const convertPrerunToVariables = useCallback(
		(prerunVars: IRuntimeVariable[], isRemote: boolean): IVariable[] => {
			return prerunVars
				.filter((v) => {
					// For remote execution, skip secrets (they can't be sent to remote)
					if (isRemote && v.secret) return false;
					return true;
				})
				.map((v) => ({
					id: v.id,
					name: v.name,
					description: v.description ?? null,
					data_type: v.data_type as IVariable["data_type"],
					value_type: v.value_type as IVariable["value_type"],
					secret: v.secret,
					runtime_configured: true,
					default_value: null,
					schema: v.schema ?? null,
					editable: true,
					exposed: false,
				}));
		},
		[],
	);

	/**
	 * Shows the WASM sandbox warning if the board has WASM nodes and consent hasn't been saved.
	 * Returns true if execution should proceed, false if cancelled.
	 */
	const checkWasmConsent = useCallback(
		(
			packageIds: string[],
			boardId: string,
			eventId: string,
			permissions?: Record<string, string[]>,
		): Promise<boolean> => {
			if (
				packageIds.length === 0 ||
				hasWasmConsent("board", boardId) ||
				(eventId && hasWasmConsent("event", eventId)) ||
				allPackagesTrusted(packageIds)
			) {
				return Promise.resolve(true);
			}

			return new Promise((resolve) => {
				setWasmPackageIds(packageIds);
				setWasmPackagePermissions(permissions ?? {});
				setPendingWasmBoardId(boardId);
				setPendingWasmEventId(eventId);
				setWasmConsentResolve(() => resolve);
				setWasmDialogOpen(true);
			});
		},
		[],
	);

	const handleWasmConfirm = useCallback(
		(rememberFor: "none" | "board" | "event" | "package") => {
			if (rememberFor === "package") {
				for (const id of wasmPackageIds) {
					saveWasmConsent("package", id);
				}
			} else if (rememberFor === "board" && pendingWasmBoardId) {
				saveWasmConsent("board", pendingWasmBoardId);
			} else if (rememberFor === "event" && pendingWasmEventId) {
				saveWasmConsent("event", pendingWasmEventId);
			}
			setWasmDialogOpen(false);
			wasmConsentResolve?.(true);
			setWasmConsentResolve(null);
		},
		[
			wasmConsentResolve,
			pendingWasmBoardId,
			pendingWasmEventId,
			wasmPackageIds,
		],
	);

	const handleWasmCancel = useCallback(() => {
		setWasmDialogOpen(false);
		wasmConsentResolve?.(false);
		setWasmConsentResolve(null);
	}, [wasmConsentResolve]);

	const checkAndExecute = useCallback(
		async (
			appId: string,
			boardId: string,
			payload: IRunPayload,
			streamState: boolean | undefined,
			eventId: ((id: string) => void) | undefined,
			cb: ((event: IIntercomEvent[]) => void) | undefined,
			skipConsentCheck: boolean | undefined,
			isRemote: boolean,
		): Promise<ILogMetadata | undefined> => {
			// Run WASM consent check first (independent of runtime vars).
			// Fetch prerun once and reuse the result for runtime vars later.
			let prerunResult: Awaited<
				ReturnType<NonNullable<typeof backend.boardState.prerunBoard>>
			> | null = null;

			if (backend.boardState.prerunBoard) {
				try {
					prerunResult = await backend.boardState.prerunBoard(appId, boardId);

					if (
						prerunResult.has_wasm_nodes &&
						prerunResult.wasm_package_ids?.length
					) {
						const granted = await checkWasmConsent(
							prerunResult.wasm_package_ids,
							boardId,
							"",
							prerunResult.wasm_package_permissions,
						);
						if (!granted) return undefined;
					}
				} catch {
					// Prerun failed — continue without it; WASM guard in Rust is the final safety net
				}
			}

			// If no runtime vars context, execute directly
			if (!runtimeVarsContext) {
				if (isRemote && backend.boardState.executeBoardRemote) {
					return backend.boardState.executeBoardRemote(
						appId,
						boardId,
						payload,
						streamState,
						eventId,
						cb,
					);
				}
				return backend.boardState.executeBoard(
					appId,
					boardId,
					payload,
					streamState,
					eventId,
					cb,
					skipConsentCheck,
				);
			}

			// Determine execution mode from the board/prerun and override isRemote if needed
			let varsNeedingValues: IVariable[];
			let effectiveIsRemote = isRemote;

			if (prerunResult) {
				// Force remote when board's execution_mode is Remote
				if (
					prerunResult.execution_mode === IExecutionMode.Remote &&
					backend.boardState.executeBoardRemote
				) {
					effectiveIsRemote = true;
				}

				if (effectiveIsRemote) {
					varsNeedingValues = convertPrerunToVariables(
						prerunResult.runtime_variables,
						effectiveIsRemote,
					);
				} else {
					// Local execution - use local board for full variable info (includes secrets)
					try {
						const board = await backend.boardState.getBoard(appId, boardId);
						varsNeedingValues = getVariablesNeedingPrompt(
							board,
							effectiveIsRemote,
						);
					} catch {
						varsNeedingValues = convertPrerunToVariables(
							prerunResult.runtime_variables,
							effectiveIsRemote,
						);
					}
				}
			} else if (backend.boardState.prerunBoard) {
				// prerunBoard exists but the earlier call failed — fall back to local board
				try {
					const board = await backend.boardState.getBoard(appId, boardId);
					const executionMode = board.execution_mode ?? IExecutionMode.Hybrid;
					if (
						executionMode === IExecutionMode.Remote &&
						backend.boardState.executeBoardRemote
					) {
						effectiveIsRemote = true;
					}
					varsNeedingValues = getVariablesNeedingPrompt(
						board,
						effectiveIsRemote,
					);
				} catch {
					// Board not found either, execute anyway
					if (effectiveIsRemote && backend.boardState.executeBoardRemote) {
						return backend.boardState.executeBoardRemote(
							appId,
							boardId,
							payload,
							streamState,
							eventId,
							cb,
						);
					}
					return backend.boardState.executeBoard(
						appId,
						boardId,
						payload,
						streamState,
						eventId,
						cb,
						skipConsentCheck,
					);
				}
			} else {
				// prerunBoard not available - use getBoard
				try {
					const board = await backend.boardState.getBoard(appId, boardId);
					const executionMode = board.execution_mode ?? IExecutionMode.Hybrid;
					if (
						executionMode === IExecutionMode.Remote &&
						backend.boardState.executeBoardRemote
					) {
						effectiveIsRemote = true;
					}
					varsNeedingValues = getVariablesNeedingPrompt(
						board,
						effectiveIsRemote,
					);
				} catch {
					// Board not found, execute anyway
					if (effectiveIsRemote && backend.boardState.executeBoardRemote) {
						return backend.boardState.executeBoardRemote(
							appId,
							boardId,
							payload,
							streamState,
							eventId,
							cb,
						);
					}
					return backend.boardState.executeBoard(
						appId,
						boardId,
						payload,
						streamState,
						eventId,
						cb,
						skipConsentCheck,
					);
				}
			}

			if (varsNeedingValues.length === 0) {
				// No runtime-configured variables needed, execute directly
				if (effectiveIsRemote && backend.boardState.executeBoardRemote) {
					return backend.boardState.executeBoardRemote(
						appId,
						boardId,
						payload,
						streamState,
						eventId,
						cb,
					);
				}
				return backend.boardState.executeBoard(
					appId,
					boardId,
					payload,
					streamState,
					eventId,
					cb,
					skipConsentCheck,
				);
			}

			// Check if all needed variables are configured
			const variableIds = varsNeedingValues.map((v) => v.id);
			const hasAll = await runtimeVarsContext.hasAllValues(appId, variableIds);

			// For local execution, include secrets; for remote, exclude them
			const includeSecrets = !effectiveIsRemote;

			if (hasAll) {
				// All variables configured, convert to runtime variables map and execute
				const runtimeVariablesMap = await convertToRuntimeVariablesMap(
					appId,
					varsNeedingValues,
					includeSecrets,
				);
				const payloadWithVars: IRunPayload = {
					...payload,
					runtime_variables: runtimeVariablesMap,
				};
				if (effectiveIsRemote && backend.boardState.executeBoardRemote) {
					return backend.boardState.executeBoardRemote(
						appId,
						boardId,
						payloadWithVars,
						streamState,
						eventId,
						cb,
					);
				}
				return backend.boardState.executeBoard(
					appId,
					boardId,
					payloadWithVars,
					streamState,
					eventId,
					cb,
					skipConsentCheck,
				);
			}

			// Need to prompt for runtime variables
			const existingValues = await runtimeVarsContext.getValues(appId);

			return new Promise((resolve, reject) => {
				setRuntimeConfiguredVars(varsNeedingValues);
				setExistingRuntimeVars(existingValues);
				setPendingExecution({
					appId,
					boardId,
					payload,
					streamState,
					eventId,
					cb,
					skipConsentCheck,
					isRemote: effectiveIsRemote,
					isEvent: false,
					resolve,
					reject,
				});
				setPromptOpen(true);
			});
		},
		[
			backend.boardState,
			runtimeVarsContext,
			convertToRuntimeVariablesMap,
			getVariablesNeedingPrompt,
			convertPrerunToVariables,
			checkWasmConsent,
		],
	);

	const checkAndExecuteEvent = useCallback(
		async (
			appId: string,
			eventIdStr: string,
			payload: IRunPayload,
			streamState: boolean | undefined,
			onEventId: ((id: string) => void) | undefined,
			cb: ((event: IIntercomEvent[]) => void) | undefined,
			skipConsentCheck: boolean | undefined,
		): Promise<ILogMetadata | undefined> => {
			// Run WASM consent check first (independent of runtime vars).
			// Fetch prerun once and reuse the result for runtime vars later.
			let prerunResult: Awaited<
				ReturnType<NonNullable<typeof backend.eventState.prerunEvent>>
			> | null = null;

			if (backend.eventState.prerunEvent) {
				try {
					prerunResult = await backend.eventState.prerunEvent(
						appId,
						eventIdStr,
					);

					if (
						prerunResult.has_wasm_nodes &&
						prerunResult.wasm_package_ids?.length
					) {
						const granted = await checkWasmConsent(
							prerunResult.wasm_package_ids,
							prerunResult.board_id,
							eventIdStr,
							prerunResult.wasm_package_permissions,
						);
						if (!granted) return undefined;
					}
				} catch {
					// Prerun failed — continue without it; WASM guard in Rust is the final safety net
				}
			}

			// If no runtime vars context, execute directly
			if (!runtimeVarsContext) {
				return backend.eventState.executeEvent(
					appId,
					eventIdStr,
					payload,
					streamState,
					onEventId,
					cb,
					skipConsentCheck,
				);
			}

			// Try prerunEvent result if available, otherwise fall back to fetching event + board
			let varsNeedingValues: IVariable[];
			let boardId: string;
			const backendAlwaysRemote = backend.eventState.alwaysRemote === true;
			let isRemote = backendAlwaysRemote;

			if (prerunResult) {
				boardId = prerunResult.board_id;
				isRemote =
					backendAlwaysRemote ||
					!prerunResult.can_execute_locally ||
					prerunResult.execution_mode === IExecutionMode.Remote;

				varsNeedingValues = convertPrerunToVariables(
					prerunResult.runtime_variables,
					isRemote,
				);

				if (!isRemote) {
					try {
						const board = await backend.boardState.getBoard(appId, boardId);
						varsNeedingValues = getVariablesNeedingPrompt(board, false);
					} catch {
						// Fall back to prerun variables if board fetch fails
					}
				}
			} else if (backend.eventState.prerunEvent) {
				// prerunEvent exists but the earlier call failed — fall back to event + board
				try {
					const event = await backend.eventState.getEvent(appId, eventIdStr);
					boardId = event.board_id;
					const version = event.board_version as
						| [number, number, number]
						| undefined;
					const board = await backend.boardState.getBoard(
						appId,
						event.board_id,
						version ?? undefined,
					);
					const executionMode = board.execution_mode ?? IExecutionMode.Hybrid;
					isRemote =
						backendAlwaysRemote || executionMode === IExecutionMode.Remote;
					varsNeedingValues = getVariablesNeedingPrompt(board, isRemote);
				} catch {
					// Event or board not found, execute anyway
					return backend.eventState.executeEvent(
						appId,
						eventIdStr,
						payload,
						streamState,
						onEventId,
						cb,
						skipConsentCheck,
					);
				}
			} else {
				// prerunEvent not available, use traditional approach
				try {
					const event = await backend.eventState.getEvent(appId, eventIdStr);
					boardId = event.board_id;
					const version = event.board_version as
						| [number, number, number]
						| undefined;
					const board = await backend.boardState.getBoard(
						appId,
						event.board_id,
						version ?? undefined,
					);
					const executionMode = board.execution_mode ?? IExecutionMode.Hybrid;
					isRemote =
						backendAlwaysRemote || executionMode === IExecutionMode.Remote;
					varsNeedingValues = getVariablesNeedingPrompt(board, isRemote);
				} catch {
					// Event or board not found, execute anyway
					return backend.eventState.executeEvent(
						appId,
						eventIdStr,
						payload,
						streamState,
						onEventId,
						cb,
						skipConsentCheck,
					);
				}
			}

			if (varsNeedingValues.length === 0) {
				// No runtime-configured variables, execute directly
				if (isRemote && backend.eventState.executeEventRemote) {
					return backend.eventState.executeEventRemote(
						appId,
						eventIdStr,
						payload,
						streamState,
						onEventId,
						cb,
					);
				}
				return backend.eventState.executeEvent(
					appId,
					eventIdStr,
					payload,
					streamState,
					onEventId,
					cb,
					skipConsentCheck,
				);
			}

			// Check if all runtime variables are configured
			const variableIds = varsNeedingValues.map((v) => v.id);
			const hasAll = await runtimeVarsContext.hasAllValues(appId, variableIds);

			if (hasAll) {
				// All variables configured, convert to runtime variables map and execute
				// Only include secrets for local execution
				const includeSecrets = !isRemote;
				const runtimeVariablesMap = await convertToRuntimeVariablesMap(
					appId,
					varsNeedingValues,
					includeSecrets,
				);
				const payloadWithVars: IRunPayload = {
					...payload,
					runtime_variables: runtimeVariablesMap,
				};
				if (isRemote && backend.eventState.executeEventRemote) {
					return backend.eventState.executeEventRemote(
						appId,
						eventIdStr,
						payloadWithVars,
						streamState,
						onEventId,
						cb,
					);
				}
				return backend.eventState.executeEvent(
					appId,
					eventIdStr,
					payloadWithVars,
					streamState,
					onEventId,
					cb,
					skipConsentCheck,
				);
			}

			// Need to prompt for runtime variables
			const existingValues = await runtimeVarsContext.getValues(appId);

			return new Promise((resolve, reject) => {
				setRuntimeConfiguredVars(varsNeedingValues);
				setExistingRuntimeVars(existingValues);
				setPendingExecution({
					appId,
					boardId,
					payload,
					streamState,
					eventId: onEventId,
					cb,
					skipConsentCheck,
					isRemote,
					isEvent: true,
					eventIdStr,
					resolve,
					reject,
				});
				setPromptOpen(true);
			});
		},
		[
			backend.eventState,
			backend.boardState,
			runtimeVarsContext,
			convertToRuntimeVariablesMap,
			getVariablesNeedingPrompt,
			convertPrerunToVariables,
			checkWasmConsent,
		],
	);

	const executeBoard = useCallback(
		(
			appId: string,
			boardId: string,
			payload: IRunPayload,
			streamState?: boolean,
			eventId?: (id: string) => void,
			cb?: (event: IIntercomEvent[]) => void,
			skipConsentCheck?: boolean,
		) =>
			checkAndExecute(
				appId,
				boardId,
				payload,
				streamState,
				eventId,
				cb,
				skipConsentCheck,
				false,
			),
		[checkAndExecute],
	);

	const executeBoardRemote = useCallback(
		(
			appId: string,
			boardId: string,
			payload: IRunPayload,
			streamState?: boolean,
			eventId?: (id: string) => void,
			cb?: (event: IIntercomEvent[]) => void,
		) =>
			checkAndExecute(
				appId,
				boardId,
				payload,
				streamState,
				eventId,
				cb,
				undefined,
				true,
			),
		[checkAndExecute],
	);

	const executeBoardDirect = useCallback(
		(
			appId: string,
			boardId: string,
			payload: IRunPayload,
			streamState?: boolean,
			eventId?: (id: string) => void,
			cb?: (event: IIntercomEvent[]) => void,
			skipConsentCheck?: boolean,
		) =>
			backend.boardState.executeBoard(
				appId,
				boardId,
				payload,
				streamState,
				eventId,
				cb,
				skipConsentCheck,
			),
		[backend.boardState],
	);

	const executeEvent = useCallback(
		(
			appId: string,
			eventIdStr: string,
			payload: IRunPayload,
			streamState?: boolean,
			onEventId?: (id: string) => void,
			cb?: (event: IIntercomEvent[]) => void,
			skipConsentCheck?: boolean,
		) =>
			checkAndExecuteEvent(
				appId,
				eventIdStr,
				payload,
				streamState,
				onEventId,
				cb,
				skipConsentCheck,
			),
		[checkAndExecuteEvent],
	);

	const executeEventDirect = useCallback(
		(
			appId: string,
			eventIdStr: string,
			payload: IRunPayload,
			streamState?: boolean,
			onEventId?: (id: string) => void,
			cb?: (event: IIntercomEvent[]) => void,
			skipConsentCheck?: boolean,
		) =>
			backend.eventState.executeEvent(
				appId,
				eventIdStr,
				payload,
				streamState,
				onEventId,
				cb,
				skipConsentCheck,
			),
		[backend.eventState],
	);

	const handleSave = useCallback(
		async (values: RuntimeVariableValue[]) => {
			if (!pendingExecution || !runtimeVarsContext) return;

			const {
				appId,
				boardId,
				payload,
				streamState,
				eventId,
				cb,
				skipConsentCheck,
				isRemote,
				isEvent,
				eventIdStr,
				resolve,
				reject,
			} = pendingExecution;

			try {
				// Save the runtime variable values
				const saveValues = values.map((v) => {
					const variable = runtimeConfiguredVars.find(
						(rv) => rv.id === v.variableId,
					);
					return {
						variableId: v.variableId,
						variableName: variable?.name || "",
						value: v.value,
						isSecret: variable?.secret || false,
					};
				});

				await runtimeVarsContext.saveValues(appId, boardId, saveValues);

				// Build the runtime variables map from the just-saved values
				// For remote execution, filter out secrets
				const includeSecrets = !isRemote;
				const runtimeVariablesMap: Record<string, IVariable> = {};

				for (const v of values) {
					const variable = runtimeConfiguredVars.find(
						(rv) => rv.id === v.variableId,
					);
					if (variable) {
						// Skip secrets for remote execution
						if (!includeSecrets && variable.secret) continue;

						// v.value is already a number[] (byte array)
						runtimeVariablesMap[variable.id] = {
							...variable,
							default_value: v.value,
						};
					}
				}

				// Close the prompt
				setPromptOpen(false);
				setPendingExecution(null);

				// Execute with runtime variables in the payload
				let result: ILogMetadata | undefined;
				const varsMap =
					Object.keys(runtimeVariablesMap).length > 0
						? runtimeVariablesMap
						: undefined;
				const payloadWithVars: IRunPayload = {
					...payload,
					runtime_variables: varsMap,
				};

				if (isEvent && eventIdStr) {
					if (isRemote && backend.eventState.executeEventRemote) {
						result = await backend.eventState.executeEventRemote(
							appId,
							eventIdStr,
							payloadWithVars,
							streamState,
							eventId,
							cb,
						);
					} else {
						result = await backend.eventState.executeEvent(
							appId,
							eventIdStr,
							payloadWithVars,
							streamState,
							eventId,
							cb,
							skipConsentCheck,
						);
					}
				} else if (isRemote && backend.boardState.executeBoardRemote) {
					result = await backend.boardState.executeBoardRemote(
						appId,
						boardId,
						payloadWithVars,
						streamState,
						eventId,
						cb,
					);
				} else {
					result = await backend.boardState.executeBoard(
						appId,
						boardId,
						payloadWithVars,
						streamState,
						eventId,
						cb,
						skipConsentCheck,
					);
				}

				resolve(result);
			} catch (error) {
				reject(error instanceof Error ? error : new Error(String(error)));
			}
		},
		[
			pendingExecution,
			runtimeVarsContext,
			runtimeConfiguredVars,
			backend.boardState,
			backend.eventState,
		],
	);

	const handleCancel = useCallback(() => {
		if (pendingExecution) {
			pendingExecution.reject(
				new Error("Execution cancelled: runtime variables not configured"),
			);
		}
		setPromptOpen(false);
		setPendingExecution(null);
	}, [pendingExecution]);

	const contextValue = useMemo(
		() => ({
			executeBoard,
			executeBoardRemote,
			executeBoardDirect,
			executeEvent,
			executeEventDirect,
		}),
		[
			executeBoard,
			executeBoardRemote,
			executeBoardDirect,
			executeEvent,
			executeEventDirect,
		],
	);

	return (
		<ExecutionServiceContext.Provider value={contextValue}>
			{children}
			<RuntimeVariablesPrompt
				open={promptOpen}
				onOpenChange={setPromptOpen}
				variables={runtimeConfiguredVars}
				existingValues={existingRuntimeVars}
				onSave={handleSave}
				onCancel={handleCancel}
			/>
			<WasmSandboxWarningDialog
				open={wasmDialogOpen}
				packageIds={wasmPackageIds}
				packagePermissions={wasmPackagePermissions}
				onConfirm={handleWasmConfirm}
				onCancel={handleWasmCancel}
			/>
		</ExecutionServiceContext.Provider>
	);
}
