"use client";

import { Loader2, RefreshCw } from "lucide-react";
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
		<div className="flex min-h-24 h-full flex-col items-center justify-center gap-3 p-5 text-center text-sm text-muted-foreground">
			{icon}
			<div className="max-w-sm text-balance">{children}</div>
			{action}
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
				This content could not load. Try again when your connection is
				available.
			</HomeEmpty>
		);
	return null;
}

export const homeItemClass =
	"group flex min-w-0 items-center gap-3 rounded-xl border border-border/60 bg-background/35 p-3 transition-colors hover:bg-accent/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary";
