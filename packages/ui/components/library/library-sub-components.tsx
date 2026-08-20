"use client";

import {
	DndContext,
	type DragEndEvent,
	PointerSensor,
	closestCenter,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import {
	SortableContext,
	rectSortingStrategy,
	useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	ChevronDown,
	ChevronUp,
	Heart,
	Link2,
	Pin,
	Search,
	X,
} from "lucide-react";
import { useRouter } from "next/navigation";
import { useCallback, useRef, useState } from "react";
import { toast } from "sonner";
import { AppCard } from "../ui/app-card";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Skeleton } from "../ui/skeleton";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import type { LibraryItem } from "./library-types";
import {
	CARD_MIN_W_DESKTOP,
	CARD_MIN_W_MOBILE,
	COLLAPSED_ROWS,
} from "./library-types";
import { useGridColumns } from "./use-grid-columns";

export function SortableFavoriteCard({
	item,
	onAppClick,
	settingsHref,
	appHref,
}: Readonly<{
	item: LibraryItem;
	onAppClick: (id: string) => void;
	settingsHref?: (id: string) => string;
	appHref?: (id: string) => string;
}>) {
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({ id: item.id });

	const style = {
		transform: CSS.Transform.toString(transform),
		transition,
		zIndex: isDragging ? 50 : undefined,
		opacity: isDragging ? 0.6 : 1,
	};

	return (
		<div ref={setNodeRef} style={style} {...attributes} {...listeners}>
			<AppCard
				isOwned
				app={item.app}
				metadata={item}
				variant="extended"
				onClick={() => onAppClick(item.id)}
				settingsHref={settingsHref?.(item.id)}
				href={appHref?.(item.id)}
				className="w-full"
			/>
		</div>
	);
}

export function Section({
	title,
	icon,
	items,
	onAppClick,
	settingsHref,
	visibilityMode,
	activeAppIds,
	onToggleVisibility,
	defaultExpanded = false,
	categoryColor,
	isMobile = false,
	showSeeAll = false,
	appHref,
}: Readonly<{
	title: string;
	icon?: React.ReactNode;
	items: LibraryItem[];
	onAppClick: (id: string) => void;
	settingsHref?: (id: string) => string;
	visibilityMode?: boolean;
	activeAppIds?: Set<string>;
	onToggleVisibility?: (id: string) => void;
	defaultExpanded?: boolean;
	categoryColor?: string;
	isMobile?: boolean;
	showSeeAll?: boolean;
	appHref?: (id: string) => string;
}>) {
	const { t } = useTranslation("common");
	const containerRef = useRef<HTMLDivElement>(null);
	const cardMin = isMobile ? CARD_MIN_W_MOBILE : CARD_MIN_W_DESKTOP;
	const cols = useGridColumns(containerRef, cardMin);
	const [expanded, setExpanded] = useState(defaultExpanded);
	const variant = isMobile ? "small" : "extended";

	const collapsedCount = cols * COLLAPSED_ROWS;
	const needsExpand = items.length > collapsedCount;
	const visibleItems = expanded ? items : items.slice(0, collapsedCount);
	const hiddenCount = items.length - collapsedCount;

	if (items.length === 0) return null;

	const handleClick = (id: string) => {
		if (visibilityMode && onToggleVisibility) {
			onToggleVisibility(id);
		} else {
			onAppClick(id);
		}
	};

	if (isMobile) {
		return (
			<section>
				<div className="flex items-center justify-between mb-2">
					<div className="flex items-center gap-2">
						{categoryColor && (
							<span
								className="w-2 h-2 rounded-full shrink-0"
								style={{ backgroundColor: categoryColor, opacity: 0.6 }}
							/>
						)}
						{icon}
						<h2 className="text-base font-bold tracking-tight text-foreground">
							{title}
						</h2>
					</div>
					{showSeeAll && needsExpand && !expanded && (
						<button
							type="button"
							onClick={() => setExpanded(true)}
							className="text-sm font-medium text-primary"
						>
							{t("seeAll", "See All")}
						</button>
					)}
				</div>

				<div className="divide-y divide-border/30">
					{visibleItems.map((meta) => {
						const isActive = activeAppIds?.has(meta.id) ?? true;
						return (
							<div
								key={`${title}-${meta.id}`}
								className={`transition-opacity duration-200 ${
									visibilityMode && !isActive ? "opacity-35" : ""
								}`}
							>
								<AppCard
									isOwned
									app={meta.app}
									metadata={meta}
									variant="small"
									onClick={() => handleClick(meta.id)}
									settingsHref={
										visibilityMode ? undefined : settingsHref?.(meta.id)
									}
									href={!visibilityMode ? appHref?.(meta.id) : undefined}
									className="w-full rounded-none border-0 shadow-none bg-transparent"
								/>
							</div>
						);
					})}
				</div>

				{needsExpand && (
					<div className="flex justify-center mt-2">
						<button
							type="button"
							onClick={() => setExpanded((e) => !e)}
							className="flex items-center gap-1.5 text-xs text-muted-foreground/60 hover:text-foreground px-4 py-1.5 rounded-full border border-border/30 hover:border-border/50 hover:bg-muted/30 transition-colors"
						>
							{expanded ? (
								<>
									{t("less", "Less")} <ChevronUp className="h-3 w-3" />
								</>
							) : (
								<>
									{t("hiddencountMore", "{{hiddenCount}} more", {
										hiddenCount,
									})}
									<ChevronDown className="h-3 w-3" />
								</>
							)}
						</button>
					</div>
				)}
			</section>
		);
	}

	return (
		<section>
			<div className="flex items-center gap-2 mb-3">
				{categoryColor && (
					<span
						className="w-2 h-2 rounded-full shrink-0"
						style={{ backgroundColor: categoryColor, opacity: 0.6 }}
					/>
				)}
				{icon}
				<h2 className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
					{title}
				</h2>
				<span className="text-xs text-muted-foreground/30">{items.length}</span>
			</div>

			<div
				ref={containerRef}
				className="grid gap-3"
				style={{
					gridTemplateColumns: `repeat(auto-fill, minmax(${cardMin}px, 1fr))`,
				}}
			>
				{visibleItems.map((meta) => {
					const isActive = activeAppIds?.has(meta.id) ?? true;
					return (
						<div
							key={`${title}-${meta.id}`}
							className={`transition-all duration-300 ${
								visibilityMode && !isActive ? "opacity-35 hover:opacity-70" : ""
							}`}
						>
							<AppCard
								isOwned
								app={meta.app}
								metadata={meta}
								variant={variant}
								onClick={() => handleClick(meta.id)}
								settingsHref={
									visibilityMode ? undefined : settingsHref?.(meta.id)
								}
								href={!visibilityMode ? appHref?.(meta.id) : undefined}
								className="w-full"
							/>
						</div>
					);
				})}
			</div>

			{needsExpand && (
				<div className="flex justify-center mt-3">
					<button
						type="button"
						onClick={() => setExpanded((e) => !e)}
						className="flex items-center gap-1.5 text-xs text-muted-foreground/60 hover:text-foreground px-4 py-1.5 rounded-full border border-border/30 hover:border-border/50 hover:bg-muted/30 transition-colors"
					>
						{expanded ? (
							<>
								{t("less", "Less")} <ChevronUp className="h-3 w-3" />
							</>
						) : (
							<>
								{t("hiddencountMore", "{{hiddenCount}} more", { hiddenCount })}
								<ChevronDown className="h-3 w-3" />
							</>
						)}
					</button>
				</div>
			)}
		</section>
	);
}

export function FavoritesSection({
	items,
	onAppClick,
	settingsHref,
	onReorder,
	appHref,
	isMobile = false,
}: Readonly<{
	items: LibraryItem[];
	onAppClick: (id: string) => void;
	settingsHref?: (id: string) => string;
	onReorder: (orderedIds: string[]) => void;
	appHref?: (id: string) => string;
	isMobile?: boolean;
}>) {
	const { t } = useTranslation("common");
	const sensors = useSensors(
		useSensor(PointerSensor, {
			activationConstraint: { distance: 8 },
		}),
	);

	const handleDragEnd = useCallback(
		(event: DragEndEvent) => {
			const { active, over } = event;
			if (!over || active.id === over.id) return;

			const oldIndex = items.findIndex((i) => i.id === active.id);
			const newIndex = items.findIndex((i) => i.id === over.id);
			if (oldIndex === -1 || newIndex === -1) return;

			const reordered = [...items];
			const [moved] = reordered.splice(oldIndex, 1);
			reordered.splice(newIndex, 0, moved);

			onReorder(reordered.map((i) => i.id));
		},
		[items, onReorder],
	);

	if (items.length === 0) return null;

	return (
		<section>
			<div className="flex items-center gap-2.5 mb-2 md:mb-4">
				<Heart className="h-3.5 w-3.5 text-primary/70 fill-primary/70" />
				<h2
					className={
						isMobile
							? "text-base font-bold tracking-tight text-foreground"
							: "text-xs font-medium uppercase tracking-widest text-muted-foreground/60"
					}
				>
					{t("favorites", "Favorites")}
				</h2>
				{!isMobile && (
					<span className="text-xs text-muted-foreground/30">
						{items.length}
					</span>
				)}
			</div>

			{isMobile ? (
				<div className="divide-y divide-border/30">
					{items.map((item) => (
						<AppCard
							key={item.id}
							isOwned
							href={appHref?.(item.id)}
							app={item.app}
							metadata={item}
							variant="small"
							onClick={() => onAppClick(item.id)}
							settingsHref={settingsHref?.(item.id)}
							className="w-full rounded-none border-0 shadow-none bg-transparent"
						/>
					))}
				</div>
			) : (
				<DndContext
					sensors={sensors}
					collisionDetection={closestCenter}
					onDragEnd={handleDragEnd}
				>
					<SortableContext
						items={items.map((i) => i.id)}
						strategy={rectSortingStrategy}
					>
						<div
							className="grid gap-3"
							style={{
								gridTemplateColumns: `repeat(auto-fill, minmax(${CARD_MIN_W_MOBILE}px, 1fr))`,
							}}
						>
							{items.map((item) => (
								<SortableFavoriteCard
									key={item.id}
									item={item}
									onAppClick={onAppClick}
									settingsHref={settingsHref}
									appHref={appHref}
								/>
							))}
						</div>
					</SortableContext>
				</DndContext>
			)}
		</section>
	);
}

export function PinnedHero({
	items,
	onAppClick,
	settingsHref,
	appHref,
	isMobile = false,
}: Readonly<{
	items: LibraryItem[];
	onAppClick: (id: string) => void;
	settingsHref?: (id: string) => string;
	appHref?: (id: string) => string;
	isMobile?: boolean;
}>) {
	const { t } = useTranslation("common");
	if (items.length === 0) return null;

	return (
		<section>
			<div className="flex items-center gap-2 mb-2 md:mb-3">
				<Pin className="h-3.5 w-3.5 text-primary/60 fill-primary/60" />
				<h2
					className={
						isMobile
							? "text-base font-bold tracking-tight text-foreground"
							: "text-xs font-medium uppercase tracking-widest text-muted-foreground/60"
					}
				>
					{t("pinned", "Pinned")}
				</h2>
			</div>
			{isMobile ? (
				<div className="divide-y divide-border/30">
					{items.map((meta) => (
						<div key={`pinned-${meta.id}`}>
							<AppCard
								isOwned
								href={appHref?.(meta.id)}
								app={meta.app}
								metadata={meta}
								variant="small"
								onClick={() => onAppClick(meta.id)}
								settingsHref={settingsHref?.(meta.id)}
								className="w-full rounded-none border-0 shadow-none bg-transparent"
							/>
						</div>
					))}
				</div>
			) : (
				<div
					className="grid gap-3"
					style={{
						gridTemplateColumns: `repeat(auto-fill, minmax(${CARD_MIN_W_MOBILE}px, 1fr))`,
					}}
				>
					{items.map((meta) => (
						<div
							key={`pinned-${meta.id}`}
							className="ring-1 ring-primary/10 rounded-xl"
						>
							<AppCard
								isOwned
								href={appHref?.(meta.id)}
								app={meta.app}
								metadata={meta}
								variant="extended"
								onClick={() => onAppClick(meta.id)}
								settingsHref={settingsHref?.(meta.id)}
								className="w-full"
							/>
						</div>
					))}
				</div>
			)}
		</section>
	);
}

export function SearchResults({
	items,
	query,
	onAppClick,
	settingsHref,
	visibilityMode,
	activeAppIds,
	onToggleVisibility,
	appHref,
	isMobile = false,
}: Readonly<{
	items: LibraryItem[];
	query: string;
	onAppClick: (id: string) => void;
	settingsHref?: (id: string) => string;
	visibilityMode?: boolean;
	activeAppIds?: Set<string>;
	onToggleVisibility?: (id: string) => void;
	appHref?: (id: string) => string;
	isMobile?: boolean;
}>) {
	const { t } = useTranslation("common");
	const handleClick = (id: string) => {
		if (visibilityMode && onToggleVisibility) {
			onToggleVisibility(id);
		} else {
			onAppClick(id);
		}
	};

	if (items.length === 0) {
		return (
			<div className="flex flex-col items-center justify-center py-32 text-center">
				<div className="rounded-full bg-muted/30 p-5 mb-5">
					<Search className="h-7 w-7 text-muted-foreground/40" />
				</div>
				<p className="text-sm text-foreground/60 mb-1">
					{t(
						"nothingFoundForLdquoqueryrdquo",
						"Nothing found for “{{query}}”",
						{ query },
					)}
				</p>
				<p className="text-xs text-muted-foreground/60">
					{t("tryDifferentKeywords", "Try different keywords")}
				</p>
			</div>
		);
	}

	return (
		<div>
			<p className="text-xs text-muted-foreground/60 mb-3">
				{t("lengthResult", "{{length}} result", { length: items.length })}
				{items.length !== 1 ? "s" : ""}
			</p>
			{isMobile ? (
				<div className="divide-y divide-border/30">
					{items.map((meta) => {
						const isActive = activeAppIds?.has(meta.id) ?? true;
						return (
							<div
								key={`search-${meta.id}`}
								className={`transition-opacity duration-200 ${
									visibilityMode && !isActive ? "opacity-35" : ""
								}`}
							>
								<AppCard
									isOwned
									href={!visibilityMode ? appHref?.(meta.id) : undefined}
									app={meta.app}
									metadata={meta}
									variant="small"
									onClick={() => handleClick(meta.id)}
									settingsHref={
										visibilityMode ? undefined : settingsHref?.(meta.id)
									}
									className="w-full rounded-none border-0 shadow-none bg-transparent"
								/>
							</div>
						);
					})}
				</div>
			) : (
				<div
					className="grid gap-3"
					style={{
						gridTemplateColumns: `repeat(auto-fill, minmax(${CARD_MIN_W_MOBILE}px, 1fr))`,
					}}
				>
					{items.map((meta) => {
						const isActive = activeAppIds?.has(meta.id) ?? true;
						return (
							<div
								key={`search-${meta.id}`}
								className={`transition-opacity duration-200 ${
									visibilityMode && !isActive
										? "opacity-35 hover:opacity-70"
										: ""
								}`}
							>
								<AppCard
									isOwned
									href={!visibilityMode ? appHref?.(meta.id) : undefined}
									app={meta.app}
									metadata={meta}
									variant="extended"
									onClick={() => handleClick(meta.id)}
									settingsHref={
										visibilityMode ? undefined : settingsHref?.(meta.id)
									}
									className="w-full"
								/>
							</div>
						);
					})}
				</div>
			)}
		</div>
	);
}

export function JoinInline() {
	const { t } = useTranslation("common");
	const router = useRouter();
	const [value, setValue] = useState("");
	const [expanded, setExpanded] = useState(false);
	const inputRef = useRef<HTMLInputElement>(null);

	const handleJoin = useCallback(() => {
		try {
			const url = new URL(value);
			const appId = url.searchParams.get("appId");
			const token = url.searchParams.get("token");
			if (!appId || !token) {
				toast.error("Invalid invite link");
				return;
			}
			router.push(`/join?appId=${appId}&token=${token}`);
			setValue("");
			setExpanded(false);
		} catch {
			toast.error("Invalid invite link");
		}
	}, [value, router]);

	if (!expanded) {
		return (
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						className="h-8 w-8 text-muted-foreground/60 hover:text-foreground"
						onClick={() => {
							setExpanded(true);
							setTimeout(() => inputRef.current?.focus(), 50);
						}}
					>
						<Link2 className="h-4 w-4" />
					</Button>
				</TooltipTrigger>
				<TooltipContent>
					{t("joinWithInviteLink", "Join with invite link")}
				</TooltipContent>
			</Tooltip>
		);
	}

	return (
		<div className="flex items-center gap-1.5 animate-in fade-in-0 slide-in-from-right-4 duration-200">
			<Input
				ref={inputRef}
				placeholder={t("pasteInviteLink", "Paste invite link…")}
				value={value}
				onChange={(e) => setValue(e.target.value)}
				onKeyDown={(e) => {
					if (e.key === "Enter" && value.trim()) handleJoin();
					if (e.key === "Escape") {
						setExpanded(false);
						setValue("");
					}
				}}
				className="h-8 w-48 sm:w-56 text-sm"
			/>
			<Button
				size="sm"
				className="h-8 px-2.5"
				disabled={!value.trim()}
				onClick={handleJoin}
			>
				<ArrowRight className="h-3.5 w-3.5" />
			</Button>
			<Button
				variant="ghost"
				size="sm"
				className="h-8 w-8 p-0 text-muted-foreground"
				onClick={() => {
					setExpanded(false);
					setValue("");
				}}
			>
				<X className="h-3.5 w-3.5" />
			</Button>
		</div>
	);
}

export function JoinInlineExpanded() {
	const { t } = useTranslation("common");
	const router = useRouter();
	const [value, setValue] = useState("");

	const handleJoin = useCallback(() => {
		try {
			const url = new URL(value);
			const appId = url.searchParams.get("appId");
			const token = url.searchParams.get("token");
			if (!appId || !token) {
				toast.error("Invalid invite link");
				return;
			}
			router.push(`/join?appId=${appId}&token=${token}`);
		} catch {
			toast.error("Please paste a valid invite link");
		}
	}, [value, router]);

	return (
		<div className="flex gap-2">
			<Input
				placeholder={t("pasteInviteLink", "Paste invite link…")}
				value={value}
				onChange={(e) => setValue(e.target.value)}
				onKeyDown={(e) => {
					if (e.key === "Enter" && value.trim()) handleJoin();
				}}
				className="flex-1"
			/>
			<Button
				onClick={handleJoin}
				disabled={!value.trim()}
				className="shrink-0"
			>
				<Link2 className="mr-2 h-4 w-4" />
				{t("join", "Join")}
			</Button>
		</div>
	);
}

export function LibrarySkeleton() {
	return (
		<div className="space-y-8 md:space-y-12 px-4 sm:px-8 pt-6">
			<Skeleton className="h-10 w-full max-w-lg rounded-lg" />
			{Array.from({ length: 3 }).map((_, row) => (
				<div key={`skel-row-${row.toString()}`} className="space-y-3">
					<Skeleton className="h-4 w-28 rounded" />
					<div className="hidden md:grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-3">
						{Array.from({ length: 5 }).map((_, i) => (
							<Skeleton
								key={`skel-${row}-${i.toString()}`}
								className="h-72 rounded-xl"
							/>
						))}
					</div>
					<div className="md:hidden space-y-0 divide-y divide-border/20">
						{Array.from({ length: 3 }).map((_, i) => (
							<div
								key={`skel-m-${row}-${i.toString()}`}
								className="flex items-center gap-3 py-3"
							>
								<Skeleton className="h-12 w-12 rounded-xl shrink-0" />
								<div className="flex-1 space-y-1.5">
									<Skeleton className="h-3.5 w-32 rounded" />
									<Skeleton className="h-3 w-48 rounded" />
								</div>
							</div>
						))}
					</div>
				</div>
			))}
		</div>
	);
}
