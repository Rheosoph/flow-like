"use client";

import {
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { RuntimeVariablesPrompt } from "../components/flow/runtime-variables-prompt";
import {
	type RememberChoice,
	WasmSandboxWarningDialog,
} from "../components/flow/wasm-sandbox-warning-dialog";
import { NodePaymentPrompt } from "../components/payments/node-payment";
import type { IIntercomEvent, ILogMetadata, IRunPayload } from "../lib";
import { asArray } from "../lib/response-shape";
import { recordRunStep, runTimingNow, timeRunStep } from "../lib/run-timing";
import type { IBoard, IVariable } from "../lib/schema/flow/board";
import { IExecutionMode } from "../lib/schema/flow/board";
import { normalizeBoardVersion } from "../lib/schema/flow/board-version";
import type { IEvent } from "../lib/schema/flow/event";
import { IEventExecutionMode } from "../lib/schema/flow/event";
import type { PageTrigger } from "../lib/schema/flow/page-trigger";
import { useBackend } from "./backend-state";
import {
	prerunBoardKey,
	prerunEventKey,
	prerunSwr,
} from "./backend-state/prerun-cache";
import type { IRuntimeVariable } from "./backend-state/types";
import { ExecutionServiceContext } from "./execution-service-context-value";
import {
	type RuntimeVariableValue,
	type RuntimeVariablesContextValue,
	useRuntimeVariables,
} from "./runtime-variables-context";

const DRIFT_TOAST_THROTTLE_MS = 30_000;
const recentDriftToasts = new Map<string, number>();
function notifyPrerunDrift(key: string): void {
	const now = Date.now();
	const last = recentDriftToasts.get(key) ?? 0;
	if (now - last < DRIFT_TOAST_THROTTLE_MS) return;
	recentDriftToasts.set(key, now);
	toast.warning("Workflow changed", {
		description:
			"The workflow has been updated since you opened it. Reload to use the latest version.",
	});
}

const RUNTIME_VARIABLES_CANCELLED =
	"Execution cancelled: runtime variables not configured";
const PROVIDER_UNMOUNTED =
	"Execution cancelled: the execution service unmounted while the run waited for a prompt";
const NO_VARIABLES: IVariable[] = [];
const NO_VALUES = new Map<string, RuntimeVariableValue>();
const NO_PACKAGES: string[] = [];

interface RuntimeVariablesRequest {
	readonly appId: string;
	readonly boardId: string;
	readonly variables: IVariable[];
	readonly isRemote: boolean;
	readonly beforeDispatch?: () => void;
}

interface QueuedPrompt {
	readonly id: number;
	readonly question: string;
}

interface RuntimeVariablesPromptRequest extends QueuedPrompt {
	readonly variables: IVariable[];
	readonly existingValues: Map<string, RuntimeVariableValue>;
	readonly submit: (values: RuntimeVariableValue[]) => Promise<void>;
	/** False when the prompt was already answered. */
	readonly cancel: (reason: Error) => boolean;
}

interface WasmConsentPromptRequest extends QueuedPrompt {
	readonly packageIds: string[];
	readonly packagePermissions: Record<string, string[]>;
	readonly confirm: (rememberFor: RememberChoice) => void;
	/** False when the prompt was already answered. */
	readonly cancel: () => boolean;
}

function toError(error: unknown): Error {
	return error instanceof Error ? error : new Error(String(error));
}

/** A prompt's answer: `claim` succeeds once, so a save, a cancel and an unmount cannot all settle it. */
function createPromptAnswer<T>() {
	let claimed = false;
	let resolve!: (value: T) => void;
	let reject!: (error: Error) => void;
	const promise = new Promise<T>((onResolve, onReject) => {
		resolve = onResolve;
		reject = onReject;
	});
	const claim = () => {
		if (claimed) return false;
		claimed = true;
		return true;
	};
	return { promise, claim, resolve, reject };
}

/** Runs prompt tasks one after another, in the order they were queued. */
function createPromptQueue() {
	let tail: Promise<unknown> = Promise.resolve();
	return function enqueue<T>(task: () => Promise<T>): Promise<T> {
		const turn = tail.then(task);
		tail = turn.catch(() => undefined);
		return turn;
	};
}

/**
 * One dialog serves every execution that needs it: requests wait for their turn, and the
 * dialog keeps showing the last request while it animates closed. Declining a question also
 * declines the requests already waiting with the same one — page interval ticks do not wait for
 * each other, so otherwise one dialog per tick would pile up behind the one on screen.
 */
function usePromptQueue<P extends QueuedPrompt>() {
	const [enqueue] = useState(createPromptQueue);
	const [slot, setSlot] = useState<{
		readonly prompt: P;
		readonly open: boolean;
	} | null>(null);
	const active = useRef<P | null>(null);
	const queued = useRef(0);
	const declinedThrough = useRef(new Map<string, number>());
	const ask = useCallback(
		<T,>(
			question: string,
			task: () => Promise<T>,
			declined: () => T,
		): Promise<T> => {
			const queuedAt = ++queued.current;
			return enqueue(async () =>
				(declinedThrough.current.get(question) ?? 0) >= queuedAt
					? declined()
					: task(),
			);
		},
		[enqueue],
	);
	const decline = useCallback((question: string) => {
		declinedThrough.current.set(question, queued.current);
	}, []);
	const present = useCallback(
		async <T,>(prompt: P, answer: Promise<T>): Promise<T> => {
			active.current = prompt;
			setSlot({ prompt, open: true });
			try {
				return await answer;
			} finally {
				if (active.current === prompt) active.current = null;
				setSlot((current) =>
					current?.prompt === prompt ? { prompt, open: false } : current,
				);
			}
		},
		[],
	);
	return { ask, decline, slot, active, present };
}

async function saveEnteredRuntimeVariables(
	request: RuntimeVariablesRequest,
	values: RuntimeVariableValue[],
	store: RuntimeVariablesContextValue,
): Promise<Record<string, IVariable> | undefined> {
	const { appId, boardId, variables, isRemote, beforeDispatch } = request;
	beforeDispatch?.();
	const saveValues = values.map((v) => {
		const variable = variables.find((rv) => rv.id === v.variableId);
		return {
			variableId: v.variableId,
			variableName: variable?.name || "",
			value: v.value,
			isSecret: variable?.secret || false,
		};
	});

	await timeRunStep("service.runtime_vars.save_values", () =>
		store.saveValues(appId, boardId, saveValues),
	);
	beforeDispatch?.();

	// Build the runtime variables map from the just-saved values
	// For remote execution, filter out secrets
	const runtimeVariablesMap: Record<string, IVariable> = {};
	for (const v of values) {
		const variable = variables.find((rv) => rv.id === v.variableId);
		if (!variable || (isRemote && variable.secret)) continue;
		// v.value is already a number[] (byte array)
		runtimeVariablesMap[variable.id] = {
			...variable,
			default_value: v.value,
		};
	}
	return Object.keys(runtimeVariablesMap).length > 0
		? runtimeVariablesMap
		: undefined;
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

function isWasmTrusted(
	packageIds: string[],
	boardId: string,
	eventId: string,
): boolean {
	return (
		packageIds.length === 0 ||
		hasWasmConsent("board", boardId) ||
		(eventId !== "" && hasWasmConsent("event", eventId)) ||
		allPackagesTrusted(packageIds)
	);
}

function rememberWasmConsent(
	rememberFor: RememberChoice,
	packageIds: string[],
	boardId: string,
	eventId: string,
): void {
	if (rememberFor === "package") {
		for (const id of packageIds) {
			saveWasmConsent("package", id);
		}
	} else if (rememberFor === "board" && boardId) {
		saveWasmConsent("board", boardId);
	} else if (rememberFor === "event" && eventId) {
		saveWasmConsent("event", eventId);
	}
}

interface ExecutionServiceProviderProps {
	children: ReactNode;
}

export function ExecutionServiceProvider({
	children,
}: ExecutionServiceProviderProps) {
	const backend = useBackend();
	const runtimeVarsContext = useRuntimeVariables();

	const {
		ask: askRuntimeVariables,
		decline: declineRuntimeVariables,
		present: presentRuntimePrompt,
		slot: runtimeSlot,
		active: runtimeActive,
	} = usePromptQueue<RuntimeVariablesPromptRequest>();
	const {
		ask: askWasmConsent,
		decline: declineWasmConsent,
		present: presentWasmPrompt,
		slot: wasmSlot,
		active: wasmActive,
	} = usePromptQueue<WasmConsentPromptRequest>();
	const promptIds = useRef(0);
	const mountedRef = useRef(false);
	const disposedRef = useRef(false);

	useEffect(() => {
		mountedRef.current = true;
		disposedRef.current = false;
		return () => {
			mountedRef.current = false;
			// Deferred so a StrictMode or Fast Refresh effect replay keeps its prompts.
			setTimeout(() => {
				if (mountedRef.current) return;
				disposedRef.current = true;
				runtimeActive.current?.cancel(new Error(PROVIDER_UNMOUNTED));
				wasmActive.current?.cancel();
			}, 0);
		};
	}, [runtimeActive, wasmActive]);

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
			// Read outside the prerun try: an older hub omitting the list must not abort the run.
			return asArray(prerunVars)
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
			if (isWasmTrusted(packageIds, boardId, eventId)) {
				return Promise.resolve(true);
			}

			const question = [boardId, eventId, ...packageIds].join("\n");
			return askWasmConsent(
				question,
				async () => {
					// The prompt ahead of this one may have trusted the same board or packages.
					if (isWasmTrusted(packageIds, boardId, eventId)) return true;
					if (disposedRef.current) return false;
					const answer = createPromptAnswer<boolean>();
					return presentWasmPrompt(
						{
							id: ++promptIds.current,
							question,
							packageIds,
							packagePermissions: permissions ?? {},
							confirm: (rememberFor) => {
								if (!answer.claim()) return;
								rememberWasmConsent(rememberFor, packageIds, boardId, eventId);
								answer.resolve(true);
							},
							cancel: () => {
								if (!answer.claim()) return false;
								answer.resolve(false);
								return true;
							},
						},
						answer.promise,
					);
				},
				() => false,
			);
		},
		[askWasmConsent, presentWasmPrompt],
	);

	const promptRuntimeVariables = useCallback(
		async (
			question: string,
			request: RuntimeVariablesRequest,
			store: RuntimeVariablesContextValue,
		): Promise<Record<string, IVariable> | undefined> => {
			const existingValues = await timeRunStep(
				"service.runtime_vars.get_values",
				() => store.getValues(request.appId),
			);
			request.beforeDispatch?.();
			if (disposedRef.current) throw new Error(PROVIDER_UNMOUNTED);

			const answer = createPromptAnswer<
				Record<string, IVariable> | undefined
			>();
			const openedAt = runTimingNow();
			const claim = () => {
				if (!answer.claim()) return false;
				recordRunStep("service.runtime_vars_prompt", openedAt);
				return true;
			};
			return presentRuntimePrompt(
				{
					id: ++promptIds.current,
					question,
					variables: request.variables,
					existingValues,
					submit: async (values) => {
						if (!claim()) return;
						try {
							answer.resolve(
								await saveEnteredRuntimeVariables(request, values, store),
							);
						} catch (error) {
							answer.reject(toError(error));
						}
					},
					cancel: (reason) => {
						if (!claim()) return false;
						answer.reject(reason);
						return true;
					},
				},
				answer.promise,
			);
		},
		[presentRuntimePrompt],
	);

	/**
	 * Stored values when every variable has one, otherwise a prompt. Prompts queue, so two runs
	 * that need input never share one dialog, and a queued run re-reads the store first: the
	 * prompt ahead of it may have saved exactly what it needs.
	 */
	const resolveRuntimeVariables = useCallback(
		async (
			request: RuntimeVariablesRequest,
			store: RuntimeVariablesContextValue,
		): Promise<Record<string, IVariable> | undefined> => {
			const { appId, variables, isRemote } = request;
			const variableIds = variables.map((v) => v.id);
			const readStored = async () => {
				const hasAll = await timeRunStep("service.runtime_vars.has_all", () =>
					store.hasAllValues(appId, variableIds),
				);
				if (!hasAll) return null;
				// Only include secrets for local execution
				return {
					values: await timeRunStep("service.runtime_vars.convert", () =>
						convertToRuntimeVariablesMap(appId, variables, !isRemote),
					),
				};
			};

			const stored = await readStored();
			if (stored) return stored.values;
			const question = [appId, ...variableIds].join("\n");
			return askRuntimeVariables(
				question,
				async () => {
					const storedMeanwhile = await readStored();
					if (storedMeanwhile) return storedMeanwhile.values;
					return promptRuntimeVariables(question, request, store);
				},
				() => {
					throw new Error(RUNTIME_VARIABLES_CANCELLED);
				},
			);
		},
		[convertToRuntimeVariablesMap, askRuntimeVariables, promptRuntimeVariables],
	);

	/**
	 * A board this device cannot read cannot host a local run either — a user who
	 * may run an event but not read its flow is the normal shape of a published
	 * app. Escalating to the server beats a guaranteed local failure, unless the
	 * app is local-only, where the missing board is the real answer.
	 */
	const canEscalateUnreadableBoard = useCallback(
		async (appId: string, hasRemoteExecutor: boolean): Promise<boolean> => {
			if (!hasRemoteExecutor) return false;
			if (backend.isLocalOnly) {
				return !(await backend.isLocalOnly(appId).catch(() => false));
			}
			return !(await backend.isOffline(appId).catch(() => true));
		},
		[backend],
	);

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
			const boardVersion = normalizeBoardVersion(payload.version);
			const executeBoardRemote = backend.boardState.executeBoardRemote;

			const dispatch = (
				remote: boolean,
				runPayload: IRunPayload,
			): Promise<ILogMetadata | undefined> =>
				remote && executeBoardRemote
					? executeBoardRemote.call(
							backend.boardState,
							appId,
							boardId,
							runPayload,
							streamState,
							eventId,
							cb,
						)
					: backend.boardState.executeBoard(
							appId,
							boardId,
							runPayload,
							streamState,
							eventId,
							cb,
							skipConsentCheck,
						);

			// Run WASM consent check first (independent of runtime vars).
			// Fetch prerun once and reuse the result for runtime vars later.
			let prerunResult: Awaited<
				ReturnType<NonNullable<typeof backend.boardState.prerunBoard>>
			> | null = null;

			if (backend.boardState.prerunBoard) {
				try {
					const fetchPrerun = backend.boardState.prerunBoard;
					prerunResult = await prerunSwr(
						prerunBoardKey(appId, boardId, boardVersion),
						() => fetchPrerun(appId, boardId, boardVersion),
						{ onDrift: (key) => notifyPrerunDrift(key) },
					);

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
				return dispatch(isRemote, payload);
			}

			// Determine execution mode from the board/prerun and override isRemote if needed
			let varsNeedingValues: IVariable[];
			let effectiveIsRemote = isRemote;

			if (prerunResult) {
				// Force remote when the board is pinned to Remote or this device
				// is not allowed to read it
				effectiveIsRemote =
					effectiveIsRemote ||
					!prerunResult.can_execute_locally ||
					prerunResult.execution_mode === IExecutionMode.Remote;

				if (effectiveIsRemote) {
					varsNeedingValues = convertPrerunToVariables(
						prerunResult.runtime_variables,
						effectiveIsRemote,
					);
				} else {
					// Local execution - use local board for full variable info (includes secrets)
					try {
						const board = await backend.boardState.getBoard(
							appId,
							boardId,
							boardVersion,
						);
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
			} else {
				// Prerun is unavailable or failed — read the board itself
				try {
					const board = await backend.boardState.getBoard(
						appId,
						boardId,
						boardVersion,
					);
					const executionMode = board.execution_mode ?? IExecutionMode.Hybrid;
					effectiveIsRemote =
						effectiveIsRemote || executionMode === IExecutionMode.Remote;
					varsNeedingValues = getVariablesNeedingPrompt(
						board,
						effectiveIsRemote,
					);
				} catch (error) {
					if (!effectiveIsRemote) {
						effectiveIsRemote = await canEscalateUnreadableBoard(
							appId,
							Boolean(executeBoardRemote),
						);
						if (!effectiveIsRemote) throw error;
					}
					varsNeedingValues = [];
				}
			}

			if (varsNeedingValues.length === 0) {
				// No runtime-configured variables needed, execute directly
				return dispatch(effectiveIsRemote, payload);
			}

			const runtimeVariables = await resolveRuntimeVariables(
				{
					appId,
					boardId,
					variables: varsNeedingValues,
					isRemote: effectiveIsRemote,
				},
				runtimeVarsContext,
			);
			return dispatch(effectiveIsRemote, {
				...payload,
				runtime_variables: runtimeVariables,
			});
		},
		[
			backend,
			runtimeVarsContext,
			getVariablesNeedingPrompt,
			convertPrerunToVariables,
			checkWasmConsent,
			canEscalateUnreadableBoard,
			resolveRuntimeVariables,
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
			pageTrigger: PageTrigger | undefined,
			beforeDispatch: (() => void) | undefined,
		): Promise<ILogMetadata | undefined> => {
			beforeDispatch?.();
			const backendAlwaysRemote = backend.eventState.alwaysRemote === true;
			const executeEventRemote = backend.eventState.executeEventRemote;

			const dispatch = (
				isRemote: boolean,
				runPayload: IRunPayload,
			): Promise<ILogMetadata | undefined> => {
				beforeDispatch?.();
				return isRemote && executeEventRemote
					? executeEventRemote.call(
							backend.eventState,
							appId,
							eventIdStr,
							runPayload,
							streamState,
							onEventId,
							cb,
							pageTrigger,
							beforeDispatch,
						)
					: backend.eventState.executeEvent(
							appId,
							eventIdStr,
							runPayload,
							streamState,
							onEventId,
							cb,
							skipConsentCheck,
							pageTrigger,
							beforeDispatch,
						);
			};

			// Run WASM consent check first (independent of runtime vars).
			// Fetch prerun once and reuse the result for runtime vars later.
			let prerunResult: Awaited<
				ReturnType<NonNullable<typeof backend.eventState.prerunEvent>>
			> | null = null;

			if (backend.eventState.prerunEvent) {
				try {
					const fetchPrerun = backend.eventState.prerunEvent.bind(
						backend.eventState,
					);
					prerunResult = pageTrigger
						? await timeRunStep("service.prerun", () =>
								fetchPrerun(appId, eventIdStr, undefined, pageTrigger),
							)
						: await timeRunStep("service.prerun", () =>
								prerunSwr(
									prerunEventKey(appId, eventIdStr),
									() => fetchPrerun(appId, eventIdStr),
									{ onDrift: (key) => notifyPrerunDrift(key) },
								),
							);

					if (
						prerunResult.has_wasm_nodes &&
						prerunResult.wasm_package_ids?.length
					) {
						const packageIds = prerunResult.wasm_package_ids;
						const { board_id, wasm_package_permissions } = prerunResult;
						const granted = await timeRunStep("service.wasm_consent", () =>
							checkWasmConsent(
								packageIds,
								board_id,
								eventIdStr,
								wasm_package_permissions,
							),
						);
						if (!granted) return undefined;
					}
				} catch {
					// Prerun failed — continue without it; WASM guard in Rust is the final safety net
				}
			}

			// If no runtime vars context, execute directly
			beforeDispatch?.();
			if (!runtimeVarsContext) {
				return backend.eventState.executeEvent(
					appId,
					eventIdStr,
					payload,
					streamState,
					onEventId,
					cb,
					skipConsentCheck,
					pageTrigger,
					beforeDispatch,
				);
			}

			// Try prerunEvent result if available, otherwise fall back to fetching event + board
			let varsNeedingValues: IVariable[];
			let boardId: string;
			let isRemote = backendAlwaysRemote;

			if (prerunResult) {
				boardId = prerunResult.board_id;
				isRemote =
					backendAlwaysRemote ||
					!prerunResult.can_execute_locally ||
					prerunResult.execution_mode === IExecutionMode.Remote ||
					prerunResult.event_execution_mode === IEventExecutionMode.Remote;

				varsNeedingValues = convertPrerunToVariables(
					prerunResult.runtime_variables,
					isRemote,
				);

				if (!isRemote) {
					try {
						const event = await timeRunStep("service.get_event", () =>
							backend.eventState.getEvent(appId, eventIdStr),
						);
						const board = await timeRunStep("service.get_board", () =>
							backend.boardState.getBoard(
								appId,
								boardId,
								normalizeBoardVersion(event.board_version),
							),
						);
						varsNeedingValues = getVariablesNeedingPrompt(board, false);
					} catch {
						// Fall back to prerun variables if board fetch fails
					}
				}
			} else if (pageTrigger) {
				// A governed Page action must not fall back to reading its backing
				// Event or Board. The invoke endpoint remains the authority when
				// prerun is temporarily unavailable.
				return dispatch(true, payload);
			} else {
				// Prerun is unavailable or failed. The event record alone already says
				// where the run belongs, so it is read before the board: an event pinned
				// to Remote has no board on this device, and loading one would turn a
				// server-side run into a "board not found" failure.
				let event: IEvent;
				try {
					event = await timeRunStep("service.get_event", () =>
						backend.eventState.getEvent(appId, eventIdStr),
					);
				} catch {
					return dispatch(isRemote, payload);
				}

				boardId = event.board_id;
				isRemote =
					backendAlwaysRemote ||
					event.execution_mode === IEventExecutionMode.Remote;

				try {
					const board = await timeRunStep("service.get_board", () =>
						backend.boardState.getBoard(
							appId,
							event.board_id,
							normalizeBoardVersion(event.board_version) ?? undefined,
						),
					);
					const executionMode = board.execution_mode ?? IExecutionMode.Hybrid;
					isRemote = isRemote || executionMode === IExecutionMode.Remote;
					varsNeedingValues = getVariablesNeedingPrompt(board, isRemote);
				} catch (error) {
					if (!isRemote) {
						isRemote = await timeRunStep("service.can_escalate", () =>
							canEscalateUnreadableBoard(appId, Boolean(executeEventRemote)),
						);
						if (!isRemote) throw error;
					}
					varsNeedingValues = [];
				}
			}

			if (varsNeedingValues.length === 0) {
				// No runtime-configured variables, execute directly
				return dispatch(isRemote, payload);
			}

			const runtimeVariables = await resolveRuntimeVariables(
				{
					appId,
					boardId,
					variables: varsNeedingValues,
					isRemote,
					beforeDispatch,
				},
				runtimeVarsContext,
			);
			return dispatch(isRemote, {
				...payload,
				runtime_variables: runtimeVariables,
			});
		},
		[
			backend,
			runtimeVarsContext,
			getVariablesNeedingPrompt,
			convertPrerunToVariables,
			checkWasmConsent,
			canEscalateUnreadableBoard,
			resolveRuntimeVariables,
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
			pageTrigger?: PageTrigger,
			beforeDispatch?: () => void,
		) =>
			checkAndExecuteEvent(
				appId,
				eventIdStr,
				payload,
				streamState,
				onEventId,
				cb,
				skipConsentCheck,
				pageTrigger,
				beforeDispatch,
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
			pageTrigger?: PageTrigger,
			beforeDispatch?: () => void,
		) => {
			beforeDispatch?.();
			return backend.eventState.executeEvent(
				appId,
				eventIdStr,
				payload,
				streamState,
				onEventId,
				cb,
				skipConsentCheck,
				pageTrigger,
				beforeDispatch,
			);
		},
		[backend.eventState],
	);

	const handleRuntimeSave = useCallback(
		async (values: RuntimeVariableValue[]) => {
			await runtimeSlot?.prompt.submit(values);
		},
		[runtimeSlot],
	);
	const handleRuntimeCancel = useCallback(() => {
		const prompt = runtimeSlot?.prompt;
		if (prompt?.cancel(new Error(RUNTIME_VARIABLES_CANCELLED)))
			declineRuntimeVariables(prompt.question);
	}, [runtimeSlot, declineRuntimeVariables]);
	const handleRuntimeOpenChange = useCallback(
		(open: boolean) => {
			if (!open) handleRuntimeCancel();
		},
		[handleRuntimeCancel],
	);

	const handleWasmConfirm = useCallback(
		(rememberFor: RememberChoice) => wasmSlot?.prompt.confirm(rememberFor),
		[wasmSlot],
	);
	const handleWasmCancel = useCallback(() => {
		const prompt = wasmSlot?.prompt;
		if (prompt?.cancel()) declineWasmConsent(prompt.question);
	}, [wasmSlot, declineWasmConsent]);

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
			<NodePaymentPrompt />
			<RuntimeVariablesPrompt
				key={runtimeSlot?.prompt.id}
				open={runtimeSlot?.open ?? false}
				onOpenChange={handleRuntimeOpenChange}
				variables={runtimeSlot?.prompt.variables ?? NO_VARIABLES}
				existingValues={runtimeSlot?.prompt.existingValues ?? NO_VALUES}
				onSave={handleRuntimeSave}
				onCancel={handleRuntimeCancel}
			/>
			<WasmSandboxWarningDialog
				key={wasmSlot?.prompt.id}
				open={wasmSlot?.open ?? false}
				packageIds={wasmSlot?.prompt.packageIds ?? NO_PACKAGES}
				packagePermissions={wasmSlot?.prompt.packagePermissions}
				onConfirm={handleWasmConfirm}
				onCancel={handleWasmCancel}
			/>
		</ExecutionServiceContext.Provider>
	);
}
