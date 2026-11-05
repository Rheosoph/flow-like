"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2 } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { useBot } from "../../../hooks/use-bots";
import { useBackend } from "../../../state/backend-state";
import type { IBot } from "../../../state/backend-state/bot-state";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "../../ui/dialog";
import type { BotDiscardPrompt } from "./bot-editor-footer";
import { BotEditorSuite } from "./bot-editor-suite";

export interface BotEditorDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	/** Edit this bot; omit to create one. */
	botId?: string | null;
	onSaved?: (bot: IBot) => void;
	/** Gets the copy made by Duplicate, after the dialog closed; the Bots tab opens it. */
	onDuplicated?: (bot: IBot) => void;
	/** A new bot starts on this managed device. */
	initialDeviceId?: string;
}

interface BodyProps {
	onSaved?: (bot: IBot) => void;
	onDuplicated?: (bot: IBot) => void;
	initialDeviceId?: string;
	close: () => void;
	discard: BotDiscardPrompt;
	onDirtyChange: (dirty: boolean) => void;
}

/**
 * Creates a bot (describe, look, abilities, brain, host, safety) or edits one, secrets included,
 * built-in bots too, and duplicates a saved one.
 */
export function BotEditorDialog({
	open,
	onOpenChange,
	botId,
	onSaved,
	onDuplicated,
	initialDeviceId,
}: Readonly<BotEditorDialogProps>) {
	const dirty = useRef(false);
	const [discardRequested, setDiscardRequested] = useState(false);
	const close = useCallback(() => {
		dirty.current = false;
		setDiscardRequested(false);
		onOpenChange(false);
	}, [onOpenChange]);
	const handleOpenChange = useCallback(
		(next: boolean) => {
			if (!next && dirty.current) {
				setDiscardRequested(true);
				return;
			}
			setDiscardRequested(false);
			onOpenChange(next);
		},
		[onOpenChange],
	);
	const onDirtyChange = useCallback((next: boolean) => {
		dirty.current = next;
		if (!next) setDiscardRequested(false);
	}, []);
	const body: BodyProps = {
		onSaved,
		onDuplicated,
		initialDeviceId,
		close,
		discard: {
			requested: discardRequested,
			keep: () => setDiscardRequested(false),
			discard: close,
		},
		onDirtyChange,
	};
	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent className="flex h-[calc(100dvh-1rem-var(--fl-safe-top,0px)-var(--fl-safe-bottom,0px))] max-h-[calc(100dvh-1rem-var(--fl-safe-top,0px)-var(--fl-safe-bottom,0px))] w-[calc(100%-1rem)] max-w-[calc(100%-1rem)] flex-col gap-0 overflow-hidden p-0 sm:h-[min(58rem,calc(100dvh-2rem))] sm:max-w-6xl">
				<BotEditorBody botId={botId ?? null} {...body} />
			</DialogContent>
		</Dialog>
	);
}

function BotEditorBody({
	botId,
	...body
}: Readonly<BodyProps & { botId: string | null }>) {
	const { t } = useTranslation("bots");
	const available = Boolean(useBackend().botState);
	if (!available)
		return (
			<BotEditorMessage
				title={botId ? t("editor.title.edit") : t("editor.title.create")}
				message={t("editor.unavailable")}
			/>
		);
	if (botId) return <EditBotLoader botId={botId} {...body} />;
	return <BotEditorSuite mode="create" bot={null} {...body} />;
}

function EditBotLoader({
	botId,
	...body
}: Readonly<BodyProps & { botId: string }>) {
	const { t } = useTranslation("bots");
	const query = useBot(botId);
	if (query.data)
		return (
			<BotEditorSuite
				key={query.data.id}
				mode="edit"
				bot={query.data}
				{...body}
			/>
		);
	return (
		<BotEditorMessage
			title={t("editor.title.edit")}
			message={query.isError ? t("editor.loadFailed") : t("editor.loading")}
			loading={!query.isError}
		/>
	);
}

function BotEditorMessage({
	title,
	message,
	loading = false,
}: Readonly<{ title: string; message: string; loading?: boolean }>) {
	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-3 p-8 text-center">
			<DialogTitle>{title}</DialogTitle>
			<DialogDescription role={loading ? "status" : "alert"}>
				{loading && (
					<Loader2 className="mr-2 inline size-4 animate-spin" aria-hidden />
				)}
				{message}
			</DialogDescription>
		</div>
	);
}
