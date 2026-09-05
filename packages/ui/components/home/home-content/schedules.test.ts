import { describe, expect, it } from "bun:test";
import { nextHomeSchedule } from "./schedules";

describe("home schedule previews", () => {
	it("respects the saved timezone for recurring events", () => {
		expect(
			nextHomeSchedule(
				{ expression: "0 9 * * *", timezone: "Europe/Berlin" },
				new Date("2026-09-05T06:00:00Z"),
			)?.toISOString(),
		).toBe("2026-09-05T07:00:00.000Z");
	});
	it("resolves a one-time date in its timezone and does not show past dates", () => {
		const config = {
			scheduled_for: { date: "2027-01-01", time: "09:30" },
			timezone: "America/New_York",
		};
		expect(
			nextHomeSchedule(config, new Date("2026-09-05T06:00:00Z"))?.toISOString(),
		).toBe("2027-01-01T14:30:00.000Z");
		expect(
			nextHomeSchedule(config, new Date("2027-01-02T06:00:00Z")),
		).toBeNull();
	});
	it("does not move a nonexistent daylight-saving time to another deadline", () => {
		expect(
			nextHomeSchedule(
				{
					scheduled_for: { date: "2027-03-28", time: "02:30" },
					timezone: "Europe/Berlin",
				},
				new Date("2027-01-01T00:00:00Z"),
			),
		).toBeNull();
	});
});
