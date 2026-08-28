import type { SurfaceComponent } from "./types";
import {
	type WidgetElementScope,
	flattenSurfaceComponentsForElements,
	mergeStoredElementValues,
} from "./workflow-elements";

/** What the live page knows about itself: the input to a `requestElements` answer. */
export interface ElementSource {
	surfaceId: string;
	components: Record<string, SurfaceComponent> | undefined;
	storedValues: Record<string, unknown>;
	/** Set when the run was triggered from inside a widget instance. */
	widgetScope?: WidgetElementScope;
}

type ElementMap = Record<string, unknown>;

const SELECTOR_PREFIX = /^([A-Za-z_][A-Za-z0-9_-]*):(.*)$/s;

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function componentOf(entry: unknown): Record<string, unknown> | undefined {
	if (!isRecord(entry) || !isRecord(entry.component)) return undefined;
	return entry.component;
}

function explicitChildren(entry: unknown): string[] {
	const children = componentOf(entry)?.children;
	if (!isRecord(children) || !Array.isArray(children.explicitList)) return [];
	return children.explicitList.filter(
		(id): id is string => typeof id === "string" && id.length > 0,
	);
}

function keyPrefix(key: string): string | undefined {
	const separator = key.indexOf("/");
	return separator < 0 ? undefined : key.slice(0, separator);
}

function lastSegment(key: string): string {
	return key.slice(key.lastIndexOf("/") + 1);
}

const MAX_GLOB_LENGTH = 256;
const MAX_GLOB_WILDCARDS = 8;

/** Board-authored patterns compile to anchored regexes; runaway ones match nothing. */
function globToRegExp(pattern: string): RegExp | undefined {
	const literals = pattern.replace(/\*+/g, "*").split("*");
	if (
		pattern.length > MAX_GLOB_LENGTH ||
		literals.length - 1 > MAX_GLOB_WILDCARDS
	) {
		return undefined;
	}
	const source = literals
		.map((literal) => literal.replace(/[.+?^${}()|[\]\\]/g, "\\$&"))
		.join(".*");
	return new RegExp(`^${source}$`);
}

class ElementIndex {
	readonly keys: string[];
	private readonly prefixes: Set<string>;

	constructor(
		private readonly all: ElementMap,
		private readonly surfaceId: string,
	) {
		this.keys = Object.keys(all);
		this.prefixes = new Set<string>();
		for (const key of this.keys) {
			const prefix = keyPrefix(key);
			if (prefix !== undefined) this.prefixes.add(prefix);
		}
	}

	has(key: string): boolean {
		return Object.hasOwn(this.all, key) && this.all[key] !== undefined;
	}

	/**
	 * `pageId/elementId` → exact, else retargeted to the current surface unless
	 * the prefix is a scope present in the map (a widget instance).
	 * `elementId` → the current surface's entry, else any key with that suffix.
	 */
	resolve(key: string): string | undefined {
		if (!key) return undefined;
		if (this.has(key)) return key;

		const prefix = keyPrefix(key);
		const elementId = lastSegment(key);
		if (!elementId) return undefined;

		if (prefix === undefined) {
			const scoped = `${this.surfaceId}/${elementId}`;
			if (this.has(scoped)) return scoped;
			const suffix = `/${elementId}`;
			return this.keys.find((candidate) => candidate.endsWith(suffix));
		}

		if (prefix === this.surfaceId || this.prefixes.has(prefix)) {
			return undefined;
		}
		const retargeted = `${this.surfaceId}/${elementId}`;
		if (this.has(retargeted)) return retargeted;
		const suffix = `/${elementId}`;
		return this.keys.find((candidate) => candidate.endsWith(suffix));
	}

	/** A child id listed by `parentKey`, preferring the parent's own scope. */
	resolveChild(parentKey: string, childId: string): string | undefined {
		const prefix = keyPrefix(parentKey);
		if (prefix !== undefined && prefix !== this.surfaceId) {
			const scoped = `${prefix}/${childId}`;
			if (this.has(scoped)) return scoped;
		}
		return this.resolve(childId);
	}

	byType(type: string): string[] {
		const wanted = type.toLowerCase();
		return this.keys.filter((key) => {
			const componentType = componentOf(this.all[key])?.type;
			return (
				typeof componentType === "string" &&
				componentType.toLowerCase() === wanted
			);
		});
	}

	byGlob(pattern: string): string[] {
		if (!pattern) return [];
		const matcher = globToRegExp(pattern);
		if (!matcher) return [];
		return this.keys.filter((key) => matcher.test(key));
	}

	children(key: string): string[] {
		const parent = this.resolve(key);
		if (parent === undefined) return [];
		const selected = [parent];
		for (const childId of explicitChildren(this.all[parent])) {
			const child = this.resolveChild(parent, childId);
			if (child !== undefined) selected.push(child);
		}
		return selected;
	}

	parent(key: string): string[] {
		const childId = lastSegment(key);
		if (!childId) return [];
		const resolved = this.resolve(key);
		const preferredPrefix =
			resolved !== undefined
				? keyPrefix(resolved)
				: (keyPrefix(key) ?? this.surfaceId);

		const candidates = this.keys.filter((candidate) =>
			explicitChildren(this.all[candidate]).includes(childId),
		);
		const scoped = candidates.find(
			(candidate) => keyPrefix(candidate) === preferredPrefix,
		);
		const parent = scoped ?? candidates[0];
		return parent === undefined ? [] : [parent];
	}

	values(instanceId: string): string[] {
		if (!instanceId) return [];
		const key = `${instanceId}/values`;
		return this.has(key) ? [key] : [];
	}

	select(selector: string): string[] {
		const prefixed = SELECTOR_PREFIX.exec(selector);
		if (!prefixed) {
			const key = this.resolve(selector);
			return key === undefined ? [] : [key];
		}
		const [, kind, argument] = prefixed;
		switch (kind) {
			case "host":
				return this.select(argument);
			case "type":
				return this.byType(argument);
			case "glob":
				return this.byGlob(argument);
			case "children":
				return this.children(argument);
			case "parent":
				return this.parent(argument);
			case "values":
				return this.values(argument);
			default:
				return [];
		}
	}
}

/**
 * Pick entries of an `_elements` map by selector. Unknown prefixes and
 * unresolvable selectors contribute nothing; every entry appears at most once.
 */
export function selectElements(
	all: ElementMap,
	selectors: readonly string[],
	surfaceId: string,
): ElementMap {
	const index = new ElementIndex(all, surfaceId);
	const selected: ElementMap = {};
	for (const selector of selectors) {
		if (typeof selector !== "string") continue;
		for (const key of index.select(selector.trim())) {
			selected[key] = all[key];
		}
	}
	return selected;
}

/**
 * Answer a `requestElements` query from the live surface. A widget-scoped run
 * selects from the instance-addressed map its event would send.
 */
export function materializeSurfaceElements(
	source: ElementSource,
	selectors: readonly string[],
	widgetScope?: WidgetElementScope,
): ElementMap {
	const { surfaceId, components, storedValues } = source;
	if (!surfaceId) return {};
	const all = mergeStoredElementValues(
		flattenSurfaceComponentsForElements(components, surfaceId, widgetScope),
		storedValues,
		components,
		surfaceId,
		widgetScope,
	);
	return selectElements(all, selectors, surfaceId);
}
