export function formatAppCategory(category?: string | null): string {
	if (!category) return "Other";

	const normalizedCategory = category
		.replace(/_/g, " ")
		.replace(/[A-Z](?=[A-Z][a-z])/g, "$& ")
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.replace(/\s+/g, " ")
		.trim();

	if (!normalizedCategory) return "Other";

	return normalizedCategory
		.split(" ")
		.map((word) => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
		.join(" ");
}