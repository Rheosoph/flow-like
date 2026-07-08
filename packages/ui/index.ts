"use client";
export * from "./components/index";
export * from "./hooks/index";
export * from "./lib/index";
export * from "./state/assistant-surface";
export * from "./state/backend-state";
export * from "./state/download-manager";
export * from "./state/flow-board-parent-state";
export * from "./state/run-execution-state";
export * from "./state/log-aggregation-state";
export * from "./state/execution-engine-context";
export * from "./state/spotlight-state";
export * from "./state/runtime-variables-context";
export * from "./state/execution-service-context";
export type { IRunUpdateEvent } from "./state/run-execution-state";
export * from "./types";
export * from "./db/index";

// Dependency exports
export {
	QueryClient,
	useMutation,
	useQuery,
	useQueryClient,
	type QueryObserverResult,
	type UseQueryResult,
} from "@tanstack/react-query";
export { PersistQueryClientProvider } from "@tanstack/react-query-persist-client";
export * from "@xyflow/react";
export { useTheme } from "next-themes";
export { useMiniSearch } from "react-minisearch";
export { isEqual } from "lodash-es";

// Resolve star-export name collisions (TypeScript TS2308)
export type { ChatRole, PlanStep } from "./lib/schema/flow/copilot";
export type { CanvasSettings } from "./state/backend-state/page-state";
