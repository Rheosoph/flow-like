import type { RemotePushPayload } from "./remote-push";

export interface NativeBotPushIdentity {
	scope: string;
	subject: string;
}

/** Retained pushes must not cross account or workspace boundaries after a sign-in change. */
export function matchesBotPushIdentity(
	notification: RemotePushPayload,
	identity?: NativeBotPushIdentity,
): boolean {
	const category = notification.category ?? notification.data.category;
	const isBot =
		(typeof category === "string" && category.startsWith("flow_like_bot_")) ||
		typeof notification.data.bot_id === "string";
	return (
		!isBot ||
		(!!identity &&
			notification.data.native_scope === identity.scope &&
			notification.data.user_id === identity.subject)
	);
}
