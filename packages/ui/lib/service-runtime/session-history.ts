import { openRuntimeNamespace, runtimeRowAppId } from "./session-scope";

/** Retire the namespace before awaiting cleanup so queued writes are already fenced. */
export function startRuntimeHistory(appId: string): () => Promise<void> {
	const retire = openRuntimeNamespace(appId);
	let closing: Promise<void> | undefined;
	return () => {
		retire();
		closing ??= clearRuntimeHistory(appId);
		return closing;
	};
}

async function clearRuntimeHistory(appId: string): Promise<void> {
	const [{ chatDb }, { uiStateDb }, { releaseFrontendStateStore }] =
		await Promise.all([
			import("../../components/interfaces/chat-default/chat-db"),
			import("../../db/ui-state-db"),
			import("../../components/a2ui/frontend-state"),
		]);
	releaseFrontendStateStore(appId);
	await Promise.all([
		chatDb.transaction("rw", chatDb.tables, async () => {
			const sessions = await chatDb.sessions
				.where("appId")
				.equals(appId)
				.primaryKeys();
			if (sessions.length) {
				await chatDb.messages.where("sessionId").anyOf(sessions).delete();
				await chatDb.drafts.where("sessionId").anyOf(sessions).delete();
			}
			// A cancelled request may have saved a message before it saved its session.
			await chatDb.messages
				.filter((row) => runtimeRowAppId(row) === appId)
				.delete();
			await chatDb.drafts
				.filter((row) => runtimeRowAppId(row) === appId)
				.delete();
			await chatDb.localStage.where("appId").equals(appId).delete();
			await chatDb.globalState.where("appId").equals(appId).delete();
			await chatDb.sessions.where("appId").equals(appId).delete();
		}),
		uiStateDb.transaction("rw", uiStateDb.tables, async () => {
			for (const table of uiStateDb.tables)
				await table.where("appId").equals(appId).delete();
		}),
	]);
}
