"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import type {
	AgentBackendProvider,
	CopilotAuthStatus,
	CopilotConnectionConfig,
	CopilotModel,
} from "../components/flowpilot/types";
import {
	type AgentBackendDiagnostic,
	classifyAgentBackendError,
} from "../lib/flowpilot/agent-backend-diagnostics";
import { isTauri } from "../lib/platform";
import { copilotBackendConnectionCoordinator } from "./copilot-backend-coordinator";

// Offline / loading fallback only. The authoritative list is fetched from the
// backend (`flowpilot_agent_backend_list_models`), which discovers the
// auth-available models dynamically from the installed runtime — Codex via its
// `app-server`, Claude Code via its `initialize` control handshake.
const STATIC_BACKEND_MODELS: Partial<
	Record<AgentBackendProvider, CopilotModel[]>
> = {
	codex: [{ id: "default", name: "Codex configured default" }],
	"claude-code": [{ id: "default", name: "Claude Code configured default" }],
};

function staticModelsForBackend(backend: AgentBackendProvider): CopilotModel[] {
	return STATIC_BACKEND_MODELS[backend] ?? [];
}

function withTimeout<T>(
	promise: Promise<T>,
	timeoutMs: number,
	label: string,
): Promise<T> {
	let timeoutId: ReturnType<typeof setTimeout> | undefined;
	const timeout = new Promise<never>((_, reject) => {
		timeoutId = setTimeout(() => {
			reject(new Error(`${label} timed out after ${timeoutMs / 1000}s`));
		}, timeoutMs);
	});

	return Promise.race([promise, timeout]).finally(() => {
		if (timeoutId) clearTimeout(timeoutId);
	});
}

interface UseCopilotSDKResult {
	/** Whether the Copilot SDK client is running */
	isRunning: boolean;
	/** Whether currently starting/stopping */
	isConnecting: boolean;
	/** Available Copilot models */
	models: CopilotModel[];
	/** Whether `models` came from a completed backend catalog request rather than the static fallback. */
	hasLoadedModelCatalog: boolean;
	/** Current auth status */
	authStatus: CopilotAuthStatus | null;
	/** Error message if any */
	error: string | null;
	/** Actionable interpretation of the current backend/auth failure. */
	diagnostic: AgentBackendDiagnostic | null;
	/** Start the Copilot SDK client */
	start: (config?: CopilotConnectionConfig) => Promise<void>;
	/** Stop the Copilot SDK client */
	stop: () => Promise<void>;
	/** Refresh models list */
	refreshModels: () => Promise<void>;
	/** Refresh auth status */
	refreshAuthStatus: () => Promise<void>;
	/** Retry startup or refresh a running backend after the user fixes the issue. */
	retry: () => Promise<void>;
}

interface BackendRequestScope {
	backend: AgentBackendProvider;
	active: boolean;
	generation: number;
}

interface BackendViewState {
	scope: BackendRequestScope;
	isRunning: boolean;
	isConnecting: boolean;
	models: CopilotModel[];
	hasLoadedModelCatalog: boolean;
	authStatus: CopilotAuthStatus | null;
	error: string | null;
}

function initialBackendState(
	backend: AgentBackendProvider,
	scope: BackendRequestScope,
): BackendViewState {
	return {
		...copilotBackendConnectionCoordinator.snapshot(backend),
		scope,
		models: staticModelsForBackend(backend),
		hasLoadedModelCatalog: false,
		authStatus: null,
	};
}

/**
 * Hook for managing a FlowPilot agent backend connection and state.
 *
 * GitHub Copilot, Codex, and Claude Code are exposed through the same FlowPilot
 * agent backend contract so the UI and routing do not special-case providers.
 * Only works in Tauri environment - returns a disabled state for web.
 */
export function useCopilotSDK(
	backend: AgentBackendProvider = "github-copilot",
): UseCopilotSDKResult {
	const scope = useMemo<BackendRequestScope>(
		() => ({ backend, active: false, generation: 0 }),
		[backend],
	);
	const [state, setState] = useState(() => initialBackendState(backend, scope));
	// Reset before effects run so a provider never inherits another one's readiness.
	const currentState =
		state.scope === scope ? state : initialBackendState(backend, scope);
	if (currentState !== state) setState(currentState);
	const {
		isRunning,
		isConnecting,
		models,
		hasLoadedModelCatalog,
		authStatus,
		error,
	} = currentState;

	const isTauriEnv = isTauri();

	const updateState = useCallback(
		(
			update:
				| Partial<BackendViewState>
				| ((current: BackendViewState) => Partial<BackendViewState>),
		) => {
			if (!scope.active) return;
			setState((current) =>
				current.scope === scope
					? {
							...current,
							...(typeof update === "function" ? update(current) : update),
						}
					: current,
			);
		},
		[scope],
	);

	useEffect(() => {
		scope.active = true;
		scope.generation += 1;
		const unsubscribe = copilotBackendConnectionCoordinator.subscribe(
			backend,
			(snapshot) => updateState(snapshot),
		);
		return () => {
			scope.active = false;
			unsubscribe();
		};
	}, [backend, scope, updateState]);

	const requestIsCurrent = useCallback(
		(invalidatePending = false) => {
			if (invalidatePending) scope.generation += 1;
			const generation = scope.generation;
			return () => scope.active && scope.generation === generation;
		},
		[scope],
	);

	const start = useCallback(
		async (config?: CopilotConnectionConfig) => {
			const targetBackend = config?.backend ?? backend;
			const isCurrent = requestIsCurrent(
				targetBackend === backend &&
					!copilotBackendConnectionCoordinator.snapshot(backend).isRunning,
			);
			if (!isTauriEnv) {
				updateState({
					error:
						"FlowPilot agent backends are only available in the desktop app",
				});
				return;
			}

			try {
				const { invoke } = await import("@tauri-apps/api/core");
				await withTimeout(
					copilotBackendConnectionCoordinator.start(targetBackend, () =>
						invoke("flowpilot_agent_backend_start", {
							backend: targetBackend,
							useStdio: config?.useStdio ?? true,
							cliUrl: config?.serverUrl,
						}),
					),
					15_000,
					`Starting ${targetBackend}`,
				);
			} catch (e) {
				const errMsg = e instanceof Error ? e.message : String(e);
				// An immediate repeat can hit the coordinator's short cooldown.
				// Preserve the actual native failure instead of replacing it with
				// that secondary backoff message.
				if (isCurrent())
					updateState((current) => ({
						error:
							current.error && errMsg.toLowerCase().includes("cooling down")
								? current.error
								: errMsg,
					}));
				throw e;
			}
		},
		[backend, isTauriEnv, requestIsCurrent, updateState],
	);

	const stop = useCallback(async () => {
		if (!isTauriEnv) return;
		const isCurrent = requestIsCurrent(true);

		updateState({ error: null });

		try {
			const { invoke } = await import("@tauri-apps/api/core");
			await withTimeout(
				copilotBackendConnectionCoordinator.stop(backend, () =>
					invoke("flowpilot_agent_backend_stop", { backend }),
				),
				10_000,
				`Stopping ${backend}`,
			);
			if (isCurrent())
				updateState({
					models: staticModelsForBackend(backend),
					hasLoadedModelCatalog: false,
					authStatus: null,
				});
		} catch (e) {
			const errMsg = e instanceof Error ? e.message : String(e);
			if (isCurrent()) updateState({ error: errMsg });
			throw e;
		}
	}, [backend, isTauriEnv, requestIsCurrent, updateState]);

	const refreshModels = useCallback(async () => {
		if (!isTauriEnv) return;
		const isCurrent = requestIsCurrent();

		try {
			const { invoke } = await import("@tauri-apps/api/core");
			if (!isCurrent()) return;
			const connection = copilotBackendConnectionCoordinator.snapshot(backend);
			if (
				backend === "github-copilot" &&
				(!connection.isRunning || connection.isConnecting)
			)
				return;
			const result = await withTimeout(
				invoke<CopilotModel[]>("flowpilot_agent_backend_list_models", {
					backend,
				}),
				// Above the backend's discovery bound (Claude Code's control
				// handshake allows up to 12s) so cold starts don't fall back early.
				15_000,
				`Loading ${backend} models`,
			);
			if (isCurrent())
				updateState({
					models: result.length > 0 ? result : staticModelsForBackend(backend),
					hasLoadedModelCatalog: true,
				});
		} catch (e) {
			const errMsg = e instanceof Error ? e.message : String(e);
			if (isCurrent())
				updateState({
					error: errMsg,
					models: staticModelsForBackend(backend),
					hasLoadedModelCatalog: false,
				});
		}
	}, [backend, isTauriEnv, requestIsCurrent, updateState]);

	const refreshAuthStatus = useCallback(async () => {
		if (!isTauriEnv) return;
		const isCurrent = requestIsCurrent();

		try {
			const { invoke } = await import("@tauri-apps/api/core");
			const connection = copilotBackendConnectionCoordinator.snapshot(backend);
			if (!isCurrent() || !connection.isRunning || connection.isConnecting)
				return;
			const result = await withTimeout(
				invoke<CopilotAuthStatus>("flowpilot_agent_backend_get_auth_status", {
					backend,
				}),
				8_000,
				`Loading ${backend} auth status`,
			);
			if (isCurrent()) updateState({ authStatus: result });
		} catch (e) {
			const errMsg = e instanceof Error ? e.message : String(e);
			if (isCurrent()) updateState({ error: errMsg });
		}
	}, [backend, isTauriEnv, requestIsCurrent, updateState]);

	const retry = useCallback(async () => {
		if (!isRunning) {
			await start();
			return;
		}
		requestIsCurrent(true);
		updateState({ error: null });
		await refreshModels();
		await refreshAuthStatus();
	}, [
		isRunning,
		refreshAuthStatus,
		refreshModels,
		requestIsCurrent,
		start,
		updateState,
	]);

	const diagnostic = useMemo(() => {
		if (error) return classifyAgentBackendError(backend, error);
		if (authStatus?.authenticated === false) {
			return classifyAgentBackendError(
				backend,
				authStatus.message || `${backend} authentication required`,
			);
		}
		return null;
	}, [authStatus, backend, error]);

	// Check initial running state
	useEffect(() => {
		if (!isTauriEnv) return;
		const isCurrent = requestIsCurrent();

		const checkRunning = async () => {
			try {
				const { invoke } = await import("@tauri-apps/api/core");
				if (!isCurrent()) return;
				const before = copilotBackendConnectionCoordinator.snapshot(backend);
				if (before.isConnecting) return;
				const running = await withTimeout(
					invoke<boolean>("flowpilot_agent_backend_is_running", { backend }),
					5_000,
					`Checking ${backend}`,
				);
				if (!isCurrent()) return;
				const current = copilotBackendConnectionCoordinator.snapshot(backend);
				// A startup/stop completed while the native probe was pending.
				if (
					current.isConnecting ||
					current.isRunning !== before.isRunning ||
					current.error !== before.error
				)
					return;
				copilotBackendConnectionCoordinator.reconcile(backend, running);
				if (!running && backend === "github-copilot") {
					updateState({
						models: staticModelsForBackend(backend),
						hasLoadedModelCatalog: false,
						authStatus: null,
					});
				}
			} catch {
				// Ignore errors during initial check
			}
		};

		checkRunning();
	}, [backend, isTauriEnv, requestIsCurrent, updateState]);

	// Auto-fetch models and auth when running
	useEffect(() => {
		if (backend !== "github-copilot" || isRunning) {
			refreshModels();
		}
		if (isRunning) {
			refreshAuthStatus();
		}
	}, [backend, isRunning, refreshModels, refreshAuthStatus]);

	return {
		isRunning,
		isConnecting,
		models,
		hasLoadedModelCatalog,
		authStatus,
		error,
		diagnostic,
		start,
		stop,
		refreshModels,
		refreshAuthStatus,
		retry,
	};
}
