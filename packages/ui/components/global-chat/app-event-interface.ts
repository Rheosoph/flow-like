import { isChatEventType } from "../../lib/event-config";

export type EventInterfaceKind = "chat" | "page" | "headless" | "unavailable";

export type EventConsumerTool =
	| "call_app_chat"
	| "open_app_page"
	| "call_app_event";

export interface AppEventInterfaceRecord {
	id: string;
	name?: string | null;
	event_type: string;
	active: boolean;
	default_page_id?: string | null;
	route?: string | null;
}

export interface PageEventCandidate {
	event_id: string;
	name: string;
	page_id: string;
	route?: string;
}

export type OpenAppPageResolution<T extends AppEventInterfaceRecord> =
	| {
			ok: true;
			event: T;
			canonicalized_from?: "page_id";
	  }
	| {
			ok: false;
			code:
				| "event_not_found"
				| "event_inactive"
				| "event_interface_changed"
				| "page_target_missing"
				| "no_page_event";
			actual_kind?: EventInterfaceKind;
			relist_required: boolean;
	  };

export type OpenAppPageLookup<T extends AppEventInterfaceRecord> =
	| {
			status: "resolved";
			resolution: OpenAppPageResolution<T>;
			candidate_events: T[];
	  }
	| { status: "inventory_unavailable" };

function nonEmpty(value: string | null | undefined): string | undefined {
	const trimmed = value?.trim();
	return trimmed ? trimmed : undefined;
}

function canonicalPageId(value: string | null | undefined): string | undefined {
	const trimmed = nonEmpty(value);
	return trimmed && trimmed === value ? trimmed : undefined;
}

/** One shared definition for inventory routing and actual interface execution. */
export function classifyAppEventInterface(
	event: Pick<AppEventInterfaceRecord, "event_type" | "default_page_id">,
): EventInterfaceKind {
	if (isChatEventType(event.event_type)) return "chat";
	if (canonicalPageId(event.default_page_id)) return "page";
	if (
		event.event_type.trim().toLowerCase() === "page" ||
		nonEmpty(event.default_page_id)
	) {
		return "unavailable";
	}
	return "headless";
}

export function consumerToolForEventKind(
	kind: EventInterfaceKind,
): EventConsumerTool | undefined {
	switch (kind) {
		case "chat":
			return "call_app_chat";
		case "page":
			return "open_app_page";
		case "headless":
			return "call_app_event";
		case "unavailable":
			return undefined;
	}
}

export function activePageEventCandidates<T extends AppEventInterfaceRecord>(
	events: readonly T[],
): PageEventCandidate[] {
	return events.flatMap((event) => {
		const pageId = canonicalPageId(event.default_page_id);
		if (
			!event.active ||
			!pageId ||
			classifyAppEventInterface(event) !== "page"
		) {
			return [];
		}
		const route = nonEmpty(event.route);
		return [
			{
				event_id: event.id,
				name: nonEmpty(event.name) ?? event.id,
				page_id: pageId,
				...(route ? { route } : {}),
			},
		];
	});
}

/**
 * Resolve an exact page Event without guessing a sibling. A common event_id/page_id mix-up can be
 * repaired in-place only when the supplied page id maps uniquely to one active page Event.
 */
export function resolveOpenAppPageEvent<T extends AppEventInterfaceRecord>(
	events: readonly T[],
	requestedEventId?: string,
): OpenAppPageResolution<T> {
	const requested = nonEmpty(requestedEventId);
	if (!requested) {
		const first = events.find(
			(event) => event.active && classifyAppEventInterface(event) === "page",
		);
		return first
			? { ok: true, event: first }
			: { ok: false, code: "no_page_event", relist_required: false };
	}

	const exact = events.find((event) => event.id === requested);
	if (!exact) {
		const pageIdMatches = events.filter(
			(event) =>
				event.active &&
				classifyAppEventInterface(event) === "page" &&
				nonEmpty(event.default_page_id) === requested,
		);
		if (pageIdMatches.length === 1) {
			const matchedEvent = pageIdMatches[0];
			if (!matchedEvent) {
				return {
					ok: false,
					code: "event_not_found",
					relist_required: true,
				};
			}
			return {
				ok: true,
				event: matchedEvent,
				canonicalized_from: "page_id",
			};
		}
		return {
			ok: false,
			code: "event_not_found",
			relist_required: true,
		};
	}

	if (!exact.active) {
		return {
			ok: false,
			code: "event_inactive",
			actual_kind: classifyAppEventInterface(exact),
			relist_required: true,
		};
	}

	const actualKind = classifyAppEventInterface(exact);
	if (actualKind !== "page") {
		return {
			ok: false,
			code:
				actualKind === "unavailable"
					? "page_target_missing"
					: "event_interface_changed",
			actual_kind: actualKind,
			relist_required: true,
		};
	}

	return { ok: true, event: exact };
}

/**
 * Resolve one open request against injected Event readers. An exact read is authoritative for an
 * explicit Event id; a later bulk snapshot may supply alternatives but cannot contradict that
 * target. Bulk discovery is authoritative only when the exact lookup failed or no id was supplied.
 */
export async function resolveOpenAppPageRequest<
	T extends AppEventInterfaceRecord,
>(
	requestedEventId: string | undefined,
	readExact: (eventId: string) => Promise<T>,
	readAll: () => Promise<T[]>,
): Promise<OpenAppPageLookup<T>> {
	const requested = nonEmpty(requestedEventId);
	if (requested) {
		try {
			const exact = await readExact(requested);
			const resolution = resolveOpenAppPageEvent([exact], requested);
			if (resolution.ok) {
				return { status: "resolved", resolution, candidate_events: [] };
			}
			const candidateEvents = await readAll().catch(() => []);
			return {
				status: "resolved",
				resolution,
				candidate_events: candidateEvents.filter(
					(candidate) => candidate.id !== exact.id,
				),
			};
		} catch {
			try {
				const candidateEvents = await readAll();
				return {
					status: "resolved",
					resolution: resolveOpenAppPageEvent(candidateEvents, requested),
					candidate_events: candidateEvents,
				};
			} catch {
				return { status: "inventory_unavailable" };
			}
		}
	}

	try {
		const candidateEvents = await readAll();
		return {
			status: "resolved",
			resolution: resolveOpenAppPageEvent(candidateEvents),
			candidate_events: candidateEvents,
		};
	} catch {
		return { status: "inventory_unavailable" };
	}
}
