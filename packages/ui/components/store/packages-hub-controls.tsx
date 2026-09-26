"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	Loader2,
	LogIn,
	type LucideIcon,
	Search,
	X,
} from "lucide-react";
import { type ComponentProps, useCallback, useRef } from "react";
import { cn } from "../../lib/utils";
import { Alert, AlertDescription } from "../ui/alert";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import type { LibraryAuth } from "./package-library/use-library-packages";

/** Text and dot classes of one state in a hub pill or filter chip. */
export interface HubTone {
	text: string;
	dot: string;
}

/** Sends the viewer to sign-in and back to the hub tab they were on. */
export function useHubSignIn(auth: LibraryAuth) {
	return useCallback(() => {
		void auth?.signinRedirect?.({
			url_state:
				typeof window === "undefined"
					? undefined
					: window.location.pathname + window.location.search,
		});
	}, [auth]);
}

export function HubSearchToolbar({
	label,
	controlsId,
	query,
	onQueryChange,
}: {
	label: string;
	controlsId: string;
	query: string;
	onQueryChange: (query: string) => void;
}) {
	const { t } = useTranslation();
	const searchRef = useRef<HTMLInputElement>(null);
	return (
		<div className="relative min-w-0">
			<Search
				aria-hidden="true"
				className="pointer-events-none absolute left-4 top-1/2 h-5 w-5 -translate-y-1/2 text-muted-foreground"
			/>
			<Input
				ref={searchRef}
				type="search"
				aria-label={label}
				aria-controls={controlsId}
				placeholder={label}
				value={query}
				onChange={(event) => onQueryChange(event.target.value)}
				className="h-12 rounded-xl border-border/60 bg-muted/30 pr-12 pl-12 text-sm shadow-none transition-colors focus-visible:bg-background [&::-webkit-search-cancel-button]:appearance-none"
			/>
			{query && (
				<button
					type="button"
					aria-label={t("clearSearch", "Clear search")}
					onClick={() => {
						onQueryChange("");
						searchRef.current?.focus();
					}}
					className="absolute right-1 top-1/2 flex h-10 w-10 -translate-y-1/2 items-center justify-center rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
				>
					<X aria-hidden="true" className="h-4 w-4" />
				</button>
			)}
		</div>
	);
}

export function HubFilterChips<K extends string>({
	label,
	keys,
	labels,
	counts,
	value,
	onChange,
	dot,
}: {
	label: string;
	keys: readonly K[];
	labels: Record<K, string>;
	counts: Record<K, number>;
	value: K;
	onChange: (value: K) => void;
	dot?: (key: K) => string | null;
}) {
	return (
		<fieldset
			aria-label={label}
			className="m-0 flex min-w-0 flex-wrap items-center gap-1.5 border-0 p-0"
		>
			{keys.map((key) => {
				const active = value === key;
				const dotClass = dot?.(key);
				return (
					<button
						key={key}
						type="button"
						aria-pressed={active}
						onClick={() => onChange(key)}
						className={cn(
							"inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border px-2.5 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
							active
								? "border-border bg-muted text-foreground"
								: "border-border/60 text-muted-foreground hover:text-foreground",
						)}
					>
						{dotClass && (
							<span
								aria-hidden="true"
								className={cn("size-1.5 rounded-full", dotClass)}
							/>
						)}
						{labels[key]}
						<span
							className={cn(
								"font-mono text-[11px] tabular-nums",
								active ? "text-foreground" : "text-muted-foreground",
							)}
						>
							{counts[key]}
						</span>
					</button>
				);
			})}
		</fieldset>
	);
}

export function HubStatePill({
	label,
	tone,
}: { label: string; tone: HubTone }) {
	return (
		<span
			className={cn(
				"inline-flex h-5 max-w-full items-center gap-1.5 rounded-full border border-border/60 bg-background/85 px-2 text-[11px] font-medium backdrop-blur-sm",
				tone.text,
			)}
		>
			<span
				aria-hidden="true"
				className={cn("size-1.5 shrink-0 rounded-full", tone.dot)}
			/>
			<span className="truncate">{label}</span>
		</span>
	);
}

/** Pass a `data-*` marker through `rest`; it lands on the section. */
export function HubSignInPrompt({
	auth,
	icon: Icon,
	title,
	description,
	className,
	...rest
}: {
	auth: LibraryAuth;
	icon: LucideIcon;
	title: string;
	description: string;
} & Omit<ComponentProps<"section">, "title" | "children">) {
	const { t } = useTranslation();
	const signIn = useHubSignIn(auth);
	return (
		<section
			{...rest}
			className={cn(
				"flex flex-col items-center gap-4 rounded-2xl border border-dashed border-border/60 px-6 py-14 text-center",
				className,
			)}
		>
			<span className="grid size-12 place-items-center rounded-xl bg-muted text-muted-foreground">
				<Icon aria-hidden="true" className="size-6" />
			</span>
			<div className="max-w-md space-y-1">
				<h2 className="text-lg font-semibold">{title}</h2>
				<p className="text-sm text-muted-foreground">{description}</p>
			</div>
			<Button onClick={signIn}>
				<LogIn />
				{t("signIn", "Sign in")}
			</Button>
		</section>
	);
}

/** Pass a `data-*` marker through `rest`; it lands on the alert. */
export function HubRetryAlert({
	message,
	onRetry,
	busy = false,
	className,
	...rest
}: {
	message: string;
	onRetry: () => unknown;
	busy?: boolean;
} & Omit<ComponentProps<typeof Alert>, "children" | "variant">) {
	const { t } = useTranslation();
	return (
		<Alert
			{...rest}
			variant="destructive"
			className={cn("rounded-xl", className)}
		>
			<AlertCircle className="h-4 w-4" />
			<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
				{message}
				<Button
					variant="outline"
					size="sm"
					disabled={busy}
					onClick={() => void onRetry()}
				>
					{busy && <Loader2 className="animate-spin" />}
					{t("retry", "Retry")}
				</Button>
			</AlertDescription>
		</Alert>
	);
}

export function HubNoMatches({
	text,
	onShowAll,
}: {
	text: string;
	onShowAll: () => void;
}) {
	const { t } = useTranslation();
	return (
		<div className="flex items-center gap-3 rounded-xl border border-dashed border-border/60 p-4 text-sm text-muted-foreground">
			<Search aria-hidden="true" className="h-4 w-4 shrink-0" />
			<span className="flex-1">{text}</span>
			<Button variant="secondary" size="sm" onClick={onShowAll}>
				{t("showAll", "Show all")}
			</Button>
		</div>
	);
}
