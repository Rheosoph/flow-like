import { FlowPreview } from "@flow-like/flow-like-ui/components/flow/flow-preview";
import type { IComment } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import type { INode } from "@flow-like/flow-like-ui/lib/schema/flow/node";
import { cn } from "@flow-like/flow-like-ui/lib/utils";
import "@xyflow/react/dist/style.css";
import { useEffect, useMemo, useRef, useState } from "react";
import {
	type ShowcaseDriver,
	type TimelineStep,
	useAutoplay,
} from "./use-autoplay";

export interface ShowcaseBoardProps {
	/** URL of a static board graph ({ nodes, edges }), served like /board.json. */
	data: string;
	timeline?: TimelineStep[];
	className?: string;
}

export default function ShowcaseBoard({
	data,
	timeline,
	className,
}: Readonly<ShowcaseBoardProps>) {
	const rootRef = useRef<HTMLDivElement | null>(null);

	const [nodes, setNodes] = useState<INode[]>([]);
	const [comments, setComments] = useState<Record<string, IComment>>({});
	const [caption, setCaption] = useState<string | null>(null);

	useEffect(() => {
		let cancelled = false;
		(async () => {
			try {
				const res = await fetch(data);
				const json = await res.json();
				if (cancelled) return;
				const entries = Array.isArray(json?.nodes) ? json.nodes : [];
				setNodes(
					entries.flatMap((entry: { data?: { node?: INode } } | INode) => {
						if ("data" in entry && entry.data?.node) return [entry.data.node];
						if ("pins" in entry) return [entry];
						return [];
					}),
				);
				setComments(
					Object.fromEntries(
						entries.flatMap((entry: { data?: { comment?: IComment } }) =>
							entry.data?.comment
								? [[entry.data.comment.id, entry.data.comment]]
								: [],
						),
					),
				);
			} catch {
				if (!cancelled) {
					setNodes([]);
					setComments({});
				}
			}
		})();
		return () => {
			cancelled = true;
		};
	}, [data]);

	const driver = useMemo<ShowcaseDriver>(
		() => ({
			board: {
				// FlowPreview owns its safe, read-only ReactFlow instance. Timeline
				// focus steps still provide pacing before their paired callouts.
				focus: () => {},
				fitAll: () => {},
			},
			onCallout: (text) => setCaption(text),
		}),
		[],
	);

	useAutoplay(rootRef, timeline, driver);

	return (
		<div
			ref={rootRef}
			className={cn("relative h-full w-full bg-background", className)}
		>
			<div className="absolute inset-0">
				<FlowPreview nodes={nodes} comments={comments} colorMode="dark" />
			</div>
			{caption && (
				<div className="showcase-callout pointer-events-none absolute left-3 top-3 z-10 rounded-md border border-border/50 bg-background/85 px-2.5 py-1 text-xs font-medium text-foreground/90 shadow-lg backdrop-blur">
					{caption}
				</div>
			)}
		</div>
	);
}
