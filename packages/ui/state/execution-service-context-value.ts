"use client";

import { createContext, useContext } from "react";
import type { IIntercomEvent, ILogMetadata, IRunPayload } from "../lib";
import type { PageTrigger } from "../lib/schema/flow/page-trigger";

export interface ExecutionServiceContextValue {
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
		pageTrigger?: PageTrigger,
		beforeDispatch?: () => void,
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
		pageTrigger?: PageTrigger,
		beforeDispatch?: () => void,
	) => Promise<ILogMetadata | undefined>;
}

/**
 * Lives apart from the provider so editing the provider hot-swaps it in place: a module that
 * exports anything but components is no Fast Refresh boundary, and re-running this one would
 * mint a new context and remount everything below the provider.
 */
export const ExecutionServiceContext = createContext<
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
