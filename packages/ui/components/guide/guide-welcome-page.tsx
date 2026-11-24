"use client";

import { useRouter, useSearchParams } from "next/navigation";
import { useCallback } from "react";
import type { GuideTourId } from "../../lib/guide/guide-model";
import { useBackend, useSignedIn } from "../../state/backend-state";
import { useGuideStore } from "../../state/guide-state";
import { GuideIntro } from "./intro/guide-intro";

/**
 * The onboarding page every app mounts at `/onboarding/welcome`. Answering
 * starts the chosen tour on Home and marks the account as introduced, so the
 * introduction doesn't come back on another device.
 */
export function GuideWelcomePage() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const answerWelcome = useGuideStore((state) => state.answerWelcome);
	const backend = useBackend();
	const signedIn = useSignedIn();
	const start =
		searchParams?.get("start") === "question" ? "question" : "story";

	const onAnswer = useCallback(
		(tour: GuideTourId | null) => {
			answerWelcome(tour);
			if (signedIn) {
				void Promise.resolve()
					.then(() =>
						backend.userState.updateUser({ tutorial_completed: true }),
					)
					.catch((error) =>
						console.warn("Could not mark the introduction as seen:", error),
					);
			}
			router.replace("/");
		},
		[answerWelcome, backend.userState, router, signedIn],
	);

	return <GuideIntro start={start} onAnswer={onAnswer} />;
}
