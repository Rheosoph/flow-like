import { useCallback, useRef, useState } from "react";
import { completedRunFailure } from "../lib/execution/outcome";
import type { IIntercomEvent } from "../lib/schema/events/intercom-event";
import type { IEvent } from "../lib/schema/flow/event";
import type { IEventState } from "../state/backend-state/event-state";

interface QuickActionOptions {
	appId: string | null | undefined;
	executeEvent: IEventState["executeEvent"];
	streamState?: boolean;
	onError: (error: unknown) => void;
}

function actionKey(appId: string, eventId: string): string {
	return JSON.stringify([appId, eventId]);
}

export function useQuickAction({
	appId,
	executeEvent,
	streamState,
	onError,
}: QuickActionOptions) {
	const inFlight = useRef(new Set<string>());
	const [running, setRunning] = useState(new Set<string>());

	const execute = useCallback(
		async (event: Pick<IEvent, "id" | "node_id">): Promise<void> => {
			if (!appId) return;
			const key = actionKey(appId, event.id);
			// The ref also blocks a second click before React renders the disabled button.
			if (inFlight.current.has(key)) return;
			inFlight.current.add(key);
			setRunning(new Set(inFlight.current));

			try {
				const outcomes: IIntercomEvent[] = [];
				const metadata = await executeEvent(
					appId,
					event.id,
					{ id: event.node_id },
					streamState,
					() => {},
					(events) => {
						outcomes.push(
							...events.filter(
								(event) =>
									event.event_type === "completed" ||
									event.event_type === "error",
							),
						);
					},
				);
				// Remote runs can complete without returning local log metadata.
				const failure = completedRunFailure(outcomes, metadata);
				if (failure) throw failure;
			} catch (error) {
				onError(error);
			} finally {
				inFlight.current.delete(key);
				setRunning(new Set(inFlight.current));
			}
		},
		[appId, executeEvent, streamState, onError],
	);

	const isRunning = useCallback(
		(eventId: string) => !!appId && running.has(actionKey(appId, eventId)),
		[appId, running],
	);

	return { execute, isRunning };
}
