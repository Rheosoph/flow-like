import type {
	AppDevicesRoute,
	AttentionItem,
	AttentionKey,
	CopyParams,
	CopyRef,
	DeployRoute,
	DeviceRoute,
	DeviceRow,
	DevicesRoute,
	KeysRoute,
	ServiceRoute,
	SetupRoute,
} from "../../../../lib/device-management/model/types";
import type { LocalVaultSummary } from "../../../../lib/device-management/workspace/types";

export type ResolveCode =
	| "device_not_found"
	| "device_not_shared"
	| "device_other_account"
	| "device_revoked"
	| "service_not_found"
	| "certificate_not_found"
	| "app_not_found"
	| "setup_not_found";

/** What the area knows after the first fleet load. Every optional list: absent = not known, so nothing is reported missing. */
export interface ResolveFleet {
	devices: readonly DeviceRow[];
	/** Hub host for "not found on <hub>". */
	hub?: string;
	/** A `shared` vault for a device the hub no longer lists means the share ended. */
	vaults?: readonly Pick<LocalVaultSummary, "deviceId" | "role">[];
	/** Devices this computer knows under another signed-in account. */
	otherAccountDeviceIds?: ReadonlySet<string>;
	/** For the legacy `?device=<id>` link without a tab. */
	attention?: readonly Pick<AttentionItem, "key" | "subject">[];
	/** Placement ids per device from a readable plane; a device without an entry is not known. */
	services?: Readonly<Record<string, readonly string[]>>;
	/** Certificate ids per device from a readable inventory; a device without an entry is not known. */
	certificates?: Readonly<Record<string, readonly string[]>>;
	appIds?: ReadonlySet<string>;
	pendingEnrollmentIds?: ReadonlySet<string>;
}

export type ResolveResult =
	| { ok: true; route: DevicesRoute; changed: boolean }
	| { ok: false; banner: CopyRef<ResolveCode>; fallback: DevicesRoute };

const CERTIFICATE_KEYS: ReadonlySet<AttentionKey> = new Set([
	"certificate_expired",
	"certificate_expiring",
	"certificate_not_yet_valid",
	"renewal_delegation_error",
	"renewal_authority_expiring",
	"acme_error",
	"acme_staging_in_use",
	"signing_request_attention",
	"certificate_inventory_stale",
	"certificate_slots_nearly_full",
]);

const FLEET: DevicesRoute = { screen: "fleet", view: "devices" };

type Lookup =
	| { row: DeviceRow; problem?: undefined }
	| { row?: undefined; problem: CopyRef<ResolveCode> };

function banner(code: ResolveCode, params: CopyParams): CopyRef<ResolveCode> {
	return { code, params };
}

function deviceLabel(row: DeviceRow): string {
	return row.display_name || row.name;
}

function revoked(row: DeviceRow): CopyRef<ResolveCode> | undefined {
	if (row.status !== "revoked") return undefined;
	const params: CopyParams = { device: deviceLabel(row) };
	if (typeof row.revoked_at === "number") params.revokedAt = row.revoked_at;
	return banner("device_revoked", params);
}

function lookup(deviceId: string, fleet: ResolveFleet): Lookup {
	const row = fleet.devices.find((device) => device.device_id === deviceId);
	if (row) return { row };
	if (fleet.otherAccountDeviceIds?.has(deviceId))
		return { problem: banner("device_other_account", { device: deviceId }) };
	if (
		fleet.vaults?.some(
			(vault) => vault.deviceId === deviceId && vault.role === "shared",
		)
	)
		return { problem: banner("device_not_shared", { device: deviceId }) };
	const params: CopyParams = { device: deviceId };
	if (fleet.hub) params.hub = fleet.hub;
	return { problem: banner("device_not_found", params) };
}

function knownMissing(
	known: Readonly<Record<string, readonly string[]>> | undefined,
	deviceId: string,
	id: string,
): boolean {
	const ids = known?.[deviceId];
	return ids !== undefined && !ids.includes(id);
}

function hasCertificateAttention(
	deviceId: string,
	fleet: ResolveFleet,
): boolean {
	return (fleet.attention ?? []).some(
		({ key, subject }) =>
			"deviceId" in subject &&
			subject.deviceId === deviceId &&
			(subject.kind === "certificate" || CERTIFICATE_KEYS.has(key)),
	);
}

function settle<R extends DevicesRoute>(
	original: R,
	route: R,
	problems: readonly CopyRef<ResolveCode>[],
): ResolveResult {
	const [first] = problems;
	if (first) return { ok: false, banner: first, fallback: route };
	return { ok: true, route, changed: route !== original };
}

/** Only ever called with optional route fields, so the result is still an `R`. */
function without<R extends object>(route: R, key: keyof R): R {
	const { [key]: _removed, ...rest } = route;
	return rest as R;
}

/** IA §6.1.4: a legacy link opens Certificates when the device has a certificate item, else Overview. */
function resolveDevice(route: DeviceRoute, fleet: ResolveFleet): ResolveResult {
	const found = lookup(route.deviceId, fleet);
	if (found.problem)
		return { ok: false, banner: found.problem, fallback: FLEET };
	const problems: CopyRef<ResolveCode>[] = [];
	let next = route;
	if (!next.tab) {
		const certificates =
			next.certificateId !== undefined ||
			hasCertificateAttention(route.deviceId, fleet);
		next = { ...next, tab: certificates ? "certificates" : "overview" };
	}
	const revocation = revoked(found.row);
	if (next.action && revocation) {
		problems.push(revocation);
		next = without(next, "action");
	}
	if (
		next.certificateId &&
		knownMissing(fleet.certificates, route.deviceId, next.certificateId)
	) {
		problems.push(
			banner("certificate_not_found", {
				device: deviceLabel(found.row),
				certificate: next.certificateId,
			}),
		);
		next = { ...without(next, "certificateId"), tab: "certificates" };
	}
	return settle(route, next, problems);
}

function resolveService(
	route: ServiceRoute,
	fleet: ResolveFleet,
): ResolveResult {
	const found = lookup(route.deviceId, fleet);
	if (found.problem)
		return { ok: false, banner: found.problem, fallback: FLEET };
	const revocation = revoked(found.row);
	if (revocation)
		return {
			ok: false,
			banner: revocation,
			fallback: { screen: "device", deviceId: route.deviceId, tab: "overview" },
		};
	if (knownMissing(fleet.services, route.deviceId, route.serviceId))
		return {
			ok: false,
			banner: banner("service_not_found", {
				device: deviceLabel(found.row),
				service: route.serviceId,
			}),
			fallback: { screen: "device", deviceId: route.deviceId, tab: "services" },
		};
	return { ok: true, route, changed: false };
}

/** Unknown or revoked targets leave the plan; the first problem is the banner. */
function resolveDeploy(route: DeployRoute, fleet: ResolveFleet): ResolveResult {
	const problems: CopyRef<ResolveCode>[] = [];
	let next = route;
	if (next.appId && fleet.appIds && !fleet.appIds.has(next.appId)) {
		problems.push(banner("app_not_found", { app: next.appId }));
		next = without(next, "appId");
	}
	const kept: string[] = [];
	const rows = new Map<string, DeviceRow>();
	for (const deviceId of next.deviceIds) {
		const found = lookup(deviceId, fleet);
		const problem = found.problem ?? revoked(found.row);
		if (problem) problems.push(problem);
		else if (found.row) {
			kept.push(deviceId);
			rows.set(deviceId, found.row);
		}
	}
	if (kept.length !== next.deviceIds.length) {
		next = { ...next, deviceIds: kept };
		if (kept.length === 0) next = without(next, "serviceId");
	}
	const [only] = next.deviceIds;
	const row = only ? rows.get(only) : undefined;
	if (
		row &&
		next.deviceIds.length === 1 &&
		next.serviceId &&
		knownMissing(fleet.services, row.device_id, next.serviceId)
	) {
		problems.push(
			banner("service_not_found", {
				device: deviceLabel(row),
				service: next.serviceId,
			}),
		);
		next = without(next, "serviceId");
	}
	return settle(route, next, problems);
}

function resolveSetup(route: SetupRoute, fleet: ResolveFleet): ResolveResult {
	if (
		route.enrollmentId &&
		fleet.pendingEnrollmentIds &&
		!fleet.pendingEnrollmentIds.has(route.enrollmentId)
	)
		return {
			ok: false,
			banner: banner("setup_not_found", { enrollment: route.enrollmentId }),
			fallback: FLEET,
		};
	return { ok: true, route, changed: false };
}

/** A highlight of a device that is not listed is dropped without a banner. */
function dropUnknownFocus(
	route: KeysRoute | AppDevicesRoute,
	fleet: ResolveFleet,
): ResolveResult {
	const focus = route.focusDeviceId;
	if (!focus || fleet.devices.some((device) => device.device_id === focus))
		return { ok: true, route, changed: false };
	return { ok: true, route: without(route, "focusDeviceId"), changed: true };
}

/** Run once after the first fleet load. A banner names the reason; the caller replaces the URL with `fallback`. */
export function resolveTarget(
	route: DevicesRoute,
	fleet: ResolveFleet,
): ResolveResult {
	switch (route.screen) {
		case "device":
			return resolveDevice(route, fleet);
		case "service":
			return resolveService(route, fleet);
		case "deploy":
			return resolveDeploy(route, fleet);
		case "setup":
			return resolveSetup(route, fleet);
		case "keys":
		case "app-devices":
			return dropUnknownFocus(route, fleet);
		default:
			return { ok: true, route, changed: false };
	}
}
