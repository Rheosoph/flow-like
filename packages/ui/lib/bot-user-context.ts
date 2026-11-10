import type { AssistantSurfaceState } from "../state/assistant-surface";
import type {
	IUserContextRequest,
	IUserContextResponse,
	UserContextField,
} from "../state/backend-state/inbox-state";
import { AVAILABILITY_STATUSES, type UserAvailability } from "./availability";
import { DeviceError, readLocationForUserContext } from "./device-bridge";

export const USER_CONTEXT_FIELDS = [
	"location",
	"timezone",
	"locale",
	"preferences",
	"availability",
	"free_busy",
	"current_context",
	"device",
] as const;

export const SESSION_MS = 30 * 60_000;
const VALUE_MS = 5 * 60_000;
interface UserContextGrant {
	expires: number;
	input: UserContextInput;
	fields: UserContextField[];
	precision: IUserContextRequest["precision"];
}

const grants = new Map<string, UserContextGrant>();
const activeCollections = new Map<string, Set<AbortController>>();
let sessionId: string | undefined;
let accountId: string | null | undefined;

export function setUserContextAccount(account: string | null): void {
	if (accountId !== account) clearUserContextSessions();
	accountId = account;
}

export function userContextSessionId(): string {
	sessionId ??= crypto.randomUUID();
	return sessionId;
}

function userContextPlatform(): "native" | "web" {
	return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
		? "native"
		: "web";
}

function grantKey(request: IUserContextRequest): string {
	return JSON.stringify([
		request.bot_id,
		request.subject_id,
		request.conversation_id,
		request.audience,
		[...request.audience_member_ids].sort(),
		userContextSessionId(),
	]);
}

function liveGrant(
	request: IUserContextRequest,
	now: number,
): UserContextGrant | undefined {
	const grant = grants.get(grantKey(request));
	if (grant === undefined) return;
	if (grant.expires <= now) {
		revokeUserContextSession(request);
		return;
	}
	return grant;
}

export function hasUserContextGrant(
	request: IUserContextRequest,
	now = Date.now(),
): boolean {
	if (request.status_change) return false;
	const grant = liveGrant(request, now);
	return (
		grant !== undefined &&
		request.fields.every((field) => grant.fields.includes(field)) &&
		(request.precision === "approximate" || grant.precision === "precise")
	);
}

export function grantUserContextSession(
	request: IUserContextRequest,
	input: UserContextInput,
	now = Date.now(),
): void {
	if (request.status_change) return;
	const previous = liveGrant(request, now);
	grants.set(grantKey(request), {
		expires: now + SESSION_MS,
		input: { ...input },
		fields: [...new Set([...(previous?.fields ?? []), ...request.fields])],
		precision:
			previous?.precision === "precise" || request.precision === "precise"
				? "precise"
				: "approximate",
	});
}

export function userContextSessionInput(
	request: IUserContextRequest,
): UserContextInput | undefined {
	return hasUserContextGrant(request)
		? grants.get(grantKey(request))?.input
		: undefined;
}

export function revokeUserContextSession(request: IUserContextRequest): void {
	const key = grantKey(request);
	grants.delete(key);
	for (const controller of activeCollections.get(key) ?? []) controller.abort();
}

export function clearUserContextSessions(): void {
	grants.clear();
	for (const controllers of activeCollections.values()) {
		for (const controller of controllers) controller.abort();
	}
}

export function trackUserContextCollection(
	request: IUserContextRequest,
	controller: AbortController,
): () => void {
	const key = grantKey(request);
	const controllers = activeCollections.get(key) ?? new Set<AbortController>();
	controllers.add(controller);
	activeCollections.set(key, controllers);
	return () => {
		controllers.delete(controller);
		if (controllers.size === 0) activeCollections.delete(key);
	};
}

export interface UserContextInput {
	locationMode: "device" | "manual";
	place: string;
	timezone: string;
	language: string;
	displayName: string;
	units: "metric" | "imperial" | "unspecified";
	responseStyle: "default" | "concise" | "detailed";
	availabilityMode?: "account" | "manual";
	availability: "available" | "busy" | "focus" | "away" | "unspecified";
	availabilityNote: string;
	activity: string;
}

export function defaultUserContextInput(): UserContextInput {
	return {
		locationMode: "device",
		place: "",
		timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
		language: typeof navigator === "undefined" ? "en" : navigator.language,
		displayName: "",
		units: "unspecified",
		responseStyle: "default",
		availability: "unspecified",
		availabilityMode: "account",
		availabilityNote: "",
		activity: "",
	};
}

export function currentUserContextActivity(
	surfaces: AssistantSurfaceState,
): Record<string, unknown> {
	const board = surfaces.boardSurface;
	if (board)
		return {
			kind: "board",
			app_id: board.appId,
			board_id: board.boardId,
			selected_ids: board.selectedNodeIds.slice(0, 50),
		};
	const widget = surfaces.widgetSurface;
	if (widget)
		return {
			kind: widget.kind,
			...(widget.appId ? { app_id: widget.appId } : {}),
			...(widget.boardId ? { board_id: widget.boardId } : {}),
			...(widget.pageId ? { page_id: widget.pageId } : {}),
			selected_ids: widget.selectedComponentIds.slice(0, 50),
		};
	const data = surfaces.dataStudioSurface;
	if (data) return { kind: "data_studio", app_id: data.appId };
	return { kind: "none" };
}

/** A coarse cell with an uncertainty radius that covers the original reading. */
export function approximateUserLocation(
	latitude: number,
	longitude: number,
	accuracy: number,
) {
	const lat = Math.round(latitude * 100) / 100;
	const lon = Math.round(longitude * 100) / 100;
	return {
		latitude: lat,
		longitude: lon,
		accuracy_m: Math.max(1_500, accuracy + 800),
		precision: "approximate",
	};
}

function assertCollectionActive(
	request: IUserContextRequest,
	subjectId: string,
	signal?: AbortSignal,
) {
	if (request.subject_id !== subjectId)
		throw new DeviceError(
			"permission_denied",
			"Only the requested person can share this context",
		);
	if (signal?.aborted)
		throw new DeviceError("cancelled", "Context sharing was cancelled");
	if (Date.now() >= request.expires_at)
		throw new DeviceError("expired", "This context request has expired");
	if (typeof document === "undefined" || document.visibilityState !== "visible")
		throw new DeviceError(
			"foreground_required",
			"Keep this app open to share context",
		);
}

export async function collectUserContext(
	request: IUserContextRequest,
	input: UserContextInput,
	options: {
		subjectId: string;
		decision: "once" | "session";
		activity: Record<string, unknown>;
		signal?: AbortSignal;
		location?: typeof readLocationForUserContext;
		availability?: () => Promise<UserAvailability>;
	},
): Promise<IUserContextResponse> {
	assertCollectionActive(request, options.subjectId, options.signal);
	const proposal = request.status_change;
	if (
		proposal &&
		(options.decision !== "once" ||
			request.fields.length !== 1 ||
			request.fields[0] !== "availability" ||
			!AVAILABILITY_STATUSES.includes(proposal.status) ||
			!Number.isInteger(proposal.duration_minutes) ||
			proposal.duration_minutes < 1 ||
			proposal.duration_minutes > 240)
	) {
		throw new DeviceError(
			"permission_denied",
			"Status changes require one confirmation and a duration from 1 to 240 minutes",
		);
	}
	let availability: Promise<UserAvailability> | undefined;
	const accountAvailability = () => {
		if (!options.availability)
			throw new DeviceError(
				"unavailable",
				"Account availability is unavailable",
			);
		availability ??= options.availability();
		return availability;
	};
	const values: IUserContextResponse["values"] = {};
	const unavailable: IUserContextResponse["unavailable"] = {};
	for (const field of request.fields) {
		assertCollectionActive(request, options.subjectId, options.signal);
		try {
			switch (field) {
				case "location": {
					if (input.locationMode === "manual") {
						if (!input.place.trim())
							throw new DeviceError("unavailable", "Enter a place to share");
						values.location = {
							place: input.place.trim().slice(0, 200),
							precision: "approximate",
						};
						break;
					}
					const fix = await (options.location ?? readLocationForUserContext)(
						{
							highAccuracy: request.precision === "precise",
							maximumAgeMs: 0,
							timeoutMs: 20_000,
						},
						{
							appId: request.bot_id,
							signal: options.signal,
							deadline: request.expires_at,
						},
					);
					values.location =
						request.precision === "precise"
							? {
									latitude: fix.latitude,
									longitude: fix.longitude,
									accuracy_m: fix.accuracy,
									precision: "precise",
								}
							: approximateUserLocation(
									fix.latitude,
									fix.longitude,
									fix.accuracy,
								);
					break;
				}
				case "timezone": {
					const formatter = new Intl.DateTimeFormat("en-CA", {
						timeZone: input.timezone,
						dateStyle: "short",
						timeStyle: "long",
					});
					values.timezone = {
						name: input.timezone,
						local_time: formatter.format(new Date()),
					};
					break;
				}
				case "locale":
					values.locale = {
						language: input.language.trim().slice(0, 100),
						units: input.units,
					};
					break;
				case "preferences":
					values.preferences = {
						...(input.displayName.trim()
							? { display_name: input.displayName.trim().slice(0, 100) }
							: {}),
						response_style: input.responseStyle,
					};
					break;
				case "availability": {
					if (proposal) break;
					if (input.availabilityMode !== "manual" && options.availability) {
						const snapshot = await accountAvailability();
						if (
							snapshot.source === "default" &&
							snapshot.calendar_error &&
							snapshot.calendar_links.length > 0
						)
							throw new DeviceError(
								"unavailable",
								"Calendar availability could not be checked",
							);
						values.availability = {
							status: snapshot.status,
							source: snapshot.source,
							...(snapshot.until === null ? {} : { until: snapshot.until }),
						};
						break;
					}
					values.availability = {
						status: input.availability,
						...(input.availabilityNote.trim()
							? { note: input.availabilityNote.trim().slice(0, 300) }
							: {}),
					};
					break;
				}
				case "free_busy": {
					const snapshot = await accountAvailability();
					if (!snapshot.free_busy)
						throw new DeviceError(
							"unavailable",
							"Link a calendar in account settings to share free/busy times",
						);
					values.free_busy = {
						window_start: snapshot.free_busy.window_start,
						window_end: snapshot.free_busy.window_end,
						busy: snapshot.free_busy.busy.map(({ start, end }) => ({
							start,
							end,
						})),
					};
					break;
				}
				case "current_context":
					values.current_context = input.activity.trim()
						? {
								kind: "manual",
								description: input.activity.trim().slice(0, 500),
							}
						: options.activity;
					break;
				case "device":
					values.device = {
						viewport: window.matchMedia("(max-width: 767px)").matches
							? "compact"
							: "wide",
						touch: navigator.maxTouchPoints > 0,
						fine_pointer: window.matchMedia("(pointer: fine)").matches,
					};
					break;
			}
		} catch (error) {
			unavailable[field] =
				error instanceof DeviceError ? error.code : "unavailable";
		}
	}
	assertCollectionActive(request, options.subjectId, options.signal);
	const captured = Date.now();
	if (proposal)
		values.availability = {
			status: proposal.status,
			source: "bot",
			until: captured + proposal.duration_minutes * 60_000,
		};
	return {
		request_id: request.id,
		bot_id: request.bot_id,
		subject_id: request.subject_id,
		conversation_id: request.conversation_id,
		decision: options.decision,
		captured_at: captured,
		expires_at: captured + VALUE_MS,
		source: {
			session_id: userContextSessionId(),
			platform: userContextPlatform(),
		},
		values,
		unavailable,
	};
}

export function deniedUserContext(
	request: IUserContextRequest,
): IUserContextResponse {
	const now = Date.now();
	return {
		request_id: request.id,
		bot_id: request.bot_id,
		subject_id: request.subject_id,
		conversation_id: request.conversation_id,
		decision: "deny",
		captured_at: now,
		expires_at: now,
		source: {
			session_id: userContextSessionId(),
			platform: userContextPlatform(),
		},
		values: {},
		unavailable: Object.fromEntries(
			request.fields.map((field) => [field, "denied"]),
		),
	};
}

export function revokeUserContextBody(request: IUserContextRequest) {
	return {
		text: "Context session access revoked.",
		user_context_revoke: {
			request_id: request.id,
			bot_id: request.bot_id,
			subject_id: request.subject_id,
			conversation_id: request.conversation_id,
			revoked_at: Date.now(),
		},
	};
}

export function liveUserContextSession(
	request: IUserContextRequest,
	response: IUserContextResponse | undefined,
	revoked: boolean | undefined,
	now = Date.now(),
): boolean {
	return (
		request.audience === "direct" &&
		response?.decision === "session" &&
		!revoked &&
		now < response.captured_at + SESSION_MS
	);
}

export function contextField(value: unknown): value is UserContextField {
	return (
		typeof value === "string" &&
		(USER_CONTEXT_FIELDS as readonly string[]).includes(value)
	);
}
