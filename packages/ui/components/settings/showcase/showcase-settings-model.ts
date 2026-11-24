import {
	type IAppShowcaseSettings,
	emptyShowcaseSettings,
} from "../../../lib/schema/app/showcase";

export type ShowcaseBoolPath =
	| "map"
	| "data.enabled"
	| "data.row_counts"
	| "data.schema"
	| "data.samples"
	| "data.ontology"
	| "data.sources"
	| "triggers.enabled"
	| "triggers.technical"
	| "pages.enabled"
	| "pages.previews"
	| "widgets.enabled"
	| "templates.enabled"
	| "flows.enabled"
	| "hood.enabled"
	| "hood.runtime"
	| "hood.models"
	| "hood.packages"
	| "hood.accounts"
	| "activity.enabled"
	| "activity.runs"
	| "activity.versions";

export type ShowcaseListKey = "tables" | "events";

export function readFlag(
	settings: IAppShowcaseSettings,
	path: ShowcaseBoolPath,
): boolean {
	const [group, field] = path.split(".");
	if (!field) return settings[group as "map"];
	return (settings as unknown as Record<string, Record<string, boolean>>)[
		group
	][field];
}

export function withFlag(
	settings: IAppShowcaseSettings,
	path: ShowcaseBoolPath,
	value: boolean,
): IAppShowcaseSettings {
	const [group, field] = path.split(".");
	if (!field) return { ...settings, map: value };
	const current = (settings as unknown as Record<string, object>)[group];
	return { ...settings, [group]: { ...current, [field]: value } };
}

export function readList(
	settings: IAppShowcaseSettings,
	key: ShowcaseListKey,
): string[] {
	return key === "tables" ? settings.data.tables : settings.triggers.events;
}

export function withList(
	settings: IAppShowcaseSettings,
	key: ShowcaseListKey,
	list: string[],
): IAppShowcaseSettings {
	return key === "tables"
		? { ...settings, data: { ...settings.data, tables: list } }
		: { ...settings, triggers: { ...settings.triggers, events: list } };
}

export function withListItem(
	settings: IAppShowcaseSettings,
	key: ShowcaseListKey,
	item: string,
	ticked: boolean,
): IAppShowcaseSettings {
	const rest = readList(settings, key).filter((entry) => entry !== item);
	return withList(settings, key, ticked ? [...rest, item] : rest);
}

function setAllFlags(value: boolean): IAppShowcaseSettings {
	const blank = emptyShowcaseSettings();
	const fill = <T extends object>(group: T): T =>
		Object.fromEntries(
			Object.entries(group).map(([key, entry]) => [
				key,
				typeof entry === "boolean" ? value : entry,
			]),
		) as T;
	return {
		map: value,
		data: fill(blank.data),
		triggers: fill(blank.triggers),
		pages: fill(blank.pages),
		widgets: fill(blank.widgets),
		templates: fill(blank.templates),
		flows: fill(blank.flows),
		hood: fill(blank.hood),
		activity: fill(blank.activity),
	};
}

export function allOff(): IAppShowcaseSettings {
	return setAllFlags(false);
}

export function allOn(
	tables: readonly string[],
	events: readonly string[],
): IAppShowcaseSettings {
	const on = setAllFlags(true);
	on.data.tables = [...tables];
	on.triggers.events = [...events];
	return on;
}

/** Drops allowlist entries whose table or event no longer exists. */
export function pruneSettings(
	settings: IAppShowcaseSettings,
	tables: readonly string[] | undefined,
	events: readonly string[] | undefined,
): IAppShowcaseSettings {
	let next = settings;
	if (tables) {
		const known = new Set(tables);
		next = withList(
			next,
			"tables",
			settings.data.tables.filter((name) => known.has(name)),
		);
	}
	if (events) {
		const known = new Set(events);
		next = withList(
			next,
			"events",
			next.triggers.events.filter((id) => known.has(id)),
		);
	}
	return next;
}
