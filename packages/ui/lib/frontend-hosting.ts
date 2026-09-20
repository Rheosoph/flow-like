import type { IEvent } from "./schema/flow/event";
import type { IFrontendHosting } from "./schema/flow/event-payload";

export type HostedFrontendKind = "c" | "f" | "u";

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

export function getHostedFrontendUrl(
	origin: string,
	kind: HostedFrontendKind,
	slugOrId: string,
): string {
	return `${origin.replace(/\/+$/, "")}/${kind}/${encodeURIComponent(slugOrId)}`;
}
