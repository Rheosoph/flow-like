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
import type { IIntercomEvent } from "../lib/schema/events/intercom-event";
import { useBackend } from "./backend-state";
import { useExecutionServiceOptional } from "./execution-service-context";

export type { OnIncrementalSaveFn };

const ExecutionEngineContext = createContext<ExecutionEngineProvider | null>(
	null,
);

export function ExecutionEngineProviderComponent({
	children,
}: { children: React.ReactNode }) {
	const backend = useBackend();
	const auth = useAuth();
	const scope = JSON.stringify([
		getApiOrigin(backend.profile),
		backend.profile?.id ?? "",
		(auth.isAuthenticated ? auth.user?.profile.sub : undefined) ?? "local",
	]);
	const executionService = useExecutionServiceOptional();
	const engineRef = useRef<ExecutionEngineProvider | null>(null);

	if (!engineRef.current) {
		engineRef.current = new ExecutionEngineProvider();
	}

	useEffect(() => {
		if (engineRef.current && backend) {
			engineRef.current.setBackend(backend);
			engineRef.current.setExecutionScope(scope);
		}
	}, [backend, scope]);

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
			<RunningTasksIndicator />
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
