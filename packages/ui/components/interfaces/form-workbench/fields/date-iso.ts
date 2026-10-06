/*
 * Rail dates are `YYYY-MM-DD` text; the calendar works with local `Date`s. These two conversions use local
 * calendar parts on both sides, so a day never shifts with the time zone.
 */

const ISO_DAY = /^(\d{4})-(\d{2})-(\d{2})$/;

/** `YYYY-MM-DD` of a real day → that day at local midnight; anything else → undefined. */
export function isoToDate(iso: string): Date | undefined {
	const match = ISO_DAY.exec(iso);
	if (!match) return undefined;
	const [year, month, day] = [
		Number(match[1]),
		Number(match[2]),
		Number(match[3]),
	];
	const date = new Date(0);
	date.setFullYear(year, month - 1, day);
	date.setHours(0, 0, 0, 0);
	const real =
		date.getFullYear() === year &&
		date.getMonth() === month - 1 &&
		date.getDate() === day;
	return real ? date : undefined;
}

const pad = (value: number, size = 2) => String(value).padStart(size, "0");

/** A local day → `YYYY-MM-DD`. */
export function dateToIso(date: Date): string {
	return `${pad(date.getFullYear(), 4)}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** The first day of the week for a locale as `Date#getDay` numbers (0 = Sunday); Monday when Intl cannot say. */
export function weekStartOf(locale: string): 0 | 1 | 2 | 3 | 4 | 5 | 6 {
	try {
		const intl = new Intl.Locale(locale) as Intl.Locale & {
			getWeekInfo?: () => { firstDay: number };
			weekInfo?: { firstDay: number };
		};
		const info = intl.getWeekInfo?.() ?? intl.weekInfo;
		const first = info?.firstDay;
		if (typeof first === "number")
			return (first % 7) as 0 | 1 | 2 | 3 | 4 | 5 | 6;
	} catch {
		return 1;
	}
	return 1;
}

/** The day the calendar starts on: the chosen one, else the anchor's. */
export function startOf(selected: string, anchor: string, today: string): Date {
	return (
		isoToDate(selected) ?? isoToDate(anchor) ?? isoToDate(today) ?? new Date()
	);
}
