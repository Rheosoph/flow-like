import type { IRegistryState } from "../../state/backend-state/registry-state";
import {
	type ConsentStorage,
	type MicroWidgetConsentEntry,
	clearMicroWidgetConsents,
	clearPackageMicroWidgetConsents,
	listMicroWidgetConsents,
	revokeMicroWidgetConsent,
	unmuteMicroWidgetRuntime,
} from "../a2ui/micro-widget-capability-consent";
import {
	WIDGET_CSP_KEYS,
	type WidgetCspKey,
	type WidgetSourceLevel,
	maxWidgetSourceLevel,
	policySourceEntries,
} from "../a2ui/micro-widget-policy";

/** `appId: null` matches grants made outside a project; `undefined` matches every project. */
export interface MicroWidgetConsentQuery {
	appId?: string | null;
	packageId?: string;
	widgetId?: string;
}

type WidgetGrantRegistry = Pick<IRegistryState, "revokeWidgetGrants">;

export function listWidgetConsentEntries(
	query: MicroWidgetConsentQuery = {},
	storage?: ConsentStorage | null,
): MicroWidgetConsentEntry[] {
	const { appId, packageId, widgetId } = query;
	return listMicroWidgetConsents(appId ?? undefined, storage).filter(
		({ target }) =>
			(appId === undefined || (target.appId ?? null) === appId) &&
			(packageId === undefined || target.packageId === packageId) &&
			(widgetId === undefined || target.widgetId === widgetId),
	);
}

function grantRevoker(
	registry: WidgetGrantRegistry | null | undefined,
): NonNullable<WidgetGrantRegistry["revokeWidgetGrants"]> | null {
	try {
		const revoke = registry?.revokeWidgetGrants;
		return typeof revoke === "function" ? revoke.bind(registry) : null;
	} catch {
		return null;
	}
}

/**
 * Desktop grants bind (package, widget), not the project. They are dropped only
 * once no grant on this device still covers the widget, so revoking in one
 * project never silently downgrades a widget another project still allows.
 */
async function revokeUncoveredGrants(
	entries: readonly MicroWidgetConsentEntry[],
	registry: WidgetGrantRegistry | null | undefined,
	storage: ConsentStorage | null | undefined,
): Promise<void> {
	const revoke = grantRevoker(registry);
	if (!revoke || entries.length === 0) return;
	const remaining = listMicroWidgetConsents(undefined, storage);
	const widgets = new Map<string, { packageId: string; widgetId: string }>();
	for (const { target } of entries) {
		const covered = remaining.some(
			(entry) =>
				entry.target.packageId === target.packageId &&
				entry.target.widgetId === target.widgetId,
		);
		if (!covered) {
			widgets.set(JSON.stringify([target.packageId, target.widgetId]), target);
		}
	}
	await Promise.all(
		[...widgets.values()].map(({ packageId, widgetId }) =>
			revoke(packageId, widgetId),
		),
	);
}

export async function revokeWidgetPermissions(
	entries: readonly MicroWidgetConsentEntry[],
	registry: WidgetGrantRegistry | null | undefined,
	storage?: ConsentStorage | null,
): Promise<void> {
	for (const { target } of entries) revokeMicroWidgetConsent(target, storage);
	await revokeUncoveredGrants(entries, registry, storage);
}

/** Revokes every grant of one project, including session grants held by other tabs. */
export async function revokeAppWidgetPermissions(
	appId: string,
	registry: WidgetGrantRegistry | null | undefined,
	storage?: ConsentStorage | null,
): Promise<number> {
	const entries = listMicroWidgetConsents(appId, storage);
	clearMicroWidgetConsents({ appId }, storage);
	await revokeUncoveredGrants(entries, registry, storage);
	return entries.length;
}

/** Revokes every grant for a package's widgets on this device, in every project, including session grants held by other tabs. */
export async function clearPackageWidgetPermissions(
	packageId: string,
	registry: WidgetGrantRegistry | null | undefined,
	storage?: ConsentStorage | null,
): Promise<number> {
	const entries = listWidgetConsentEntries({ packageId }, storage);
	clearPackageMicroWidgetConsents(packageId, storage);
	await grantRevoker(registry)?.(packageId);
	return entries.length;
}

/** "Ask again": runtime addresses given to the widget prompt again. */
export function askAgainForWidgetRuntime(
	entry: Pick<MicroWidgetConsentEntry, "target">,
	storage?: ConsentStorage | null,
): void {
	unmuteMicroWidgetRuntime(entry.target, storage);
}

export interface WidgetConsentSourceRow {
	source: string;
	directives: WidgetCspKey[];
	/** Absent for grants that predate levels. */
	level?: WidgetSourceLevel;
}

export interface WidgetConsentRuntimeRow {
	source: string;
	directives: WidgetCspKey[];
	level: WidgetSourceLevel;
	/** When the address was last approved or used. */
	at: number;
}

function orderedDirectives(keys: Iterable<WidgetCspKey>): WidgetCspKey[] {
	const used = new Set(keys);
	return WIDGET_CSP_KEYS.filter((key) => used.has(key));
}

/** Declared addresses of a grant, each once, with every directive and the level it was approved at. */
export function widgetConsentSourceRows(
	entry: Pick<MicroWidgetConsentEntry, "policy" | "levels">,
): WidgetConsentSourceRow[] {
	const directives = new Map<string, WidgetCspKey[]>();
	for (const [key, source] of policySourceEntries(entry.policy)) {
		directives.set(source, [...(directives.get(source) ?? []), key]);
	}
	return [...directives.entries()]
		.sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
		.map(([source, keys]) => {
			const level = Object.hasOwn(entry.levels, source)
				? entry.levels[source]
				: undefined;
			const row: WidgetConsentSourceRow = {
				source,
				directives: orderedDirectives(keys),
			};
			if (level) row.level = level;
			return row;
		});
}

/** Approved runtime addresses, each once, newest first. */
export function widgetConsentRuntimeRows(
	entry: Pick<MicroWidgetConsentEntry, "runtime">,
): WidgetConsentRuntimeRow[] {
	const rows = new Map<
		string,
		{ directives: WidgetCspKey[]; levels: WidgetSourceLevel[]; at: number }
	>();
	for (const { d, s, l, at } of entry.runtime) {
		const row = rows.get(s) ?? { directives: [], levels: [], at: 0 };
		row.directives.push(d);
		row.levels.push(l);
		row.at = Math.max(row.at, at);
		rows.set(s, row);
	}
	return [...rows.entries()]
		.map(([source, row]) => ({
			source,
			directives: orderedDirectives(row.directives),
			level: maxWidgetSourceLevel(row.levels) ?? "broad",
			at: row.at,
		}))
		.sort(
			(left, right) =>
				right.at - left.at ||
				(left.source < right.source ? -1 : left.source > right.source ? 1 : 0),
		);
}
