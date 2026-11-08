import { describe, expect, test } from "vitest";
import {
	DEFAULT_AVAILABILITY,
	clearLocalAvailability,
	effectiveAvailability,
	readLocalAvailability,
	shouldPresentNotification,
	updateLocalAvailability,
} from "./availability";

describe("availability", () => {
	test("an expired manual override falls back to the current calendar interval", () => {
		const value = {
			...DEFAULT_AVAILABILITY,
			status: "focus" as const,
			source: "manual" as const,
			until: 2000,
			free_busy: {
				window_start: 1000,
				window_end: 5000,
				busy: [{ start: 2000, end: 3000 }],
			},
		};
		expect(effectiveAvailability(value, 2500)).toMatchObject({
			status: "busy",
			source: "calendar",
			until: 3000,
		});
		expect(effectiveAvailability(value, 3500)).toMatchObject({
			status: "available",
			source: "calendar",
			until: 5000,
		});
		const stale = {
			...value,
			free_busy: {
				...value.free_busy,
				window_end: 1_000_000,
				busy: [{ start: 2000, end: 900_000 }],
			},
		};
		const fallback = effectiveAvailability(stale, 300_999);
		expect(fallback).toMatchObject({ status: "busy", until: 301_000 });
		expect(effectiveAvailability(stale, 301_000)).toMatchObject({
			status: "available",
			source: "default",
		});
		expect(effectiveAvailability(fallback, 301_000)).toMatchObject({
			status: "available",
			source: "default",
			until: null,
		});
	});
	test("local overrides expire and remain separate across accounts and hubs", () => {
		updateLocalAvailability(
			"hub-a:user-1",
			{ status: "focus", duration_minutes: 1 },
			"manual",
			1000,
		);
		expect(readLocalAvailability("hub-a:user-1", 2000).status).toBe("focus");
		expect(readLocalAvailability("hub-a:user-2", 2000).status).toBe(
			"available",
		);
		expect(readLocalAvailability("hub-b:user-1", 2000).status).toBe(
			"available",
		);
		expect(readLocalAvailability("hub-a:user-1", 61000).status).toBe(
			"available",
		);
	});
	test("duration is bounded and defaults to one hour", () => {
		expect(
			updateLocalAvailability("duration", { status: "busy" }, "manual", 1000)
				.until,
		).toBe(3601000);
		for (const duration_minutes of [0, -1, 241, 1.5, Number.NaN])
			expect(() =>
				updateLocalAvailability("duration", {
					status: "busy",
					duration_minutes,
				}),
			).toThrow();
	});
	test("focus suppresses routine delivery and honors explicit mention and call exceptions", () => {
		const focus = {
			...DEFAULT_AVAILABILITY,
			status: "focus" as const,
			until: 2000,
		};
		expect(shouldPresentNotification(focus, "routine", 1000)).toBe(false);
		expect(shouldPresentNotification(focus, "mention", 1000)).toBe(true);
		expect(shouldPresentNotification(focus, "call", 1000)).toBe(true);
		const blocked = {
			...focus,
			preferences: { allow_calls: false, allow_mentions: false },
		};
		expect(shouldPresentNotification(blocked, "mention", 1000)).toBe(false);
		expect(shouldPresentNotification(blocked, "call", 1000)).toBe(false);
		expect(shouldPresentNotification(blocked, "routine", 2000)).toBe(true);
		expect(
			shouldPresentNotification({ ...focus, status: "busy" }, "routine", 1000),
		).toBe(true);
	});
	test("clearing an override retains focus preferences", () => {
		updateLocalAvailability("clear", {
			status: "focus",
			preferences: { allow_calls: false, allow_mentions: true },
		});
		expect(clearLocalAvailability("clear")).toMatchObject({
			status: "available",
			until: null,
			preferences: { allow_calls: false },
		});
		expect(effectiveAvailability(DEFAULT_AVAILABILITY)).toEqual(
			DEFAULT_AVAILABILITY,
		);
	});
});
