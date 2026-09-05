"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronLeft, ChevronRight } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import { cn } from "../../lib/utils";

/** Horizontal snap rail with hover chevrons and edge fades. */
export function ScrollRail({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	const { t } = useTranslation("common");
	const scrollRef = useRef<HTMLDivElement>(null);
	const [canScrollLeft, setCanScrollLeft] = useState(false);
	const [canScrollRight, setCanScrollRight] = useState(false);

	const checkScroll = useCallback(() => {
		const el = scrollRef.current;
		if (!el) return;
		setCanScrollLeft(el.scrollLeft > 1);
		setCanScrollRight(el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
	}, []);

	useEffect(() => {
		const el = scrollRef.current;
		if (!el) return;
		checkScroll();
		el.addEventListener("scroll", checkScroll, { passive: true });
		const ro = new ResizeObserver(checkScroll);
		const observeItems = () => {
			ro.disconnect();
			ro.observe(el);
			for (const child of el.children) ro.observe(child);
			checkScroll();
		};
		// Pagination can increase the content width without resizing the viewport.
		const mutations = new MutationObserver(observeItems);
		mutations.observe(el, { childList: true });
		observeItems();
		return () => {
			el.removeEventListener("scroll", checkScroll);
			mutations.disconnect();
			ro.disconnect();
		};
	}, [checkScroll]);

	const scrollBy = (direction: "left" | "right") => {
		const el = scrollRef.current;
		if (!el) return;
		const amount = el.clientWidth * 0.85;
		el.scrollBy({
			left: direction === "left" ? -amount : amount,
			behavior: "smooth",
		});
	};

	return (
		<div className="group/rail relative">
			<div
				ref={scrollRef}
				className={cn(
					"flex snap-x snap-mandatory gap-4 overflow-x-auto scrollbar-hide pb-1",
					className,
				)}
			>
				{children}
			</div>
			{canScrollLeft && (
				<>
					<div className="pointer-events-none absolute inset-y-0 left-0 w-12 bg-linear-to-r from-background/90 to-transparent" />
					<button
						type="button"
						aria-label={t("scrollLeft", "Scroll left")}
						onClick={() => scrollBy("left")}
						className="absolute left-1 top-1/2 z-10 -translate-y-1/2 rounded-full border border-border/50 bg-background/90 p-2 shadow-md backdrop-blur-sm opacity-100 transition-opacity md:opacity-0 md:group-hover/rail:opacity-100 focus-visible:opacity-100 hover:bg-background"
					>
						<ChevronLeft className="h-4 w-4" />
					</button>
				</>
			)}
			{canScrollRight && (
				<>
					<div className="pointer-events-none absolute inset-y-0 right-0 w-12 bg-linear-to-l from-background/90 to-transparent" />
					<button
						type="button"
						aria-label={t("scrollRight", "Scroll right")}
						onClick={() => scrollBy("right")}
						className="absolute right-1 top-1/2 z-10 -translate-y-1/2 rounded-full border border-border/50 bg-background/90 p-2 shadow-md backdrop-blur-sm opacity-100 transition-opacity md:opacity-0 md:group-hover/rail:opacity-100 focus-visible:opacity-100 hover:bg-background"
					>
						<ChevronRight className="h-4 w-4" />
					</button>
				</>
			)}
		</div>
	);
}
