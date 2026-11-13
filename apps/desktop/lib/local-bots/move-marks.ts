import type { IProfile } from "@flow-like/flow-like-ui";
import type { AuthContextProps } from "react-oidc-context";
import { patch, put } from "../api";
import { localInbox } from "./commands";

const STARS_PAGE = 200;

/** The pins and stars a local chat holds, oldest first so the account's newest-first order is kept. */
export interface IConversationMarks {
	pinnedAt: string | null;
	pins: string[];
	stars: string[];
}

export type MarksByConversation = ReadonlyMap<string, IConversationMarks>;

/** One write the account still owes the moved chats. */
export interface IMarkWrite {
	readonly description: string;
	readonly method: "put" | "patch";
	readonly path: string;
	readonly body: unknown;
	readonly pinnedAt?: string;
}

interface IAccountTarget {
	readonly profile: IProfile;
	readonly auth: AuthContextProps;
}

const conversationPath = (id: string) =>
	`inbox/conversations/${encodeURIComponent(id)}`;

async function starredIds(conversationId: string): Promise<string[]> {
	const ids: string[] = [];
	let before: string | undefined;
	do {
		const page = await localInbox.stars({
			conversationId,
			before,
			limit: STARS_PAGE,
		});
		ids.push(...page.stars.map((star) => star.message_id));
		before = page.next_before ?? undefined;
	} while (before);
	return ids.reverse();
}

/** Read before the move: the local copy of a moved chat is forgotten with it. */
export async function readLocalMarks(
	botId: string,
): Promise<MarksByConversation> {
	const conversations = await localInbox.conversations();
	const marks = new Map<string, IConversationMarks>();
	for (const conversation of conversations) {
		if (!conversation.members.some((member) => member.principal_id === botId))
			continue;
		const pins = await localInbox.pins(conversation.id);
		const stars = await starredIds(conversation.id);
		const pinnedAt = conversation.pinned_at ?? null;
		if (pinnedAt || pins.length || stars.length) {
			marks.set(conversation.id, {
				pinnedAt,
				pins: pins.map((pin) => pin.message_id).reverse(),
				stars,
			});
		}
	}
	return marks;
}

/** The writes that give `accountId` the marks of its local chat; message ids survive the move. */
export function markWrites(
	accountId: string,
	marks: IConversationMarks,
): IMarkWrite[] {
	const base = conversationPath(accountId);
	const writes: IMarkWrite[] = [
		...marks.pins.map((id) => ({
			description: `pin ${id} in conversation ${accountId}`,
			method: "put" as const,
			path: `${base}/pins/${encodeURIComponent(id)}`,
			body: {},
		})),
		...marks.stars.map((id) => ({
			description: `star ${id} in conversation ${accountId}`,
			method: "put" as const,
			path: `${base}/stars/${encodeURIComponent(id)}`,
			body: {},
		})),
	];
	if (marks.pinnedAt) {
		writes.push({
			description: `pin conversation ${accountId}`,
			method: "patch",
			path: `${base}/pin`,
			body: { pinned: true },
			pinnedAt: marks.pinnedAt,
		});
	}
	return writes;
}

/** Conversation pins last and oldest first, so the account lists them in the order they had here. */
function inOrder(writes: readonly IMarkWrite[]): IMarkWrite[] {
	const stamp = (write: IMarkWrite) => Date.parse(write.pinnedAt ?? "") || 0;
	return [...writes].sort(
		(a, b) =>
			Number(!!a.pinnedAt) - Number(!!b.pinnedAt) || stamp(a) - stamp(b),
	);
}

/** Writes what it can and returns what the account refused, for a later try. */
export async function applyMarkWrites(
	target: IAccountTarget,
	writes: readonly IMarkWrite[],
): Promise<IMarkWrite[]> {
	const failed: IMarkWrite[] = [];
	for (const write of inOrder(writes)) {
		const send = write.method === "patch" ? patch : put;
		try {
			await send(target.profile, write.path, write.body, target.auth);
		} catch (error) {
			console.warn(`[LocalBots] Could not ${write.description}:`, error);
			failed.push(write);
		}
	}
	return failed;
}
