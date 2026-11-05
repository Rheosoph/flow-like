import { createContext, useContext } from "react";
import type { ExecutionTask } from "../../packages/ui/lib/execution/types";

export interface FixtureExecutionEngine {
	getTasks(): ExecutionTask[];
	subscribeToGlobalUpdates(listener: () => void): () => void;
	cancelTask(taskId: string): Promise<void>;
	dismissTask(taskId: string): void;
	replyToTask(taskId: string, requestId: string, value: unknown): Promise<void>;
}

export const FixtureEngineContext =
	createContext<FixtureExecutionEngine | null>(null);
export function useExecutionEngine() {
	const engine = useContext(FixtureEngineContext);
	if (!engine) throw new Error("Fixture engine missing");
	return engine;
}
