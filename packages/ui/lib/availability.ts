export const AVAILABILITY_STATUSES = [
	"available",
	"busy",
	"focus",
	"away",
] as const;
export type AvailabilityStatus = (typeof AVAILABILITY_STATUSES)[number];
export interface PublicAvailability {
	status: AvailabilityStatus;
	source: "manual" | "bot" | "calendar" | "default";
	until: number | null;
}
export interface FreeBusy {
	window_start: number;
	window_end: number;
	busy: { start: number; end: number }[];
}
export interface UserAvailability extends PublicAvailability {
	preferences: { allow_mentions: boolean; allow_calls: boolean };
	calendar_links: { id: string; provider_id: string; label: string }[];
	free_busy?: FreeBusy;
	calendar_error?: string;
}
export interface AvailabilityUpdate {
	status?: AvailabilityStatus;
	duration_minutes?: number;
	preferences?: UserAvailability["preferences"];
}
export interface CalendarLinkRequest {
	provider_id: string;
	connection_handle: string;
	label?: string;
}
export const DEFAULT_AVAILABILITY: UserAvailability = {
	status: "available",
	source: "default",
	until: null,
	preferences: { allow_mentions: true, allow_calls: true },
	calendar_links: [],
};

export function effectiveAvailability<T extends PublicAvailability>(
	value: T,
	now = Date.now(),
): T {
	if (value.until === null || value.until > now) return value;
	const calendar =
		"free_busy" in value
			? (value.free_busy as FreeBusy | undefined)
			: undefined;
	if (
		calendar &&
		calendar.window_start <= now &&
		now - calendar.window_start < 5 * 60_000 &&
		now < calendar.window_end
	) {
		const current = calendar.busy.find(
			(interval) => interval.start <= now && now < interval.end,
		);
		const next = calendar.busy.find((interval) => interval.start > now);
		return {
			...value,
			status: current ? "busy" : "available",
			source: "calendar",
			until: Math.min(
				current?.end ?? next?.start ?? calendar.window_end,
				calendar.window_start + 5 * 60_000,
			),
		};
	}
	return { ...value, status: "available", source: "default", until: null };
}

export function shouldPresentNotification(
	availability: UserAvailability | undefined,
	priority: "routine" | "mention" | "call",
	now = Date.now(),
): boolean {
	if (
		!availability ||
		effectiveAvailability(availability, now).status !== "focus"
	)
		return true;
	return priority === "mention"
		? availability.preferences.allow_mentions
		: priority === "call"
			? availability.preferences.allow_calls
			: false;
}

const localValues = new Map<string, UserAvailability>();
export function readLocalAvailability(
	scope: string,
	now = Date.now(),
): UserAvailability {
	return effectiveAvailability(
		localValues.get(scope) ?? DEFAULT_AVAILABILITY,
		now,
	);
}
export function updateLocalAvailability(
	scope: string,
	update: AvailabilityUpdate,
	source: "manual" | "bot" = "manual",
	now = Date.now(),
	until?: number,
): UserAvailability {
	const current = readLocalAvailability(scope, now);
	const duration = update.duration_minutes ?? 60;
	if (
		update.status &&
		(!AVAILABILITY_STATUSES.includes(update.status) ||
			!Number.isInteger(duration) ||
			duration < 1 ||
			duration > 240)
	)
		throw new Error("Choose a duration from 1 to 240 minutes.");
	const next: UserAvailability = {
		...current,
		...(update.status
			? {
					status: update.status,
					source,
					until: until ?? now + duration * 60_000,
				}
			: {}),
		preferences: update.preferences ?? current.preferences,
	};
	localValues.set(scope, next);
	return next;
}
export function clearLocalAvailability(scope: string): UserAvailability {
	const current = readLocalAvailability(scope);
	const next = {
		...current,
		status: "available",
		source: "default",
		until: null,
	} as UserAvailability;
	localValues.set(scope, next);
	return next;
}
