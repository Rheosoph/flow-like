"use client";

import { useTranslation } from "@flow-like/locales";
import { Code, History, Plus } from "lucide-react";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import { TEMPLATE_LANGUAGES } from "../../../../lib/schema/developer";
import { Button } from "../../../ui/button";

const RECENT_KEY = "flow-like.explore.recent-searches";
const MAX_RECENT = 5;
const SCAFFOLD_HREF = "/developer/new";

function readRecent(): string[] {
	try {
		const raw = globalThis.localStorage?.getItem(RECENT_KEY);
		const parsed: unknown = raw ? JSON.parse(raw) : [];
		return Array.isArray(parsed)
			? parsed.filter((entry): entry is string => typeof entry === "string")
			: [];
	} catch {
		return [];
	}
}

function writeRecent(entries: string[]) {
	try {
		globalThis.localStorage?.setItem(RECENT_KEY, JSON.stringify(entries));
	} catch {}
}

/** Stores an explicitly submitted query (not every debounced keystroke) first in the recent list. */
export function rememberRecentSearch(query: string | undefined): string[] {
	const trimmed = query?.trim();
	const current = readRecent();
	if (!trimmed) return current;
	const next = [
		trimmed,
		...current.filter((entry) => entry.toLowerCase() !== trimmed.toLowerCase()),
	].slice(0, MAX_RECENT);
	writeRecent(next);
	return next;
}

/** The last few submitted queries, per browser; a throwing or missing localStorage just keeps none. */
export function useRecentSearches() {
	const [recent, setRecent] = useState<string[]>([]);

	useEffect(() => {
		setRecent(readRecent());
	}, []);

	const remember = useCallback((query: string | undefined) => {
		setRecent(rememberRecentSearch(query));
	}, []);

	return { recent, remember };
}

export function RecentSearches({
	recent,
	current,
	onPick,
}: Readonly<{
	recent: readonly string[];
	current?: string;
	onPick: (query: string) => void;
}>) {
	const { t } = useTranslation("store");
	const shown = recent.filter((entry) => entry !== current).slice(0, 3);
	if (!shown.length) return null;
	return (
		<div className="flex min-w-0 items-center gap-2 text-[13px] text-muted-foreground">
			<History aria-hidden="true" className="size-3.75 shrink-0" />
			<span className="shrink-0">{t("exploreRecentSearches", "Recent:")}</span>
			<div className="flex min-w-0 gap-1.5 overflow-hidden">
				{shown.map((entry) => (
					<button
						key={entry}
						type="button"
						onClick={() => onPick(entry)}
						className="h-7 max-w-40 shrink-0 truncate rounded-full border border-border px-2.5 text-[13px] text-foreground transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					>
						{entry}
					</button>
				))}
			</div>
		</div>
	);
}

/** Dev + desktop only: start a node package from a template when nothing in the store fits. */
export function BuildItYourselfCard() {
	const { t } = useTranslation("store");
	return (
		<aside
			aria-labelledby="explore-build-it-yourself"
			className="relative flex flex-col gap-3 overflow-hidden rounded-2xl border border-dashed border-border bg-card/60 p-5"
		>
			<div
				aria-hidden="true"
				className="pointer-events-none absolute inset-0 bg-[radial-gradient(var(--border)_0.5px,transparent_0.5px)] bg-size-[7px_7px] opacity-50"
			/>
			<div className="relative flex items-center justify-between gap-3">
				<h2
					id="explore-build-it-yourself"
					className="flex items-center gap-2 text-sm font-semibold"
				>
					<Code aria-hidden="true" className="size-4 text-primary" />
					{t("exploreBuildItYourself", "Build it yourself")}
				</h2>
				<Button asChild variant="outline" size="sm" className="rounded-lg">
					<Link href={SCAFFOLD_HREF}>
						<Plus aria-hidden="true" className="size-3.5" />
						{t("exploreNewPackage", "New package")}
					</Link>
				</Button>
			</div>
			<p className="relative text-[13px] leading-5 text-muted-foreground">
				{t(
					"exploreBuildItYourselfBody",
					"No package does exactly what you need? Start a node package from a template and ship it to the store.",
				)}
			</p>
			<div className="relative flex flex-wrap gap-1.5">
				{TEMPLATE_LANGUAGES.slice(0, 8).map((language) => (
					<span
						key={language.value}
						className="inline-flex h-6.5 items-center gap-1.5 rounded-full border border-border bg-background/60 pl-0.75 pr-2.5 font-mono text-[11px]"
					>
						<img
							src={language.img}
							alt=""
							className="size-5 rounded-full object-cover"
						/>
						{language.label}
					</span>
				))}
			</div>
		</aside>
	);
}
