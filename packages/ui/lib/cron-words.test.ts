import { expect, test } from "bun:test";
import { cronWords, humanizeCron } from "./cron-words";

test("a schedule reads as a sentence, with or without a seconds field", () => {
	const words: [string, string][] = [
		["0 0 9 * * 1-5", "At 09:00 on weekdays"],
		["0 9 * * 1-5", "At 09:00 on weekdays"],
		["0 0 2 * * *", "At 02:00 every day"],
		["0 */5 * * * *", "Every 5 minutes"],
		["0 0 * * * *", "At :00 past every hour"],
		["0 0 18 * * 0", "At 18:00 on Sunday"],
		["0 0 18 * * 7", "At 18:00 on Sunday"],
		["0 30 6 1,15 * *", "At 06:30 on the 1st and 15th of every month"],
		["*/30 * * * * *", "Every 30 seconds"],
	];
	for (const [expression, sentence] of words) {
		expect(humanizeCron(expression)).toBe(sentence);
		expect(cronWords(expression)).toBe(sentence);
	}
	expect(humanizeCron("0 0 9 * * 1-5", { tz: "Europe/Berlin" })).toBe(
		"At 09:00 on weekdays (Europe/Berlin)",
	);
});

test("an expression no sentence fits falls back to the expression itself", () => {
	expect(cronWords("0 0 9 * JAN MON")).toBeNull();
	expect(humanizeCron("0 0 9 * JAN MON")).toBe("Cron: 0 0 9 * JAN MON");
	expect(humanizeCron("0 0 9 * JAN MON", { tz: "UTC" })).toBe(
		"Cron: 0 0 9 * JAN MON (UTC)",
	);
	expect(humanizeCron("  ")).toBe("");
	expect(cronWords("")).toBeNull();
});
