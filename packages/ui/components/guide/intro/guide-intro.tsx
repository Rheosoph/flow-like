"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowRightIcon, RotateCcwIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import type { GuideTourId } from "../../../lib/guide/guide-model";
import type { GuideWelcomeStart } from "../../../lib/guide/guide-tours";
import { isTauri } from "../../../lib/platform";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";
import {
	AppScene,
	ChatScene,
	ChoiceScene,
	FlowScene,
	type IntroHost,
	IntroRail,
	type IntroStepId,
} from "./guide-intro-scenes";
import "./guide-intro.css";

const STEPS: readonly IntroStepId[] = ["ask", "app", "flow", "choose"];
const CHOOSE = STEPS.length - 1;
const FLOW = STEPS.indexOf("flow");

interface INarrative {
	readonly kicker: string;
	readonly title: string;
	readonly body: string;
	readonly hint?: string;
}

function useNarrative(step: IntroStepId, seenInside: boolean): INarrative {
	const { t } = useTranslation("guide");
	switch (step) {
		case "ask":
			return {
				kicker: t("intro.ask.kicker", "You ask"),
				title: t("intro.ask.title", "It starts with a sentence."),
				body: t(
					"intro.ask.body",
					"Tell a bot what you need, the way you'd tell a colleague. Bots are teammates with a name, a face and a job, and they ask before anything that matters.",
				),
				hint: t("intro.ask.hint", "Try it: press Send, then answer Post."),
			};
		case "app":
			return {
				kicker: t("intro.app.kicker", "The bot uses an app"),
				title: t("intro.app.title", "Post did it with an app."),
				body: t(
					"intro.app.body",
					"An app is a finished tool, here a table of invoices with a summary. Use the ones your team made, take more from Explore, or build your own.",
				),
			};
		case "flow":
			return {
				kicker: t("intro.flow.kicker", "Inside the app"),
				title: t("intro.flow.title", "Inside every app is a flow."),
				body: t(
					"intro.flow.body",
					"The steps, drawn on a board. You never have to open it, but it's always there to read. And it runs where you choose: on this device, on your own servers or in the cloud.",
				),
			};
		case "choose":
			return seenInside
				? {
						kicker: t("intro.choose.kicker", "Your turn"),
						title: t("intro.choose.title", "Where do you want to start?"),
						body: t(
							"intro.choose.body",
							"You've seen it from the top and from the inside. Pick a side; the guide comes along in the real app.",
						),
					}
				: {
						kicker: t("welcome.eyebrow", "Welcome to Flow-Like"),
						title: t("welcome.title", "What brings you here?"),
						body: t(
							"welcome.body",
							"We'll walk you through the parts you'll actually use, right in the app. It takes about two minutes and you can stop any time.",
						),
					};
	}
}

/** Runs where the app runs: this device in the desktop and iPhone apps, else the cloud. */
function useIntroHost(): IntroHost {
	const [host, setHost] = useState<IntroHost>("cloud");
	useEffect(() => setHost(isTauri() ? "device" : "cloud"), []);
	return host;
}

function isTyping(target: EventTarget | null): boolean {
	return (
		target instanceof HTMLElement &&
		target.closest("input, textarea, select, [contenteditable='true']") !== null
	);
}

/**
 * The introduction before the tours: one job told from you, through a bot
 * and an app, down to the flow inside it, ending with the welcome question.
 */
export function GuideIntro({
	start,
	onAnswer,
}: Readonly<{
	start: GuideWelcomeStart;
	onAnswer: (tour: GuideTourId | null) => void;
}>) {
	const { t } = useTranslation("guide");
	const host = useIntroHost();
	const [first, setFirst] = useState(start === "question" ? CHOOSE : 0);
	const [step, setStep] = useState(first);
	const [seenInside, setSeenInside] = useState(false);
	const stepId = STEPS[step];
	const narrative = useNarrative(stepId, seenInside);
	const showRail = step < CHOOSE || seenInside;

	const go = useCallback(
		(next: number) => {
			const clamped = Math.max(first, Math.min(CHOOSE, next));
			if (clamped >= FLOW && clamped < CHOOSE) setSeenInside(true);
			setStep(clamped);
		},
		[first],
	);

	const replay = useCallback(() => {
		setFirst(0);
		setSeenInside(false);
		setStep(0);
	}, []);

	useEffect(() => {
		const onKey = (event: KeyboardEvent) => {
			if (event.altKey || event.metaKey || event.ctrlKey) return;
			if (isTyping(event.target)) return;
			if (event.key === "ArrowRight") go(step + 1);
			else if (event.key === "ArrowLeft") go(step - 1);
			else if (event.key === "Escape") go(CHOOSE);
		};
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [go, step]);

	return (
		<div className="relative z-10 mx-auto flex w-full max-w-6xl flex-col gap-6 py-6 sm:py-10">
			<header className="flex items-center gap-3">
				<img
					src="/app-logo.webp"
					width={30}
					height={30}
					alt=""
					className="rounded-lg"
				/>
				<span className="font-semibold">
					{t("welcome.eyebrow", "Welcome to Flow-Like")}
				</span>
				{step < CHOOSE && (
					<Button
						variant="ghost"
						className="ml-auto"
						onClick={() => go(CHOOSE)}
					>
						{t("intro.skip", "Skip to the question")}
					</Button>
				)}
			</header>

			<div className="flex flex-wrap items-center gap-8 lg:gap-12">
				<section
					className="flex min-w-0 max-w-md flex-[1_1_320px] flex-col gap-4"
					aria-live="polite"
				>
					{first === 0 && (
						<div className="flex max-w-56 gap-1.5" aria-hidden="true">
							{STEPS.map((id, index) => (
								<span
									key={id}
									className={cn(
										"h-1.5 flex-1 rounded-full transition-colors",
										index <= step ? "bg-primary" : "bg-border",
									)}
								/>
							))}
						</div>
					)}
					<span className="font-semibold text-primary text-sm">
						{first === 0
							? t("intro.progress", "{{current}} of {{total}} · {{kicker}}", {
									current: step + 1,
									total: STEPS.length,
									kicker: narrative.kicker,
								})
							: narrative.kicker}
					</span>
					<h1 className="m-0 font-bold text-4xl leading-tight tracking-tight sm:text-5xl">
						{narrative.title}
					</h1>
					<p className="m-0 text-base text-muted-foreground leading-relaxed sm:text-lg">
						{narrative.body}
					</p>
					{narrative.hint && (
						<p className="m-0 text-muted-foreground text-sm">
							{narrative.hint}
						</p>
					)}
					<div className="mt-2 flex flex-wrap gap-2.5">
						{step > first && (
							<Button variant="outline" size="lg" onClick={() => go(step - 1)}>
								{t("intro.back", "Back")}
							</Button>
						)}
						{step < CHOOSE && (
							<Button size="lg" onClick={() => go(step + 1)}>
								{t("intro.next", "Next")}
								<ArrowRightIcon aria-hidden="true" />
							</Button>
						)}
						{first === CHOOSE && (
							<Button variant="ghost" size="lg" onClick={replay}>
								<RotateCcwIcon aria-hidden="true" />
								{t("intro.replay", "Replay intro")}
							</Button>
						)}
					</div>
				</section>

				<section
					className="flex min-w-0 flex-[999_1_520px] flex-col gap-3"
					aria-label={t("intro.stageLabel", "The example")}
				>
					<div className="flex min-h-[30rem] flex-wrap gap-6 rounded-3xl border bg-muted/40 p-5 sm:p-8">
						{showRail && <IntroRail step={stepId} />}
						<div className="flex min-w-0 flex-[1_1_340px] flex-col justify-center gap-4">
							{stepId === "ask" && <ChatScene />}
							{stepId === "app" && <AppScene onLookInside={() => go(FLOW)} />}
							{stepId === "flow" && <FlowScene host={host} />}
							{stepId === "choose" && (
								<ChoiceScene seenInside={seenInside} onAnswer={onAnswer} />
							)}
						</div>
					</div>
					{step < CHOOSE && (
						<p className="m-0 text-muted-foreground text-xs">
							{t(
								"intro.sample",
								"A sample. Nothing here touches your real inbox.",
							)}
						</p>
					)}
				</section>
			</div>
		</div>
	);
}
