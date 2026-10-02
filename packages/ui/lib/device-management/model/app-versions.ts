import { sha256 } from "@noble/hashes/sha2";
import { eventEligibility } from "../deployment";
import type { ActivityItem } from "../workspace/types";
import type {
	AppEventInput,
	AppServiceRow,
	AppVersionInput,
	AppVersionPin,
	AppVersionView,
	AppView,
	VersionDiffRow,
} from "./app-plan";
import { fleetFacts } from "./device-view";
import type { AttentionInput, PlacementEvent, ServiceView } from "./types";

/*
 * App versions without a hub history (APP A7): what the app pins now, plus
 * every older revision a readable service still runs. Pure.
 */

type Pin = Omit<AppVersionPin, "eventId">;
type Triple = readonly number[];

const sameTriple = (a: Triple, b: Triple) =>
	a.length === b.length && a.every((value, index) => value === b[index]);

const samePin = (a: Pin, b: Pin) =>
	sameTriple(a.eventVersion, b.eventVersion) &&
	sameTriple(a.boardVersion, b.boardVersion);

const compareTriple = (a: Triple, b: Triple) => {
	for (let index = 0; index < Math.max(a.length, b.length); index++) {
		const delta = (a[index] ?? 0) - (b[index] ?? 0);
		if (delta) return delta;
	}
	return 0;
};

const pinOf = (event: PlacementEvent): AppVersionPin => ({
	eventId: event.event_id,
	eventVersion: event.event_version,
	boardVersion: event.board_version,
});

/** "1.5.0" → "v1.5.0"; any other text is shown as the app wrote it. */
export function versionLabel(text: string | null | undefined): string | null {
	const value = text?.trim();
	if (!value) return null;
	return /^\d/.test(value) ? `v${value}` : value;
}

/** "v1.5.0", or the short hash when the version has no name. */
export function versionName(
	version: Pick<AppVersionView, "label" | "short">,
): string {
	return version.label ?? version.short;
}

/** A7: the newest app version pins every event that can run on a device at the versions published now. */
export function newestPins(events: readonly AppEventInput[]): AppVersionPin[] {
	return events.flatMap((event) => {
		const rule = eventEligibility(event, {
			ineligibleReason: event.ineligibleReason,
		});
		return rule.eligible && rule.eventVersion && rule.boardVersion
			? [
					{
						eventId: event.id,
						eventVersion: rule.eventVersion,
						boardVersion: rule.boardVersion,
					},
				]
			: [];
	});
}

/** A stable identifier of a pin set, the same on every computer. */
export function definitionHash(pins: readonly AppVersionPin[]): string {
	const rows = [...pins]
		.sort((a, b) => a.eventId.localeCompare(b.eventId))
		.map((pin) => [pin.eventId, pin.eventVersion, pin.boardVersion]);
	const bytes = new TextEncoder().encode(JSON.stringify(rows));
	return Array.from(sha256(bytes), (byte) =>
		byte.toString(16).padStart(2, "0"),
	).join("");
}

export interface VersionSources {
	/** The app's own version text. */
	label?: string | null;
	/** Unix seconds the app last changed. */
	changedAt?: number | null;
	events: readonly AppEventInput[];
	/** Readable services of this app. */
	services: readonly Pick<ServiceView, "appVersion" | "events">[];
	/** Unix seconds this computer sent a revision, by hash. */
	sentAt?: Readonly<Record<string, number>>;
}

interface Observed {
	hash: string;
	pins: Map<string, AppVersionPin>;
}

function observedRevisions(services: VersionSources["services"]): Observed[] {
	const found = new Map<string, Observed>();
	for (const service of services) {
		const hash = service.appVersion?.hash;
		if (!hash || !service.events?.length) continue;
		const entry = found.get(hash) ?? { hash, pins: new Map() };
		for (const event of service.events)
			entry.pins.set(event.event_id, pinOf(event));
		found.set(hash, entry);
	}
	return [...found.values()].sort((a, b) => a.hash.localeCompare(b.hash));
}

/** Every pin the revision serves is the newest one of its event. */
function isCurrent(
	pins: Iterable<AppVersionPin>,
	newest: ReadonlyMap<string, AppVersionPin>,
): boolean {
	let seen = 0;
	for (const pin of pins) {
		const target = newest.get(pin.eventId);
		if (!target || !samePin(pin, target)) return false;
		seen++;
	}
	return seen > 0;
}

/** +1 when `pin` is the newer of the two, −1 when it is the older one. */
const pinOrder = (pin: Pin, other: Pin) =>
	Math.sign(compareTriple(pin.eventVersion, other.eventVersion)) ||
	Math.sign(compareTriple(pin.boardVersion, other.boardVersion));

/** How many of the events both revisions serve are newer in `a` than in `b`, minus the older ones. */
function pinScore(a: Observed, b: Observed): number {
	let score = 0;
	for (const [eventId, pin] of a.pins) {
		const other = b.pins.get(eventId);
		if (other) score += pinOrder(pin, other);
	}
	return score;
}

/** Newer first: by when this computer sent them, else by the versions they pin. */
function byAge(sentAt: VersionSources["sentAt"] = {}) {
	const sent = (entry: Observed) => sentAt[entry.hash] ?? 0;
	return (a: Observed, b: Observed) =>
		sent(b) - sent(a) || -pinScore(a, b) || a.hash.localeCompare(b.hash);
}

/**
 * The hub keeps no history of app versions and a device reports only the
 * revision it runs. The list is therefore: what the app pins now, then every
 * older revision a readable service still runs, newest first.
 */
export function versionInputs(sources: VersionSources): AppVersionInput[] {
	const pins = newestPins(sources.events);
	const newest = new Map(pins.map((pin) => [pin.eventId, pin]));
	const observed = observedRevisions(sources.services);
	const current = observed.find((entry) =>
		isCurrent(entry.pins.values(), newest),
	);
	const older = observed
		.filter((entry) => !isCurrent(entry.pins.values(), newest))
		.sort(byAge(sources.sentAt));
	if (!pins.length && !older.length) return [];
	return [
		{
			hash: current?.hash ?? definitionHash(pins),
			label: versionLabel(sources.label),
			builtAt: sources.changedAt ?? null,
			by: null,
			pins,
		},
		...older.map((entry) => ({
			hash: entry.hash,
			label: null,
			builtAt: sources.sentAt?.[entry.hash] ?? null,
			by: null,
			pins: [...entry.pins.values()],
		})),
	];
}

function rowIsCurrent(row: AppServiceRow, newest: AppVersionView): boolean {
	if (!row.events?.length) return false;
	const pins = new Map(newest.pins.map((pin) => [pin.eventId, pin]));
	return isCurrent(row.events.map(pinOf), pins);
}

/**
 * An older revision is known only through the events its services serve, so
 * "new in" can't be told from "not served there": only changed pins count,
 * plus events the newest version no longer has.
 */
function knownDiff(
	version: AppVersionView,
	newestIndex: number,
): VersionDiffRow[] | null {
	if (!version.diff) return null;
	return version.diff.filter(
		(row) =>
			row.kind === "changed" ||
			(row.kind === "removed" && version.index === newestIndex),
	);
}

/**
 * Aligns the model's hash match with the pins: a service that serves the
 * newest version of each of its events runs the newest app version, whatever
 * revision carried it there.
 */
export function refineView(view: AppView): AppView {
	const newest = view.versions[0];
	if (!newest) return view;
	const versions = view.versions.map((version) => ({
		...version,
		diff: knownDiff(version, 0),
		runningOn: [] as AppVersionView["runningOn"],
	}));
	const byHash = new Map(versions.map((version) => [version.hash, version]));
	const refine = (row: AppServiceRow): AppServiceRow => {
		const version = rowIsCurrent(row, newest)
			? versions[0]
			: row.version
				? (byHash.get(row.version.hash) ?? null)
				: null;
		if (version)
			version.runningOn.push({
				deviceId: row.deviceId,
				serviceId: row.serviceId,
				conv: row.view.conv,
				lastKnown: row.lastKnown,
			});
		return { ...row, version, behind: version ? version.index : null };
	};
	const groups = view.groups.map((group) => ({
		...group,
		services: group.services.map(refine),
	}));
	const services = groups.flatMap((group) => group.services);
	const unknownOn = [
		...groups.filter((group) => group.unknown).map((group) => group.deviceId),
		...services
			.filter((row) => !row.version)
			.map((row) => `${row.deviceId}/${row.serviceId}`),
	];
	for (const version of versions) version.unknownOn = unknownOn;
	return {
		...view,
		groups,
		services,
		versions,
		howRuns: { ...view.howRuns, newest: versions[0] },
		events: {
			...view.events,
			rows: view.events.rows.map((row) => ({ ...row, newIn: null })),
			ineligible: view.events.ineligible.map((row) => ({
				...row,
				newIn: null,
			})),
		},
		newestRuns: {
			version: versions[0],
			services: versions[0].runningOn.length,
			of: services.length,
			unknown: services.filter((row) => !row.version).length,
		},
	};
}

/** The readable services of one app across the fleet: what its older revisions are known from. */
export function appServices(
	input: AttentionInput,
	appId: string,
): ServiceView[] {
	return fleetFacts(input).devices.flatMap((device) =>
		device.active && Array.isArray(device.services)
			? device.services.filter((service) => service.projectId === appId)
			: [],
	);
}

/** When this computer sent each revision of the app, by hash (unix seconds; the activity tray is the only log, BG12). */
export function revisionsSent(
	items: readonly ActivityItem[],
	appId: string,
): Record<string, number> {
	const sent: Record<string, number> = {};
	for (const item of items) {
		const handle = item.resume;
		if (handle?.type !== "transfer" || handle.projectId !== appId) continue;
		if (item.state !== "done" || item.finishedAt === undefined) continue;
		const at = Math.floor(item.finishedAt / 1000);
		sent[handle.manifestSha256] = Math.max(
			sent[handle.manifestSha256] ?? 0,
			at,
		);
	}
	return sent;
}
