import type * as GeoJSON from "geojson";
import { formatRelativeTime } from "../../lib/date";
import type { BotFact } from "../../state/global-chat/bot-memory-facts";

export const DAY_MS = 86_400_000;

export function parseInstant(value?: string | null): number | null {
	if (!value) return null;
	const time = Date.parse(value);
	return Number.isFinite(time) ? time : null;
}

/** Dates said without a time of day are saved at midnight UTC or at the speaker's local midnight. */
export function wholeDayZone(time: number): "UTC" | "local" | null {
	const date = new Date(time);
	if (
		date.getUTCHours() === 0 &&
		date.getUTCMinutes() === 0 &&
		date.getUTCSeconds() === 0
	)
		return "UTC";
	if (
		date.getHours() === 0 &&
		date.getMinutes() === 0 &&
		date.getSeconds() === 0
	)
		return "local";
	return null;
}

/** The local instant that shows the same calendar day as `time` does where it was meant. */
function calendarInstant(time: number): number {
	if (wholeDayZone(time) !== "UTC") return time;
	const date = new Date(time);
	return new Date(
		date.getUTCFullYear(),
		date.getUTCMonth(),
		date.getUTCDate(),
	).getTime();
}

function startOfDay(time: number, offsetDays = 0): number {
	const date = new Date(time);
	return new Date(
		date.getFullYear(),
		date.getMonth(),
		date.getDate() + offsetDays,
	).getTime();
}

/** Mirrors the backend's `Fact::anchor`: its reference time, when it became true, or when it was last said. */
export function memoryAnchor(fact: BotFact): number {
	const explicit = parseInstant(fact.time) ?? parseInstant(fact.valid_from);
	if (explicit !== null) return explicit;
	const said = fact.evidence
		.map((item) => parseInstant(item.at))
		.filter((time): time is number => time !== null);
	return said.length > 0
		? Math.max(...said)
		: (parseInstant(fact.recorded_at) ?? 0);
}

export type MemoryGroupKey =
	| { kind: "upcoming" }
	| { kind: "today" }
	| { kind: "yesterday" }
	| { kind: "week" }
	| { kind: "month"; year: number; month: number }
	| { kind: "year"; year: number };

export interface MemoryGroup {
	id: string;
	key: MemoryGroupKey;
	facts: BotFact[];
}

function groupKey(anchor: number, now: number): MemoryGroupKey {
	const day = calendarInstant(anchor);
	if (day >= startOfDay(now, 1)) return { kind: "upcoming" };
	if (day >= startOfDay(now)) return { kind: "today" };
	if (day >= startOfDay(now, -1)) return { kind: "yesterday" };
	if (day >= startOfDay(now, -7)) return { kind: "week" };
	const date = new Date(day);
	const current = new Date(now);
	const monthsAgo =
		(current.getFullYear() - date.getFullYear()) * 12 +
		current.getMonth() -
		date.getMonth();
	return monthsAgo < 12
		? { kind: "month", year: date.getFullYear(), month: date.getMonth() }
		: { kind: "year", year: date.getFullYear() };
}

const FIXED_RANKS = { upcoming: 0, today: 1, yesterday: 2, week: 3 } as const;

/** Upcoming, the recent days, then months and years newest first. */
function groupRank(key: MemoryGroupKey): number {
	if (key.kind === "month") return 10_000_000 - (key.year * 12 + key.month);
	if (key.kind === "year") return 20_000_000 - key.year;
	return FIXED_RANKS[key.kind];
}

function groupId(key: MemoryGroupKey): string {
	if (key.kind === "month") return `month-${key.year}-${key.month}`;
	if (key.kind === "year") return `year-${key.year}`;
	return key.kind;
}

/**
 * Files memories under the day they concern. Within a group the backend's newest-anchor-first
 * order stays, except that upcoming memories read soonest first.
 */
export function groupMemories(
	facts: readonly BotFact[],
	now: number,
): MemoryGroup[] {
	const groups = new Map<string, MemoryGroup>();
	for (const fact of facts) {
		const key = groupKey(memoryAnchor(fact), now);
		const id = groupId(key);
		const group = groups.get(id) ?? { id, key, facts: [] };
		group.facts.push(fact);
		groups.set(id, group);
	}
	groups.get("upcoming")?.facts.reverse();
	return [...groups.values()].sort(
		(a, b) => groupRank(a.key) - groupRank(b.key),
	);
}

export interface MemoryTtl {
	expiresAt: number;
	remaining: number;
	/** Share of its lifetime still ahead, 0–1. */
	fraction: number;
	urgent: boolean;
}

export function memoryTtl(fact: BotFact, now: number): MemoryTtl | null {
	const expiresAt = parseInstant(fact.expires_at);
	if (expiresAt === null) return null;
	const recorded = parseInstant(fact.recorded_at) ?? now;
	const lifetime = Math.max(1, expiresAt - recorded);
	const remaining = Math.max(0, expiresAt - now);
	const fraction = Math.max(0, Math.min(1, remaining / lifetime));
	return {
		expiresAt,
		remaining,
		fraction,
		urgent: remaining < DAY_MS || fraction < 0.15,
	};
}

export type ValidityState = "current" | "ended" | "pending";

export interface MemoryValidity {
	from: number | null;
	to: number | null;
	state: ValidityState;
}

export function memoryValidity(
	fact: BotFact,
	now: number,
): MemoryValidity | null {
	const from = parseInstant(fact.valid_from);
	const to = parseInstant(fact.valid_to);
	if (from === null && to === null) return null;
	const state: ValidityState =
		to !== null && to < now
			? "ended"
			: from !== null && from > now
				? "pending"
				: "current";
	return { from, to, state };
}

export type PlaceShape = "point" | "points" | "line" | "area" | "mixed";
type Position = [number, number];

export interface MemoryPlace {
	geometry: GeoJSON.Geometry;
	shape: PlaceShape;
	/** Longitude, latitude: the point itself or the middle of the shape's bounds. */
	center: Position;
	bounds: [Position, Position];
	/** Members of a multi geometry or collection. */
	count: number;
}

const SHAPES: Record<string, PlaceShape> = {
	Point: "point",
	MultiPoint: "points",
	LineString: "line",
	MultiLineString: "line",
	Polygon: "area",
	MultiPolygon: "area",
};

function collectPositions(coordinates: unknown, into: Position[]): boolean {
	if (!Array.isArray(coordinates) || coordinates.length === 0) return false;
	if (typeof coordinates[0] === "number") {
		const [lon, lat] = coordinates as number[];
		if (
			!Number.isFinite(lon) ||
			!Number.isFinite(lat) ||
			Math.abs(lon) > 180 ||
			Math.abs(lat) > 90
		)
			return false;
		into.push([lon, lat]);
		return true;
	}
	return coordinates.every((child) => collectPositions(child, into));
}

function shapeOf(
	geometry: Record<string, unknown>,
	positions: Position[],
): { shape: PlaceShape; count: number } | null {
	const type = geometry.type;
	if (type === "GeometryCollection") {
		const members = geometry.geometries;
		if (!Array.isArray(members) || members.length === 0) return null;
		const shapes = new Set<PlaceShape>();
		for (const member of members) {
			if (typeof member !== "object" || member === null) return null;
			const inner = shapeOf(member as Record<string, unknown>, positions);
			if (!inner) return null;
			shapes.add(inner.shape === "points" ? "point" : inner.shape);
		}
		const [only] = shapes;
		return {
			shape: shapes.size === 1 ? (only === "point" ? "points" : only) : "mixed",
			count: members.length,
		};
	}
	if (typeof type !== "string" || !Object.hasOwn(SHAPES, type)) return null;
	if (!collectPositions(geometry.coordinates, positions)) return null;
	const multi = type.startsWith("Multi");
	return {
		shape: SHAPES[type],
		count: multi ? (geometry.coordinates as unknown[]).length : 1,
	};
}

/** A remembered location, or null when the value isn't a usable WGS84 GeoJSON geometry. */
export function memoryPlace(geojson: unknown): MemoryPlace | null {
	if (typeof geojson !== "object" || geojson === null) return null;
	const positions: Position[] = [];
	const shape = shapeOf(geojson as Record<string, unknown>, positions);
	if (!shape || positions.length === 0) return null;
	let west = 180;
	let east = -180;
	let south = 90;
	let north = -90;
	for (const [lon, lat] of positions) {
		west = Math.min(west, lon);
		east = Math.max(east, lon);
		south = Math.min(south, lat);
		north = Math.max(north, lat);
	}
	return {
		geometry: geojson as GeoJSON.Geometry,
		shape: shape.shape,
		count: shape.count,
		center:
			shape.shape === "point"
				? positions[0]
				: [(west + east) / 2, (south + north) / 2],
		bounds: [
			[west, south],
			[east, north],
		],
	};
}

export interface PlaceGlyph {
	areas: string;
	lines: string;
	points: Position[];
}

/** The place drawn into a `size` square (repeated points once): equirectangular, scaled by latitude so shapes keep their proportions. */
export function placeGlyph(place: MemoryPlace, size = 40, pad = 8): PlaceGlyph {
	const [[west, south], [east, north]] = place.bounds;
	const scaleX = Math.cos((((south + north) / 2) * Math.PI) / 180);
	const width = (east - west) * scaleX;
	const height = north - south;
	const inner = size - pad * 2;
	const scale =
		Math.max(width, height) > 0 ? inner / Math.max(width, height) : 0;
	const offsetX = (size - width * scale) / 2;
	const offsetY = (size - height * scale) / 2;
	const project = ([lon, lat]: Position): Position => [
		Math.round((offsetX + (lon - west) * scaleX * scale) * 100) / 100,
		Math.round((offsetY + (north - lat) * scale) * 100) / 100,
	];
	const path = (ring: Position[], close: boolean) =>
		ring.length === 0
			? ""
			: `M${ring
					.map(project)
					.map(([x, y]) => `${x} ${y}`)
					.join("L")}${close ? "Z" : ""}`;
	const glyph: PlaceGlyph = { areas: "", lines: "", points: [] };
	const pinned = new Set<string>();
	const pin = (position: Position) => {
		const [x, y] = project(position);
		if (pinned.has(`${x}-${y}`)) return;
		pinned.add(`${x}-${y}`);
		glyph.points.push([x, y]);
	};
	const draw = (geometry: GeoJSON.Geometry) => {
		switch (geometry.type) {
			case "Point":
				pin(geometry.coordinates as Position);
				break;
			case "MultiPoint":
				for (const point of geometry.coordinates) pin(point as Position);
				break;
			case "LineString":
				glyph.lines += path(geometry.coordinates as Position[], false);
				break;
			case "MultiLineString":
				for (const line of geometry.coordinates)
					glyph.lines += path(line as Position[], false);
				break;
			case "Polygon":
				for (const ring of geometry.coordinates)
					glyph.areas += path(ring as Position[], true);
				break;
			case "MultiPolygon":
				for (const polygon of geometry.coordinates)
					for (const ring of polygon)
						glyph.areas += path(ring as Position[], true);
				break;
			case "GeometryCollection":
				geometry.geometries.forEach(draw);
				break;
		}
	};
	draw(place.geometry);
	return glyph;
}

export interface MemoryFormat {
	locale: string;
	/** "Fri, Nov 14" (with the year outside the current one). */
	day: (time: number, now: number) => string;
	/** "NOV" and "14" for the calendar page. */
	monthShort: (time: number) => string;
	dayOfMonth: (time: number) => string;
	timeOfDay: (time: number) => string;
	/** "Nov 14, 2026, 15:00", or the date alone for whole days. */
	full: (time: number) => string;
	range: (from: number, to: number) => string;
	date: (time: number) => string;
	monthYear: (year: number, month: number, withYear: boolean) => string;
	year: (year: number) => string;
	/** "tomorrow", "in 5 days", "3 hours ago": by calendar day for whole days. */
	relative: (time: number, now: number) => string;
	coordinate: (value: number, digits: number) => string;
}

const formats = new Map<string, MemoryFormat>();

function zoned(
	locale: string,
	options: Intl.DateTimeFormatOptions,
): (time: number) => string {
	const local = new Intl.DateTimeFormat(locale, options);
	const utc = new Intl.DateTimeFormat(locale, { ...options, timeZone: "UTC" });
	return (time) =>
		(wholeDayZone(time) === "UTC" ? utc : local).format(new Date(time));
}

export function memoryFormat(locale: string): MemoryFormat {
	const cached = formats.get(locale);
	if (cached) return cached;
	const dayWithYear = zoned(locale, {
		weekday: "short",
		month: "short",
		day: "numeric",
		year: "numeric",
	});
	const dayInYear = zoned(locale, {
		weekday: "short",
		month: "short",
		day: "numeric",
	});
	const date = zoned(locale, { dateStyle: "medium" });
	const dateTime = zoned(locale, { dateStyle: "medium", timeStyle: "short" });
	const rangeFormat = new Intl.DateTimeFormat(locale, { dateStyle: "medium" });
	const monthYear = new Intl.DateTimeFormat(locale, {
		month: "long",
		year: "numeric",
	});
	const month = new Intl.DateTimeFormat(locale, { month: "long" });
	const year = new Intl.DateTimeFormat(locale, { year: "numeric" });
	const days = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
	const numbers = new Map<number, Intl.NumberFormat>();
	const format: MemoryFormat = {
		locale,
		day: (time, now) =>
			new Date(calendarInstant(time)).getFullYear() ===
			new Date(now).getFullYear()
				? dayInYear(time)
				: dayWithYear(time),
		monthShort: zoned(locale, { month: "short" }),
		dayOfMonth: zoned(locale, { day: "numeric" }),
		timeOfDay: zoned(locale, { timeStyle: "short" }),
		full: (time) => (wholeDayZone(time) ? date(time) : dateTime(time)),
		range: (from, to) =>
			rangeFormat.formatRange(
				new Date(calendarInstant(from)),
				new Date(calendarInstant(to)),
			),
		date,
		monthYear: (y, m, withYear) =>
			(withYear ? monthYear : month).format(new Date(y, m, 1)),
		year: (y) => year.format(new Date(y, 0, 1)),
		relative: (time, now) => {
			if (!wholeDayZone(time))
				return formatRelativeTime(time, "long", "", { now, locale });
			const diff = Math.round(
				(calendarInstant(time) - startOfDay(now)) / DAY_MS,
			);
			return Math.abs(diff) < 7
				? days.format(diff, "day")
				: formatRelativeTime(calendarInstant(time), "long", "", {
						now: startOfDay(now),
						locale,
					});
		},
		coordinate: (value, digits) => {
			let number = numbers.get(digits);
			if (!number) {
				number = new Intl.NumberFormat(locale, {
					minimumFractionDigits: digits,
					maximumFractionDigits: digits,
				});
				numbers.set(digits, number);
			}
			return number.format(value);
		},
	};
	formats.set(locale, format);
	return format;
}
