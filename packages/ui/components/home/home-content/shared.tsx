"use client";

import { ChevronDown, Info, Loader2, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import { useAuth } from "react-oidc-context";
import { getApiOrigin } from "../../../lib/api-url";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";

export function useHomeScope() {
	const backend = useBackend();
	const auth = useAuth();
	return [
		getApiOrigin(backend.profile),
		backend.profile?.id ?? "",
		auth.user?.profile.sub ?? "local",
	];
}

export function HomeEmpty({
	children,
	icon,
	action,
}: { children: ReactNode; icon?: ReactNode; action?: ReactNode }) {
	return (
		<div className="flex min-h-0 w-full items-start gap-3 py-3 text-sm text-muted-foreground">
			{icon && (
				<span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-muted/50 [&>svg]:size-4">
					{icon}
				</span>
			)}
			<div className="min-w-0 space-y-3">
				<div className="max-w-sm text-pretty text-xs leading-relaxed">
					{children}
				</div>
				{action}
			</div>
		</div>
	);
}

export function HomeQueryState({
	loading,
	error,
	retry,
}: { loading: boolean; error: boolean; retry?: () => void }) {
	if (loading)
		return (
			<HomeEmpty icon={<Loader2 className="size-5 animate-spin" />}>
				Loading…
			</HomeEmpty>
		);
	if (error)
		return (
			<HomeEmpty
				action={
					retry && (
						<Button size="sm" variant="outline" onClick={retry}>
							<RefreshCw className="mr-2 size-3.5" />
							Try again
						</Button>
					)
				}
			>
				This content could not load. Check your connection and try again.
			</HomeEmpty>
		);
	return null;
}

export const homeItemClass =
	"group flex min-w-0 items-center gap-3 rounded-xl border border-border/60 bg-background/35 p-3 transition-colors hover:bg-accent/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary";

export const homeRowClass =
	"group flex min-w-0 items-center gap-3 rounded-lg px-2 py-3 transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

export function HomeSourceNote({
	label,
	children,
}: { label: string; children: ReactNode }) {
	return (
		<details className="group/source border-t border-border/50 pt-3 text-[10px] leading-relaxed text-muted-foreground">
			<summary className="flex cursor-pointer list-none items-center gap-1.5 rounded focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
				<Info className="size-3 shrink-0" />
				<span className="min-w-0 flex-1">{label}</span>
				<ChevronDown className="size-3 shrink-0 transition-transform group-open/source:rotate-180" />
			</summary>
			<p className="mt-2 text-pretty">{children}</p>
		</details>
	);
}
