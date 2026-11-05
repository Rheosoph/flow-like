"use client";

import { Suspense, createContext, useContext, useEffect, useRef } from "react";
import { useAuth } from "react-oidc-context";
import { RunningTasksIndicator } from "../components/execution-indicator";
import { FrontendAudioLifecycle } from "../components/frontend-audio-lifecycle";
import { getApiOrigin } from "../lib/api-url";
import {
	ExecutionEngineProvider,
	type OnIncrementalSaveFn,
} from "../lib/execution-engine";
import {
	getExecutionRuntime,
	releaseExecutionRuntime,
} from "../lib/execution/runtime-client";
import { attachExecutionEffects } from "../lib/execution/session-effects";
import type { IIntercomEvent } from "../lib/schema/events/intercom-event";
import { useBackend } from "./backend-state";
import { useExecutionServiceOptional } from "./execution-service-context";

export type { OnIncrementalSaveFn };

const ExecutionEngineContext = createContext<ExecutionEngineProvider | null>(
	null,
);

export function ExecutionEngineProviderComponent({
	children,
	executionScope,
	showRunningTasks = true,
}: {
	children: React.ReactNode;
	executionScope?: string;
	showRunningTasks?: boolean;
}) {
	const backend = useBackend();
	const auth = useAuth();
	const scope =
		executionScope ??
		JSON.stringify([
			getApiOrigin(backend.profile),
			backend.profile?.id ?? "",
			(auth.isAuthenticated ? auth.user?.profile.sub : undefined) ?? "local",
		]);
	const executionService = useExecutionServiceOptional();
	const engineRef = useRef<ExecutionEngineProvider | null>(null);
	const previousScope = useRef(scope);

	if (!engineRef.current) {
		engineRef.current = new ExecutionEngineProvider();
	}

	useEffect(() => {
		if (engineRef.current && backend) {
			engineRef.current.setExecutionScope(scope);
			engineRef.current.setBackend(backend);
		}
	}, [backend, scope]);

	useEffect(() => {
		if (previousScope.current !== scope)
			releaseExecutionRuntime(previousScope.current);
		previousScope.current = scope;
		let disposed = false;
		let detachEffects: (() => void) | undefined;
		void getExecutionRuntime(scope)
			.then((client) => {
				if (disposed) return;
				engineRef.current?.setRuntimeClient(client);
				detachEffects = attachExecutionEffects(client);
			})
			.catch((error) =>
				console.error("Execution runtime could not connect", error),
			);
		return () => {
			disposed = true;
			detachEffects?.();
			engineRef.current?.setRuntimeClient(undefined);
		};
	}, [scope]);

	useEffect(() => {
		if (engineRef.current && executionService) {
			engineRef.current.setExecuteEventFn(executionService.executeEvent);
		}
	}, [executionService]);

	return (
		<ExecutionEngineContext.Provider value={engineRef.current}>
			<Suspense fallback={null}>
				<FrontendAudioLifecycle scope={scope} />
			</Suspense>
			{children}
			{showRunningTasks && <RunningTasksIndicator />}
		</ExecutionEngineContext.Provider>
	);
}

export function useExecutionEngine(): ExecutionEngineProvider {
	const context = useContext(ExecutionEngineContext);
	if (!context) {
		throw new Error(
			"useExecutionEngine must be used within ExecutionEngineProviderComponent",
		);
	}
	return context;
}

export function useEventStream(
	streamId: string,
	subscriberId: string,
	onEvents: (events: IIntercomEvent[]) => void,
	onComplete?: (events: IIntercomEvent[]) => void,
) {
	const engine = useExecutionEngine();

	useEffect(() => {
		engine.subscribeToEventStream(streamId, subscriberId, onEvents, onComplete);

		return () => {
			engine.unsubscribeFromEventStream(streamId, subscriberId);
		};
	}, [engine, streamId, subscriberId, onEvents, onComplete]);
}
