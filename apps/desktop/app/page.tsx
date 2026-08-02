"use client";
import {
	HomeSwimlanes,
	Skeleton,
	TutorialDialog,
	useBackend,
} from "@flow-like/flow-like-ui";
import { HeroSearchBarBubble } from "@flow-like/flow-like-ui/components/global-chat/hero-variants";
import type { ISettingsProfile } from "@flow-like/flow-like-ui/types";
import { usePathname, useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTauriInvoke } from "../components/useInvoke";

export default function Home() {
	const backend = useBackend();
	const router = useRouter();
	const pathname = usePathname();
	const [isCheckingProfiles, setIsCheckingProfiles] = useState(true);
	const profiles = useTauriInvoke<Record<string, ISettingsProfile>>(
		"get_profiles",
		{},
	);

	// `checkProfiles` awaits a refetch; a deep link can navigate away in the meantime and the
	// redirect below would clobber it. Read the live route instead of the render-time one.
	const pathnameRef = useRef(pathname);
	useEffect(() => {
		pathnameRef.current = pathname;
	});

	const goToOnboarding = useCallback(() => {
		if (pathnameRef.current !== "/") return;
		router.replace("/onboarding");
	}, [router]);

	const checkProfiles = useCallback(async () => {
		if (profiles.isLoading) return;

		if (profiles.data) {
			const profileCount = Object.keys(profiles.data).length;

			if (profileCount > 0) {
				setIsCheckingProfiles(false);
				return;
			}

			// Cache may be stale after login sync — refetch before redirecting.
			const { data: fresh } = await profiles.refetch();
			if (fresh && Object.keys(fresh).length > 0) {
				setIsCheckingProfiles(false);
				return;
			}

			goToOnboarding();
			return;
		}

		if (profiles.isError) {
			console.error("Failed to load profiles:", profiles.error);
			goToOnboarding();
		}
	}, [
		profiles.data,
		profiles.isLoading,
		profiles.isError,
		profiles.error,
		goToOnboarding,
	]);

	useEffect(() => {
		checkProfiles();
	}, [checkProfiles]);

	if (profiles.isLoading || isCheckingProfiles) {
		return (
			<main className="flex flex-col flex-1 w-full min-h-0 overflow-hidden">
				<TutorialDialog />
				<div className="flex-1 min-h-0 overflow-auto p-4 grid grid-cols-6 justify-start gap-2">
					<Skeleton className="col-span-6 h-full min-h-[30dvh]" />
					<Skeleton className="col-span-3 h-full min-h-[20dvh]" />
					<Skeleton className="col-span-3 h-full" />
					<Skeleton className="col-span-2 h-full" />
					<Skeleton className="col-span-2 h-full" />
					<Skeleton className="col-span-2 h-full" />
				</div>
			</main>
		);
	}

	return (
		// One page-level scroll: hero + swimlanes move together (the swimlanes are flow content).
		<main className="flex flex-col flex-1 w-full min-h-0 overflow-y-auto">
			<TutorialDialog />
			<HeroSearchBarBubble />
			<HomeSwimlanes />
		</main>
	);
}
