import type {
	IEvent,
	IEventState,
	IIntercomEvent,
	ILogMetadata,
	IOAuthToken,
	IRunPayload,
	IVersionType,
	PageTrigger,
} from "@flow-like/flow-like-ui";
import type { IUserSchedules } from "../event-state";

export class EmptyEventState implements IEventState {
	getEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<IEvent> {
		throw new Error("Method not implemented.");
	}
	getEvents(appId: string, _force?: boolean): Promise<IEvent[]> {
		throw new Error("Method not implemented.");
	}
	getUserSchedules(): Promise<IUserSchedules> {
		throw new Error("Method not implemented.");
	}
	getEventAuthoritative(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<IEvent> {
		throw new Error("Method not implemented.");
	}
	getEventsAuthoritative(appId: string): Promise<IEvent[]> {
		throw new Error("Method not implemented.");
	}
	getEventVersions(
		appId: string,
		eventId: string,
	): Promise<[number, number, number][]> {
		throw new Error("Method not implemented.");
	}
	upsertEvent(
		appId: string,
		event: IEvent,
		versionType?: IVersionType,
		personalAccessToken?: string,
		oauthTokens?: Record<string, IOAuthToken>,
	): Promise<IEvent> {
		throw new Error("Method not implemented.");
	}
	deleteEvent(appId: string, eventId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	validateEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	upsertEventFeedback(
		appId: string,
		eventId: string,
		feedbackId: string,
		feedback: {
			rating: number;
			history?: unknown[];
			globalState?: Record<string, unknown>;
			localState?: Record<string, unknown>;
			comment?: string;
		},
	): Promise<string> {
		throw new Error("Method not implemented.");
	}
	executeEvent(
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
		pageTrigger?: PageTrigger,
	): Promise<ILogMetadata | undefined> {
		throw new Error("Method not implemented.");
	}
	cancelExecution(runId: string): Promise<void> {
		return Promise.reject(new Error("Method not implemented."));
	}

	isEventSinkActive(eventId: string): Promise<boolean> {
		// Empty state always returns false - no sinks active in non-Tauri environments
		return Promise.resolve(false);
	}
}
