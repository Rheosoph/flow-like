import type { A2UIServerMessage } from "./types";

export type A2UINavigationMessage = Extract<
	A2UIServerMessage,
	{ type: "navigateTo" | "setQueryParam" }
>;
export type A2UINavigateToMessage = Extract<
	A2UINavigationMessage,
	{ type: "navigateTo" }
>;

/**
 * Receives page-owned navigation instead of changing the host router. Embedded runtimes use this
 * to keep a page's route and query state inside the surface that owns it.
 */
export type A2UINavigationMessageInterceptor = (
	message: A2UINavigationMessage,
) => void;

/** Normalize the built-in `navigate_page` action onto the same contract as server navigation. */
export function createNavigateToMessage(
	route: string,
	queryParams?: Record<string, string>,
): A2UINavigateToMessage {
	return {
		type: "navigateTo",
		route,
		replace: false,
		...(queryParams && Object.keys(queryParams).length > 0
			? { queryParams }
			: {}),
	};
}

/** Return true when an optional embedded owner consumed this navigation message. */
export function interceptA2UINavigationMessage(
	message: A2UIServerMessage,
	interceptor?: A2UINavigationMessageInterceptor,
): boolean {
	if (
		!interceptor ||
		(message.type !== "navigateTo" && message.type !== "setQueryParam")
	) {
		return false;
	}
	interceptor(message);
	return true;
}
