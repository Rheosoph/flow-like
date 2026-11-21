"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDownIcon, SparklesIcon, XIcon } from "lucide-react";
import { usePathname } from "next/navigation";
import { useCallback, useState } from "react";
import { useClientRouter } from "../../lib/client-navigation";
import {
	type GuideTourId,
	currentStep,
	routeMatches,
	visibleProgress,
} from "../../lib/guide/guide-model";
import { GUIDE_TOURS, guideStepHref } from "../../lib/guide/guide-tours";
import { cn } from "../../lib/utils";
import { useGuideStore } from "../../state/guide-state";
import { Button } from "../ui/button";
import { useGuideCopy } from "./guide-copy";
import { findGuideTarget } from "./guide-spotlight";

export type GuideCardVariant = "dock" | "float";

const ghost =
	"h-8 px-2.5 text-background/80 hover:bg-background/10 hover:text-background";

function Shell({
	variant,
	children,
	label,
}: Readonly<{
	variant: GuideCardVariant;
	children: React.ReactNode;
	label: string;
}>) {
	return (
		<section
			aria-label={label}
			aria-live="polite"
			className={cn(
				"flex flex-col gap-2.5 rounded-xl bg-foreground p-3.5 text-background shadow-xl",
				variant === "float" && "w-[min(20rem,calc(100vw-2rem))]",
			)}
		>
			{children}
		</section>
	);
}

function ActiveCard({ variant }: Readonly<{ variant: GuideCardVariant }>) {
	const { t } = useTranslation("guide");
	const copy = useGuideCopy();
	const pathname = usePathname() ?? "/";
	const router = useClientRouter();
	const run = useGuideStore((state) => state.run);
	const skip = useGuideStore((state) => state.skip);
	const endTour = useGuideStore((state) => state.endTour);
	const [minimized, setMinimized] = useState(false);

	const tour = run ? GUIDE_TOURS[run.tour] : undefined;
	const step = tour && run ? currentStep(tour, run) : undefined;
	const href = guideStepHref(step?.route);
	const away = Boolean(step?.route && !routeMatches(pathname, step.route));

	const doIt = useCallback(() => {
		if (!step) return;
		if (step.act === "next") {
			skip();
			return;
		}
		const element = step.target ? findGuideTarget(step.target) : null;
		if (element) element.click();
		else if (href) router.push(href);
	}, [step, skip, href, router]);

	if (!tour || !run || !step) return null;
	const { index, total } = visibleProgress(tour, run);
	const text = copy.step(run.tour, step.id);
	const progress = t("card.progress", "{{index}} of {{total}}", {
		index,
		total,
	});

	if (minimized) {
		return (
			<button
				type="button"
				onClick={() => setMinimized(false)}
				className="flex items-center gap-2 rounded-full bg-foreground px-3 py-2 font-medium text-background text-xs shadow-lg"
				aria-label={t("card.expand", "Show guide")}
			>
				<SparklesIcon className="size-3.5 text-primary" aria-hidden="true" />
				{t("card.label", "Guide")} · {progress}
			</button>
		);
	}

	const isLast = step.act === "next" && run.step === tour.steps.length - 1;
	return (
		<Shell variant={variant} label={t("card.label", "Guide")}>
			<div className="flex items-center gap-2">
				<span className="flex size-5 items-center justify-center rounded-md bg-primary text-primary-foreground">
					<SparklesIcon className="size-3" aria-hidden="true" />
				</span>
				<span className="font-semibold text-xs">
					{t("card.label", "Guide")}
				</span>
				<span className="ml-auto text-background/70 text-xs tabular-nums">
					{progress}
				</span>
				<button
					type="button"
					onClick={() => setMinimized(true)}
					className="-mr-1 rounded p-1 text-background/70 hover:bg-background/10 hover:text-background"
					aria-label={t("card.minimize", "Minimize guide")}
				>
					<ChevronDownIcon className="size-3.5" aria-hidden="true" />
				</button>
			</div>
			<div className="h-1 overflow-hidden rounded-full bg-background/15">
				<div
					className="h-full rounded-full bg-primary transition-[width] duration-300"
					style={{ width: `${Math.round(((index - 1) / total) * 100)}%` }}
				/>
			</div>
			<p className="font-semibold text-sm leading-snug">{text.title}</p>
			{text.body && (
				<p className="text-background/80 text-xs leading-relaxed">
					{text.body}
				</p>
			)}
			<div className="flex flex-wrap items-center gap-1.5 pt-0.5">
				{away && href ? (
					<Button size="sm" className="h-8" onClick={() => router.push(href)}>
						{t("card.takeMe", "Take me there")}
					</Button>
				) : step.act ? (
					<Button size="sm" className="h-8" onClick={doIt}>
						{step.act === "next"
							? isLast
								? t("card.done", "Done")
								: t("card.next", "Next")
							: t("card.doIt", "Do it for me")}
					</Button>
				) : (
					<Button
						size="sm"
						variant="ghost"
						className={ghost}
						onClick={() => skip()}
					>
						{t("card.skipStep", "Skip step")}
					</Button>
				)}
				<Button
					size="sm"
					variant="ghost"
					className={cn(ghost, "ml-auto")}
					onClick={() => endTour()}
				>
					{t("card.end", "End tour")}
				</Button>
			</div>
		</Shell>
	);
}

function FinishedCard({
	tour,
	variant,
}: Readonly<{ tour: GuideTourId; variant: GuideCardVariant }>) {
	const { t } = useTranslation("guide");
	const startTour = useGuideStore((state) => state.startTour);
	const closeFinished = useGuideStore((state) => state.closeFinished);
	const other: GuideTourId = tour === "use" ? "build" : "use";
	return (
		<Shell variant={variant} label={t("card.label", "Guide")}>
			<div className="flex items-start gap-2">
				<p className="font-semibold text-sm leading-snug">
					{tour === "use"
						? t("finished.use.title", "You're set.")
						: t("finished.build.title", "You built and ran a flow.")}
				</p>
				<button
					type="button"
					onClick={() => closeFinished()}
					className="-mt-0.5 -mr-1 ml-auto rounded p-1 text-background/70 hover:bg-background/10 hover:text-background"
					aria-label={t("finished.close", "Close")}
				>
					<XIcon className="size-3.5" aria-hidden="true" />
				</button>
			</div>
			<p className="text-background/80 text-xs leading-relaxed">
				{tour === "use"
					? t(
							"finished.use.body",
							"You gave a bot a job and found an app. Curious how apps are made?",
						)
					: t(
							"finished.build.body",
							"Next, add a page or a chat to your app, or ask FlowPilot to add steps for you.",
						)}
			</p>
			<div className="flex flex-wrap gap-1.5">
				<Button size="sm" className="h-8" onClick={() => startTour(other)}>
					{other === "build"
						? t("finished.toBuild", "Take the builder tour")
						: t("finished.toUse", "Take the bots and apps tour")}
				</Button>
				<Button
					size="sm"
					variant="ghost"
					className={ghost}
					onClick={() => closeFinished()}
				>
					{t("finished.close", "Close")}
				</Button>
			</div>
		</Shell>
	);
}

/** The guide's card: the active step, or a short wrap-up once a tour is done. */
export function GuideCard({
	variant,
}: Readonly<{ variant: GuideCardVariant }>) {
	const running = useGuideStore((state) => state.run !== null);
	const finished = useGuideStore((state) => state.finished);
	if (running) return <ActiveCard variant={variant} />;
	if (finished) return <FinishedCard tour={finished} variant={variant} />;
	return null;
}
