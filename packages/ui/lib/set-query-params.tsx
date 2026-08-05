"use client";

import { useRouter } from "next/navigation";
import { useCallback } from "react";

export interface ISetQueryParamsOptions {
	/** Rewrite the current history entry instead of pushing a new one. */
	readonly replace?: boolean;
}

export type ISetQueryParams = (
	key: string,
	value: string | undefined,
	options?: ISetQueryParamsOptions,
) => void;

export interface IQueryParamRequest {
	readonly pathname: string;
	/** Search string the request was issued from, without the leading "?". */
	readonly from: string;
	/** Search string the request navigates to, without the leading "?". */
	readonly to: string;
	readonly at: number;
}

/**
 * A router commit takes milliseconds. Anything older is a request that never
 * landed, or a URL the user has since left and returned to.
 */
export const QUERY_PARAM_REQUEST_TTL_MS = 2_000;

/**
 * Next.js commits a search-param navigation asynchronously, so a writer that
 * reads `window.location.search` before the previous commit landed silently
 * drops the parameter that is still in flight. Two components that each restore
 * their own parameter (the interface its event, the chat its session) then push
 * each other in a loop.
 *
 * A mutation is therefore layered onto the URL we last requested, but only for
 * as long as the browser still shows the URL that request started from — any
 * other address means something else navigated and now owns the location.
 *
 * Returns `null` when the parameter already holds the wanted value, so an
 * effect that re-runs cannot navigate to the URL it is already on.
 */
export function nextQueryParamRequest(
	location: { readonly pathname: string; readonly search: string },
	pending: IQueryParamRequest | null,
	key: string,
	value: string | undefined,
	now: number = Date.now(),
): IQueryParamRequest | null {
	const live = location.search.replace(/^\?/, "");
	const applies = Boolean(
		pending &&
			pending.pathname === location.pathname &&
			pending.from === live &&
			now - pending.at < QUERY_PARAM_REQUEST_TTL_MS,
	);
	const base = applies ? (pending as IQueryParamRequest).to : live;

	const params = new URLSearchParams(base);
	// Compare against the re-encoded base: a raw `route=/` in the address bar
	// serializes to `route=%2F`, which would otherwise read as a change.
	const unchanged = params.toString();

	if (value === undefined || value === null) {
		params.delete(key);
	} else {
		params.set(key, value);
	}

	const to = params.toString();
	if (to === unchanged) return null;

	return {
		pathname: location.pathname,
		from: applies ? (pending as IQueryParamRequest).from : live,
		to,
		at: applies ? (pending as IQueryParamRequest).at : now,
	};
}

let inFlight: IQueryParamRequest | null = null;

export function useSetQueryParams(): ISetQueryParams {
	const router = useRouter();

	return useCallback(
		(key, value, options) => {
			if (typeof window === "undefined") return;

			const request = nextQueryParamRequest(
				window.location,
				inFlight,
				key,
				value,
			);
			if (!request) return;

			inFlight = request;
			const href = `?${request.to}`;
			if (options?.replace) {
				router.replace(href);
			} else {
				router.push(href);
			}
		},
		[router],
	);
}
