"use client";

import { usePathname, useSearchParams } from "next/navigation";
import { useEffect } from "react";
import { stopAllFrontendAudio } from "../lib/frontend-audio";

export function FrontendAudioLifecycle({ scope }: { scope: string }) {
	const pathname = usePathname();
	const search = useSearchParams()?.toString() ?? "";
	// Playback belongs to the screen and account that started its Event.
	// biome-ignore lint/correctness/useExhaustiveDependencies: Changes invalidate sounds and pending requests from the previous screen.
	useEffect(() => {
		const hidden = () => {
			if (document.visibilityState !== "visible") stopAllFrontendAudio();
		};
		// Invalidate pending Events even when they have not requested a sound yet.
		document.addEventListener("visibilitychange", hidden);
		window.addEventListener("pagehide", stopAllFrontendAudio);
		window.addEventListener("flow-like:device-inactive", stopAllFrontendAudio);
		return () => {
			document.removeEventListener("visibilitychange", hidden);
			window.removeEventListener("pagehide", stopAllFrontendAudio);
			window.removeEventListener(
				"flow-like:device-inactive",
				stopAllFrontendAudio,
			);
			stopAllFrontendAudio();
		};
	}, [pathname, search, scope]);
	return null;
}
