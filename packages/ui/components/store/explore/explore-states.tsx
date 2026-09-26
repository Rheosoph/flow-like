"use client";

import { useTranslation } from "@flow-like/locales";
import { AlertCircle, LogIn, RotateCw, WifiOff } from "lucide-react";
import Link from "next/link";
import { useAuth } from "react-oidc-context";
import { useNetworkStatus } from "../../../hooks/use-network-status";
import { isTransportFailure } from "../../../lib/api-error";
import { isRecord } from "../../../lib/response-shape";
import { Alert, AlertDescription } from "../../ui/alert";
import { Button } from "../../ui/button";
import { EmptyState } from "../../ui/empty-state";
import { Skeleton } from "../../ui/skeleton";
import { useExploreViewer } from "./use-explore";

const NO_HUB_MESSAGE = "Explore needs a hub connection";

export function exploreErrorStatus(error: unknown): number | undefined {
	const status = isRecord(error) ? error.status : undefined;
	return typeof status === "number" ? status : undefined;
}

export function isExploreOffline(error: unknown): boolean {
	if (isTransportFailure(error)) return true;
	const message = isRecord(error) ? error.message : undefined;
	return typeof message === "string" && message.startsWith(NO_HUB_MESSAGE);
}

/** A hub without unauthorized reads answers 401/403 to a signed-out viewer. */
export function needsSignIn(
	error: unknown,
	signedIn: boolean | undefined,
): boolean {
	const status = exploreErrorStatus(error);
	return signedIn === false && (status === 401 || status === 403);
}

export function ExploreSkeleton() {
	return (
		<div aria-hidden="true" className="flex flex-col gap-10">
			<div className="grid grid-cols-1 gap-4 @3xl/explore:grid-cols-6 @5xl/explore:grid-cols-12 @5xl/explore:grid-rows-[176px_204px_92px_92px]">
				<Skeleton className="h-104 rounded-2xl @3xl/explore:col-span-6 @5xl/explore:col-span-7 @5xl/explore:row-span-2 @5xl/explore:h-auto" />
				<Skeleton className="h-44 rounded-2xl @3xl/explore:col-span-3 @5xl/explore:col-span-5 @5xl/explore:h-auto" />
				<Skeleton className="h-44 rounded-2xl @3xl/explore:col-span-3 @5xl/explore:col-span-5 @5xl/explore:h-auto" />
				<Skeleton className="h-50 rounded-2xl @3xl/explore:col-span-6 @5xl/explore:col-span-6 @5xl/explore:row-span-2 @5xl/explore:h-auto" />
				<Skeleton className="h-50 rounded-2xl @3xl/explore:col-span-2 @5xl/explore:col-span-2 @5xl/explore:row-span-2 @5xl/explore:h-auto" />
				<Skeleton className="h-50 rounded-2xl @3xl/explore:col-span-4 @5xl/explore:col-span-4 @5xl/explore:row-span-2 @5xl/explore:h-auto" />
			</div>
			<div className="space-y-4">
				<Skeleton className="h-6 w-48 rounded-full" />
				<div className="flex gap-4 overflow-hidden">
					{[0, 1, 2, 3].map((index) => (
						<Skeleton
							key={index}
							className="h-95 w-64 shrink-0 rounded-xl @5xl/explore:flex-1"
						/>
					))}
				</div>
			</div>
		</div>
	);
}

function ExploreSignIn() {
	const { t } = useTranslation("store");
	const auth = useAuth();
	return (
		<div className="flex flex-col items-center justify-center gap-4 rounded-2xl border border-dashed border-border px-6 py-16 text-center">
			<span className="flex size-12 items-center justify-center rounded-xl bg-muted">
				<LogIn aria-hidden="true" className="size-5 text-muted-foreground" />
			</span>
			<div className="max-w-md space-y-1.5">
				<h2 className="text-lg font-semibold">
					{t("exploreSignInTitle", "Sign in to explore this hub")}
				</h2>
				<p className="text-sm text-muted-foreground">
					{t(
						"exploreSignInBody",
						"This hub only shows its apps and packages to signed-in members.",
					)}
				</p>
			</div>
			{auth?.signinRedirect && (
				<Button
					onClick={() => {
						void auth.signinRedirect({
							url_state:
								typeof window === "undefined"
									? undefined
									: window.location.pathname + window.location.search,
						});
					}}
				>
					<LogIn aria-hidden="true" className="size-4" />
					{t("exploreSignIn", "Sign in")}
				</Button>
			)}
		</div>
	);
}

/**
 * The failure states Explore and Browse share once the hub is known to support Explore: a sign-in prompt, the
 * offline state, or an inline error with Retry. Never a blank page.
 */
export function ExploreQueryError({
	error,
	onRetry,
	retrying,
	fallbackLink,
}: Readonly<{
	error: unknown;
	onRetry: () => void;
	retrying: boolean;
	/** Where to go instead: Browse from the landing, the landing from Browse. */
	fallbackLink: { label: string; href: string };
}>) {
	const { t } = useTranslation("store");
	const { signedIn } = useExploreViewer();
	const online = useNetworkStatus();

	if (needsSignIn(error, signedIn)) return <ExploreSignIn />;
	if (!online || isExploreOffline(error)) {
		return (
			<div className="flex justify-center py-10">
				<EmptyState
					icons={[WifiOff]}
					title={t("exploreOfflineTitle", "You're offline")}
					description={t(
						"exploreOfflineBody",
						"Explore needs a connection to your hub. Apps you already have keep working.",
					)}
					action={{ label: t("retry", "Retry"), onClick: onRetry }}
				/>
			</div>
		);
	}
	return (
		<Alert variant="destructive" className="rounded-xl">
			<AlertCircle className="size-4" />
			<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
				<span>
					{t(
						"exploreLoadFailed",
						"Explore could not be loaded. Please try again.",
					)}
				</span>
				<span className="flex items-center gap-2">
					<Button asChild variant="ghost" size="sm" className="rounded-lg">
						<Link href={fallbackLink.href}>{fallbackLink.label}</Link>
					</Button>
					<Button
						variant="outline"
						size="sm"
						className="rounded-lg"
						disabled={retrying}
						onClick={onRetry}
					>
						<RotateCw
							aria-hidden="true"
							className={retrying ? "size-3.5 animate-spin" : "size-3.5"}
						/>
						{t("retry", "Retry")}
					</Button>
				</span>
			</AlertDescription>
		</Alert>
	);
}
