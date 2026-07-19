/** Two-letter monogram for entities without artwork. */
export function initials(value?: string | null): string {
	const cleaned = (value ?? "").replace(/[^A-Za-z0-9 ]/g, "").trim();
	if (!cleaned) return "?";
	const parts = cleaned.split(/\s+/);
	return ((parts[0]?.[0] ?? "") + (parts[1]?.[0] ?? "")).toUpperCase() || "?";
}

/** Deterministic soft gradient for suites/apps without their own artwork. */
export function seedGradient(seed: string): string {
	let hash = 0;
	for (let i = 0; i < seed.length; i++)
		hash = (hash * 31 + seed.charCodeAt(i)) | 0;
	const hue = ((hash % 360) + 360) % 360;
	return `linear-gradient(135deg, hsl(${hue} 62% 52%), hsl(${(hue + 42) % 360} 58% 44%))`;
}
