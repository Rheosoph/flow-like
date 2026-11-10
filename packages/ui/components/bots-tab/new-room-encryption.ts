import type { TFunction } from "i18next";
import { apiErrorMessage } from "../../lib/api-error";
import { INBOX_KEY_ALGORITHM } from "../../lib/inbox-e2e";
import type { IBot } from "../../state/backend-state/bot-state";
import type { IInboxKey } from "../../state/backend-state/inbox-state";
import type { PickedPrincipal } from "./principal-picker";

export type SealBlock = "local" | "server" | "no-ability" | "no-key";

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
 * Why `principal` cannot be in an end-to-end encrypted room, or null. A bot of someone else is
 * only known to the server, so it passes here and the server answers on submit. `keyed` is
 * undefined while the key lookup is pending; invited people hold no key until they accept.
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
	if (principal.mode === "invite" || !keyed) return null;
	if (principal.kind === "bot" && !bot) return null;
	return keyed.has(principal.id) ? null : "no-key";
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
