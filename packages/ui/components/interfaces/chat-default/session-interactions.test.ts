import { expect, test } from "bun:test";
import type { IInteractionRequest } from "../../../lib/schema/interaction";
import {
	addSessionInteractions,
	updateSessionInteractions,
} from "./session-interactions";

const request = (id: string, status: IInteractionRequest["status"]) =>
	({ id, status }) as IInteractionRequest;

test("a replayed request does not reopen one that was answered", () => {
	const recorded: IInteractionRequest[][] = [];
	addSessionInteractions("replay", [request("ask", "pending")]);
	updateSessionInteractions("replay", (prev) => {
		const next = prev.map((interaction) => ({
			...interaction,
			status: "responded" as const,
		}));
		recorded.push(next);
		return next;
	});
	addSessionInteractions("replay", [request("ask", "pending")]);
	updateSessionInteractions("replay", (prev) => {
		recorded.push(prev);
		return prev;
	});
	expect(recorded.at(-1)?.map((interaction) => interaction.status)).toEqual([
		"responded",
	]);
});
