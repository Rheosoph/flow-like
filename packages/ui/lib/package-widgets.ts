import type { WidgetContract } from "@flow-like/widget-sdk";
import type { PackageWidgetEntry } from "./schema/wasm";

/**
 * Pure helpers around package-manifest widgets (manifest v2): tolerant
 * manifest readers (the wire mixes camelCase and snake_case field names),
 * contract summary derivation for widget cards, the live-preview LRU that
 * caps concurrently mounted preview iframes, and the typed accessor listing
 * widgets of packages added to an app for the builder's widget library.
 */

/** Maximum number of concurrently live (mounted) widget preview iframes. */
export const MICRO_WIDGET_PREVIEW_LIMIT = 3;

/** Selector prefix marking a package widget: `pkg:{packageId}/{widgetId}`. */
export const PACKAGE_WIDGET_REF_PREFIX = "pkg:";

/**
 * Encode the `widget_selector` value of a package widget. Mirror of
 * `encode_package_widget_ref` in `packages/core/src/a2ui/micro_widget.rs`.
 */
export function encodePackageWidgetRef(
	packageId: string,
	widgetId: string,
): string {
	return `${PACKAGE_WIDGET_REF_PREFIX}${packageId}/${widgetId}`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readString(value: unknown): string | undefined {
	return typeof value === "string" && value.length > 0 ? value : undefined;
}

/**
 * Read the widget entries from a package manifest object without trusting its
 * exact shape (registry wire, installed manifest and local fallbacks differ in
 * casing for sibling fields; entries themselves are validated minimally).
 */
export function readManifestWidgets(manifest: unknown): PackageWidgetEntry[] {
	if (!isRecord(manifest)) return [];
	const raw = manifest.widgets;
	if (!Array.isArray(raw)) return [];
	const entries: PackageWidgetEntry[] = [];
	for (const item of raw) {
		if (!isRecord(item)) continue;
		const id = readString(item.id);
		const name = readString(item.name);
		if (!id || !name || !isRecord(item.contract)) continue;
		entries.push({
			id,
			name,
			description: readString(item.description) ?? "",
			icon: readString(item.icon) ?? null,
			thumbnail: readString(item.thumbnail) ?? null,
			contract: item.contract as unknown as WidgetContract,
			keywords: Array.isArray(item.keywords)
				? item.keywords.filter((k): k is string => typeof k === "string")
				: [],
			...(item.network === undefined ? {} : { network: item.network }),
		});
	}
	return entries;
}

/**
 * Read the widget bundle sha256 from a manifest, accepting both the Rust
 * snake_case wire form (`widget_bundle_hash`) and the camelCase mirror.
 */
export function readManifestWidgetBundleHash(
	manifest: unknown,
): string | undefined {
	if (!isRecord(manifest)) return undefined;
	return (
		readString(manifest.widgetBundleHash) ??
		readString(manifest.widget_bundle_hash)
	);
}

export interface WidgetContractSummary {
	inputs: number;
	events: number;
	queries: number;
}

/** Count inputs/events/queries of a widget contract for the card badges. */
export function summarizeWidgetContract(
	contract: WidgetContract | null | undefined,
): WidgetContractSummary {
	return {
		inputs: Object.keys(contract?.inputs ?? {}).length,
		events: Object.keys(contract?.events ?? {}).length,
		queries: Object.keys(contract?.queries ?? {}).length,
	};
}

function pluralize(count: number, noun: string): string {
	return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

/** `"2 inputs · 1 event · 0 queries"` style summary line. */
export function formatWidgetContractSummary(
	summary: WidgetContractSummary,
): string {
	const queries =
		summary.queries === 1 ? "1 query" : `${summary.queries} queries`;
	return [
		pluralize(summary.inputs, "input"),
		pluralize(summary.events, "event"),
		queries,
	].join(" · ");
}

/**
 * Least-recently-used registry of live preview instances. `activate` claims a
 * slot (evicting the oldest entries beyond capacity by invoking their
 * `onEvict` callbacks) and returns the evicted ids; `release` frees a slot
 * without invoking the callback (the owner unmounted itself).
 */
export class MicroWidgetPreviewLru {
	private readonly entries = new Map<string, () => void>();

	constructor(readonly capacity: number = MICRO_WIDGET_PREVIEW_LIMIT) {}

	get size(): number {
		return this.entries.size;
	}

	has(id: string): boolean {
		return this.entries.has(id);
	}

	activate(id: string, onEvict: () => void): string[] {
		this.entries.delete(id);
		this.entries.set(id, onEvict);
		const evicted: string[] = [];
		while (this.entries.size > this.capacity) {
			const oldest = this.entries.entries().next().value as
				| [string, () => void]
				| undefined;
			if (!oldest) break;
			const [oldestId, evict] = oldest;
			this.entries.delete(oldestId);
			evicted.push(oldestId);
			evict();
		}
		return evicted;
	}

	/** Mark an entry as recently used without changing its callback. */
	touch(id: string): void {
		const callback = this.entries.get(id);
		if (!callback) return;
		this.entries.delete(id);
		this.entries.set(id, callback);
	}

	release(id: string): void {
		this.entries.delete(id);
	}
}

/** Shared pool used by every widget preview card in the app. */
export const microWidgetPreviewLru = new MicroWidgetPreviewLru();

/** A widget shipped by a package that is added to the current app. */
export interface AppPackageWidget {
	packageId: string;
	packageName: string;
	/** Version of the resolved manifest (falls back to the app's pin). */
	packageVersion: string;
	/** Widget bundle sha256 (desktop `flow-widget://` serving); may be absent. */
	bundleHash?: string;
	widget: PackageWidgetEntry;
}

/** Narrow structural slice of the backend used by `listAppPackageWidgets`. */
export interface AppPackageWidgetSources {
	/** `IAppState.listPackages` — package id → pinned version for the app. */
	listPackages?: (appId: string) => Promise<Record<string, string>>;
	/** `IRegistryState.getPackage` through the app, so members resolve the packages it pins. */
	getPackage: (
		packageId: string,
		appId: string,
	) => Promise<{
		version: string;
		manifest: unknown;
		metadata?: { name?: string };
	} | null>;
}

/**
 * List the widgets of every package added to an app, resolved from the
 * package manifests (§6.1 — one widget list everywhere): the local install on
 * desktop, the registry entry of the pinned version on web. Hosts without
 * `appState.listPackages` resolve to an empty list.
 */
export async function listAppPackageWidgets(
	sources: AppPackageWidgetSources,
	appId: string,
	options: { strict?: boolean; packageId?: string } = {},
): Promise<AppPackageWidget[]> {
	if (!sources.listPackages) return [];
	let pinned: Record<string, string>;
	try {
		pinned = await sources.listPackages(appId);
	} catch (error) {
		if (options.strict) throw error;
		return [];
	}
	if (options.packageId && !Object.hasOwn(pinned ?? {}, options.packageId)) {
		if (options.strict)
			throw new Error(
				`Package ${options.packageId} is no longer added to this app.`,
			);
		return [];
	}
	const packageIds = options.packageId
		? [options.packageId]
		: Object.keys(pinned ?? {});
	if (packageIds.length === 0) return [];

	const resolved = await Promise.all(
		packageIds.map(async (packageId) => {
			try {
				const installed = await sources.getPackage(packageId, appId);
				if (!installed) {
					if (options.strict)
						throw new Error(`Package ${packageId} could not be loaded.`);
					return [];
				}
				const widgets = readManifestWidgets(installed.manifest);
				if (widgets.length === 0) return [];
				const manifestRecord = isRecord(installed.manifest)
					? installed.manifest
					: undefined;
				const packageName =
					installed.metadata?.name ??
					readString(manifestRecord?.name) ??
					packageId;
				const bundleHash = readManifestWidgetBundleHash(installed.manifest);
				const packageVersion = installed.version || pinned[packageId];
				return widgets.map(
					(widget): AppPackageWidget => ({
						packageId,
						packageName,
						packageVersion,
						bundleHash,
						widget,
					}),
				);
			} catch (error) {
				if (options.strict) throw error;
				return [];
			}
		}),
	);
	return resolved.flat();
}
