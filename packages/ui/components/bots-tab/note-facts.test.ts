import { describe, expect, test } from "bun:test";
import type { ConversationTrigger } from "./fold-entries";
import {
	addressLabel,
	mailFacts,
	payloadText,
	triggerModelText,
	triggerPreview,
	webhookFacts,
} from "./note-facts";

function trigger(patch: Partial<ConversationTrigger>): ConversationTrigger {
	return {
		type: "trigger",
		id: "t1",
		createdAt: "2026-11-04T09:00:00.000Z",
		time: 0,
		source: "schedule",
		refId: "task_1",
		title: "Morning brief",
		text: "Summarize the news",
		...patch,
	};
}

describe("mail facts", () => {
	test("reads addresses, body, verification and kept files from the payload", () => {
		const mail = mailFacts(
			trigger({
				source: "email",
				title: "Invoice",
				payload: {
					from: [{ name: "Anna", email: "anna@example.com" }],
					to: [{ name: "", email: "bot@example.com" }, { name: "x" }],
					subject: "Invoice 42",
					text: "Please pay.",
					authenticated: true,
					attachments: [
						{ name: "a.pdf", size: 10 },
						{ name: "big.zip", skipped: "too large" },
						{ size: 3 },
					],
				},
			}),
		);
		expect(mail).toEqual({
			from: [{ name: "Anna", email: "anna@example.com" }],
			to: [{ email: "bot@example.com" }],
			cc: [],
			subject: "Invoice 42",
			body: "Please pay.",
			authenticated: true,
			attachments: [
				{ name: "a.pdf", size: 10 },
				{ name: "big.zip", skipped: "too large" },
			],
		});
		expect(addressLabel(mail?.from[0] ?? { email: "" })).toBe(
			"Anna <anna@example.com>",
		);
	});

	test("other sources and payload-less mails have none", () => {
		expect(mailFacts(trigger({}))).toBeNull();
		expect(mailFacts(trigger({ source: "email" }))).toBeNull();
	});
});

describe("webhook facts", () => {
	test("split the content type from the body", () => {
		const hook = trigger({
			source: "webhook",
			payload: { content_type: "application/json", body: { id: 7 } },
		});
		expect(webhookFacts(hook)).toEqual({
			contentType: "application/json",
			body: { id: 7 },
		});
		expect(triggerPreview(hook)).toBe('{\n  "id": 7\n}');
	});

	test("a payload without a body is no webhook request", () => {
		expect(
			webhookFacts(trigger({ source: "webhook", payload: { other: 1 } })),
		).toBeNull();
	});
});

describe("texts", () => {
	test("payloads print strings as they are and the rest as JSON", () => {
		expect(payloadText("plain")).toBe("plain");
		expect(payloadText([1])).toBe("[\n  1\n]");
	});

	test("the model reads the instruction with the payload, as the hosts render it", () => {
		expect(
			triggerModelText(
				trigger({
					source: "webhook",
					text: "",
					title: "Order",
					payload: { body: 1 },
				}),
			),
		).toBe('Order\n\nPayload (webhook): {"body":1}');
	});
});
