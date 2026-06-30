import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { IUserLookup } from "../state/backend-state/types";

/** A curated palette of visually distinct colors for peer identification */
const PEER_COLORS = [
	"hsl(0 70% 50%)", // Red
	"hsl(30 70% 50%)", // Orange
	"hsl(60 70% 45%)", // Yellow
	"hsl(120 60% 40%)", // Green
	"hsl(180 70% 40%)", // Cyan
	"hsl(210 70% 50%)", // Blue
	"hsl(270 60% 55%)", // Purple
	"hsl(300 60% 50%)", // Magenta
	"hsl(330 70% 50%)", // Pink
	"hsl(15 70% 50%)", // Coral
	"hsl(165 70% 40%)", // Teal
	"hsl(195 70% 45%)", // Sky
	"hsl(255 60% 55%)", // Indigo
	"hsl(345 70% 50%)", // Rose
	"hsl(45 70% 50%)", // Amber
	"hsl(90 60% 40%)", // Lime
] as const;

/** Hash a string to a number for consistent color assignment */
function hashString(str: string): number {
	let hash = 0;
	for (let i = 0; i < str.length; i++) {
		hash = (hash * 31 + str.charCodeAt(i)) >>> 0;
	}
	return hash;
}

/** Get a consistent color from the palette based on sub hash */
export function colorFromSub(sub?: string): string {
	if (!sub) return "hsl(0 0% 50%)";
	const index = hashString(sub) % PEER_COLORS.length;
	return PEER_COLORS[index];
}

/** Truncate a name with ellipsis if it exceeds maxLength */
export function truncateName(name: string | undefined, maxLength = 12): string {
	if (!name) return "User";
	if (name.length <= maxLength) return name;
	return `${name.slice(0, maxLength - 1)}…`;
}

export interface PeerUserInfo {
	sub: string;
	color: string;
	name: string;
	truncatedName: string;
	avatarUrl?: string;
	loading: boolean;
}

interface UsePeerUsersOptions {
	/** Function to lookup user by sub/id */
	lookupUser: (sub: string) => Promise<IUserLookup>;
	/** Maximum name length before truncation */
	maxNameLength?: number;
}

/**
 * Hook to manage peer user information with caching.
 * Fetches user info only once per sub and provides cached results.
 */
export function usePeerUsers({
	lookupUser,
	maxNameLength = 12,
}: UsePeerUsersOptions) {
	const [users, setUsers] = useState<Map<string, PeerUserInfo>>(new Map());
	const pendingFetches = useRef<Set<string>>(new Set());
	const cache = useRef<Map<string, IUserLookup>>(new Map());

	/** Get or fetch user info for a given sub */
	const getUser = useCallback(
		async (sub: string): Promise<PeerUserInfo> => {
			// Check cache first
			const cached = cache.current.get(sub);
			if (cached) {
				return {
					sub,
					color: colorFromSub(sub),
					name: cached.name ?? cached.username ?? "User",
					truncatedName: truncateName(
						cached.name ?? cached.username,
						maxNameLength,
					),
					avatarUrl: cached.avatar_url,
					loading: false,
				};
			}

			// Return placeholder while fetching
			const placeholder: PeerUserInfo = {
				sub,
				color: colorFromSub(sub),
				name: "User",
				truncatedName: "User",
				loading: true,
			};

			// Don't fetch if already pending
			if (pendingFetches.current.has(sub)) {
				return placeholder;
			}

			// Start fetch
			pendingFetches.current.add(sub);

			try {
				const user = await lookupUser(sub);
				cache.current.set(sub, user);
				const info: PeerUserInfo = {
					sub,
					color: colorFromSub(sub),
					name: user.name ?? user.username ?? "User",
					truncatedName: truncateName(
						user.name ?? user.username,
						maxNameLength,
					),
					avatarUrl: user.avatar_url,
					loading: false,
				};
				setUsers((prev) => {
					const next = new Map(prev);
					next.set(sub, info);
					return next;
				});
				return info;
			} catch {
				// On error, use placeholder with sub-based color
				const fallback: PeerUserInfo = {
					sub,
					color: colorFromSub(sub),
					name: "User",
					truncatedName: "User",
					loading: false,
				};
				setUsers((prev) => {
					const next = new Map(prev);
					next.set(sub, fallback);
					return next;
				});
				return fallback;
			} finally {
				pendingFetches.current.delete(sub);
			}
		},
		[lookupUser, maxNameLength],
	);

	/** Get cached user info synchronously (returns placeholder if not cached) */
	const getCachedUser = useCallback(
		(sub: string): PeerUserInfo => {
			const existing = users.get(sub);
			if (existing) return existing;

			// Return placeholder
			return {
				sub,
				color: colorFromSub(sub),
				name: "User",
				truncatedName: "User",
				loading: !cache.current.has(sub),
			};
		},
		[users],
	);

	/** Prefetch user info for a list of subs */
	const prefetchUsers = useCallback(
		(subs: string[]) => {
			for (const sub of subs) {
				if (!sub) continue;
				if (cache.current.has(sub)) continue;
				if (pendingFetches.current.has(sub)) continue;
				// Fire and forget
				void getUser(sub);
			}
		},
		[getUser],
	);

	return {
		users,
		getUser,
		getCachedUser,
		prefetchUsers,
		colorFromSub,
		truncateName: (name: string | undefined) =>
			truncateName(name, maxNameLength),
	};
}

/**
 * Hook to automatically fetch and cache peer user info based on peer subs.
 * Use this in components that display peer information.
 */
export function usePeerUserInfo(
	subs: (string | undefined)[],
	lookupUser: (sub: string) => Promise<IUserLookup>,
	maxNameLength = 12,
) {
	const { users, prefetchUsers, getCachedUser } = usePeerUsers({
		lookupUser,
		maxNameLength,
	});

	// Prefetch all peer users on mount and when subs change
	useEffect(() => {
		const validSubs = subs.filter((s): s is string => !!s);
		if (validSubs.length > 0) {
			prefetchUsers(validSubs);
		}
	}, [subs.join(","), prefetchUsers]);

	// Return a map of sub -> user info with a STABLE identity: only swap the
	// reference when the relevant content (peer set or a sub's resolved name/avatar/
	// loading state) actually changes. This prevents downstream consumers (e.g. the
	// node peerUsers injection) from re-rendering on every unrelated render.
	const mapRef = useRef<Map<string, PeerUserInfo>>(new Map());
	const peerUsers = useMemo(() => {
		const next = new Map<string, PeerUserInfo>();
		for (const sub of subs) {
			if (!sub) continue;
			next.set(sub, getCachedUser(sub));
		}
		const prev = mapRef.current;
		let changed = next.size !== prev.size;
		if (!changed) {
			for (const [k, v] of next) {
				const p = prev.get(k);
				if (
					!p ||
					p.loading !== v.loading ||
					p.name !== v.name ||
					p.truncatedName !== v.truncatedName ||
					p.avatarUrl !== v.avatarUrl ||
					p.color !== v.color
				) {
					changed = true;
					break;
				}
			}
		}
		if (!changed) return prev;
		mapRef.current = next;
		return next;
	}, [subs, getCachedUser, users]);

	return peerUsers;
}
