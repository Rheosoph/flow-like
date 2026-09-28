import { describe, expect, test } from "bun:test";
import {
	EVENT_DEFINITIONS,
	isServerOnlyEventType,
	serverEventBlocker,
	serverEventBlockerMessage,
	serverEventSetupReady,
	sinkSupportsEventExecution,
} from "./event-definitions";
import { describeEventEntry } from "./event-entry";
import { getEventGuide, getEventSections } from "./event-sections";
import {
	normalizeInboundEmailAlias,
	validateInboundEmailAlias,
} from "./inbound-email";
import type { IEvent } from "./schema/flow/event";

describe("inbound email event", () => {
	test("has its own remote-only sink and preserves legacy IMAP email", () => {
		const definition = EVENT_DEFINITIONS.events_inbound_email;
		expect(definition.eventTypes).toEqual(["inbound_email"]);
		expect(definition.configs.inbound_email).toEqual({
			sink_type: "inbound_email",
		});
		expect(definition.withSink).toEqual(["inbound_email"]);
		const sink = definition.sinkAvailability?.inbound_email;
		expect(sinkSupportsEventExecution(sink, "Remote", true)).toBe(true);
		expect(sinkSupportsEventExecution(sink, "Local", true)).toBe(false);
		expect(EVENT_DEFINITIONS.events_mail.defaultEventType).toBe("email");
		expect(EVENT_DEFINITIONS.events_mail.configs.email.imap_port).toBe(993);
	});

	test("blocks saving only on a confirmed offline app or a Local board", () => {
		expect(isServerOnlyEventType("inbound_email")).toBe(true);
		expect(isServerOnlyEventType("teams")).toBe(true);
		expect(isServerOnlyEventType("email")).toBe(false);
		expect(serverEventBlocker(true, "Remote")).toBe("offline");
		expect(serverEventBlocker(false, "Local")).toBe("local_board");
		for (const pending of [undefined, null])
			expect(serverEventBlocker(pending, "Remote")).toBeUndefined();
		for (const unknownBoard of [undefined, null])
			expect(serverEventBlocker(false, unknownBoard)).toBeUndefined();
		expect(serverEventBlocker(false, "Remote")).toBeUndefined();
		expect(serverEventBlocker(false, "Hybrid")).toBeUndefined();
	});

	test("words blockers per event type through the common namespace", () => {
		const keys: string[] = [];
		const t = (key: string, fallback: string) => {
			keys.push(key);
			return fallback;
		};
		expect(serverEventBlockerMessage(t, "offline", "teams")).toContain(
			"Teams bots",
		);
		expect(
			serverEventBlockerMessage(t, "local_board", "inbound_email"),
		).toContain("Inbound email");
		expect(keys.every((key) => key.startsWith("common:"))).toBe(true);
	});

	test("setup is requested only once the event is saved as that remote type", () => {
		const saved = { event_type: "inbound_email", execution_mode: "Remote" };
		expect(serverEventSetupReady("inbound_email", saved)).toBe(true);
		expect(serverEventSetupReady("inbound_email", undefined)).toBe(true);
		expect(
			serverEventSetupReady("teams", { ...saved, event_type: "simple_chat" }),
		).toBe(false);
		expect(
			serverEventSetupReady("inbound_email", {
				...saved,
				execution_mode: "Local",
			}),
		).toBe(false);
		expect(
			serverEventSetupReady("discord", { ...saved, event_type: "telegram" }),
		).toBe(true);
	});

	test("normalizes the local part and reserves server-owned addresses", () => {
		expect(normalizeInboundEmailAlias(" Invoices ")).toBe("invoices");
		expect(normalizeInboundEmailAlias(" ")).toBeNull();
		for (const valid of ["", "invoices", "team-123", "x".repeat(64)])
			expect(validateInboundEmailAlias(valid)).toBeUndefined();
		for (const invalid of [
			"a",
			"ab",
			"x".repeat(65),
			"-invoices",
			"invoices-",
			"a@domain.com",
			"a+b",
			"sales.team",
		])
			expect(validateInboundEmailAlias(invalid)).toBe("format");
		for (const reserved of [
			"m-receipt",
			"POSTMASTER",
			"abuse",
			"no-reply",
			"mailer-daemon",
			"webmaster",
		])
			expect(validateInboundEmailAlias(reserved)).toBe("reserved");
	});

	test("shows hosted address guidance without deriving an address from untrusted config", () => {
		const event = { event_type: "inbound_email", active: true } as IEvent;
		expect(getEventSections(event)[0].label).toBe("Email address");
		expect(
			getEventGuide(event).find((step) => step.id === "send-test"),
		).toBeDefined();
		expect(describeEventEntry(event, { address: "wrong@spoof.test" })).toEqual({
			text: "Server email address",
			muted: true,
		});
	});
});
