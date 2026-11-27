const parts = new Intl.DateTimeFormat("en", {
	day: "numeric",
	month: "short",
	hour: "2-digit",
	minute: "2-digit",
	hourCycle: "h23",
	timeZone: "UTC",
}).formatToParts(0);

const dayAt = parts.findIndex((part) => part.type === "day");
const hourAt = parts.findIndex((part) => part.type === "hour");

/** What this runtime's ICU puts between a day and a time: " at " in older CLDR, ", " in newer. */
export const DAY_TIME_JOINER = parts
	.slice(dayAt + 1, hourAt)
	.map((part) => part.value)
	.join("");

const MOMENT = /([A-Z][a-z]{2} \d{1,2}) at (\d{2}:\d{2})/g;

/** Rewrites "Oct 1 at 02:00" in an expected string to this runtime's joiner. */
export const moment = (expected: string) =>
	expected.replace(MOMENT, `$1${DAY_TIME_JOINER}$2`);
