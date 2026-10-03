import { z } from "zod";
import type { IApiState } from "../../../state/backend-state/api-state";
import type { IProfile } from "../../../types";
import {
	apiResponseError,
	isTransportFailure,
	upstreamFailureInSuccess,
} from "../../api-error";
import type { IHub } from "../../schema/hub/hub";
import type { PublicCertificateInventory } from "../certificates";
import type {
	AccountBackupList,
	AppDevicePlacements,
	AppScheduleRow,
	ArchiveUsage,
	DeviceUsageResponse,
	EffectiveLimit,
	FleetCertificateInventoryRow,
	GateId,
	HubEnrollment,
	HubErrorCode,
	MyAccess,
	ResourceSummary,
} from "../model/types";
import type { Capability, PolicyView } from "../types";

/** §1.3: a 404/405 from a route an older hub lacks is "needs a newer hub", never an error. */
export type HubResult<T> = { kind: "ok"; data: T } | { kind: "missing_on_hub" };

/** Which audience a route serves: decides the gate a refusal maps to and how a 404 reads. */
export type HubRouteScope = "account" | "device" | "app";

export class HubError extends Error {
	readonly code: HubErrorCode;
	readonly status?: number;
	readonly gate?: GateId;
	/** How long the hub asked to wait (429), in seconds, when the API error carries it. */
	readonly retryAfterS?: number;

	constructor(
		code: HubErrorCode,
		message: string,
		options: {
			status?: number;
			gate?: GateId;
			cause?: unknown;
			retryAfterS?: number;
		} = {},
	) {
		super(message, { cause: options.cause });
		this.name = "HubError";
		this.code = code;
		this.status = options.status;
		this.gate = options.gate;
		this.retryAfterS = options.retryAfterS;
	}
}

const RESTRICTED_TOKEN = /unrestricted personal access token/i;

function statusOf(error: unknown): number | undefined {
	const status = (error as { status?: unknown } | null)?.status;
	return typeof status === "number" ? status : undefined;
}

/** `ApiResponseError.retryAfter` (seconds) once the API error keeps the hub's `Retry-After`. */
const retryAfterOf = (error: unknown) => {
	const seconds = (error as { retryAfter?: unknown } | null)?.retryAfter;
	return typeof seconds === "number" && Number.isFinite(seconds) && seconds >= 0
		? seconds
		: undefined;
};

function codeOf(error: unknown): string | undefined {
	const code = (error as { code?: unknown } | null)?.code;
	return typeof code === "string" && code ? code : undefined;
}

function messageOf(error: unknown): string {
	if (error instanceof z.ZodError)
		return `unexpected response shape (${error.issues
			.slice(0, 3)
			.map((issue) => `${issue.path.join(".") || "body"}: ${issue.message}`)
			.join("; ")})`;
	const server = (error as { serverMessage?: unknown } | null)?.serverMessage;
	if (typeof server === "string" && server.trim()) return server;
	return error instanceof Error ? error.message : String(error);
}

const STATUS_CODES: Readonly<Record<number, HubErrorCode>> = {
	401: "unauthorized",
	404: "not_found",
	408: "timeout",
	410: "not_found",
	429: "rate_limited",
};

/** No status: the request never got an answer, or the answer was not usable. */
function unansweredCode(error: unknown): HubErrorCode {
	if (error instanceof Error && error.name === "RequestTimeoutError")
		return "timeout";
	return isTransportFailure(error) ? "network" : "invalid_response";
}

const refusedCode = (error: unknown): HubErrorCode =>
	RESTRICTED_TOKEN.test(messageOf(error)) ? "token_restricted" : "forbidden";

function classify(error: unknown, status: number | undefined): HubErrorCode {
	if (status === undefined) return unansweredCode(error);
	if (status === 403) return refusedCode(error);
	return (
		STATUS_CODES[status] ??
		(status >= 500 ? "server_error" : "invalid_response")
	);
}

/** 401 and a restricted token are G3; a refusal is G12 for an app, G4 for a device, G3 otherwise. */
export function hubErrorGate(
	code: HubErrorCode,
	scope: HubRouteScope,
): GateId | undefined {
	if (code === "unauthorized" || code === "token_restricted") return "G3";
	if (code !== "forbidden") return undefined;
	return scope === "app" ? "G12" : scope === "device" ? "G4" : "G3";
}

export function toHubError(
	error: unknown,
	scope: HubRouteScope = "account",
	operation?: string,
): HubError {
	if (error instanceof HubError) return error;
	const status = statusOf(error);
	const code = classify(error, status);
	const detail = messageOf(error);
	return new HubError(code, operation ? `${operation}: ${detail}` : detail, {
		status,
		gate: hubErrorGate(code, scope),
		cause: error,
		retryAfterS: retryAfterOf(error),
	});
}

/**
 * Account routes on an older hub fall into `GET /devices/{id}` (a coded 404) or
 * hit a POST-only route (405). Device and app routes there match nothing (an
 * uncoded 404), while a coded 404 is the hub refusing an unknown or hidden object.
 */
export function isMissingOnHub(error: unknown, scope: HubRouteScope): boolean {
	const status = statusOf(error);
	if (status === 405) return true;
	if (status !== 404) return false;
	return scope === "account" || codeOf(error) === undefined;
}

export const isHubResultMissing = <T>(
	result: HubResult<T> | undefined,
): boolean => result?.kind === "missing_on_hub";

/** A route every supported hub has: any failure is a coded `HubError`. */
async function hubRead<R, T>(
	scope: HubRouteScope,
	operation: string,
	request: () => Promise<R>,
	parse: (value: R) => T,
): Promise<T> {
	try {
		return parse(await request());
	} catch (error) {
		throw toHubError(error, scope, operation);
	}
}

/** A route this work adds: an older hub's 404/405 is `missing_on_hub`. */
async function hubCall<T>(
	scope: HubRouteScope,
	operation: string,
	request: () => Promise<unknown>,
	parse: (value: unknown) => T,
): Promise<HubResult<T>> {
	try {
		return {
			kind: "ok",
			data: await hubRead(scope, operation, request, parse),
		};
	} catch (error) {
		if (error instanceof HubError && isMissingOnHub(error.cause, scope))
			return { kind: "missing_on_hub" };
		throw error;
	}
}

/* Lenient schemas: unknown fields are dropped, every documented field is checked. */

const time = z.number().int().safe();
const count = z.number().int().nonnegative().safe();
const id = z.string().min(1).max(256);
const micros = count;
/** The hub may send `null` or leave an empty optional out; both read as `null`. */
const nullable = <T extends z.ZodTypeAny>(schema: T) =>
	schema.nullish().transform((value): z.output<T> | null => value ?? null);

const CAPABILITIES: Record<Capability, true> = {
	status: true,
	logs: true,
	metrics: true,
	deploy: true,
	start: true,
	stop: true,
	restart: true,
	remove: true,
	scale: true,
	update_agent: true,
	manage_certificates: true,
	reboot: true,
};
const isCapability = (value: string): value is Capability =>
	Object.hasOwn(CAPABILITIES, value);
/** A capability this client does not know cannot be used by it; drop it instead of failing. */
const capabilities = z
	.array(z.string())
	.max(64)
	.transform((values) => values.filter(isCapability));

const inventoryScope = z.discriminatedUnion("kind", [
	z.object({ kind: z.literal("device") }),
	z.object({ kind: z.literal("project"), project_id: id }),
	z.object({
		kind: z.literal("placement"),
		project_id: id,
		placement_id: id,
	}),
]);

const enrollmentSchema: z.ZodType<HubEnrollment, z.ZodTypeDef, unknown> =
	z.object({
		enrollment_id: id,
		device_id: id,
		name: z.string(),
		state: z.enum(["pending", "expired", "cancelled"]),
		created_at: time,
		expires_at: time,
		controller_key_thumbprint: z.string(),
	});

const usageSchema: z.ZodType<DeviceUsageResponse, z.ZodTypeDef, unknown> =
	z.object({
		server_time: time,
		limits: z.object({
			max_devices: count,
			max_pending_enrollments: count,
			enrollment_ttl_seconds: count,
			max_enrollments_per_day: count,
			max_account_backups: count,
		}),
		usage: z.object({
			active_devices: count,
			revoked_devices: count,
			pending_enrollments: count,
			enrollments_last_24h: count,
			account_backups: count,
		}),
	});

const accountBackupsSchema: z.ZodType<
	AccountBackupList,
	z.ZodTypeDef,
	unknown
> = z.object({
	vaults: z
		.array(
			z.object({
				key_id: id,
				revision: count,
				updated_at: time,
				public_key_thumbprint: z.string(),
			}),
		)
		.max(256),
	used: count,
	max: count,
});

const myAccessSchema: z.ZodType<MyAccess, z.ZodTypeDef, unknown> = z.object({
	device_id: id,
	role: z.enum(["owner", "grantee"]),
	owner_id: z.string(),
	policy_version: count,
	policy_expires_at: nullable(time),
	applied_version: count,
	applied: z.boolean(),
	grants: z
		.array(
			z.object({
				grant_id: id,
				scope: inventoryScope,
				capabilities,
				expires_at: time,
				controller_key_thumbprint: z.string(),
				group_id: nullable(z.string()),
			}),
		)
		.max(256),
});

const publicCertificatesSchema = z.object({
	revision: count,
	updated_at: nullable(time),
	certificates: z
		.array(
			z.object({
				certificate_id: z.string().uuid(),
				revision: count,
				fingerprint_sha256: z.string().regex(/^[a-f0-9]{64}$/),
				not_after: time,
			}),
		)
		.max(32),
});
const fleetCertificateSchema: z.ZodType<
	FleetCertificateInventoryRow,
	z.ZodTypeDef,
	unknown
> = publicCertificatesSchema.extend({ device_id: id });

const policyViewSchema: z.ZodType<PolicyView, z.ZodTypeDef, unknown> = z.object(
	{
		policy_jws: z.string().nullable(),
		version: count,
		digest: z.string().nullable(),
		applied_version: count,
		applied_digest: z.string().nullable(),
	},
);

export interface CertificateNotice {
	certificate_id: string;
	certificate_revision: number;
	not_after: number;
	/** e.g. `30d`, `7d`, `1d`, `expired`, `test`. */
	stage: string;
	channel: string;
	status: string;
	attempts: number;
	completed_at: number | null;
	next_attempt_at: number | null;
}
const noticeSchema: z.ZodType<CertificateNotice, z.ZodTypeDef, unknown> =
	z.object({
		certificate_id: z.string().uuid(),
		certificate_revision: count,
		not_after: time,
		stage: z.string(),
		channel: z.string(),
		status: z.string(),
		attempts: count,
		completed_at: nullable(time),
		next_attempt_at: nullable(time),
	});

export interface CertificateNoticeMute {
	/** `null` mutes every certificate of the device. */
	certificate_id: string | null;
	/** `null` mutes until unmuted. */
	until: number | null;
}
const muteSchema: z.ZodType<CertificateNoticeMute, z.ZodTypeDef, unknown> =
	z.object({ certificate_id: nullable(z.string()), until: nullable(time) });

export type NoticeChannel = "push" | "email" | "both";
export interface TestNoticeResult {
	sent: string[];
	skipped: { channel: string; reason: string }[];
}
const testNoticeSchema: z.ZodType<TestNoticeResult, z.ZodTypeDef, unknown> =
	z.object({
		sent: z.array(z.string()).max(8),
		skipped: z
			.array(z.object({ channel: z.string(), reason: z.string() }))
			.max(8),
	});

const archiveUsageSchema: z.ZodType<ArchiveUsage, z.ZodTypeDef, unknown> =
	z.object({
		tier: z.string(),
		max_bytes: count,
		retention_seconds: count,
		used_bytes: count,
		devices: z
			.array(
				z.object({
					device_id: id,
					used_bytes: count,
					segments: count,
					oldest_created_at: nullable(time),
					newest_expires_at: nullable(time),
				}),
			)
			.max(1000),
	});

const grantStatus = z.enum(["active", "revoked"]);
const effectiveLimit = z.enum(["approval", "sharing_grant", "access_rules"]);
const onlineAccess = nullable(z.enum(["read_only", "read_write"]));
const writeBlocked = z.literal("storage_full").nullable().catch(null);
/** Without a tighter bound from the hub, an approval ends when it says and is limited by itself. */
const effectiveBounds = {
	effective_expires_at: time.optional(),
	effective_limit: effectiveLimit.optional(),
};
const withEffective = <
	T extends {
		expires_at: number;
		effective_expires_at?: number;
		effective_limit?: EffectiveLimit;
	},
>(
	row: T,
) => ({
	...row,
	effective_expires_at: row.effective_expires_at ?? row.expires_at,
	effective_limit: row.effective_limit ?? ("approval" as const),
});

const resourceSummarySchema: z.ZodType<ResourceSummary, z.ZodTypeDef, unknown> =
	z.object({
		server_time: time,
		devices: z
			.array(
				z.object({
					device_id: id,
					approvals: z.array(
						z
							.object({
								grant_id: id,
								placement_id: id,
								app_id: nullable(id),
								status: grantStatus,
								expires_at: time,
								...effectiveBounds,
								online_access: onlineAccess,
								online_write_blocked: writeBlocked,
								payer_is_me: z.boolean(),
								approver_is_me: z.boolean(),
							})
							.transform(withEffective),
					),
					billing: z.array(
						z.object({
							billing_grant_id: id,
							grant_id: id,
							limit_micros: micros,
							used_micros: micros,
							reserved_micros: micros,
							expires_at: time,
							payer_is_me: z.boolean(),
						}),
					),
				}),
			)
			.max(1000),
	});

const scheduleService = {
	device_id: id.optional(),
	placement_id: id.optional(),
};
const scheduleRow: z.ZodType<AppScheduleRow, z.ZodTypeDef, unknown> =
	z.discriminatedUnion("state", [
		z.object({
			event_id: id,
			state: z.literal("device"),
			since: time,
			seen_at: time,
			grant_id: id.optional(),
			...scheduleService,
		}),
		z.object({
			event_id: id,
			state: z.literal("released"),
			since: time,
			hub_resumes_at: time.optional(),
			...scheduleService,
		}),
		z.object({
			event_id: id,
			state: z.literal("returning"),
			hub_resumes_at: time,
		}),
	]);
/** A newer hub may list a state this client does not know: that entry is dropped, not the list. */
const scheduleRows = z
	.array(z.unknown())
	.max(512)
	.transform((rows) =>
		rows.flatMap((row) => {
			const parsed = scheduleRow.safeParse(row);
			return parsed.success ? [parsed.data] : [];
		}),
	);

/** Type names this client keeps of the hub's export list; a malformed list reads as an older hub's. */
const eventTypes = z
	.array(z.unknown())
	.max(256)
	.transform((names) =>
		names.filter(
			(name): name is string =>
				typeof name === "string" && /^[a-z][a-z0-9_]{0,63}$/u.test(name),
		),
	)
	.optional()
	.catch(undefined);

const appPlacementsSchema: z.ZodType<
	AppDevicePlacements,
	z.ZodTypeDef,
	unknown
> = z.object({
	// Absent on a hub that can't hand schedules to devices: its presence is that capability.
	schedules: scheduleRows.optional(),
	// Absent on a hub before Endpoints, forms and bots on devices: its presence is that capability.
	event_types: eventTypes,
	device_event_creation: z.boolean().optional().catch(undefined),
	server_time: time,
	placements: z
		.array(
			z.object({
				device_id: id,
				placement_id: id,
				deployment_id: id,
				relationship: z.enum(["owner", "shared", "cloud_approval"]),
				grant: z
					.object({
						grant_id: id,
						status: grantStatus,
						expires_at: time,
						...effectiveBounds,
						online_access: onlineAccess,
						model_ids: z.array(id).max(64),
						max_instances: count,
						approved_by_user_id: nullable(z.string()),
						created_at: nullable(time),
					})
					.transform(withEffective),
				billing: nullable(
					z.object({
						billing_grant_id: id,
						limit_micros: micros,
						used_micros: micros,
						reserved_micros: micros,
						expires_at: time,
						payer_is_me: z.boolean(),
					}),
				),
				instances: z.object({
					active: count,
					newest_lease_expires_at: nullable(time),
				}),
			}),
		)
		.max(500),
});

/** E17: whether the caller's plan covers each approved model. */
export interface BillingEligibility {
	payer_id: string;
	plan: string | null;
	eligible: boolean;
	models: { model_id: string; tier: string | null; allowed: boolean }[];
}
const eligibilitySchema: z.ZodType<BillingEligibility, z.ZodTypeDef, unknown> =
	z.object({
		payer_id: z.string(),
		plan: nullable(z.string()),
		eligible: z.boolean(),
		models: z
			.array(
				z.object({
					model_id: z.string(),
					tier: nullable(z.string()),
					allowed: z.boolean(),
				}),
			)
			.max(64),
	});

/** E18: spend on one spending approval, per instance. */
export interface BillingUsage {
	billing_grant_id: string;
	totals: { used_micros: number; reserved_micros: number; operations: number };
	instances: {
		instance_id: string;
		used_micros: number;
		reserved_micros: number;
		operations: number;
		first_at: number;
		last_at: number;
	}[];
}
const billingUsageSchema: z.ZodType<BillingUsage, z.ZodTypeDef, unknown> =
	z.object({
		billing_grant_id: id,
		totals: z.object({
			used_micros: micros,
			reserved_micros: micros,
			operations: count,
		}),
		instances: z
			.array(
				z.object({
					instance_id: id,
					used_micros: micros,
					reserved_micros: micros,
					operations: count,
					first_at: time,
					last_at: time,
				}),
			)
			.max(100),
	});

function sameId<T>(field: keyof T & string, expected: string, what: string) {
	return (value: T): T => {
		if (value[field] !== expected)
			throw new Error(
				`The hub returned ${what} ${String(value[field])} for a request about ${expected}.`,
			);
		return value;
	};
}

const segment = encodeURIComponent;
const devicePath = (deviceId: string) => `devices/${segment(deviceId)}`;
const withQuery = (path: string, query: Record<string, string>) =>
	`${path}?${new URLSearchParams(query).toString()}`;

/* E5 */
export function listEnrollments(
	api: IApiState,
	profile: IProfile,
	state: "open" | "recent" = "open",
): Promise<HubResult<HubEnrollment[]>> {
	return hubCall(
		"account",
		"GET devices/enrollments",
		() => api.get(profile, withQuery("devices/enrollments", { state })),
		(value) => z.array(enrollmentSchema).max(200).parse(value),
	);
}

/* E6 */
export function getDeviceUsage(
	api: IApiState,
	profile: IProfile,
): Promise<HubResult<DeviceUsageResponse>> {
	return hubCall(
		"account",
		"GET devices/usage",
		() => api.get(profile, "devices/usage"),
		(value) => usageSchema.parse(value),
	);
}

/* E8 */
export function getMyAccess(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
): Promise<HubResult<MyAccess>> {
	return hubCall(
		"device",
		"GET devices/{id}/management/my-access",
		() => api.get(profile, `${devicePath(deviceId)}/management/my-access`),
		(value) =>
			sameId<MyAccess>(
				"device_id",
				deviceId,
				"access for device",
			)(myAccessSchema.parse(value)),
	);
}

/* E7 */
export function listAccountBackups(
	api: IApiState,
	profile: IProfile,
): Promise<HubResult<AccountBackupList>> {
	return hubCall(
		"account",
		"GET devices/controller-vaults",
		() => api.get(profile, "devices/controller-vaults"),
		(value) => accountBackupsSchema.parse(value),
	);
}

/* E13 */
export function getFleetCertificateInventory(
	api: IApiState,
	profile: IProfile,
): Promise<HubResult<FleetCertificateInventoryRow[]>> {
	return hubCall(
		"account",
		"GET devices/certificate-inventory",
		() => api.get(profile, "devices/certificate-inventory"),
		(value) => z.array(fleetCertificateSchema).max(1000).parse(value),
	);
}

/* E9 */
export function listCertificateNotices(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	certificateId?: string,
	limit = 50,
): Promise<HubResult<CertificateNotice[]>> {
	const query = {
		...(certificateId ? { certificate: certificateId } : {}),
		limit: String(limit),
	};
	return hubCall(
		"device",
		"GET devices/{id}/certificate-notices",
		() =>
			api.get(
				profile,
				withQuery(`${devicePath(deviceId)}/certificate-notices`, query),
			),
		(value) => z.array(noticeSchema).max(200).parse(value),
	);
}

/* E10 */
export function muteCertificateNotices(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	certificateId: string | null,
	until: number | null,
): Promise<HubResult<CertificateNoticeMute>> {
	return hubCall(
		"device",
		"PUT devices/{id}/certificate-notices/mute",
		() =>
			api.put(profile, `${devicePath(deviceId)}/certificate-notices/mute`, {
				certificate_id: certificateId,
				until,
			}),
		(value) => muteSchema.parse(value),
	);
}

/** The caller's active mutes on one device; a `null` id mutes every certificate of it. */
export function listCertificateNoticeMutes(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
): Promise<HubResult<CertificateNoticeMute[]>> {
	return hubCall(
		"device",
		"GET devices/{id}/certificate-notices/mute",
		() => api.get(profile, `${devicePath(deviceId)}/certificate-notices/mute`),
		(value) => z.array(muteSchema).max(64).parse(value),
	);
}

/** `null` lifts the device-wide mute (`*`). */
export function unmuteCertificateNotices(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	certificateId: string | null,
): Promise<HubResult<void>> {
	return hubCall(
		"device",
		"DELETE devices/{id}/certificate-notices/mute",
		() =>
			api.del(
				profile,
				withQuery(`${devicePath(deviceId)}/certificate-notices/mute`, {
					certificate: certificateId ?? "*",
				}),
			),
		() => undefined,
	);
}

/* E11: a 429 (one test per user and device per 10 min) throws `rate_limited`. */
export function sendTestCertificateNotice(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	channel: NoticeChannel,
): Promise<HubResult<TestNoticeResult>> {
	return hubCall(
		"device",
		"POST devices/{id}/certificate-notices/test",
		() =>
			api.post(profile, `${devicePath(deviceId)}/certificate-notices/test`, {
				channel,
			}),
		(value) => testNoticeSchema.parse(value),
	);
}

/* E14 */
export function getArchiveUsage(
	api: IApiState,
	profile: IProfile,
): Promise<HubResult<ArchiveUsage>> {
	return hubCall(
		"account",
		"GET devices/archive-usage",
		() => api.get(profile, "devices/archive-usage"),
		(value) => archiveUsageSchema.parse(value),
	);
}

/* E19 */
export function getResourceSummary(
	api: IApiState,
	profile: IProfile,
): Promise<HubResult<ResourceSummary>> {
	return hubCall(
		"account",
		"GET devices/resource-summary",
		() => api.get(profile, "devices/resource-summary"),
		(value) => resourceSummarySchema.parse(value),
	);
}

/* E17 */
export function getBillingEligibility(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	grantId: string,
): Promise<HubResult<BillingEligibility>> {
	return hubCall(
		"device",
		"GET devices/{id}/resource-grants/{grant}/billing/eligibility",
		() =>
			api.get(
				profile,
				`${devicePath(deviceId)}/resource-grants/${segment(grantId)}/billing/eligibility`,
			),
		(value) => eligibilitySchema.parse(value),
	);
}

/* E18 */
export function getBillingUsage(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
	billingId: string,
): Promise<HubResult<BillingUsage>> {
	return hubCall(
		"device",
		"GET devices/{id}/billing-grants/{billing}/usage",
		() =>
			api.get(
				profile,
				`${devicePath(deviceId)}/billing-grants/${segment(billingId)}/usage`,
			),
		(value) =>
			sameId<BillingUsage>(
				"billing_grant_id",
				billingId,
				"spending",
			)(billingUsageSchema.parse(value)),
	);
}

/* E20 */
export function listAppDevicePlacements(
	api: IApiState,
	profile: IProfile,
	appId: string,
): Promise<HubResult<AppDevicePlacements>> {
	return hubCall(
		"app",
		"GET apps/{app_id}/device-placements",
		() => api.get(profile, `apps/${segment(appId)}/device-placements`),
		(value) => appPlacementsSchema.parse(value),
	);
}

/**
 * A refusal the caller words itself: the person's role (`role`), a schedule
 * another service runs (`schedule_elsewhere`), one that is still on its way
 * back to the hub (`schedule_returning`), a flow someone is editing
 * (`flow_busy`) or one the hub can't compare with its versions (`flow_incomparable`).
 */
export type HubRefusal<K extends string> = { kind: K; message?: string };

const refusalStatus = (error: unknown) =>
	error instanceof HubError ? error.status : undefined;

/** The hub's own sentence for a refusal, without the operation in front. */
const serverMessageOf = (error: HubError) => messageOf(error.cause ?? error);

/** The route's own refusals, by status and hub code; anything else stays a `HubError`. */
async function withRefusals<T, K extends string>(
	call: Promise<HubResult<T>>,
	refusal: (
		status: number,
		code: string | undefined,
		error: HubError,
	) => HubRefusal<K> | undefined,
): Promise<HubResult<T> | HubRefusal<K>> {
	try {
		return await call;
	} catch (error) {
		const status = refusalStatus(error);
		const known =
			status === undefined
				? undefined
				: refusal(status, codeOf((error as HubError).cause), error as HubError);
		if (known) return known;
		throw error;
	}
}

export type ScheduleRefusal = HubRefusal<
	"schedule_role" | "schedule_elsewhere" | "schedule_returning"
>;

const SCHEDULE_CONFLICTS: Readonly<
	Record<string, "schedule_elsewhere" | "schedule_returning">
> = {
	SCHEDULE_RUNS_ELSEWHERE: "schedule_elsewhere",
	SCHEDULE_RETURNING: "schedule_returning",
};

function scheduleRefusal(
	status: number,
	code: string | undefined,
	error: HubError,
): ScheduleRefusal | undefined {
	if (status === 403 && error.code === "forbidden")
		return { kind: "schedule_role" };
	const conflict =
		status === 409 && code && Object.hasOwn(SCHEDULE_CONFLICTS, code)
			? SCHEDULE_CONFLICTS[code]
			: undefined;
	return conflict ? { kind: conflict } : undefined;
}

const appEventPath = (appId: string, eventId: string) =>
	`apps/${segment(appId)}/device-schedules/${segment(eventId)}`;

export interface ScheduleRelease {
	/** `device`: the service already runs it. */
	state: "released" | "device";
	since: number;
}

/**
 * Lets one service take a schedule off the hub once it runs (needs the right
 * to edit the app's events). The hub keeps running it until that service's
 * device claims it.
 */
export function releaseSchedule(
	api: IApiState,
	profile: IProfile,
	appId: string,
	eventId: string,
	deviceId: string,
	placementId: string,
): Promise<HubResult<ScheduleRelease> | ScheduleRefusal> {
	return withRefusals(
		hubCall(
			"app",
			"PUT apps/{app_id}/device-schedules/{event_id}",
			() =>
				api.put(profile, appEventPath(appId, eventId), {
					device_id: deviceId,
					placement_id: placementId,
				}),
			(value) =>
				z
					.object({ state: z.enum(["released", "device"]), since: time })
					.parse(value),
		),
		scheduleRefusal,
	);
}

export interface ScheduleGiveBack {
	/** null: the hub was running it all along. */
	hub_resumes_at: number | null;
}

/** Hands a schedule back to the hub, whatever state the device is in. */
export function giveBackSchedule(
	api: IApiState,
	profile: IProfile,
	appId: string,
	eventId: string,
): Promise<HubResult<ScheduleGiveBack> | ScheduleRefusal> {
	return withRefusals(
		hubCall(
			"app",
			"DELETE apps/{app_id}/device-schedules/{event_id}",
			() => api.del(profile, appEventPath(appId, eventId)),
			(value) => z.object({ hub_resumes_at: nullable(time) }).parse(value),
		),
		scheduleRefusal,
	);
}

const versionTriple = z.tuple([count, count, count]);
const boardVersionPath = (appId: string, boardId: string) =>
	`apps/${segment(appId)}/board/${segment(boardId)}/version/current`;

/** The flow as a version: the published version that equals the stored flow, and the newest one. */
export interface FlowVersionState {
	current: [number, number, number] | null;
	newest: [number, number, number] | null;
}

/** The hub route and the desktop command answer with the same shape. */
export const parseFlowVersionState = (value: unknown): FlowVersionState =>
	z
		.object({
			current: nullable(versionTriple),
			newest: nullable(versionTriple),
		})
		.parse(value);

export function readFlowVersion(
	api: IApiState,
	profile: IProfile,
	appId: string,
	boardId: string,
): Promise<HubResult<FlowVersionState>> {
	return hubCall(
		"app",
		"GET apps/{app_id}/board/{board_id}/version/current",
		() => api.get(profile, boardVersionPath(appId, boardId)),
		parseFlowVersionState,
	);
}

export interface FlowVersionPublished {
	version: [number, number, number];
	/** False when a version already equalled the flow. */
	created: boolean;
}

export const parseFlowVersionPublished = (
	value: unknown,
): FlowVersionPublished =>
	z.object({ version: versionTriple, created: z.boolean() }).parse(value);

export type FlowPublishRefusal = HubRefusal<
	"flow_role" | "flow_busy" | "flow_incomparable"
>;

/** Publishes a Patch version of the flow when no published version equals it. */
export function publishFlowVersion(
	api: IApiState,
	profile: IProfile,
	appId: string,
	boardId: string,
): Promise<HubResult<FlowVersionPublished> | FlowPublishRefusal> {
	return withRefusals(
		hubCall(
			"app",
			"POST apps/{app_id}/board/{board_id}/version/current",
			() => api.post(profile, boardVersionPath(appId, boardId)),
			parseFlowVersionPublished,
		),
		(status, _code, error): FlowPublishRefusal | undefined => {
			if (status === 403 && error.code === "forbidden")
				return { kind: "flow_role" };
			if (status === 423) return { kind: "flow_busy" };
			// The hub's own sentence: the flow never compares equal to its published version.
			if (status === 422)
				return { kind: "flow_incomparable", message: serverMessageOf(error) };
			return undefined;
		},
	);
}

/* Existing routes the area polls; errors stay coded (the older readers flatten them). */

/** The owner-signed access rules as stored on the hub; unverified (callers verify the JWS). */
export function readPolicyView(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
): Promise<PolicyView> {
	return hubRead(
		"device",
		"GET devices/{id}/management/policy",
		() => api.get(profile, `${devicePath(deviceId)}/management/policy`),
		(value) => policyViewSchema.parse(value),
	);
}

export function readCertificateInventory(
	api: IApiState,
	profile: IProfile,
	deviceId: string,
): Promise<PublicCertificateInventory> {
	return hubRead(
		"device",
		"GET devices/{id}/certificate-inventory",
		() => api.get(profile, `${devicePath(deviceId)}/certificate-inventory`),
		(value) => publicCertificatesSchema.parse(value),
	);
}

/** Wraps an existing lib read (`listDevices`, `checkDeviceSetup`, …) so its failures are coded. */
export function hubReadWith<T>(
	scope: HubRouteScope,
	operation: string,
	read: () => Promise<T>,
): Promise<T> {
	return hubRead(scope, operation, read, (value) => value);
}

/* GET /api/v1 (hub JSON, unauthenticated): the `standalone` block for G2. */

export type HubStandalone = NonNullable<IHub["standalone"]>;

const lenient = <T extends z.ZodTypeAny>(schema: T) =>
	schema.optional().catch(undefined);
const standaloneSchema = z.object({
	enabled: z.boolean().catch(false),
	max_devices_per_user: lenient(count),
	max_pending_enrollments_per_user: lenient(count),
	enrollment_ttl_seconds: lenient(count),
	api_base_url: lenient(z.string().nullable()),
	telemetry_tiers: lenient(
		z.record(z.object({ max_bytes: count, retention_seconds: count })),
	),
	release_trust: lenient(
		z
			.object({
				manifest_url: z.string(),
				public_keys: z.array(z.string()),
				minimum_sequence: count,
			})
			.nullable(),
	),
});

/** A hub without a `standalone` block (or with `null`) has device support off. */
export function parseHubStandalone(hub: unknown): HubStandalone {
	const block = (hub as { standalone?: unknown } | null)?.standalone;
	if (block === undefined || block === null) return { enabled: false };
	return standaloneSchema.parse(block);
}

export async function readHubStandalone(
	origin: string,
	fetchImpl: typeof fetch = globalThis.fetch,
	signal?: AbortSignal,
): Promise<HubStandalone> {
	const path = `${origin}/api/v1`;
	const operation = "GET /api/v1";
	let response: Response;
	try {
		response = await fetchImpl(path, {
			signal,
			cache: "no-store",
			headers: { "Cache-Control": "no-cache", Pragma: "no-cache" },
		});
	} catch (error) {
		throw toHubError(error, "account", operation);
	}
	const text = await response.text().catch(() => "");
	if (!response.ok)
		throw toHubError(
			apiResponseError(response, text, path),
			"account",
			operation,
		);
	let body: unknown;
	try {
		body = JSON.parse(text);
	} catch {
		body = text;
	}
	const upstream = upstreamFailureInSuccess(response, body, path);
	if (upstream) throw toHubError(upstream, "account", operation);
	if (typeof body !== "object" || body === null || Array.isArray(body))
		throw new HubError("invalid_response", `${operation}: no hub record`);
	try {
		return parseHubStandalone(body);
	} catch (error) {
		throw toHubError(error, "account", operation);
	}
}
