import type { A2UINavigationMessage } from "../a2ui/navigation-message";

export interface EmbeddedPageTargetChange {
	routePath?: string;
	eventId?: string | null;
	queryParams: Record<string, string>;
	externalHref?: string;
}

/** Restrict app-owned external navigation to protocols a user could safely open as a link. */
export function isSafeEmbeddedExternalHref(
	href: string,
	baseHref = "https://flowpilot.invalid",
): boolean {
	try {
		const protocol = new URL(href, baseHref).protocol.toLowerCase();
		return ["http:", "https:", "mailto:", "tel:"].includes(protocol);
	} catch {
		return false;
	}
}

function queryRecord(params: URLSearchParams): Record<string, string> {
	const result: Record<string, string> = {};
	params.forEach((value, key) => {
		result[key] = value;
	});
	return result;
}

function mergeMessageQuery(
	params: URLSearchParams,
	queryParams?: Record<string, string>,
) {
	for (const [key, value] of Object.entries(queryParams ?? {})) {
		params.set(key, value);
	}
}

/**
 * Resolve page-owned navigation without touching the host chat URL. Shell parameters from a
 * `/use` link become target fields; the remaining query belongs to the embedded page.
 */
export function resolveEmbeddedPageNavigation(
	message: A2UINavigationMessage,
	appId: string,
	currentQueryParams: Record<string, string>,
): EmbeddedPageTargetChange {
	if (message.type === "setQueryParam") {
		const queryParams = { ...currentQueryParams };
		if (message.value === undefined || message.value === "") {
			delete queryParams[message.key];
		} else {
			queryParams[message.key] = message.value;
		}
		return { queryParams };
	}

	const route = message.route.trim() || "/";
	if (/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(route)) {
		return { externalHref: route, queryParams: { ...currentQueryParams } };
	}

	const parsed = new URL(route, "https://flowpilot.invalid");
	const params = new URLSearchParams(parsed.search);
	mergeMessageQuery(params, message.queryParams);

	if (parsed.pathname === "/use") {
		const destinationAppId = params.get("id");
		if (destinationAppId && destinationAppId !== appId) {
			return { externalHref: route, queryParams: { ...currentQueryParams } };
		}

		const routePath = params.get("route") || "/";
		const eventId = params.get("eventId");
		params.delete("id");
		params.delete("route");
		params.delete("eventId");
		return {
			routePath,
			eventId: eventId || null,
			queryParams: queryRecord(params),
		};
	}

	return {
		routePath: parsed.pathname || "/",
		eventId: null,
		queryParams: queryRecord(params),
	};
}
