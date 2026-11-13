import { expect, test } from "vitest";
import { IRole } from "../../../lib/schema/llm/history";
import type { IMessage } from "./chat-db";
import { askToResend } from "./chat-error-resend";

function message(
	id: string,
	role: IRole,
	content: string,
	extra: Partial<IMessage> = {},
): IMessage {
	return {
		id,
		appId: "a",
		sessionId: "s",
		inner: { role, content },
		files: [],
		tools: [],
		actions: [],
		timestamp: 0,
		...extra,
	};
}

const ASK = message("u1", IRole.User, "Summarize my week");
const FAILED = message("a1", IRole.Assistant, "I can't answer yet");

test("the failed answer's ask is the one to send again", () => {
	expect(askToResend([ASK, FAILED], "a1")).toBe("Summarize my week");
});

test("a later message of yours means the failure was moved past", () => {
	const later = message("u2", IRole.User, "Hello?");
	expect(askToResend([ASK, FAILED, later], "a1")).toBeNull();
});

test("an ask with files, or an unknown answer, is not sent again", () => {
	const withFile = message("u1", IRole.User, "See this", {
		files: [{ name: "a.pdf" } as IMessage["files"][number]],
	});
	expect(askToResend([withFile, FAILED], "a1")).toBeNull();
	expect(askToResend([ASK, FAILED], "missing")).toBeNull();
});

test("the written text wins over its rendering", () => {
	const mention = message("u1", IRole.User, "@Ada summarize", {
		source_text: "@[Ada](bot:b) summarize",
	});
	expect(askToResend([mention, FAILED], "a1")).toBe("@[Ada](bot:b) summarize");
});
