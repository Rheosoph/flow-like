"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useId, useState } from "react";
import { toast } from "sonner";
import { BotAvatar } from "../bots/bot-avatar";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Switch } from "../ui/switch";
import { BuiltInBadge, LocalBadge } from "./bot-labels";
import { type CrewMember, crewVisibilityKey } from "./crew-grid";
import { useHiddenBots, useSetBotHidden } from "./use-bot-visibility";

const NO_ANSWERS: ReadonlyMap<string, boolean> = new Map();

/** The switches answer at once; each keeps its answer until the profile read back confirms it. */
function useShownSwitches() {
	const { t } = useTranslation("bots");
	const { keys: hidden, profileName } = useHiddenBots();
	const { mutateAsync: setHidden } = useSetBotHidden();
	const [answers, setAnswers] = useState(NO_ANSWERS);
	const isShown = useCallback(
		(key: string) => answers.get(key) ?? !hidden.has(key),
		[answers, hidden],
	);
	const setShown = useCallback(
		async (key: string, shown: boolean) => {
			setAnswers((current) => new Map(current).set(key, shown));
			try {
				await setHidden({ key, hidden: !shown });
			} catch (error) {
				console.error(`Bot ${key} kept its visibility in the profile:`, error);
				toast.error(
					t(
						"crew.visibility.failed",
						"Couldn't change which bots this profile shows.",
					),
				);
			} finally {
				setAnswers((current) => {
					if (current.get(key) !== shown) return current;
					const next = new Map(current);
					next.delete(key);
					return next;
				});
			}
		},
		[setHidden, t],
	);
	return { isShown, setShown, profileName };
}

function VisibilityRow({
	member,
	shown,
	onShownChange,
}: Readonly<{
	member: CrewMember;
	shown: boolean;
	onShownChange: (shown: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	const nameId = useId();
	return (
		<li className="flex items-center gap-3 py-2">
			<span className="flex size-9 shrink-0 items-end justify-center overflow-hidden rounded-xl bg-muted">
				<BotAvatar appearance={member.appearance} size={36} decorative />
			</span>
			<span className="flex min-w-0 flex-1 flex-col">
				<span className="flex min-w-0 items-center gap-1.5">
					<span id={nameId} className="truncate text-sm font-medium">
						{member.name}
					</span>
					{member.system && <BuiltInBadge className="shrink-0" />}
					{member.local && <LocalBadge className="shrink-0" />}
				</span>
				{member.description && (
					<span className="truncate text-xs text-muted-foreground">
						{member.description}
					</span>
				)}
			</span>
			<Switch
				checked={shown}
				onCheckedChange={onShownChange}
				aria-labelledby={nameId}
				title={
					shown
						? t("crew.visibility.shown", "Shown in this profile")
						: t("crew.visibility.hidden", "Hidden in this profile")
				}
			/>
		</li>
	);
}

/** Which of the reader's bots the current profile shows; `members` holds every bot, hidden or not. */
export function BotVisibilityDialog({
	members,
	onOpenChange,
}: Readonly<{
	members: readonly CrewMember[];
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	const { isShown, setShown, profileName } = useShownSwitches();
	return (
		<Dialog open onOpenChange={onOpenChange}>
			<DialogContent className="flex max-h-[85dvh] flex-col overflow-hidden sm:max-w-md">
				<DialogHeader>
					<DialogTitle>
						{t("crew.visibility.title", "Bots in this profile")}
					</DialogTitle>
					<DialogDescription>
						{profileName
							? t(
									"crew.visibility.descriptionNamed",
									"Choose the bots “{{profile}}” shows. A hidden bot keeps working and stays in its rooms.",
									{ profile: profileName },
								)
							: t(
									"crew.visibility.description",
									"Choose the bots this profile shows. A hidden bot keeps working and stays in its rooms.",
								)}
					</DialogDescription>
				</DialogHeader>
				<ul className="-mr-2 min-h-0 flex-1 divide-y overflow-y-auto pr-2">
					{members.map((member) => {
						const key = crewVisibilityKey(member);
						return (
							<VisibilityRow
								key={member.key}
								member={member}
								shown={isShown(key)}
								onShownChange={(shown) => void setShown(key, shown)}
							/>
						);
					})}
				</ul>
			</DialogContent>
		</Dialog>
	);
}
