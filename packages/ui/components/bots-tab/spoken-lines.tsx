"use client";

import { useTranslation } from "@flow-like/locales";
import { MicIcon } from "lucide-react";
import type { ConversationBlock } from "./fold-entries";
import type { CrewDirectory } from "./use-crew-directory";

type SpokenBlock = Extract<ConversationBlock, { type: "spoken" }>;

/** What a person said aloud in a call, as compact muted lines under their name. */
export function SpokenLines({
	block,
	directory,
}: Readonly<{ block: SpokenBlock; directory: CrewDirectory }>) {
	const { t } = useTranslation("bots");
	const person = directory(block.senderId);
	const name = person.isMe ? t("crew.you", "You") : person.name;
	return (
		<div
			data-slot="spoken-lines"
			className="flex items-start gap-2 px-3 text-xs leading-normal text-muted-foreground"
		>
			<MicIcon className="mt-0.5 size-3.5 shrink-0" aria-hidden />
			<div className="min-w-0">
				<span className="font-medium">
					{t("crew.transcript.said", "{{name}} said", { name })}
				</span>
				{block.lines.map((line) => (
					<p key={line.id} className="break-words">
						{line.text}
					</p>
				))}
			</div>
		</div>
	);
}
