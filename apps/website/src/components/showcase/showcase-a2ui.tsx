import { A2UIRenderer } from "@flow-like/flow-like-ui/components/a2ui/A2UIRenderer";
import type { Surface } from "@flow-like/flow-like-ui/components/a2ui/types";
import { cn } from "@flow-like/flow-like-ui/lib/utils";
import { useEffect, useMemo, useRef, useState } from "react";
import {
	type ShowcaseDriver,
	type TimelineStep,
	useAutoplay,
} from "./use-autoplay";

interface A2uiDefinition {
	surface: Surface;
	/** Autoplay steps; `setData` steps swap the surface dataModel to animate. */
	timeline?: TimelineStep[];
}

export interface ShowcaseA2uiProps {
	/** URL of an a2ui definition ({ surface, timeline }). */
	data: string;
	className?: string;
}

export default function ShowcaseA2ui({
	data,
	className,
}: Readonly<ShowcaseA2uiProps>) {
	const rootRef = useRef<HTMLDivElement | null>(null);
	const [surface, setSurface] = useState<Surface | null>(null);
	const [timeline, setTimeline] = useState<TimelineStep[] | undefined>();

	useEffect(() => {
		let cancelled = false;
		(async () => {
			try {
				const res = await fetch(data);
				const json: A2uiDefinition = await res.json();
				if (cancelled) return;
				setSurface(json.surface);
				setTimeline(json.timeline);
			} catch {
				/* poster stays visible on failure */
			}
		})();
		return () => {
			cancelled = true;
		};
	}, [data]);

	const driver = useMemo<ShowcaseDriver>(
		() => ({
			data: {
				set: (model) =>
					setSurface((s) =>
						s ? { ...s, dataModel: model as Surface["dataModel"] } : s,
					),
			},
		}),
		[],
	);

	useAutoplay(rootRef, surface ? timeline : undefined, driver);

	return (
		<div
			ref={rootRef}
			className={cn(
				"h-full w-full overflow-auto bg-background text-foreground",
				className,
			)}
		>
			{surface && (
				<A2UIRenderer
					surface={surface}
					isPreviewMode
					className="h-full w-full"
				/>
			)}
		</div>
	);
}
