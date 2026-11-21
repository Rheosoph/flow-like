"use client";

import { useTranslation } from "@flow-like/locales";
import { BotIcon, WorkflowIcon } from "lucide-react";
import type { GuideTourId } from "../../lib/guide/guide-model";
import { useGuideStore } from "../../state/guide-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";

function Choice({
	icon,
	title,
	body,
	cta,
	onSelect,
}: Readonly<{
	icon: React.ReactNode;
	title: string;
	body: string;
	cta: string;
	onSelect: () => void;
}>) {
	return (
		<button
			type="button"
			onClick={onSelect}
			className="group flex min-w-0 flex-1 flex-col items-start gap-2 rounded-xl border border-border bg-card p-4 text-left transition-colors hover:border-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
		>
			<span className="flex size-9 items-center justify-center rounded-lg bg-primary/10 text-primary">
				{icon}
			</span>
			<span className="font-semibold text-base">{title}</span>
			<span className="text-muted-foreground text-sm leading-relaxed">
				{body}
			</span>
			<span className="mt-auto pt-1 font-medium text-primary text-sm">
				{cta}
			</span>
		</button>
	);
}

/** The one question a new user answers: which tour to walk through, if any. */
export function GuideWelcomeDialog({ open }: Readonly<{ open: boolean }>) {
	const { t } = useTranslation("guide");
	const answerWelcome = useGuideStore((state) => state.answerWelcome);
	const choose = (tour: GuideTourId | null) => () => answerWelcome(tour);

	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				if (!next) answerWelcome(null);
			}}
		>
			<DialogContent className="sm:max-w-xl">
				<DialogHeader>
					<p className="font-medium text-primary text-sm">
						{t("welcome.eyebrow", "Welcome to Flow-Like")}
					</p>
					<DialogTitle className="text-2xl">
						{t("welcome.title", "What brings you here?")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"welcome.body",
							"We'll walk you through the parts you'll actually use, right in the app. It takes about two minutes and you can stop any time.",
						)}
					</DialogDescription>
				</DialogHeader>
				<div className="flex flex-col gap-3 sm:flex-row">
					<Choice
						icon={<BotIcon className="size-5" aria-hidden="true" />}
						title={t("welcome.use.title", "Get things done")}
						body={t(
							"welcome.use.body",
							"Hand tasks to bots and use ready-made apps. No building.",
						)}
						cta={t("welcome.use.cta", "Show me bots and apps")}
						onSelect={choose("use")}
					/>
					<Choice
						icon={<WorkflowIcon className="size-5" aria-hidden="true" />}
						title={t("welcome.build.title", "Build my own")}
						body={t(
							"welcome.build.body",
							"Make apps and automations on the board. Turns on developer mode.",
						)}
						cta={t("welcome.build.cta", "Show me the board")}
						onSelect={choose("build")}
					/>
				</div>
				<div className="flex flex-wrap items-center justify-between gap-2 text-muted-foreground text-sm">
					<span>
						{t(
							"welcome.invite",
							"Joined through an invite? Your team's apps are already in My Apps.",
						)}
					</span>
					<Button variant="ghost" size="sm" onClick={choose(null)}>
						{t("welcome.skip", "I'll look around myself")}
					</Button>
				</div>
			</DialogContent>
		</Dialog>
	);
}
