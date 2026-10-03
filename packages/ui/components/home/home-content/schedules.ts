import { CronExpressionParser } from "cron-parser";
import { resolveLocalInstant } from "../../../lib/schedule-instant";

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
	if (typeof date !== "string" || typeof time !== "string") return null;
	// A nonexistent local time or invalid date must not become a different deadline.
	const instant = resolveLocalInstant(date, time, timezone);
	if (!instant.ok) return null;
	const candidate = new Date(instant.at * 1000);
	if (candidate <= now) return null;
	if (
		typeof config.last_fired === "string" &&
		new Date(config.last_fired) >= candidate
	)
		return null;
	return candidate;
}
