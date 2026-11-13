"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useRef, useState } from "react";
import { toast } from "sonner";
import {
	useBotLabels,
	useCloneBot,
	useCreateBot,
	useDeleteBot,
	useSetBotApp,
	useSetBotAppRestrictions,
	useUpdateBot,
} from "../../../hooks/use-bots";
import { ApiResponseError } from "../../../lib/api-error";
import { getErrorMessage } from "../../../lib/error-message";
import { useBackend } from "../../../state/backend-state";
import type { IBot } from "../../../state/backend-state/bot-state";
import { forgetBotChat } from "../../../state/global-chat/global-chat-stream";
import { buildCreateBotRequest } from "./bot-draft";
import { useBotErrorMessage } from "./bot-editor-copy";
import type { BotEditor } from "./use-bot-editor";

export interface BotEditorSubmit {
	submit: () => Promise<void>;
	remove: () => Promise<void>;
	/** Set for an existing bot where the host can copy bots. */
	duplicate?: () => Promise<void>;
	saving: boolean;
	deleting: boolean;
	duplicating: boolean;
	error: string | null;
}

export interface BotEditorCallbacks {
	onSaved?: (bot: IBot) => void;
	/** Gets the copy after Duplicate, once the editor has closed. */
	onDuplicated?: (bot: IBot) => void;
	close: () => void;
}

type BotEditorTask = "saving" | "deleting" | "duplicating";

/**
 * One create, save, delete or duplicate at a time, from the click until the editor closes. The ref
 * turns away clicks that land before React re-renders; only a failure frees the editor again.
 */
function useBotEditorTask() {
	const running = useRef<BotEditorTask | null>(null);
	const [task, setTask] = useState<BotEditorTask | null>(null);
	const begin = (next: BotEditorTask) => {
		if (running.current) return false;
		running.current = next;
		setTask(next);
		return true;
	};
	const release = () => {
		running.current = null;
		setTask(null);
	};
	return { task, begin, release };
}

/**
 * Copies a saved bot and says how it went: the copy, or `null` after a toast. Anything but a hub
 * refusal (a missing local command, say) adds its own message. Unset where the host can't copy bots.
 */
export function useDuplicateBot():
	| ((bot: IBot) => Promise<IBot | null>)
	| undefined {
	const { t } = useTranslation("bots");
	const errorMessage = useBotErrorMessage();
	const labels = useBotLabels();
	const canDuplicate = Boolean(useBackend().botState?.cloneBot);
	const { mutateAsync: cloneBot } = useCloneBot();
	const duplicate = useCallback(
		async (bot: IBot) => {
			let copy: IBot;
			try {
				copy = await cloneBot(bot.id);
			} catch (cause) {
				toast.error(errorMessage(cause, t("editor.errors.duplicateFailed")), {
					description:
						cause instanceof ApiResponseError
							? undefined
							: getErrorMessage(cause, "") || undefined,
				});
				return null;
			}
			toast.success(
				t("editor.duplicated", {
					name: labels(copy).name || labels(bot).name,
				}),
			);
			return copy;
		},
		[cloneBot, errorMessage, labels, t],
	);
	return canDuplicate ? duplicate : undefined;
}

/**
 * Creates or saves the bot, attaches apps picked before it existed, deletes custom bots and
 * duplicates saved ones.
 */
export function useBotEditorSubmit(
	editor: BotEditor,
	{ onSaved, onDuplicated, close }: BotEditorCallbacks,
): BotEditorSubmit {
	const { t } = useTranslation("bots");
	const errorMessage = useBotErrorMessage();
	const duplicateBot = useDuplicateBot();
	const { mutateAsync: createBot } = useCreateBot();
	const { mutateAsync: updateBot } = useUpdateBot();
	const { mutateAsync: deleteBot } = useDeleteBot();
	const { mutateAsync: setBotApp } = useSetBotApp();
	const { mutateAsync: setAppRestrictions } = useSetBotAppRestrictions();
	const [error, setError] = useState<string | null>(null);
	const { task, begin, release } = useBotEditorTask();

	const attachPendingApps = async (bot: IBot) => {
		if (!bot.capabilities.includes("selected_apps")) return;
		const results = await Promise.allSettled(
			editor.pendingAppIds.map((appId) =>
				setBotApp({ botId: bot.id, appId, attached: true }),
			),
		);
		const failed = results.filter(
			(result): result is PromiseRejectedResult => result.status === "rejected",
		);
		if (failed.length)
			toast.warning(t("editor.appsAttachFailed", { count: failed.length }), {
				description: errorMessage(failed[0].reason, "") || undefined,
			});
	};

	const create = async () => {
		const bot = await createBot(buildCreateBotRequest(editor.draft));
		await attachPendingApps(bot);
		return bot;
	};

	const save = async (bot: IBot) => {
		const request = editor.updateRequest ?? {};
		const saved = Object.keys(request).length
			? await updateBot({ botId: bot.id, request })
			: bot;
		for (const { appId, attached, restrictions } of editor.appAccess.changes) {
			if (attached !== undefined) {
				await setBotApp({ botId: bot.id, appId, attached });
				editor.appAccess.markAttachedSaved(appId, attached);
			}
			if (restrictions) {
				const savedRestrictions = await setAppRestrictions({
					botId: bot.id,
					appId,
					restrictions,
				});
				editor.appAccess.markRestrictionsSaved(appId, savedRestrictions);
			}
		}
		return saved;
	};

	const submit = async () => {
		if (editor.problems.length || !begin("saving")) return;
		setError(null);
		const original = editor.original;
		let bot: IBot;
		try {
			bot = original ? await save(original) : await create();
		} catch (cause) {
			release();
			setError(
				errorMessage(
					cause,
					original
						? t("editor.errors.saveFailed")
						: t("editor.errors.createFailed"),
				),
			);
			return;
		}
		onSaved?.(bot);
		close();
	};

	const remove = async () => {
		const bot = editor.original;
		if (!bot || bot.system || !begin("deleting")) return;
		setError(null);
		try {
			await deleteBot(bot.id);
		} catch (cause) {
			release();
			setError(errorMessage(cause, t("editor.errors.deleteFailed")));
			return;
		}
		void forgetBotChat(bot.id);
		toast.success(t("editor.deleted", { name: bot.name ?? "" }));
		close();
	};

	/**
	 * Copies the bot as saved, so it waits while the draft has unsaved changes. A failure leaves the
	 * editor and the action as they were.
	 */
	const duplicate = async () => {
		const bot = editor.original;
		if (!bot || !duplicateBot || editor.dirty || !begin("duplicating")) return;
		const copy = await duplicateBot(bot);
		if (!copy) {
			release();
			return;
		}
		close();
		onDuplicated?.(copy);
	};

	return {
		submit,
		remove,
		duplicate: duplicateBot && editor.original ? duplicate : undefined,
		saving: task === "saving",
		deleting: task === "deleting",
		duplicating: task === "duplicating",
		error,
	};
}
