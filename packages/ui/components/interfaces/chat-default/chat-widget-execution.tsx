"use client";

import { type ReactNode, useMemo } from "react";
import type { IIntercomEvent, ILogMetadata, IRunPayload } from "../../../lib";
import { useBackend } from "../../../state/backend-state";
import {
	ExecutionServiceContext,
	type ExecutionServiceContextValue,
	useExecutionServiceOptional,
} from "../../../state/execution-service-context";

/**
 * Runs a widget/page action inside the chat. `runPayload` targets the bound
 * workflow node; `onA2UIEvents` receives every run event so the clicked widget
 * updates in place, while the chat also renders any pushed chat content as a
 * new assistant message. Returns when the run completes.
 */
export type RunWidgetAction = (
	runPayload: IRunPayload,
	onA2UIEvents?: (events: IIntercomEvent[]) => void,
) => Promise<ILogMetadata | undefined>;

/**
 * Scopes an ExecutionService to the chat that overrides `executeBoard` so that
 * embedded-widget actions (ActionHandler's `widget_event` / `workflow_event`)
 * are routed through the chat's execution engine. This gives both feedback
 * paths at once: the run's a2ui events update the widget in place, and any
 * chat pushes from the triggered workflow appear as a new assistant message.
 * Non-`executeBoard` methods delegate to the surrounding service or backend.
 */
export function ChatWidgetExecutionProvider({
	runWidgetAction,
	children,
}: {
	runWidgetAction: RunWidgetAction;
	children: ReactNode;
}) {
	const backend = useBackend();
	const base = useExecutionServiceOptional();

	const value = useMemo<ExecutionServiceContextValue>(() => {
		const executeBoard: ExecutionServiceContextValue["executeBoard"] = (
			_appId,
			_boardId,
			payload,
			_streamState,
			_eventId,
			cb,
		) => runWidgetAction(payload, cb);

		const executeEvent: ExecutionServiceContextValue["executeEvent"] =
			base?.executeEvent ??
			((appId, eventId, payload, streamState, onEventId, cb, skip) =>
				backend.eventState.executeEvent(
					appId,
					eventId,
					payload,
					streamState,
					onEventId,
					cb,
					skip,
				));

		const executeBoardRemote: ExecutionServiceContextValue["executeBoardRemote"] =
			base?.executeBoardRemote ??
			((appId, boardId, payload, streamState, eventId, cb) =>
				backend.boardState.executeBoardRemote?.(
					appId,
					boardId,
					payload,
					streamState,
					eventId,
					cb,
				) ?? Promise.resolve(undefined));

		return {
			executeBoard,
			executeBoardRemote,
			executeEvent,
			executeBoardDirect: executeBoard,
			executeEventDirect: executeEvent,
		};
	}, [runWidgetAction, backend, base]);

	return (
		<ExecutionServiceContext.Provider value={value}>
			{children}
		</ExecutionServiceContext.Provider>
	);
}
