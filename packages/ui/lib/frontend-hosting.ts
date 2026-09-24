import { normalizeRoutePath } from "./route-path";
import {
	type IEvent,
	IEventExecutionMode,
	IEventExposure,
} from "./schema/flow/event";
import type { IFrontendHosting } from "./schema/flow/event-payload";

export type HostedFrontendKind = "c" | "f" | "u";

export type HostingBlocker =
	| "no_route"
	| "inactive"
	| "local_execution"
	| "internal_exposure";

/**
 * The route a hosted Event answers: its own mapping, or `/` for the app
 * default. Mirrors `canonical_route` in packages/api/src/routes/app/page/bootstrap.rs.
 */
export function getHostedRoute(
	event: Pick<IEvent, "route" | "is_default">,
): string | null {
	if (event.route?.trim()) return normalizeRoutePath(event.route);
	return event.is_default ? "/" : null;
}

/** Mirrors `load_route` + `hosting()` in packages/api/src/routes/frontend.rs. */
export function getHostingBlockers(
	event: Pick<IEvent, "active" | "execution_mode" | "exposure">,
	route: string | null,
): HostingBlocker[] {
	const blockers: HostingBlocker[] = [];
	if (route === null) blockers.push("no_route");
	if (!event.active) blockers.push("inactive");
	if (event.execution_mode !== IEventExecutionMode.Remote)
		blockers.push("local_execution");
	if ((event.exposure ?? IEventExposure.Public) !== IEventExposure.Public)
		blockers.push("internal_exposure");
	return blockers;
}

/** The event change that clears a blocker; a missing route needs the user to name one. */
export const HOSTING_BLOCKER_FIX: Partial<
	Record<
		HostingBlocker,
		Partial<Pick<IEvent, "active" | "execution_mode" | "exposure">>
	>
> = {
	inactive: { active: true },
	local_execution: { execution_mode: IEventExecutionMode.Remote },
	internal_exposure: { exposure: IEventExposure.Public },
};

export function getHostedFrontendKind(
	event: Pick<IEvent, "event_type" | "default_page_id">,
): HostedFrontendKind | null {
	if (event.default_page_id) return "u";
	if (event.event_type === "simple_chat") return "c";
	if (
		event.event_type === "generic_form" ||
		event.event_type === "quick_action"
	)
		return "f";
	return null;
}

/** Hosting and anonymous access each require an explicit opt-in. */
export function parseFrontendHosting(config: unknown): IFrontendHosting {
	const hosting =
		config && typeof config === "object" && "frontend_hosting" in config
			? config.frontend_hosting
			: null;
	if (!hosting || typeof hosting !== "object" || Array.isArray(hosting)) {
		return { enabled: false, allow_anonymous: false, auth_proxy: true };
	}
	const fields = ["enabled", "allow_anonymous", "auth_proxy"];
	if (
		Object.entries(hosting).some(
			([key, value]) => !fields.includes(key) || typeof value !== "boolean",
		)
	) {
		return { enabled: false, allow_anonymous: false, auth_proxy: true };
	}
	const enabled = "enabled" in hosting && hosting.enabled === true;
	const allowAnonymous =
		enabled &&
		"allow_anonymous" in hosting &&
		hosting.allow_anonymous === true &&
		!("auth_proxy" in hosting && hosting.auth_proxy === true);
	return {
		enabled,
		allow_anonymous: allowAnonymous,
		auth_proxy: !allowAnonymous,
	};
}

/** The app's hosted link for one route: `{origin}/a/{appId}{route}`. */
export function getHostedFrontendUrl(
	origin: string,
	appId: string,
	route: string,
): string {
	const path = normalizeRoutePath(route)
		.split("/")
		.filter(Boolean)
		.map(encodeURIComponent)
		.join("/");
	return `${origin.replace(/\/+$/, "")}/a/${encodeURIComponent(appId)}${path ? `/${path}` : ""}`;
}
