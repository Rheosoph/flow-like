export interface NativeBotActivityToken {
	activityId: string;
	botId: string;
	turnId: string;
	scope: string;
	pushToken: string;
}

/** Registration is scoped to both the hub and the signed-in account, just like native actions. */
export function createBotActivityRegistrar() {
	const registered = new Map<
		string,
		{ targetId: string; token: NativeBotActivityToken }
	>();
	return async (
		scope: string,
		targetId: string,
		tokens: NativeBotActivityToken[],
		register: (token: NativeBotActivityToken) => Promise<void>,
		isCurrent: () => boolean,
		unregister?: (token: NativeBotActivityToken) => Promise<void>,
	) => {
		for (const [key, record] of registered) {
			if (!isCurrent()) return;
			if (
				record.targetId !== targetId ||
				record.token.scope !== scope ||
				tokens.some(
					(token) =>
						token.scope === scope &&
						token.activityId === record.token.activityId,
				)
			)
				continue;
			if (unregister) await unregister(record.token);
			if (isCurrent()) registered.delete(key);
		}
		for (const token of tokens.slice(0, 8)) {
			if (!isCurrent()) return;
			if (
				token.scope !== scope ||
				!token.activityId ||
				!token.botId ||
				!token.turnId ||
				!/^[a-f\d]{32,512}$/i.test(token.pushToken)
			)
				continue;
			const key = JSON.stringify([scope, targetId, token.activityId]);
			if (registered.get(key)?.token.pushToken === token.pushToken) continue;
			await register(token);
			if (isCurrent()) registered.set(key, { targetId, token });
		}
		if (registered.size > 64) registered.clear();
	};
}
