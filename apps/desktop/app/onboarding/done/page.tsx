"use client";

import {
	type ISettingsProfile,
	guideWelcomeHref,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { Loader2 } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { useTauriInvoke } from "../../../components/useInvoke";

/** Setup is done: refresh the new profiles, then introduce the app. */
export default function DonePage() {
	const { t } = useTranslation("common");
	const router = useRouter();
	const profiles = useTauriInvoke<Record<string, ISettingsProfile>>(
		"get_profiles",
		{},
	);
	const { refetch } = profiles;

	useEffect(() => {
		let cancelled = false;
		void refetch().finally(() => {
			if (!cancelled) router.replace(guideWelcomeHref());
		});
		return () => {
			cancelled = true;
		};
	}, [refetch, router]);

	return (
		<output className="relative z-10 flex min-h-dvh items-center justify-center gap-2 text-muted-foreground">
			<Loader2 className="size-5 animate-spin" aria-hidden="true" />
			{t(
				"onboarding.restoringHint",
				"Getting your workspace ready on this device.",
			)}
		</output>
	);
}
