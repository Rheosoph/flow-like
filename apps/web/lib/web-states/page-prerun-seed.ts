import {
	type PageTrigger,
	serializePageTrigger,
} from "@flow-like/flow-like-ui/lib/schema/flow/page-trigger";
import type { IPrerunEventResponse } from "@flow-like/flow-like-ui/state/backend-state/types";
import { type WebBackendRef, getApiBaseUrl } from "./api-utils";

/** A bootstrap's onLoad prerun serves the onLoad its page starts right after it. */
export const PAGE_LOAD_PRERUN_SEED_MS = 15_000;

const seeded = new Map<
	string,
	{ response: IPrerunEventResponse; seededAt: number }
>();

/** Who asks, for what, at which revision: a prerun answer never crosses accounts or targets. */
export function pagePrerunScope(
	backend: WebBackendRef,
	appId: string,
	eventId: string,
	version: [number, number, number] | undefined,
	trigger: PageTrigger,
): string {
	return JSON.stringify([
		getApiBaseUrl(),
		backend.profile?.hub ?? null,
		backend.profile?.id ?? null,
		backend.auth?.user?.profile?.sub ?? null,
		backend.auth?.user?.access_token ?? null,
		appId,
		eventId,
		version ?? null,
		serializePageTrigger(trigger),
	]);
}

export function seedPagePrerun(
	scope: string,
	response: IPrerunEventResponse,
	now = Date.now(),
): void {
	seeded.set(scope, { response, seededAt: now });
}

/** The seeded answer for `scope` and how old it is, once, while it is fresh. */
export function takeSeededPagePrerun(
	scope: string,
	now = Date.now(),
): { response: IPrerunEventResponse; ageMs: number } | undefined {
	const entry = seeded.get(scope);
	seeded.delete(scope);
	if (!entry) return undefined;
	const ageMs = Math.max(0, now - entry.seededAt);
	if (ageMs > PAGE_LOAD_PRERUN_SEED_MS) return undefined;
	return { response: entry.response, ageMs };
}
