import { isRecord } from "../../lib/response-shape";
import type { ConversationTrigger } from "./fold-entries";

export interface MailAddress {
	readonly name?: string;
	readonly email: string;
}

export interface MailAttachment {
	readonly name: string;
	readonly size?: number;
	/** Why the file wasn't kept, e.g. "too large". */
	readonly skipped?: string;
}

export interface MailFacts {
	readonly from: readonly MailAddress[];
	readonly to: readonly MailAddress[];
	readonly cc: readonly MailAddress[];
	readonly subject: string;
	readonly body: string;
	/** The sender's domain passed SPF/DKIM; anything else may be spoofed. */
	readonly authenticated: boolean;
	readonly attachments: readonly MailAttachment[];
}

export interface WebhookFacts {
	readonly contentType: string;
	readonly body: unknown;
}

function stringField(value: unknown): string | undefined {
	return typeof value === "string" && value.trim() ? value.trim() : undefined;
}

function addresses(value: unknown): MailAddress[] {
	if (!Array.isArray(value)) return [];
	return value.flatMap((item) => {
		const email = isRecord(item) ? stringField(item.email) : undefined;
		if (!email || !isRecord(item)) return [];
		const name = stringField(item.name);
		return [name ? { name, email } : { email }];
	});
}

function attachments(value: unknown): MailAttachment[] {
	if (!Array.isArray(value)) return [];
	return value.flatMap((item) => {
		const name = isRecord(item) ? stringField(item.name) : undefined;
		if (!name || !isRecord(item)) return [];
		const skipped = stringField(item.skipped);
		return [
			{
				name,
				...(typeof item.size === "number" ? { size: item.size } : {}),
				...(skipped ? { skipped } : {}),
			},
		];
	});
}

/** "Anna <anna@example.com>", or the address alone. */
export function addressLabel(address: MailAddress): string {
	return address.name ? `${address.name} <${address.email}>` : address.email;
}

/** The mail an email trigger carries; null for any other trigger. */
export function mailFacts(trigger: ConversationTrigger): MailFacts | null {
	if (trigger.source !== "email" || !isRecord(trigger.payload)) return null;
	const mail = trigger.payload;
	return {
		from: addresses(mail.from),
		to: addresses(mail.to),
		cc: addresses(mail.cc),
		subject: stringField(mail.subject) ?? trigger.title,
		body: typeof mail.text === "string" ? mail.text : "",
		authenticated: mail.authenticated === true,
		attachments: attachments(mail.attachments),
	};
}

/** The request a webhook trigger carries; null for any other trigger. */
export function webhookFacts(
	trigger: ConversationTrigger,
): WebhookFacts | null {
	if (
		trigger.source !== "webhook" ||
		!isRecord(trigger.payload) ||
		!("body" in trigger.payload)
	)
		return null;
	return {
		contentType: stringField(trigger.payload.content_type) ?? "",
		body: trigger.payload.body,
	};
}

/** Strings as they are, anything else as indented JSON. */
export function payloadText(value: unknown): string {
	if (typeof value === "string") return value;
	try {
		return JSON.stringify(value, null, 2) ?? "";
	} catch {
		return String(value);
	}
}

/** What the hover card previews: the mail body, the webhook body, or the bot's instruction. */
export function triggerPreview(trigger: ConversationTrigger): string {
	const mail = mailFacts(trigger);
	if (mail?.body.trim()) return mail.body;
	const hook = webhookFacts(trigger);
	if (hook) return payloadText(hook.body);
	return trigger.text;
}

/** The text the model reads for a trigger, the same the hosts render. */
export function triggerModelText(trigger: ConversationTrigger): string {
	let text = trigger.text.trim() ? trigger.text : trigger.title;
	if (trigger.payload !== undefined) {
		text += `\n\nPayload (${trigger.source}): ${JSON.stringify(trigger.payload)}`;
	}
	if (trigger.payloadFile) {
		text += `\n\nPayload (${trigger.source}) saved as file: ${trigger.payloadFile.path}`;
	}
	return text;
}
