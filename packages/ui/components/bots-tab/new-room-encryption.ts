import { useQuery } from "@tanstack/react-query";
import type { TFunction } from "i18next";
import { useCallback, useMemo } from "react";
import { apiErrorMessage } from "../../lib/api-error";
import { INBOX_KEY_ALGORITHM } from "../../lib/inbox-e2e";
import { useBackend } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import type { IInboxKey } from "../../state/backend-state/inbox-state";
import type { PickedPrincipal } from "./principal-picker";

export type SealBlock =
	| "local"
	| "server"
	| "no-ability"
	| "no-key"
	| "bot-no-key";

/** Principals per key lookup on the server. */
export const KEY_LOOKUP_LIMIT = 50;

/** Whether this device can seal: it needs an account and the browser or native crypto. */
export function canSealRooms(people: boolean, hasInbox: boolean): boolean {
	return people && hasInbox && Boolean(globalThis.crypto?.subtle);
}

/** Principals with a key this client can seal to. */
export function keyedPrincipals(keys: readonly IInboxKey[]): Set<string> {
	return new Set(
		keys
			.filter((key) => key.algorithm === INBOX_KEY_ALGORITHM)
			.map((key) => key.principal_id),
	);
}

/**
 * Why `principal` cannot be in an end-to-end encrypted room, or null. Everyone needs a published
 * key: a person's device publishes one when they open the app signed in, a bot's host while it
 * runs the bot with the encryption ability. Your own bots also say why they can't hold one;
 * another person's bot is judged by its key alone, since only the server knows its abilities.
 * `keyed` is undefined while the key lookup is pending or did not include `principal`.
 */
export function roomSealBlock(
	principal: PickedPrincipal,
	bots: ReadonlyMap<string, IBot>,
	keyed: ReadonlySet<string> | undefined,
): SealBlock | null {
	const bot = bots.get(principal.id) ?? principal.bot;
	if (bot) {
		if (bot.local) return "local";
		if (bot.location === "server") return "server";
		if (!bot.capabilities.includes("encryption")) return "no-ability";
	}
	if (!keyed || keyed.has(principal.id)) return null;
	return principal.kind === "bot" ? "bot-no-key" : "no-key";
}

export interface RoomSealing {
	/** Why the principal can't be sealed to, or null; key checks cover the `picked` only. */
	blockOf: (principal: PickedPrincipal) => SealBlock | null;
	/** The key lookup of the picked principals has not answered yet. */
	checking: boolean;
	failed: boolean;
}

/** Who of `picked` an end-to-end encrypted room could not seal to, from one key lookup. */
export function useRoomSealing(
	picked: readonly PickedPrincipal[],
	bots: readonly IBot[],
	enabled: boolean,
): RoomSealing {
	const inbox = useBackend().inboxState;
	const botsById = useMemo(
		() => new Map(bots.map((bot) => [bot.id, bot])),
		[bots],
	);
	const ids = useMemo(
		() => picked.map((principal) => principal.id).slice(0, KEY_LOOKUP_LIMIT),
		[picked],
	);
	const active = enabled && ids.length > 0 && Boolean(inbox);
	const keys = useQuery({
		queryKey: ["room-seal-keys", ids],
		enabled: active,
		queryFn: async () => keyedPrincipals((await inbox?.getKeys(ids)) ?? []),
	});
	const blockOf = useCallback(
		(principal: PickedPrincipal) =>
			roomSealBlock(
				principal,
				botsById,
				ids.includes(principal.id) ? keys.data : undefined,
			),
		[botsById, ids, keys.data],
	);
	return {
		blockOf,
		checking: active && keys.data === undefined && !keys.isError,
		failed: active && keys.isError,
	};
}

export function sealBlockText(block: SealBlock, t: TFunction<"bots">): string {
	switch (block) {
		case "local":
			return t(
				"crew.newRoomDialog.encryptionOnDevice",
				"Lives on this device only",
			);
		case "server":
			return t(
				"crew.newRoomDialog.encryptionServerBot",
				"Runs on the server, which can't read an encrypted room",
			);
		case "no-ability":
			return t(
				"crew.newRoomDialog.encryptionBotNoAbility",
				"Needs the end-to-end encryption ability",
			);
		case "bot-no-key":
			return t(
				"crew.newRoomDialog.encryptionBotNoKey",
				"Has no encryption key yet; its host publishes one while it runs the bot with end-to-end encryption",
			);
		default:
			return t(
				"crew.newRoomDialog.encryptionNoKey",
				"Has no encryption key yet; opening the app signed in publishes one",
			);
	}
}

/** The server's refusals of an encrypted room that the form could not know in advance. */
export function encryptedRoomError(
	error: unknown,
	fallback: string,
	t: TFunction<"bots">,
): string {
	const code = (error as { code?: unknown } | null)?.code;
	if (code === "INBOX_MEMBER_NOT_ALLOWED") {
		return t(
			"crew.newRoomDialog.encryptionNotAllowed",
			"One of the members can't join an encrypted room. Another person's bot can only join a plain room.",
		);
	}
	if (code === "INBOX_KEY_MISSING") {
		return t(
			"crew.newRoomDialog.encryptionKeyMissing",
			"A member has no encryption key yet, so nothing can be sealed to them.",
		);
	}
	return apiErrorMessage(error, fallback);
}
