import { isRecord } from "./response-shape";

export interface PersonalAccessToken {
	id: string;
	name: string;
	created_at: string;
	valid_until: string | null;
	permission: number;
}

function dateString(value: unknown): string | null {
	if (typeof value === "string") return value;
	if (typeof value !== "number" || !Number.isFinite(value)) return null;
	const date = new Date(value);
	return Number.isNaN(date.getTime()) ? null : date.toISOString();
}

/** Older hubs return `permissions`; the UI and creation response use `permission`. */
export function normalizePersonalAccessTokens(
	value: unknown,
): PersonalAccessToken[] {
	if (!Array.isArray(value)) return [];
	return value.flatMap((raw) => {
		if (
			!isRecord(raw) ||
			typeof raw.id !== "string" ||
			typeof raw.name !== "string"
		)
			return [];
		const permission = raw.permission ?? raw.permissions;
		return [
			{
				id: raw.id,
				name: raw.name,
				created_at: dateString(raw.created_at) ?? "",
				valid_until: dateString(raw.valid_until),
				permission: typeof permission === "number" ? permission : 0,
			},
		];
	});
}
