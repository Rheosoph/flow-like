import { CronExpressionParser } from "cron-parser";

/** Derive a preview from the saved schedule; the event's runner still owns execution. */
export function nextHomeSchedule(
	config: Record<string, unknown>,
	now = new Date(),
): Date | null {
	const timezone =
		typeof config.timezone === "string" && config.timezone
			? config.timezone
			: "UTC";
	if (typeof config.expression === "string" && config.expression.trim()) {
		return CronExpressionParser.parse(config.expression, {
			currentDate: now,
			tz: timezone,
		})
			.next()
			.toDate();
	}
	const scheduled = config.scheduled_for;
	if (!scheduled || typeof scheduled !== "object") return null;
	const { date, time } = scheduled as { date?: unknown; time?: unknown };
	if (
		typeof date !== "string" ||
		typeof time !== "string" ||
		!/^\d{4}-\d{2}-\d{2}$/.test(date) ||
		!/^\d{2}:\d{2}$/.test(time)
	)
		return null;
	const [year, month, day] = date.split("-").map(Number);
	const [hour, minute] = time.split(":").map(Number);
	const start = new Date(Date.UTC(year, month - 1, day) - 2 * 86_400_000);
	const candidate = CronExpressionParser.parse(
		`${minute} ${hour} ${day} ${month} *`,
		{ currentDate: start, tz: timezone },
	)
		.next()
		.toDate();
	const parts = new Intl.DateTimeFormat("en-CA", {
		timeZone: timezone,
		year: "numeric",
		month: "2-digit",
		day: "2-digit",
		hour: "2-digit",
		minute: "2-digit",
		hourCycle: "h23",
	}).formatToParts(candidate);
	const value = (name: string) =>
		parts.find((part) => part.type === name)?.value;
	// A nonexistent local time or invalid date must not become a different deadline.
	if (
		`${value("year")}-${value("month")}-${value("day")}` !== date ||
		`${value("hour")}:${value("minute")}` !== time
	)
		return null;
	if (candidate <= now) return null;
	if (
		typeof config.last_fired === "string" &&
		new Date(config.last_fired) >= candidate
	)
		return null;
	return candidate;
}
