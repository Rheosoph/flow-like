"use client";

import { AppWindow, ExternalLink, Pause } from "lucide-react";
import dynamic from "next/dynamic";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
	type HomeContentProps,
	type HomeEmbedTarget,
	homeEmbedHref,
	mergeHomeEmbedNavigation,
	parseHomeEmbedTarget,
	textConfig,
} from "./config";
import { HomeEmpty } from "./shared";

const Runtime = dynamic(() => import("./app-embed-runtime"), {
	ssr: false,
	loading: () => (
		<div className="p-4">
			<HomeEmpty>Loading app…</HomeEmpty>
		</div>
	),
});

export function HomeAppEmbed({ widget, editing }: HomeContentProps) {
	const initial = useMemo(
		() => parseHomeEmbedTarget(widget.config),
		[widget.config],
	);
	const configKey = JSON.stringify(initial);
	if (
		initial.appId &&
		textConfig(widget.config, "target") === "event" &&
		!initial.eventId
	) {
		return (
			<div className="p-4">
				<HomeEmpty icon={<AppWindow className="size-8 opacity-50" />}>
					Choose a chat or another app interface in widget settings.
				</HomeEmpty>
			</div>
		);
	}
	return (
		<HomeAppEmbedInstance
			key={configKey}
			title={widget.title}
			initial={initial}
			editing={editing}
		/>
	);
}

function HomeAppEmbedInstance({
	initial,
	editing,
	title,
}: { initial: HomeEmbedTarget; editing?: boolean; title?: string }) {
	const [target, setTarget] = useState(initial);
	const host = useRef<HTMLDivElement>(null);
	const [visible, setVisible] = useState(false);
	const navigate = useCallback(
		(next: Parameters<typeof mergeHomeEmbedNavigation>[1]) =>
			setTarget((current) => mergeHomeEmbedNavigation(current, next)),
		[],
	);
	useEffect(() => {
		const element = host.current;
		if (!element || typeof IntersectionObserver === "undefined") {
			setVisible(true);
			return;
		}
		const observer = new IntersectionObserver(([entry]) =>
			setVisible(entry.isIntersecting),
		);
		observer.observe(element);
		return () => observer.disconnect();
	}, []);
	if (!target.appId)
		return (
			<div className="p-4">
				<HomeEmpty icon={<AppWindow className="size-8 opacity-50" />}>
					Choose an app in widget settings, then select its landing page, a
					route, or a chat.
				</HomeEmpty>
			</div>
		);
	return (
		<div ref={host} className="flex h-full min-h-0 min-w-0 flex-col">
			<div className="flex min-h-12 shrink-0 items-center justify-between gap-3 border-b border-border/60 bg-muted/15 px-4 py-2 text-xs">
				<div className="flex min-w-0 items-center gap-2.5">
					<AppWindow className="size-4 shrink-0 text-[var(--home-accent,var(--primary))]" />
					<span className="truncate font-medium">{title || "App"}</span>
					<span className="truncate text-[11px] text-muted-foreground">
						{target.eventId ? "App interface" : target.routePath}
					</span>
				</div>
				<Link
					href={homeEmbedHref(target)}
					className="flex shrink-0 items-center gap-1.5 rounded-md px-2 py-1.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					Open app
					<ExternalLink className="size-3.5" />
				</Link>
			</div>
			{editing ? (
				<div className="p-4">
					<HomeEmpty icon={<Pause className="size-7 opacity-50" />}>
						App preview pauses while you edit your home.
					</HomeEmpty>
				</div>
			) : (
				<Runtime target={target} active={visible} onNavigate={navigate} />
			)}
		</div>
	);
}
