import type { AcmeCertificate } from "../../../../lib/device-management/certificate-acme";
import type { LocalCertificateAuthority } from "../../../../lib/device-management/certificate-authority";
import type { CertificateIssuer } from "../../../../lib/device-management/certificate-issuance";
import type {
	DeviceCertificate,
	PublicCertificateInventory,
} from "../../../../lib/device-management/certificates";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import {
	presence,
	relationshipOf,
} from "../../../../lib/device-management/model/presence";
import type {
	DeviceRow,
	LiveDeviceInput,
	Presence,
	Relationship,
} from "../../../../lib/device-management/model/types";

export const DAY_S = 86_400;
export const SOON_DAYS = 7;
export const MONTH_DAYS = 30;
/** R11: certificate rows per page, and rows of every other list before "Show N more". */
export const PAGE_SIZE = 50;
export const LIST_CAP = 8;
export const MAX_AUTHORITY_NAMES = 32;
export const AUTHORITY_SIGNING_SOON_DAYS = 30;
export const AUTHORITY_ROOT_SOON_DAYS = 90;

export type RenewalMode = "acme" | "delegated" | "manual";

/** One certificate a device reported to the hub, with what a live read added on this computer. */
export interface CertificateRow {
	key: string;
	device: DeviceRow;
	deviceName: string;
	presence: Presence;
	certificateId: string;
	fingerprint: string;
	revision: number;
	notAfter: number;
	/** Unix seconds the device last reported its certificates. */
	reportedAt: number | null;
	days: number;
	/** Names, issuer and bindings: only after a live read of the device. */
	detail?: DeviceCertificate;
	/** Null until the device's renewal settings were read live. */
	mode: RenewalMode | null;
	issuer?: CertificateIssuer;
	acme?: AcmeCertificate;
	failing: boolean;
}

/**
 * Why a device lists no certificate: it never reported, reports none, the
 * viewer's access doesn't cover certificates, or (older hub) its own read is
 * still running or failed.
 */
export type SilentState = "never" | "none" | "noaccess" | "reading" | "unread";

export interface SilentDevice {
	device: DeviceRow;
	deviceName: string;
	presence: Presence;
	relationship: Relationship;
	state: SilentState;
	reportedAt: number | null;
}

export interface CertificateFleet {
	rows: CertificateRow[];
	silent: SilentDevice[];
	revoked: DeviceRow[];
	/** Devices that aren't revoked. */
	devices: DeviceRow[];
	devicesWithCertificates: number;
}

export type PerDeviceRead = "reading" | "forbidden" | "failed";

export interface FleetInput {
	devices: readonly DeviceRow[];
	me: string;
	/** Hub-corrected unix seconds. */
	now: number;
	inventory: Readonly<Record<string, PublicCertificateInventory | undefined>>;
	/** The hub answered the fleet inventory: a device missing from it has no certificate access. */
	fleetKnown: boolean;
	/** Older hub: how each device's own read went. */
	perDevice?: Readonly<Record<string, PerDeviceRead | undefined>>;
	live: Readonly<Record<string, LiveDeviceInput | undefined>>;
}

interface RenewalRow {
	last_error: string | null;
	failures?: number;
}

const renewalBroken = (row: RenewalRow | undefined) =>
	!!row && ((row.failures ?? 0) > 0 || !!row.last_error);

function renewalOf(
	live: LiveDeviceInput | undefined,
	certificateId: string,
	now: number,
): Pick<CertificateRow, "mode" | "issuer" | "acme" | "failing"> {
	const known =
		live?.certificateIssuers !== undefined && live.acme !== undefined;
	const issuer = live?.certificateIssuers?.find(
		(row) => row.certificate_id === certificateId,
	);
	const acme = live?.acme?.find((row) => row.certificate_id === certificateId);
	const mode = acme ? "acme" : issuer ? "delegated" : known ? "manual" : null;
	return {
		mode,
		...(issuer ? { issuer } : {}),
		...(acme ? { acme } : {}),
		failing:
			renewalBroken(acme) ||
			renewalBroken(issuer) ||
			(!!issuer && issuer.not_after <= now),
	};
}

function silentState(
	input: FleetInput,
	row: DeviceRow,
	inventory: PublicCertificateInventory | undefined,
): SilentState {
	if (inventory) return inventory.updated_at === null ? "never" : "none";
	if (input.fleetKnown) return "noaccess";
	const read = input.perDevice?.[row.device_id];
	if (read === "forbidden") return "noaccess";
	return read === "reading" ? "reading" : "unread";
}

const SILENT_RANK: Record<SilentState, number> = {
	never: 0,
	noaccess: 1,
	none: 2,
	reading: 3,
	unread: 4,
};

/** Rows most urgent first; devices without rows grouped by why they have none. */
export function buildCertificateFleet(input: FleetInput): CertificateFleet {
	const rows: CertificateRow[] = [];
	const silent: SilentDevice[] = [];
	const revoked: DeviceRow[] = [];
	const devices: DeviceRow[] = [];
	const withCertificates = new Set<string>();
	for (const device of input.devices) {
		if (device.status === "revoked") {
			revoked.push(device);
			continue;
		}
		devices.push(device);
		const inventory = input.inventory[device.device_id];
		const name = deviceName(device);
		const seen = presence(device, input.now);
		if (inventory && inventory.certificates.length > 0) {
			const live = input.live[device.device_id];
			withCertificates.add(device.device_id);
			for (const certificate of inventory.certificates) {
				const detail = live?.certificates?.certificates.find(
					(row) => row.certificate_id === certificate.certificate_id,
				);
				rows.push({
					key: `${device.device_id}/${certificate.certificate_id}`,
					device,
					deviceName: name,
					presence: seen,
					certificateId: certificate.certificate_id,
					fingerprint: certificate.fingerprint_sha256,
					revision: certificate.revision,
					notAfter: certificate.not_after,
					reportedAt: inventory.updated_at,
					days: (certificate.not_after - input.now) / DAY_S,
					...(detail ? { detail } : {}),
					...renewalOf(live, certificate.certificate_id, input.now),
				});
			}
			continue;
		}
		silent.push({
			device,
			deviceName: name,
			presence: seen,
			relationship: relationshipOf(device, input.me),
			state: silentState(input, device, inventory),
			reportedAt: inventory?.updated_at ?? null,
		});
	}
	rows.sort(
		(a, b) => a.notAfter - b.notAfter || a.key.localeCompare(b.key, "en"),
	);
	silent.sort(
		(a, b) =>
			SILENT_RANK[a.state] - SILENT_RANK[b.state] ||
			a.deviceName.localeCompare(b.deviceName, "en"),
	);
	return {
		rows,
		silent,
		revoked,
		devices,
		devicesWithCertificates: withCertificates.size,
	};
}

/* Filters (toolbar chips and the summary windows). */

export const ROW_FILTERS = ["expired", "week", "month", "errors"] as const;
export type RowFilter = (typeof ROW_FILTERS)[number];
export type WindowFilter = RowFilter | "silent" | "noaccess";

export const isExpired = (row: Pick<CertificateRow, "days">) => row.days < 0;

const ROW_MATCH: Record<RowFilter, (row: CertificateRow) => boolean> = {
	expired: isExpired,
	week: (row) => row.days >= 0 && row.days <= SOON_DAYS,
	month: (row) => row.days >= 0 && row.days <= MONTH_DAYS,
	errors: (row) => row.failing,
};

export const matchesFilter = (row: CertificateRow, filter: RowFilter) =>
	ROW_MATCH[filter](row);

export const isRowFilter = (filter: WindowFilter | null): filter is RowFilter =>
	filter !== null && (ROW_FILTERS as readonly string[]).includes(filter);

/** A certificate that renews by itself and whose renewal isn't failing. */
export const selfRenewing = (row: CertificateRow) =>
	!row.failing && (row.mode === "acme" || row.mode === "delegated");

/** Expired, or expiring within a week without a working automatic renewal. */
export const needsYou = (row: CertificateRow) =>
	isExpired(row) || (row.days <= SOON_DAYS && !selfRenewing(row));

export const usedCount = (row: CertificateRow) =>
	row.detail
		? (row.detail.binding_count ?? row.detail.bindings.length)
		: undefined;

export interface FilteredFleet {
	rows: CertificateRow[];
	silent: SilentDevice[];
	/** The window filter shows devices only. */
	devicesOnly: boolean;
}

/** Which devices without rows a filter keeps: the two device windows their own state, a row filter none. */
function silentMatches(
	entry: SilentDevice,
	filter: WindowFilter | null,
): boolean {
	if (filter === "silent") return entry.state === "never";
	if (filter === "noaccess") return entry.state === "noaccess";
	return filter === null;
}

export function filterFleet(
	fleet: CertificateFleet,
	filter: WindowFilter | null,
	deviceId: string | null,
): FilteredFleet {
	const devicesOnly = filter === "silent" || filter === "noaccess";
	const ofDevice = (entry: { device: DeviceRow }) =>
		!deviceId || entry.device.device_id === deviceId;
	const rowFilter = isRowFilter(filter) ? filter : null;
	const rows = devicesOnly
		? []
		: fleet.rows.filter(
				(row) => ofDevice(row) && (!rowFilter || matchesFilter(row, rowFilter)),
			);
	const silent = fleet.silent.filter(
		(entry) => ofDevice(entry) && silentMatches(entry, filter),
	);
	return { rows, silent, devicesOnly };
}

export interface FleetCounts {
	expired: CertificateRow[];
	week: CertificateRow[];
	month: CertificateRow[];
	errors: CertificateRow[];
	never: SilentDevice[];
	noAccess: SilentDevice[];
	/** An expired certificate that a service still uses: the only critical state here. */
	expiredInUse: boolean;
	/** Within 30 days and no renewal settings read yet. */
	renewalUnknown: number;
	needYou: number;
}

export function fleetCounts(fleet: CertificateFleet): FleetCounts {
	const of = (filter: RowFilter) =>
		fleet.rows.filter((row) => matchesFilter(row, filter));
	const expired = of("expired");
	return {
		expired,
		week: of("week"),
		month: of("month"),
		errors: of("errors"),
		never: fleet.silent.filter((entry) => entry.state === "never"),
		noAccess: fleet.silent.filter((entry) => entry.state === "noaccess"),
		expiredInUse: expired.some((row) => (usedCount(row) ?? 0) > 0),
		renewalUnknown: fleet.rows.filter(
			(row) => row.mode === null && row.days <= MONTH_DAYS,
		).length,
		needYou: fleet.rows.filter(needsYou).length,
	};
}

/* Reminders (the hub's fixed stages). */

export const REMINDER_STAGES = [
	{ id: "week", daysBefore: 7 },
	{ id: "three_days", daysBefore: 3 },
	{ id: "day", daysBefore: 1 },
	{ id: "expired", daysBefore: 0 },
] as const;
export type ReminderStage = (typeof REMINDER_STAGES)[number]["id"];

export interface UpcomingReminder {
	row: CertificateRow;
	next: { stage: ReminderStage; at: number }[];
}

/** Expected reminder times per certificate, soonest first; certificates past every stage last. */
export function upcomingReminders(
	rows: readonly CertificateRow[],
	now: number,
): UpcomingReminder[] {
	const first = (entry: UpcomingReminder) =>
		entry.next[0]?.at ?? Number.POSITIVE_INFINITY;
	return rows
		.map((row) => ({
			row,
			next: REMINDER_STAGES.map((stage) => ({
				stage: stage.id,
				at: row.notAfter - stage.daysBefore * DAY_S,
			})).filter((stage) => stage.at > now),
		}))
		.sort((a, b) => first(a) - first(b) || a.row.notAfter - b.row.notAfter);
}

export type CertificateLookup =
	| { kind: "short" }
	| { kind: "found"; row: CertificateRow }
	| { kind: "missing" };

/** Reminders name a certificate by its ID only: match what the reader pasted from one. */
export function findCertificate(
	rows: readonly CertificateRow[],
	query: string,
): CertificateLookup {
	const wanted = query.toLowerCase().replace(/[^0-9a-f]/g, "");
	if (wanted.length < 8) return { kind: "short" };
	const row = rows.find((entry) =>
		entry.certificateId.replace(/-/g, "").startsWith(wanted),
	);
	return row ? { kind: "found", row } : { kind: "missing" };
}

/* Organisation authorities. */

export type AuthorityStatus = "active" | "signing_expired" | "root_expired";

export interface AuthorityView {
	authority: LocalCertificateAuthority;
	id: string;
	label: string;
	suffixes: readonly string[];
	addresses: readonly string[];
	fingerprint: string;
	createdAt: number;
	rootExpiresAt: number;
	signingExpiresAt: number;
	status: AuthorityStatus;
	/** Still signing, but for less than 30 days. */
	signingSoon: boolean;
	rootSoon: boolean;
}

export function authorityView(
	authority: LocalCertificateAuthority,
	now: number,
): AuthorityView {
	const bundle = authority.public_bundle;
	const status: AuthorityStatus =
		bundle.not_after <= now
			? "root_expired"
			: bundle.issuer_not_after <= now
				? "signing_expired"
				: "active";
	return {
		authority,
		id: bundle.authority_id,
		label: bundle.label,
		suffixes: bundle.dns_suffixes,
		addresses: bundle.ip_addresses,
		fingerprint: bundle.sha256_fingerprint,
		createdAt: bundle.not_before,
		rootExpiresAt: bundle.not_after,
		signingExpiresAt: bundle.issuer_not_after,
		status,
		signingSoon:
			status === "active" &&
			bundle.issuer_not_after - now <= AUTHORITY_SIGNING_SOON_DAYS * DAY_S,
		rootSoon:
			status === "active" &&
			bundle.not_after - now <= AUTHORITY_ROOT_SOON_DAYS * DAY_S,
	};
}

/** Whether this authority can sign today. */
export const canSign = (view: AuthorityView) => view.status === "active";

/** Certificates a live read attributes to this authority (its issuer name ends in "service issuer"). */
export function signedBy(
	rows: readonly CertificateRow[],
	label: string,
): CertificateRow[] {
	const issuer = `${label} service issuer`;
	return rows.filter((row) => row.detail?.issuer.includes(issuer));
}

/* Create authority: the same limits the signing code enforces. */

const DNS_LABEL = "[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?";
const DNS_SUFFIX = new RegExp(`^${DNS_LABEL}(\\.${DNS_LABEL})*$`);
const IPV4 = /^(25[0-5]|2[0-4]\d|1?\d?\d)(\.(25[0-5]|2[0-4]\d|1?\d?\d)){3}$/;

function isIpAddress(value: string): boolean {
	if (IPV4.test(value)) return true;
	const groups = value.split(":");
	return (
		/^[0-9a-f:]+$/i.test(value) && groups.length >= 3 && groups.length <= 8
	);
}

export type SuffixProblem =
	| "wildcard"
	| "port_or_path"
	| "trailing_dot"
	| "characters"
	| "numeric_end"
	| "duplicate";
export type AddressProblem = "not_an_address" | "duplicate";

/** In this order: the first check that fails names the problem. */
const SUFFIX_CHECKS: readonly [SuffixProblem, (suffix: string) => boolean][] = [
	["wildcard", (suffix) => suffix.includes("*")],
	["port_or_path", (suffix) => /:\d+$|\//.test(suffix)],
	["trailing_dot", (suffix) => suffix.endsWith(".")],
	["characters", (suffix) => !DNS_SUFFIX.test(suffix)],
	["numeric_end", (suffix) => /(^|\.)\d+$/.test(suffix)],
];

export function suffixProblem(
	value: string,
	seen: ReadonlySet<string>,
): SuffixProblem | null {
	const suffix = value.trim().toLowerCase();
	if (!suffix) return null;
	const failed = SUFFIX_CHECKS.find(([, fails]) => fails(suffix));
	if (failed) return failed[0];
	return seen.has(suffix) ? "duplicate" : null;
}

export function addressProblem(
	value: string,
	seen: ReadonlySet<string>,
): AddressProblem | null {
	const address = value.trim();
	if (!address) return null;
	if (!isIpAddress(address)) return "not_an_address";
	return seen.has(address) ? "duplicate" : null;
}

export interface AuthorityDraft {
	label: string;
	suffixes: readonly string[];
	addresses: readonly string[];
	years: string;
}

export const cleanSuffixes = (draft: AuthorityDraft) =>
	draft.suffixes.map((value) => value.trim().toLowerCase()).filter(Boolean);
export const cleanAddresses = (draft: AuthorityDraft) =>
	draft.addresses.map((value) => value.trim()).filter(Boolean);
export const nameCount = (draft: AuthorityDraft) =>
	cleanSuffixes(draft).length + cleanAddresses(draft).length;

/** A whole number of years from 1 to 10, or null. */
export function validYears(value: string): number | null {
	const years = Number(value);
	return Number.isInteger(years) && years >= 1 && years <= 10 ? years : null;
}

/** Root and signing-key expiry a new authority gets (signing keys last one year at most). */
export function authorityDates(years: number, now: number) {
	const root = now + years * 365 * DAY_S;
	return { root, signing: Math.min(root, now + 365 * DAY_S) };
}
