import type {
	IEvent,
	IIntercomEvent,
	ILogMetadata,
	IOAuthProvider,
	IOAuthToken,
	IRunPayload,
	PageTrigger,
	IVersionType,
} from "../../lib";
import type { IPrerunEventResponse } from "./types";

export interface IOAuthCheckResult {
	tokens?: Record<string, IOAuthToken>;
	missingProviders: IOAuthProvider[];
}

export interface IEventRegistration {
	id: string;
	event_id: string;
	event_version: string;
	kind: string;
	method?: string | null;
	path: string;
	node_id?: string | null;
	schema?: Record<string, any> | null;
	extras?: Record<string, any> | null;
	auth_id?: string | null;
}

export interface IEventRemoteAuth {
	id: string;
	event_id: string;
	event_version: string;
	kind: string;
	node_id: string;
	config: Record<string, any>;
}

export interface IListRegistrationsResponse {
	event_id: string;
	event_version?: string | null;
	registrations: IEventRegistration[];
	auths?: IEventRemoteAuth[];
}

export interface ISetupEventResponse {
	run_id: string;
	event_id: string;
	event_version: string;
	status: string;
	server_configs_received: number;
	registrations_written: number;
	auths_written: number;
	error?: string | null;
}

export interface IEventAlias {
	slug: string;
	app_id: string;
	event_id: string;
	created_by?: string | null;
}

export interface IEventState {
	/** Whether events always execute remotely (server-side). When true, secrets are handled server-side and don't need to be prompted or sent from the client. */
	readonly alwaysRemote?: boolean;

	getEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<IEvent>;
	getEvents(appId: string, force?: boolean): Promise<IEvent[]>;
	getEventVersions(
		appId: string,
		eventId: string,
	): Promise<[number, number, number][]>;
	upsertEvent(
		appId: string,
		event: IEvent,
		versionType?: IVersionType,
		personalAccessToken?: string,
		oauthTokens?: Record<string, IOAuthToken>,
	): Promise<IEvent>;
	/** Check OAuth requirements for an event's board. Returns missing providers. */
	checkEventOAuth?(appId: string, event: IEvent): Promise<IOAuthCheckResult>;
	/** Check OAuth requirements resolved by a governed pre-run endpoint. */
	checkOAuthRequirements?(
		appId: string,
		requirements: Array<{ provider_id: string; scopes: string[] }>,
	): Promise<IOAuthCheckResult>;
	deleteEvent(appId: string, eventId: string): Promise<void>;
	validateEvent(
		appId: string,
		eventId: string,
		version?: [number, number, number],
	): Promise<void>;
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
	): Promise<string>;
	executeEvent(
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		skipConsentCheck?: boolean,
		pageTrigger?: PageTrigger,
	): Promise<ILogMetadata | undefined>;

	/** Execute an event remotely via the server-side SSE invoke endpoint */
	executeEventRemote?(
		appId: string,
		eventId: string,
		payload: IRunPayload,
		streamState?: boolean,
		onEventId?: (id: string) => void,
		cb?: (event: IIntercomEvent[]) => void,
		pageTrigger?: PageTrigger,
	): Promise<ILogMetadata | undefined>;

	cancelExecution(runId: string): Promise<void>;

	isEventSinkActive(eventId: string): Promise<boolean>;

	/** List persisted REST/MCP registrations for an event (populated by remote setup). */
	listEventRegistrations?(
		appId: string,
		eventId: string,
		version?: string,
	): Promise<IListRegistrationsResponse>;

	/** Run remote setup and persist REST/MCP registrations for an event. */
	setupEvent?(
		appId: string,
		eventId: string,
		force?: boolean,
	): Promise<ISetupEventResponse>;

	/** List vanity aliases for an event. */
	listEventAliases?(appId: string, eventId: string): Promise<IEventAlias[]>;

	/** Create or replace the event's vanity alias. */
	upsertEventAlias?(
		appId: string,
		eventId: string,
		slug: string,
	): Promise<IEventAlias>;

	/** Delete a vanity alias from an event. */
	deleteEventAlias?(
		appId: string,
		eventId: string,
		slug: string,
	): Promise<void>;

	/** Pre-run analysis: get required runtime variables and OAuth for an event */
	prerunEvent?(
		appId: string,
		eventId: string,
		version?: [number, number, number],
		pageTrigger?: PageTrigger,
	): Promise<IPrerunEventResponse>;
}
