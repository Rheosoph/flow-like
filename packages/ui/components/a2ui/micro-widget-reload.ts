import {
	type ContractInput,
	type WidgetContract,
	validateInputValue,
} from "@flow-like/widget-sdk";
import { createContext, useContext } from "react";
import type { AppPackageWidget } from "../../lib/package-widgets";
import { stableStringify } from "../../lib/stable-stringify";
import { WILDCARD_EVENT } from "./event-handlers";
import { PUBLIC_MEDIA_GRANTS_PROP } from "./micro-widget-media";
import type { MicroWidgetInstanceComponent } from "./types";

type PlacedMicroWidget = Pick<
	MicroWidgetInstanceComponent,
	"packageId" | "widgetId" | "packageVersion" | "bundleHash"
>;

export interface MicroWidgetReloadReport {
	/** Values rewritten to fit the new input type or range. */
	converted: string[];
	/** Inputs that were unset or still on the old default and now take the new default. */
	defaulted: string[];
	/** Values the new input rejects; they fall back to its default or are cleared. */
	reset: string[];
	/** Values of inputs the new build no longer declares. */
	removed: string[];
	/** Configured events the new build no longer declares; their handlers are kept. */
	undeclaredEvents: string[];
}

export interface MicroWidgetReload {
	component: MicroWidgetInstanceComponent;
	report: MicroWidgetReloadReport;
}

/** Editors that can rewrite a placed micro widget offer to move it onto the installed build. */
export interface MicroWidgetReloader {
	/** The installed build when it differs from the placed one. */
	updateFor(component: PlacedMicroWidget): AppPackageWidget | null;
	reload(componentId: string): Promise<void>;
	/** Re-read the installed builds, e.g. after the placed one failed to load. */
	refresh(): void;
}

export const MicroWidgetReloadContext =
	createContext<MicroWidgetReloader | null>(null);

export function useMicroWidgetReloader(): MicroWidgetReloader | null {
	return useContext(MicroWidgetReloadContext);
}

function ownEntry<T>(
	record: Record<string, T> | null | undefined,
	key: string,
): T | undefined {
	return record && Object.hasOwn(record, key) ? record[key] : undefined;
}

function sameJson(left: unknown, right: unknown): boolean {
	return stableStringify(left) === stableStringify(right);
}

function cloneJson<T>(value: T): T {
	return JSON.parse(JSON.stringify(value)) as T;
}

/**
 * A local rebuild keeps the version and changes the bundle hash; a registry
 * update changes the version. Hosts without hashes compare versions only.
 */
export function findMicroWidgetUpdate(
	installed: readonly AppPackageWidget[] | undefined,
	placed: PlacedMicroWidget,
): AppPackageWidget | null {
	const match = installed?.find(
		(entry) =>
			entry.packageId === placed.packageId &&
			entry.widget.id === placed.widgetId,
	);
	if (!match) return null;
	const hashChanged =
		match.bundleHash !== undefined && match.bundleHash !== placed.bundleHash;
	return hashChanged || match.packageVersion !== placed.packageVersion
		? match
		: null;
}

function clampNumber(input: ContractInput, value: number): number | undefined {
	const integer = input.type === "integer";
	if (integer && !Number.isInteger(value)) return undefined;
	const min =
		input.min === undefined
			? undefined
			: integer
				? Math.ceil(input.min)
				: input.min;
	const max =
		input.max === undefined
			? undefined
			: integer
				? Math.floor(input.max)
				: input.max;
	if (min !== undefined && max !== undefined && min > max) return undefined;
	if (min !== undefined && value < min) return min;
	if (max !== undefined && value > max) return max;
	return value;
}

/** Lossless conversions between input types, plus clamping into a changed range. */
function convertContractValue(input: ContractInput, value: unknown): unknown {
	switch (input.type) {
		case "string":
			return typeof value === "number" || typeof value === "boolean"
				? String(value)
				: undefined;
		case "number":
		case "integer": {
			const number =
				typeof value === "string" && value.trim() !== ""
					? Number(value)
					: value;
			return typeof number === "number" && Number.isFinite(number)
				? clampNumber(input, number)
				: undefined;
		}
		case "boolean":
			if (value === "true") return true;
			if (value === "false") return false;
			return undefined;
		case "enum": {
			if (typeof value !== "string" && typeof value !== "number") {
				return undefined;
			}
			const wanted = String(value).toLowerCase();
			const matches = (input.choices ?? []).filter(
				(choice) => choice.toLowerCase() === wanted,
			);
			return matches.length === 1 ? matches[0] : undefined;
		}
		case "json":
			if (typeof value !== "string") return undefined;
			try {
				return JSON.parse(value);
			} catch {
				return undefined;
			}
	}
}

/**
 * Carries configured props onto a new contract. A value still on its old
 * default follows the new default; anything else is kept when the new input
 * accepts it, converted when that is lossless, and otherwise reset.
 * Host-owned props pass through untouched.
 */
export function migrateMicroWidgetProps(
	props: Record<string, unknown>,
	previous: WidgetContract | null | undefined,
	next: WidgetContract,
): {
	props: Record<string, unknown>;
	report: Omit<MicroWidgetReloadReport, "undeclaredEvents">;
} {
	const migrated: Record<string, unknown> = {};
	const report = {
		converted: [] as string[],
		defaulted: [] as string[],
		reset: [] as string[],
		removed: [] as string[],
	};

	for (const [key, value] of Object.entries(props)) {
		if (key === PUBLIC_MEDIA_GRANTS_PROP) {
			migrated[key] = value;
			continue;
		}
		const input = ownEntry(next.inputs, key);
		if (!input) {
			report.removed.push(key);
			continue;
		}
		const before = ownEntry(previous?.inputs, key);
		if (
			before?.default !== undefined &&
			input.default !== undefined &&
			sameJson(before.default, value) &&
			!sameJson(input.default, value)
		) {
			migrated[key] = cloneJson(input.default);
			report.defaulted.push(key);
			continue;
		}
		if (validateInputValue(input, value).valid) {
			migrated[key] = value;
			continue;
		}
		const converted = convertContractValue(input, value);
		if (converted !== undefined && validateInputValue(input, converted).valid) {
			migrated[key] = converted;
			report.converted.push(key);
			continue;
		}
		if (input.default !== undefined) migrated[key] = cloneJson(input.default);
		report.reset.push(key);
	}

	for (const [key, input] of Object.entries(next.inputs ?? {})) {
		if (Object.hasOwn(props, key) || input.default === undefined) continue;
		migrated[key] = cloneJson(input.default);
		report.defaulted.push(key);
	}

	return { props: migrated, report };
}

function configuredEvents(component: MicroWidgetInstanceComponent): string[] {
	const names = new Set([
		...Object.keys(component.eventHandlers ?? {}),
		...Object.keys(component.actionBindings ?? {}),
	]);
	names.delete(WILDCARD_EVENT);
	return [...names].sort();
}

/**
 * Points a placed instance at the installed build. Instance id, handlers,
 * bindings, legacy actions and style stay as they are, so every configured
 * route keeps working; saving the page re-types the Widget Action Event pins
 * from the embedded contract.
 */
export function reloadMicroWidgetInstance(
	component: MicroWidgetInstanceComponent,
	update: AppPackageWidget,
): MicroWidgetReload {
	const contract = cloneJson(update.widget.contract);
	const { props, report } = migrateMicroWidgetProps(
		component.props ?? {},
		component.contract,
		contract,
	);
	return {
		component: {
			...component,
			packageVersion: update.packageVersion,
			bundleHash: update.bundleHash ?? component.bundleHash,
			contract,
			props,
		},
		report: {
			...report,
			undeclaredEvents: configuredEvents(component).filter(
				(name) => !ownEntry(contract.events, name),
			),
		},
	};
}

export function isMicroWidgetReloadClean(report: MicroWidgetReloadReport) {
	return Object.values(report).every((keys) => keys.length === 0);
}
