"use client";

import { useTranslation } from "@flow-like/locales";
import { useId } from "react";
import { useBotLabels } from "../../../../hooks/use-bots";
import type { BotSystemKey } from "../../../../state/backend-state/bot-state";
import { BotCustomizer } from "../../../bots";
import type { BotAppearance } from "../../../bots/types";
import { Input } from "../../../ui/input";
import { Label } from "../../../ui/label";
import type { BotDraft } from "../bot-draft";
import { BOT_OWNED_PACK_IDS } from "../bot-look";
import type { BotIdentityChange } from "../bot-preview-panel";

/** The look studio as it is, with the bot's name and description above it. */
export function LookStep({
	draft,
	system,
	nameInvalid,
	onIdentityChange,
	onAppearanceChange,
}: Readonly<{
	draft: BotDraft;
	system: BotSystemKey | null;
	nameInvalid: boolean;
	onIdentityChange: (change: BotIdentityChange) => void;
	onAppearanceChange: (appearance: BotAppearance) => void;
}>) {
	const { t } = useTranslation("bots");
	const defaults = useBotLabels()({ system });
	const ids = useId();
	return (
		<div className="flex flex-col gap-6">
			<div className="grid gap-4 rounded-2xl border border-border bg-card p-4 @2xl:grid-cols-[minmax(0,1fr)_minmax(0,2fr)]">
				<div className="flex min-w-0 flex-col gap-1.5">
					<Label htmlFor={`${ids}-name`}>{t("editor.preview.name")}</Label>
					<Input
						id={`${ids}-name`}
						value={draft.name}
						aria-invalid={nameInvalid || undefined}
						placeholder={defaults.name || t("editor.preview.namePlaceholder")}
						onChange={(event) => onIdentityChange({ name: event.target.value })}
					/>
				</div>
				<div className="flex min-w-0 flex-col gap-1.5">
					<Label htmlFor={`${ids}-description`}>
						{t("editor.preview.description")}
					</Label>
					<Input
						id={`${ids}-description`}
						value={draft.description}
						placeholder={
							defaults.description || t("editor.preview.descriptionPlaceholder")
						}
						onChange={(event) =>
							onIdentityChange({ description: event.target.value })
						}
					/>
				</div>
			</div>
			<BotCustomizer
				value={draft.appearance}
				onChange={onAppearanceChange}
				ownedPackIds={BOT_OWNED_PACK_IDS}
			/>
		</div>
	);
}
