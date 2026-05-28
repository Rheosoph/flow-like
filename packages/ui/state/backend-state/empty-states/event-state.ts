import type {
	IEvent,
	IEventState,
	IIntercomEvent,
	ILogMetadata,
	IOAuthToken,
	IRunPayload,
	IVersionType,
} from "@flow-like/flow-like-ui";

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
			history?: any[];
			globalState?: Record<string, any>;
			localState?: Record<string, any>;
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
	): Promise<ILogMetadata | undefined> {
		throw new Error("Method not implemented.");
	}
	cancelExecution(runId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}

	isEventSinkActive(eventId: string): Promise<boolean> {
		// Empty state always returns false - no sinks active in non-Tauri environments
		return Promise.resolve(false);
	}
}
