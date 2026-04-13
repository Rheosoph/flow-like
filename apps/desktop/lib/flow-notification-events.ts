import type { IIntercomEvent } from "@tm9657/flow-like-ui";

export const FLOW_NOTIFICATION_EVENT = "flow-like:flow-notification";

export interface FlowNotificationBatchDetail {
	events: IIntercomEvent[];
	persistViaApi: boolean;
	appId?: string;
	boardId?: string;
}

export function dispatchFlowNotificationEvents(
	events: IIntercomEvent[],
	persistViaApi: boolean,
	appId?: string,
	boardId?: string,
): void {
	if (typeof window === "undefined") {
		return;
	}

	const notificationEvents = events.filter(
		(event) => event.event_type === "flow_notification",
	);

	if (notificationEvents.length === 0) {
		return;
	}

	window.dispatchEvent(
		new CustomEvent<FlowNotificationBatchDetail>(FLOW_NOTIFICATION_EVENT, {
			detail: {
				events: notificationEvents,
				persistViaApi,
				appId,
				boardId,
			},
		}),
	);
}

export function dispatchFlowNotificationEvent(
	event: IIntercomEvent,
	persistViaApi: boolean,
	appId?: string,
	boardId?: string,
): void {
	dispatchFlowNotificationEvents([event], persistViaApi, appId, boardId);
}
