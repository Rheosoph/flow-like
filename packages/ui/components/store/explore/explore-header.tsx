"use client";

import { useTranslation } from "@flow-like/locales";
import { CornerDownLeft, Pencil, Search } from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { type ReactNode, type Ref, useState } from "react";
import { rememberRecentSearch } from "./browse/browse-aside";
import { exploreSearchHref, exploreSearchText } from "./explore-href";
import type { ExploreTypeFilter } from "./explore-types";

export const ADMIN_EXPLORE_PATH = "/admin/explore";

/** Title, subtitle, search (Enter opens Browse), the type filter and "Edit layout" for curators. */
export function ExploreHeader({
	dev,
	type,
	typeFilter,
	canEditLayout,
	onRestoreAnnouncement,
	restoreRef,
}: Readonly<{
	/** Package content is on: developer mode and a dev page. */
	dev: boolean;
	type: ExploreTypeFilter;
	typeFilter?: ReactNode;
	canEditLayout: boolean;
	onRestoreAnnouncement?: () => void;
	restoreRef?: Ref<HTMLButtonElement>;
}>) {
	const { t } = useTranslation("store");
	const router = useRouter();
	const [query, setQuery] = useState("");
	const placeholder = dev
		? t("exploreSearchPlaceholderDev", "Search apps and packages")
		: t("exploreSearchPlaceholder", "Search apps");

	return (
		<header className="relative flex flex-col gap-4 @4xl/explore:flex-row @4xl/explore:items-end @4xl/explore:justify-between">
			<div className="flex min-w-0 flex-col gap-0.5">
				<h1 className="text-3xl font-semibold tracking-tight">
					{t("explore", "Explore")}
				</h1>
				<p className="flex flex-wrap items-center gap-x-2 text-[13px] leading-4.5 text-muted-foreground">
					{dev
						? t(
								"exploreSubtitleDev",
								"Apps to use and packages to build with, curated weekly.",
							)
						: t(
								"exploreSubtitle",
								"Apps for you and your team, curated weekly.",
							)}
					{onRestoreAnnouncement && (
						<>
							<span aria-hidden="true">·</span>
							<button
								ref={restoreRef}
								type="button"
								onClick={onRestoreAnnouncement}
								className="rounded-sm text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							>
								{t("exploreRestoreAnnouncement", "Restore announcement")}
							</button>
						</>
					)}
				</p>
			</div>
			<div className="flex min-w-0 flex-wrap items-center gap-2.5">
				<form
					className="relative w-full @2xl/explore:w-85"
					onSubmit={(event) => {
						event.preventDefault();
						const q = exploreSearchText(query);
						rememberRecentSearch(q);
						router.push(
							exploreSearchHref({
								q,
								type: type === "all" ? undefined : type,
							}),
						);
					}}
				>
					<Search
						aria-hidden="true"
						className="pointer-events-none absolute left-3 top-1/2 z-10 size-4 -translate-y-1/2 text-muted-foreground"
					/>
					<input
						type="search"
						value={query}
						onChange={(event) => setQuery(event.target.value)}
						placeholder={placeholder}
						aria-label={t("exploreSearchLabel", "Search Explore")}
						className="h-9.5 w-full rounded-lg border border-border bg-card/80 pl-9 pr-10 text-[13px] backdrop-blur-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-search-cancel-button]:appearance-none"
					/>
					<kbd className="pointer-events-none absolute right-2 top-1/2 inline-flex h-5 -translate-y-1/2 items-center rounded border border-border bg-muted px-1.5 text-muted-foreground">
						<CornerDownLeft className="size-3" />
					</kbd>
				</form>
				{typeFilter}
				{canEditLayout && (
					<Link
						href={ADMIN_EXPLORE_PATH}
						title={t("exploreEditLayoutHint", "Visible to store admins")}
						className="inline-flex h-9.5 items-center gap-2 rounded-lg border border-border bg-card/80 px-3 text-[13px] font-medium text-muted-foreground backdrop-blur-sm transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					>
						<Pencil aria-hidden="true" className="size-3.75" />
						{t("exploreEditLayout", "Edit layout")}
					</Link>
				)}
			</div>
		</header>
	);
}
