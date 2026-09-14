import { describe, expect, test } from "bun:test";
import {
	notificationIconCandidates,
	notificationIconSource,
	remoteNotificationIcon,
} from "./notification-icon";

describe("notification artwork", () => {
	test("recognizes supported images, installed Lucide names, and emoji", () => {
		expect(notificationIconSource(" shopping-bag ")).toEqual({
			kind: "lucide",
			value: "shopping-bag",
		});
		for (const value of ["😊", "🇩🇪", "1️⃣"])
			expect(notificationIconSource(value)).toEqual({ kind: "emoji", value });
		for (const value of [
			"https://example.com/image",
			"/images/icon.webp",
			"asset://localhost/icon.png",
			"data:image/png;base64,aA==",
		])
			expect(notificationIconSource(value)).toEqual({ kind: "image", value });
	});
	test("ignores invalid names, unsafe URLs, and arbitrary prose", () => {
		for (const value of [
			"not-a-real-lucide-icon",
			"hello world",
			"javascript:alert(1)",
			"file:///image.png",
			"https://user:password@example.com/image.png",
			"//example.com/icon.png",
		])
			expect(notificationIconSource(value)).toBeUndefined();
	});
	test("matches native emoji limits without splitting flags, keycaps, or families", () => {
		for (const value of [
			"😀".repeat(8),
			"👨‍👩‍👧‍👦",
			"👨‍👩‍👧‍👦".repeat(2),
			"🇩🇪",
			"1️⃣",
			"#️⃣",
			"👍🏽",
		])
			expect(notificationIconSource(value)).toEqual({ kind: "emoji", value });
		for (const value of [
			"😀".repeat(9),
			"👨‍👩‍👧‍👦".repeat(3),
			"Hello 😀",
			"😀 😀",
			"😀\n😀",
			"😀\u0000",
			"1",
			"#",
			"<😀>",
		])
			expect(notificationIconSource(value)).toBeUndefined();
	});
	test("malformed API icon values fall back without calling trim on non-strings", () => {
		for (const icon of [null, 42, false, {}, [], { trim: "not-a-function" }]) {
			expect(notificationIconSource(icon)).toBeUndefined();
			expect(
				notificationIconCandidates(icon, "/source-app.webp").map(
					(item) => item.value,
				),
			).toEqual(["/source-app.webp", "/app-logo.webp"]);
			expect(
				notificationIconCandidates("mail", icon).map((item) => item.value),
			).toEqual(["mail", "/app-logo.webp"]);
		}
	});
	test("prefers explicit artwork, then the source app, then the product logo", () => {
		expect(
			notificationIconCandidates("mail", "/source-app.webp").map(
				(item) => item.value,
			),
		).toEqual(["mail", "/source-app.webp", "/app-logo.webp"]);
		expect(
			notificationIconCandidates(undefined, "/source-app.webp").map(
				(item) => item.value,
			),
		).toEqual(["/source-app.webp", "/app-logo.webp"]);
		expect(
			notificationIconCandidates("/app-logo.webp", "/source-app.webp").map(
				(item) => item.value,
			),
		).toEqual(["/source-app.webp", "/app-logo.webp"]);
		expect(
			notificationIconCandidates("/source-app.webp", "/source-app.webp").map(
				(item) => item.value,
			),
		).toEqual(["/source-app.webp", "/app-logo.webp"]);
	});
	test("remote pushes retain symbolic icons and can use FCM image artwork", () => {
		expect(remoteNotificationIcon({ icon: "mail" })).toBe("mail");
		expect(remoteNotificationIcon({ icon: "🌍" })).toBe("🌍");
		expect(
			remoteNotificationIcon({
				icon: "invalid-icon",
				fcm_options: { image: "https://example.com/a.png" },
			}),
		).toBe("https://example.com/a.png");
	});
});
