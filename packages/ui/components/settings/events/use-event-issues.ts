"use client";

import { useMemo } from "react";
import type { EventSectionId } from "../../../lib/event-sections";
import {
	getEventSections,
	isTriggerSection,
} from "../../../lib/event-sections";
import { isRuntimeConfigured } from "../../../lib/runtime-vars-utils";
import type { IVariable } from "../../../lib/schema/flow/board";
import type { IEvent } from "../../../lib/schema/flow/event";

export type IssueSeverity = "blocking" | "check";

export interface IEventIssue {
	id: string;
	severity: IssueSeverity;
	section: EventSectionId;
	title: string;
	detail: string;
}

export interface EventIssueInput {
	event: IEvent;
	config: Record<string, unknown>;
	/** Result of the inputs drift comparison, when a board is loaded. */
	drift?: {
		hasDrift: boolean;
		isEmpty: boolean;
		added: Array<{ name: string; friendly_name: string }>;
		removed: Array<{ name: string; friendly_name: string }>;
		changed: Array<{ name: string; field: string }>;
	} | null;
	/** Whether this event type needs a sink activated to run at all. */
	requiresSink?: boolean;
	/** Whether the route path draft collides with another event. */
	routeError?: string | null;
	/** Board variables, when a board is loaded, to check override coverage. */
	boardVariables?: Record<string, IVariable> | null;
}

/**
 * Triggers that fire without anyone present. Every other event type runs from a
 * user session that can prompt for runtime variables and merge in the values
 * stored on that device, so a missing override there is not a problem.
 */
const HEADLESS_EVENT_TYPES = new Set([
	"cron",
	"rest",
	"mcp",
	"api",
	"daemon",
	"discord",
	"telegram",
	"email",
]);

export const isHeadlessEventType = (eventType: string): boolean =>
	HEADLESS_EVENT_TYPES.has(eventType);

/**
 * `keys` is a list because a credential is not always stored where the defaults
 * in EVENT_CONFIG suggest — the mail config seeds `password` but the editor
 * writes `secret_imap_password`, so either one counts as configured.
 */
const SECRET_FIELDS: Record<string, { keys: string[]; label: string }[]> = {
	discord: [{ keys: ["token"], label: "Bot token" }],
	telegram: [{ keys: ["bot_token"], label: "Bot token" }],
	email: [
		{ keys: ["secret_imap_password", "password"], label: "IMAP password" },
	],
};

const missing = (value: unknown) =>
	value === null ||
	value === undefined ||
	(typeof value === "string" && value.trim() === "");

/**
 * One source of truth for everything wrong with an event. Feeds the attention
 * strip, the rail badges, the setup checklist and the overview list, so they
 * can never disagree.
 *
 * Pure so the overview can evaluate every event of an app in a single memo —
 * hooks cannot be called in a loop.
 */
export function computeEventIssues({
	event,
	config,
	drift,
	requiresSink,
	routeError,
	boardVariables,
}: EventIssueInput): IEventIssue[] {
	const issues: IEventIssue[] = [];
	const sectionIds = getEventSections(event).map((section) => section.id);
	/**
	 * Point an issue at `preferred` when that section exists for this event,
	 * otherwise at the first type-specific section. Keeps issue routing correct
	 * whether or not the type's config component has been split yet.
	 */
	const trigger = (preferred: string): EventSectionId =>
		sectionIds.includes(preferred)
			? preferred
			: (sectionIds.find(isTriggerSection) ?? "trigger");

	for (const secret of SECRET_FIELDS[event.event_type] ?? []) {
		if (secret.keys.every((key) => missing(config?.[key]))) {
			issues.push({
				id: `secret-${secret.keys[0]}`,
				severity: "blocking",
				section: trigger("connection"),
				title: `${secret.label} missing`,
				detail: "The connection cannot be opened until this credential is set.",
			});
		}
	}

	if (event.event_type === "cron" && missing(config?.expression)) {
		issues.push({
			id: "cron-expression",
			severity: "blocking",
			section: trigger("schedule"),
			title: "No schedule set",
			detail: "Without an expression or a scheduled time this never fires.",
		});
	}

	if (event.event_type === "api" || event.event_type === "rest") {
		if (config?.public_endpoint === true && missing(config?.auth_token)) {
			issues.push({
				id: "public-no-token",
				severity: "check",
				section: trigger("access"),
				title: "Endpoint is public and unauthenticated",
				detail: "Anyone who finds the URL can trigger this flow.",
			});
		}
	}

	if (boardVariables && isHeadlessEventType(event.event_type)) {
		const unset = Object.entries(boardVariables)
			.filter(
				([key, variable]) =>
					isRuntimeConfigured(variable) && !event.variables?.[key],
			)
			.map(([, variable]) => variable.name);

		if (unset.length > 0) {
			issues.push({
				id: "runtime-vars-unset",
				severity: "check",
				section: "variables",
				title: "Runtime variables have no value",
				detail: `This trigger runs with no user to prompt, so ${unset.join(", ")} will read as empty. Set an override.`,
			});
		}
	}

	if (routeError) {
		issues.push({
			id: "route-path",
			severity: "blocking",
			section: "identity",
			title: "Route path is not usable",
			detail: routeError,
		});
	}

	if (drift?.hasDrift) {
		const parts: string[] = [];
		if (drift.added.length)
			parts.push(
				`added ${drift.added.map((p) => p.friendly_name || p.name).join(", ")}`,
			);
		if (drift.removed.length)
			parts.push(
				`removed ${drift.removed.map((p) => p.friendly_name || p.name).join(", ")}`,
			);
		if (drift.changed.length)
			parts.push(
				`changed ${drift.changed.map((c) => `${c.name} (${c.field})`).join(", ")}`,
			);
		issues.push({
			id: "input-drift",
			severity: "check",
			section: "inputs",
			title: "The node's inputs have changed",
			detail: `Since this event was published: ${parts.join("; ")}. Refresh to sync.`,
		});
	}

	// Page-target events are bound by their page and carry no entry node, so
	// requiring node_id reports every page event as broken.
	const unbound = event.default_page_id
		? !event.default_page_id
		: !event.board_id || !event.node_id;
	if (unbound) {
		issues.push({
			id: "unbound",
			severity: "blocking",
			section: "flow",
			title: "No flow bound",
			detail: "This event has nothing to run.",
		});
	}

	if (requiresSink && !event.active) {
		issues.push({
			id: "sink-inactive",
			severity: "check",
			section: trigger("connection"),
			title: "Not running",
			detail:
				"This event needs its sink activated before it can receive anything.",
		});
	}

	return issues;
}

export function useEventIssues(input: EventIssueInput): IEventIssue[] {
	const { event, config, drift, requiresSink, routeError, boardVariables } =
		input;
	return useMemo(
		() =>
			computeEventIssues({
				event,
				config,
				drift,
				requiresSink,
				routeError,
				boardVariables,
			}),
		[event, config, drift, requiresSink, routeError, boardVariables],
	);
}

export function issuesForSection(
	issues: IEventIssue[],
	section: EventSectionId,
): IEventIssue[] {
	return issues.filter((issue) => issue.section === section);
}
