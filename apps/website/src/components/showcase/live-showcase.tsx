import { cn } from "@flow-like/flow-like-ui/lib/utils";
import { Suspense, lazy, useEffect, useRef, useState } from "react";
import { showcaseBackend } from "./showcase-backend";
import { ShowcaseProviders } from "./showcase-providers";
import type { TimelineStep } from "./use-autoplay";

const ShowcaseChat = lazy(() => import("./showcase-chat"));
const ShowcaseBoard = lazy(() => import("./showcase-board"));
const ShowcaseA2ui = lazy(() => import("./showcase-a2ui"));
const ShowcaseProduct = lazy(() => import("./showcase-product"));

export type ShowcaseVariant =
	| "chat"
	| "board"
	| "a2ui"
	| "runs"
	| "data"
	| "catalog"
	| "prototype"
	| "workflow";

export type ShowcasePresentation = "full" | "embedded" | "compact";

export interface LiveShowcaseIslandProps {
	variant: ShowcaseVariant;
	timeline?: TimelineStep[];
	/** Board variant: URL of a static graph ({ nodes, edges }). */
	data?: string;
	intro?: { role: "user" | "assistant"; content: string }[];
	replyText?: string;
	tools?: string[];
	presentation?: ShowcasePresentation;
	className?: string;
}

/**
 * Client-only island (Plate-based components are not SSR-safe). The heavy
 * variant chunk is deferred until the island nears the viewport, then
 * cross-fades in over the Astro-rendered poster or UI skeleton.
 */
export default function LiveShowcaseIsland({
	variant,
	timeline,
	data,
	intro,
	replyText,
	tools,
	presentation = "full",
	className,
}: Readonly<LiveShowcaseIslandProps>) {
	const rootRef = useRef<HTMLDivElement | null>(null);
	const [show, setShow] = useState(false);
	const [visible, setVisible] = useState(false);

	useEffect(() => {
		const el = rootRef.current;
		if (!el) return;
		const panel = el.closest<HTMLElement>("[role='tabpanel']");
		let nearViewport = false;
		const maybeShow = () => {
			const active = !panel || panel.getAttribute("aria-hidden") !== "true";
			if (nearViewport && active) setShow(true);
		};
		const observer =
			"IntersectionObserver" in window
				? new IntersectionObserver(
						([entry]) => {
							nearViewport = entry.isIntersecting;
							maybeShow();
						},
						{ rootMargin: "300px 0px" },
					)
				: null;
		if (observer) observer.observe(el);
		else {
			nearViewport = true;
			maybeShow();
		}
		const panelObserver = panel
			? new MutationObserver((entries) => {
					if (entries.some((entry) => entry.attributeName === "aria-hidden")) {
						maybeShow();
					}
				})
			: null;
		panelObserver?.observe(panel as HTMLElement, {
			attributes: true,
			attributeFilter: ["aria-hidden"],
		});
		return () => {
			observer?.disconnect();
			panelObserver?.disconnect();
		};
	}, []);

	useEffect(() => {
		if (!show) return;
		const id = requestAnimationFrame(() => setVisible(true));
		return () => cancelAnimationFrame(id);
	}, [show]);

	return (
		<div
			ref={rootRef}
			className={cn(
				"absolute inset-0 transition-opacity duration-500 ease-out",
				visible ? "opacity-100" : "opacity-0",
				className,
			)}
		>
			{show && (
				<Suspense fallback={null}>
					{variant === "chat" && (
						<ShowcaseProviders>
							<ShowcaseChat
								timeline={timeline}
								intro={intro}
								replyText={replyText}
								tools={tools}
							/>
						</ShowcaseProviders>
					)}
					{variant === "board" && data && (
						<ShowcaseProviders query backend={showcaseBackend}>
							<ShowcaseBoard data={data} timeline={timeline} />
						</ShowcaseProviders>
					)}
					{variant === "a2ui" && data && (
						<ShowcaseProviders>
							<ShowcaseA2ui data={data} />
						</ShowcaseProviders>
					)}
					{(variant === "runs" ||
						variant === "data" ||
						variant === "catalog" ||
						variant === "prototype" ||
						variant === "workflow") && (
						<ShowcaseProduct variant={variant} presentation={presentation} />
					)}
				</Suspense>
			)}
		</div>
	);
}
