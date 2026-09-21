import {
	isWebWidgetGrant,
	parseWidgetGrantResponse,
	parseWidgetPolicyDescriptor,
} from "@flow-like/flow-like-ui/components/a2ui/micro-widget-policy";
import { getApiOrigin, getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import {
	serializePageTrigger,
	withCurrentManifestRevision,
} from "@flow-like/flow-like-ui/lib/schema/flow/page-trigger";
import type { IBackendState } from "@flow-like/flow-like-ui/state/backend-state";
import {
	EmptyAIState,
	EmptyApiKeyState,
	EmptyApiState,
	EmptyAppState,
	EmptyBitState,
	EmptyBoardState,
	EmptyDatabaseState,
	EmptyGraphState,
	EmptyQueryState,
	EmptyRoleState,
	EmptyStorageState,
	EmptyTeamState,
	EmptyTemplateState,
} from "@flow-like/flow-like-ui/state/backend-state/empty-states";
import { EmptyEventState } from "@flow-like/flow-like-ui/state/backend-state/empty-states/event-state";
import { EmptyHelperState } from "@flow-like/flow-like-ui/state/backend-state/empty-states/helper-state";
import { EmptyRouteState } from "@flow-like/flow-like-ui/state/backend-state/empty-states/route-state";
import { EmptyUserState } from "@flow-like/flow-like-ui/state/backend-state/empty-states/user-state";
import type { IEventState } from "@flow-like/flow-like-ui/state/backend-state/event-state";
import type {
	IPageBootstrap,
	IPageState,
} from "@flow-like/flow-like-ui/state/backend-state/page-state";
import type { IRegistryState } from "@flow-like/flow-like-ui/state/backend-state/registry-state";
import type { IPrerunEventResponse } from "@flow-like/flow-like-ui/state/backend-state/types";
import type { IWidgetState } from "@flow-like/flow-like-ui/state/backend-state/widget-state";
import {
	type HostedTarget,
	hostedApiPath,
	hostedSessionId,
} from "./hosted-route";
import type { HostedKind } from "./hosted-route";
import { consumeHostedStream } from "./hosted-stream";

export interface HostedRoute {
	path: string;
	event_id: string;
	kind: HostedKind;
	is_default: boolean;
}

export interface HostedBootstrap {
	app_id: string;
	auth_proxy: boolean;
	bootstrap: IPageBootstrap;
}

export class HostedHttpError extends Error {
	constructor(
		public readonly status: number,
		message: string,
	) {
		super(message);
	}
}

export function createHostedRequest(
	target: HostedTarget,
	accessToken?: string,
) {
	const endpoint = `${getApiOrigin()}/${hostedApiPath(target)}`;
	return async (suffix = "", init: RequestInit = {}): Promise<Response> => {
		const headers = new Headers(init.headers);
		headers.set("X-Flow-Like-Session", hostedSessionId());
		if (accessToken) headers.set("Authorization", `Bearer ${accessToken}`);
		if (init.body) headers.set("Content-Type", "application/json");
		const url = new URL(`${endpoint}${suffix}`);
		if (target.variant) url.searchParams.set("__variant", target.variant);
		const response = await fetch(url, {
			...init,
			headers,
			cache: "no-store",
			credentials: "omit",
		});
		if (!response.ok) {
			const body = await response.text();
			let message = body;
			try {
				const parsed = JSON.parse(body);
				message = parsed.message ?? parsed.error ?? body;
			} catch {}
			throw new HostedHttpError(
				response.status,
				typeof message === "string" && message
					? message
					: `Request failed (${response.status})`,
			);
		}
		return response;
	};
}

type HostedRequest = ReturnType<typeof createHostedRequest>;

function scopedState<T extends object>(methods: Partial<T>): T {
	return new Proxy(methods, {
		get: (target, property) =>
			property in target
				? Reflect.get(target, property)
				: async () => {
						throw new Error(
							"This operation is unavailable in a hosted interface.",
						);
					},
	}) as T;
}

function publicWidgetRegistry(): IRegistryState {
	const send = async (path: string, body: unknown) => {
		const response = await fetch(getApiUrl(undefined, path), {
			method: "POST",
			credentials: "omit",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(body),
		});
		if (!response.ok)
			throw new HostedHttpError(
				response.status,
				"The widget is not available for public hosting.",
			);
		return response.json();
	};
	return scopedState<IRegistryState>({
		describeWidgetPolicy: async (request) =>
			parseWidgetPolicyDescriptor(
				await send(
					`registry/package/${encodeURIComponent(request.packageId)}/widget-policy/${encodeURIComponent(request.packageVersion)}/${encodeURIComponent(request.widgetId)}`,
					{
						preview: false,
						appId: request.appId,
						runtimeSources: request.runtimeSources ?? [],
					},
				),
				{
					packageId: request.packageId,
					packageVersion: request.packageVersion,
					widgetId: request.widgetId,
					preview: false,
				},
			),
		mintWidgetGrant: async (request) =>
			parseWidgetGrantResponse(
				await send(
					`registry/package/${encodeURIComponent(request.packageId)}/widget-grant`,
					{
						version: request.packageVersion,
						widgetId: request.widgetId,
						preview: false,
						policyDigest: request.policyDigest,
						appId: request.appId,
						runtimeSources: request.runtimeSources ?? [],
					},
				),
				isWebWidgetGrant,
			),
	});
}

class HostedEventState extends EmptyEventState {
	constructor(
		private readonly data: HostedBootstrap,
		private readonly request: HostedRequest,
	) {
		super();
	}
	private assertTarget(appId: string, eventId?: string) {
		if (
			appId !== this.data.app_id ||
			(eventId && eventId !== this.data.bootstrap.event.id)
		) {
			throw new Error("This interface can only run its published event.");
		}
	}
	getEvent = async (appId: string, eventId: string) => {
		this.assertTarget(appId, eventId);
		return this.data.bootstrap.event;
	};
	getEvents = async (appId: string) => {
		this.assertTarget(appId);
		return [this.data.bootstrap.event];
	};
	getEventAuthoritative = this.getEvent;
	getEventsAuthoritative = this.getEvents;
	prerunEvent: NonNullable<IEventState["prerunEvent"]> = async (
		appId,
		eventId,
		_version,
		trigger,
	) => {
		this.assertTarget(appId, eventId);
		const response = await this.request(
			"/prerun",
			trigger
				? {
						method: "POST",
						body: JSON.stringify({
							page_trigger: serializePageTrigger(trigger),
						}),
					}
				: undefined,
		);
		return response.json() as Promise<IPrerunEventResponse>;
	};
	executeEvent: IEventState["executeEvent"] = async (
		appId,
		eventId,
		payload,
		_streamState,
		onRunId,
		onEvents,
		_skipConsentCheck,
		trigger,
		beforeDispatch,
	) => {
		this.assertTarget(appId, eventId);
		let activeTrigger = trigger;
		if (trigger) {
			const prerun = await this.prerunEvent(appId, eventId, undefined, trigger);
			activeTrigger = withCurrentManifestRevision(
				trigger,
				prerun.manifest_revision,
			);
		}
		beforeDispatch?.();
		const response = await this.request("/invoke", {
			method: "POST",
			body: JSON.stringify({
				payload: payload.payload,
				page_trigger: activeTrigger
					? serializePageTrigger(activeTrigger)
					: undefined,
			}),
		});
		if (!response.body)
			throw new Error("The server did not return an execution stream.");
		await consumeHostedStream(response.body, onEvents, onRunId);
		return undefined;
	};
}

class HostedHelperState extends EmptyHelperState {
	fileToUrl = async (file: File): Promise<string> => {
		if (file.size > 20 * 1024 * 1024)
			throw new Error("Attachments must be smaller than 20 MB.");
		return new Promise((resolve, reject) => {
			const reader = new FileReader();
			reader.onload = () => resolve(String(reader.result));
			reader.onerror = () =>
				reject(new Error("The attachment could not be read."));
			reader.readAsDataURL(file);
		});
	};
}

class HostedRouteState extends EmptyRouteState {
	constructor(private readonly request: HostedRequest) {
		super();
	}
	getRoutes = async () => {
		const routes = (await (
			await this.request("/routes")
		).json()) as HostedRoute[];
		return routes.map((route) => ({
			path: route.path,
			eventId: route.event_id,
		}));
	};
	getRouteByPath = async (_appId: string, path: string) =>
		(await this.getRoutes()).find((route) => route.path === path) ?? null;
	getDefaultRoute = async (appId: string) => this.getRouteByPath(appId, "/");
	getRouteByPathAuthoritative = this.getRouteByPath;
}

class HostedUserState extends EmptyUserState {
	getProfile = async () => ({
		name: "Hosted interface",
		bits: [],
		hub: getApiOrigin(),
		created: new Date().toISOString(),
		updated: new Date().toISOString(),
	});
}

/** Reuse the renderers while exposing only the published event's API. */
export function createHostedBackend(
	data: HostedBootstrap,
	request: HostedRequest,
): IBackendState {
	return {
		appState: new EmptyAppState(),
		apiState: new EmptyApiState(),
		apiKeyState: new EmptyApiKeyState(),
		bitState: new EmptyBitState(),
		boardState: new EmptyBoardState(),
		teamState: new EmptyTeamState(),
		roleState: new EmptyRoleState(),
		storageState: new EmptyStorageState(),
		templateState: new EmptyTemplateState(),
		aiState: new EmptyAIState(),
		dbState: new EmptyDatabaseState(),
		graphState: new EmptyGraphState(),
		queryState: new EmptyQueryState(),
		pageState: scopedState<IPageState>({}),
		eventState: new HostedEventState(data, request),
		helperState: new HostedHelperState(),
		routeState: new HostedRouteState(request),
		userState: new HostedUserState(),
		widgetState: scopedState<IWidgetState>({
			getWidget: async (appId, widgetId, version) => {
				if (appId !== data.app_id)
					throw new Error("This widget belongs to another app.");
				const params = new URLSearchParams();
				if (version) params.set("version", version.join("_"));
				if (data.bootstrap.servedVariant)
					params.set("__variant", data.bootstrap.servedVariant);
				return (
					await request(
						`/widgets/${encodeURIComponent(widgetId)}${params.size ? `?${params}` : ""}`,
					)
				).json();
			},
		}),
		registryState: publicWidgetRegistry(),
		capabilities: () => ({
			needsSignIn: false,
			canExecuteLocally: false,
			canHostEmbeddings: false,
			canHostLlamaCPP: false,
			canHostMLX: false,
		}),
		isOffline: async () => false,
	};
}
