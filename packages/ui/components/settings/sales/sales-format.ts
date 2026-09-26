export function formatCurrency(cents: number): string {
	return new Intl.NumberFormat("en-US", {
		style: "currency",
		currency: "EUR",
	}).format(cents / 100);
}

export function formatPercent(value: number): string {
	return `${value >= 0 ? "+" : ""}${value.toFixed(1)}%`;
}

/** Day labels come from the API as UTC calendar days (`YYYY-MM-DD`). */
export function formatDay(day: string): string {
	return new Date(day).toLocaleDateString("en-US", {
		month: "short",
		day: "numeric",
		timeZone: "UTC",
	});
}

export function formatDate(value: string | number): string {
	return new Date(value).toLocaleDateString("en-US", {
		month: "short",
		day: "numeric",
		year: "numeric",
	});
}
