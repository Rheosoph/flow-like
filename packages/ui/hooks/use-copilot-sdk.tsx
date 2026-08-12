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
	const initialConnection =
		copilotBackendConnectionCoordinator.snapshot(backend);
	const [isRunning, setIsRunning] = useState(initialConnection.isRunning);
	const [isConnecting, setIsConnecting] = useState(
		initialConnection.isConnecting,
	);
	const [models, setModels] = useState<CopilotModel[]>(() =>
		staticModelsForBackend(backend),
	);
	const [hasLoadedModelCatalog, setHasLoadedModelCatalog] = useState(false);
	const [authStatus, setAuthStatus] = useState<CopilotAuthStatus | null>(null);
	const [error, setError] = useState<string | null>(null);

	const isTauriEnv = isTauri();

	useEffect(
		() =>
			copilotBackendConnectionCoordinator.subscribe(backend, (snapshot) => {
				setIsRunning(snapshot.isRunning);
				setIsConnecting(snapshot.isConnecting);
				setError(snapshot.error);
			}),
		[backend],
	);

	useEffect(() => {
		setModels(staticModelsForBackend(backend));
		setHasLoadedModelCatalog(false);
		setAuthStatus(null);
	}, [backend]);

	const start = useCallback(
		async (config?: CopilotConnectionConfig) => {
			if (!isTauriEnv) {
				setError(
					"FlowPilot agent backends are only available in the desktop app",
				);
				return;
			}

			try {
				const { invoke } = await import("@tauri-apps/api/core");
				const targetBackend = config?.backend ?? backend;
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
				setError((current) =>
					current && errMsg.toLowerCase().includes("cooling down")
						? current
						: errMsg,
				);
				throw e;
			}
		},
		[backend, isTauriEnv],
	);

	const stop = useCallback(async () => {
		if (!isTauriEnv) return;

		setError(null);

		try {
			const { invoke } = await import("@tauri-apps/api/core");
			await withTimeout(
				copilotBackendConnectionCoordinator.stop(backend, () =>
					invoke("flowpilot_agent_backend_stop", { backend }),
				),
				10_000,
				`Stopping ${backend}`,
			);
			setModels(staticModelsForBackend(backend));
			setHasLoadedModelCatalog(false);
			setAuthStatus(null);
		} catch (e) {
			const errMsg = e instanceof Error ? e.message : String(e);
			setError(errMsg);
			throw e;
		}
	}, [backend, isTauriEnv]);

	const refreshModels = useCallback(async () => {
		if (!isTauriEnv) return;
		if (!isRunning && backend === "github-copilot") return;

		try {
			const { invoke } = await import("@tauri-apps/api/core");
			const result = await withTimeout(
				invoke<CopilotModel[]>("flowpilot_agent_backend_list_models", {
					backend,
				}),
				// Above the backend's discovery bound (Claude Code's control
				// handshake allows up to 12s) so cold starts don't fall back early.
				15_000,
				`Loading ${backend} models`,
			);
			setModels(result.length > 0 ? result : staticModelsForBackend(backend));
			setHasLoadedModelCatalog(true);
		} catch (e) {
			const errMsg = e instanceof Error ? e.message : String(e);
			setError(errMsg);
			setModels(staticModelsForBackend(backend));
			setHasLoadedModelCatalog(false);
		}
	}, [backend, isTauriEnv, isRunning]);

	const refreshAuthStatus = useCallback(async () => {
		if (!isTauriEnv || !isRunning) return;

		try {
			const { invoke } = await import("@tauri-apps/api/core");
			const result = await withTimeout(
				invoke<CopilotAuthStatus>("flowpilot_agent_backend_get_auth_status", {
					backend,
				}),
				8_000,
				`Loading ${backend} auth status`,
			);
			setAuthStatus(result);
		} catch (e) {
			const errMsg = e instanceof Error ? e.message : String(e);
			setError(errMsg);
		}
	}, [backend, isTauriEnv, isRunning]);

	const retry = useCallback(async () => {
		if (!isRunning) {
			await start();
			return;
		}
		setError(null);
		await refreshModels();
		await refreshAuthStatus();
	}, [isRunning, refreshAuthStatus, refreshModels, start]);

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

		const checkRunning = async () => {
			try {
				const { invoke } = await import("@tauri-apps/api/core");
				const running = await withTimeout(
					invoke<boolean>("flowpilot_agent_backend_is_running", { backend }),
					5_000,
					`Checking ${backend}`,
				);
				copilotBackendConnectionCoordinator.reconcile(backend, running);
				if (!running) {
					setModels(staticModelsForBackend(backend));
					setHasLoadedModelCatalog(false);
					setAuthStatus(null);
				}
			} catch {
				// Ignore errors during initial check
			}
		};

		checkRunning();
	}, [backend, isTauriEnv]);

	// Auto-fetch models and auth when running
	useEffect(() => {
		if (backend !== "github-copilot") {
			refreshModels();
		}
		if (isRunning) {
			refreshModels();
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
