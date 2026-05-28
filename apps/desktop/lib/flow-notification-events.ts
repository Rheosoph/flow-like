import type { IIntercomEvent } from "@flow-like/flow-like-ui";

export const FLOW_NOTIFICATION_EVENT = "flow-like:flow-notification";

export interface FlowNotificationBatchDetail {
	events: IIntercomEvent[];
	appId?: string;
}

// UI-only fanout. Notification nodes own remote persistence.
export function dispatchFlowNotificationEvents(
	events: IIntercomEvent[],
	appId?: string,
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
				appId,
			},
		}),
	);
}

export function dispatchFlowNotificationEvent(
	event: IIntercomEvent,
	appId?: string,
): void {
	dispatchFlowNotificationEvents([event], appId);
}
