import { describe, expect, test } from "bun:test";
import {
	CHAT_PLACEHOLDER_BUBBLE_STATES,
	CHAT_PLACEHOLDER_VISUALS,
	resolveChatPlaceholderBubbleState,
	resolveChatPlaceholderVisual,
} from "./chat-appearance";

describe("chat placeholder config", () => {
	test("an unset or unknown visual keeps the planet every existing chat already shows", () => {
		expect(resolveChatPlaceholderVisual(undefined)).toBe("planet");
		expect(resolveChatPlaceholderVisual(null)).toBe("planet");
		expect(resolveChatPlaceholderVisual("")).toBe("planet");
		expect(resolveChatPlaceholderVisual("orb")).toBe("planet");
		expect(resolveChatPlaceholderVisual(7)).toBe("planet");
	});

	test("every offered visual round-trips", () => {
		for (const option of CHAT_PLACEHOLDER_VISUALS) {
			expect(resolveChatPlaceholderVisual(option.value)).toBe(option.value);
		}
	});

	test("an unset or unknown bubble state falls back to idle", () => {
		expect(resolveChatPlaceholderBubbleState(undefined)).toBe("idle");
		expect(resolveChatPlaceholderBubbleState("inviting")).toBe("idle");
	});

	test("every offered bubble state round-trips", () => {
		for (const option of CHAT_PLACEHOLDER_BUBBLE_STATES) {
			expect(resolveChatPlaceholderBubbleState(option.value)).toBe(
				option.value,
			);
		}
	});

	test("'none' is a real choice, not a fallback", () => {
		expect(resolveChatPlaceholderVisual("none")).toBe("none");
	});
});
