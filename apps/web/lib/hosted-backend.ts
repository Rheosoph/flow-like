import {
	ANONYMOUS_WIDGET_ACCESS,
	isWebWidgetGrant,
	isWidgetAccessUnsupportedError,
	parseWidgetAccessResponse,
	parseWidgetGrantResponse,
	parseWidgetPolicyDescriptor,
} from "@flow-like/flow-like-ui/components/a2ui/micro-widget-policy";
import { getApiOrigin, getApiUrl } from "@flow-like/flow-like-ui/lib/api-url";
import type { IAppLegalLinks } from "@flow-like/flow-like-ui/lib/legal-links";
import type { IOAuthToken } from "@flow-like/flow-like-ui/lib/oauth/types";
import {
	HUB_REFRESH_TIMEOUT_MS,
	requestTimeoutMs,
	withRequestDeadline,
} from "@flow-like/flow-like-ui/lib/request-deadline";
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
	EmptyLockedStorageState,
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
import type {
	IOAuthRequirement,
	IPrerunEventResponse,
	IStorageItemActionResult,
} from "@flow-like/flow-like-ui/state/backend-state/types";
import type { IWidgetState } from "@flow-like/flow-like-ui/state/backend-state/widget-state";
import type { AuthContextProps } from "react-oidc-context";
import {
	type HostedTarget,
	hostedApiPath,
	hostedSessionId,
} from "./hosted-route";
import { consumeHostedStream } from "./hosted-stream";
import { fetchWithSession } from "./session-request";

/** The interface a route serves: `u` a custom page, `c` a chat, `f` a form. */
export type HostedKind = "c" | "f" | "u";

export interface HostedRoute {
	path: string;
	event_id: string;
	kind: HostedKind;
	is_default: boolean;
}

export interface HostedBootstrap {
	app_id: string;
	kind: HostedKind;
	auth_proxy: boolean;
	/** The publisher's links; absent from an API older than this field. */
	legal?: IAppLegalLinks;
	bootstrap: IPageBootstrap;
}

export class HostedHttpError extends Error {
	constructor(
		public readonly status: number,
		message: string,
		/** The API's error code, on widget registry requests. Absent when something else answered: a proxy, or an API without the route. */
		public readonly code?: string,
	) {
		super(message);
	}
}

export function createHostedRequest(
	target: HostedTarget,
	accessToken?: string,
	auth?: AuthContextProps,
) {
	const endpoint = `${getApiOrigin()}/${hostedApiPath(target)}`;
	return async (suffix = "", init: RequestInit = {}): Promise<Response> => {
		const headers = new Headers(init.headers);
		headers.set("X-Flow-Like-Session", hostedSessionId());
		if (accessToken) headers.set("Authorization", `Bearer ${accessToken}`);
		if (init.body) headers.set("Content-Type", "application/json");
		const url = new URL(`${endpoint}${suffix}`);
		url.searchParams.set("route", target.route);
		if (target.variant) url.searchParams.set("__variant", target.variant);
		const response = await fetchWithSession(
			url.href,
			{
				...init,
				headers,
				cache: "no-store",
				credentials: "omit",
			},
			auth,
		);
		if (!response.ok)
			throw new HostedHttpError(
				response.status,
				hostedErrorMessage(response.status, await response.text()),
			);
		return response;
	};
}

/** The API answers `{"error":{"code","message"}}`; a 404 means the link itself is not live. */
export function hostedErrorMessage(status: number, body: string): string {
	if (status === 404)
		return "This link is not published, or it has changed. Ask the owner for a new link.";
	let message: unknown = body;
	try {
		const parsed = JSON.parse(body);
		message = parsed?.error?.message ?? parsed?.message ?? parsed?.error;
	} catch {}
	return typeof message === "string" && message
		? message
		: `Request failed (${status})`;
}

/** The `code` of the API's error envelope; undefined for a body the API did not write. */
export function hostedErrorCode(body: string): string | undefined {
	let code: unknown;
	try {
		const parsed = JSON.parse(body);
		code = parsed?.error?.code ?? parsed?.code;
	} catch {}
	return typeof code === "string" && code.trim() ? code.trim() : undefined;
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

export type HostedAccessToken = () => string | undefined;

/** A signed-in visitor's own OAuth tokens for a run; undefined runs on the Event's saved sign-ins. */
export type HostedOAuthTokens = (
	requirements: IOAuthRequirement[],
) => Promise<Record<string, IOAuthToken> | undefined>;

const anonymous: HostedAccessToken = () => undefined;

/** A sign-in interface sends its viewer's token, so project members reach the private packages the project pins. */
function publicWidgetRegistry(
	accessToken: HostedAccessToken,
	auth?: () => AuthContextProps,
): IRegistryState {
	// A request that never settles would hold the frame, and every later check of its token, for good.
	const send = (
		path: string,
		body: unknown,
		timeoutMs = requestTimeoutMs(path, "POST"),
	) =>
		withRequestDeadline(
			path,
			async ({ signal }) => {
				const headers = new Headers({ "Content-Type": "application/json" });
				const token = accessToken();
				if (token) headers.set("Authorization", `Bearer ${token}`);
				const response = await fetchWithSession(
					getApiUrl(undefined, path),
					{
						method: "POST",
						credentials: "omit",
						headers,
						body: JSON.stringify(body),
						signal,
					},
					auth?.(),
				);
				if (!response.ok)
					throw new HostedHttpError(
						response.status,
						"The widget is not available for public hosting.",
						hostedErrorCode(await response.text().catch(() => "")),
					);
				return response.json();
			},
			{ timeoutMs },
		);
	// An API that answered an access request has the route: a bare 404 or 405 after that is a proxy's.
	let accessAnswered = false;
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
		getWidgetAccess: async (request) => {
			try {
				// Asked again when it fails, unlike describe and mint, so it gives up early.
				const access = parseWidgetAccessResponse(
					await send(
						`registry/package/${encodeURIComponent(request.packageId)}/widget-access`,
						{ version: request.packageVersion, appId: request.appId },
						HUB_REFRESH_TIMEOUT_MS,
					),
				);
				accessAnswered = true;
				return access;
			} catch (error) {
				if (!accessAnswered && isWidgetAccessUnsupportedError(error)) {
					return { ...ANONYMOUS_WIDGET_ACCESS };
				}
				throw error;
			}
		},
	});
}

class HostedEventState extends EmptyEventState {
	private readonly runs = new Map<string, AbortController>();

	constructor(
		private readonly data: HostedBootstrap,
		private readonly request: HostedRequest,
		private readonly accessToken: HostedAccessToken,
		private readonly oauthTokens?: HostedOAuthTokens,
	) {
		super();
	}
	/** Only a sign-in interface knows its visitor; an anonymous visit never brings its own sign-ins. */
	private visitorOAuth(): HostedOAuthTokens | undefined {
		return this.data.auth_proxy && this.accessToken()
			? this.oauthTokens
			: undefined;
	}
	/** A chat or form asks the prerun only for its OAuth needs; without an answer the saved sign-ins serve. */
	private async oauthRequirements(
		appId: string,
		eventId: string,
	): Promise<IOAuthRequirement[]> {
		try {
			return (await this.prerunEvent(appId, eventId)).oauth_requirements ?? [];
		} catch (error) {
			console.warn(
				`[Hosted OAuth] The prerun of event ${eventId} failed, so this run uses the app's saved sign-ins.`,
				error,
			);
			return [];
		}
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
		const oauth = this.visitorOAuth();
		let activeTrigger = trigger;
		let requirements: IOAuthRequirement[] = [];
		if (trigger) {
			const prerun = await this.prerunEvent(appId, eventId, undefined, trigger);
			activeTrigger = withCurrentManifestRevision(
				trigger,
				prerun.manifest_revision,
			);
			requirements = prerun.oauth_requirements ?? [];
		} else if (oauth) {
			requirements = await this.oauthRequirements(appId, eventId);
		}
		const oauthTokens =
			oauth && requirements.length > 0 ? await oauth(requirements) : undefined;
		beforeDispatch?.();
		const controller = new AbortController();
		let runId: string | undefined;
		try {
			const response = await this.request("/invoke", {
				method: "POST",
				body: JSON.stringify({
					payload: payload.payload,
					page_trigger: activeTrigger
						? serializePageTrigger(activeTrigger)
						: undefined,
					oauth_tokens: oauthTokens,
				}),
				signal: controller.signal,
			});
			if (!response.body)
				throw new Error("The server did not return an execution stream.");
			await consumeHostedStream(response.body, onEvents, (id) => {
				if (!runId) {
					runId = id;
					this.runs.set(id, controller);
				}
				onRunId?.(id);
			});
			return undefined;
		} finally {
			if (runId) this.runs.delete(runId);
		}
	};
	// The hosted surface has no cancel route; closing the stream stops this client listening.
	async cancelExecution(runId: string): Promise<void> {
		this.runs.get(runId)?.abort();
	}
}

/** Matches `MAX_PREFIXES` in packages/api/src/routes/app/data/batch.rs. */
const ASSET_BATCH_LIMIT = 100;

/** Signs only files the published interface itself shows; the server enforces that. */
class HostedStorageState extends EmptyStorageState {
	constructor(
		private readonly appId: string,
		private readonly request: HostedRequest,
	) {
		super();
	}
	async downloadStorageItems(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		if (appId !== this.appId)
			throw new Error("These files belong to another app.");
		const results: IStorageItemActionResult[] = [];
		for (let start = 0; start < prefixes.length; start += ASSET_BATCH_LIMIT) {
			const response = await this.request("/assets", {
				method: "POST",
				body: JSON.stringify({
					prefixes: prefixes.slice(start, start + ASSET_BATCH_LIMIT),
				}),
			});
			results.push(...((await response.json()) as IStorageItemActionResult[]));
		}
		return results;
	}
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
	accessToken: HostedAccessToken = anonymous,
	auth?: () => AuthContextProps,
	oauthTokens?: HostedOAuthTokens,
): IBackendState {
	return {
		appState: new EmptyAppState(),
		apiState: new EmptyApiState(),
		apiKeyState: new EmptyApiKeyState(),
		bitState: new EmptyBitState(),
		boardState: new EmptyBoardState(),
		teamState: new EmptyTeamState(),
		roleState: new EmptyRoleState(),
		storageState: new HostedStorageState(data.app_id, request),
		lockedStorageState: new EmptyLockedStorageState(),
		templateState: new EmptyTemplateState(),
		aiState: new EmptyAIState(),
		dbState: new EmptyDatabaseState(),
		graphState: new EmptyGraphState(),
		queryState: new EmptyQueryState(),
		pageState: scopedState<IPageState>({}),
		eventState: new HostedEventState(data, request, accessToken, oauthTokens),
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
		registryState: publicWidgetRegistry(
			data.auth_proxy ? accessToken : anonymous,
			data.auth_proxy ? auth : undefined,
		),
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
