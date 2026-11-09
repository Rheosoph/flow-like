"use client";

import { useTranslation } from "@flow-like/locales";
import { MapPinOffIcon } from "lucide-react";
import {
	Component,
	type ComponentProps,
	type ErrorInfo,
	type ReactNode,
	Suspense,
	lazy,
} from "react";
import { cn } from "../../lib/utils";

const MemoryMap = lazy(() => import("./bot-memory-map"));

class MapBoundary extends Component<
	{ fallback: ReactNode; children: ReactNode },
	{ failed: boolean }
> {
	state = { failed: false };

	static getDerivedStateFromError() {
		return { failed: true };
	}

	componentDidCatch(error: Error, info: ErrorInfo) {
		console.warn("[BotMemory] map failed:", error, info.componentStack);
	}

	render() {
		return this.state.failed ? this.props.fallback : this.props.children;
	}
}

/** The memory map, loaded on demand; a map that can't start (no WebGL, no worker) leaves the list intact. */
export function MemoryMapFrame(props: ComponentProps<typeof MemoryMap>) {
	const { t } = useTranslation("bots");
	return (
		<MapBoundary
			fallback={
				<output
					className={cn(
						"flex items-center justify-center gap-2 bg-muted/40 px-4 text-center text-xs text-muted-foreground",
						props.className,
					)}
				>
					<MapPinOffIcon className="size-4 shrink-0" aria-hidden />
					{t("memory.place.unavailable", "The map couldn't load.")}
				</output>
			}
		>
			<Suspense
				fallback={
					<div
						className={cn(
							"bg-muted/40 motion-safe:animate-pulse",
							props.className,
						)}
					/>
				}
			>
				<MemoryMap {...props} />
			</Suspense>
		</MapBoundary>
	);
}
