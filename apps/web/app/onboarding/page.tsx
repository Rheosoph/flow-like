"use client";

import { Button } from "@flow-like/flow-like-ui";
import { Trans, useTranslation } from "@flow-like/locales";
import { ArrowRight, Sparkles } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect } from "react";

export default function Onboarding() {
	const { t } = useTranslation("common");
	const router = useRouter();

	useEffect(() => {
		const timer = setTimeout(() => {
			router.push("/library");
		}, 3000);

		return () => clearTimeout(timer);
	}, [router]);

	return (
		<div className="flex flex-col items-center justify-center w-full min-h-screen px-6 py-12">
			<div className="text-center space-y-6 max-w-2xl">
				<div className="space-y-4">
					<div className="w-20 h-20 mx-auto rounded-2xl bg-primary/10 flex items-center justify-center">
						<Sparkles className="w-10 h-10 text-primary" />
					</div>
					<h1 className="text-3xl sm:text-5xl font-bold text-foreground tracking-tight">
						<Trans i18nKey="welcomeToSpanClassnametextprimaryflowlikespan">
							Welcome to <span className="text-primary">Flow-Like</span>
						</Trans>
					</h1>
					<div className="w-24 h-1 mx-auto rounded-full bg-gradient-to-r from-primary to-primary/70" />
				</div>

				<div className="space-y-4">
					<h2 className="text-xl sm:text-2xl text-muted-foreground font-medium">
						{t("yourWorkspaceIsReady", "Your workspace is ready")}
					</h2>
					<p className="text-muted-foreground/80 max-w-lg mx-auto leading-relaxed">
						{t(
							"startByExploringTheStoreToDiscoverAppsOrCreateYourOwnCustomApplicationsAllExecutionHappensInTheCloudNoSetupRequired",
							"Start by exploring the store to discover apps or create your own custom applications. All execution happens in the cloud - no setup required.",
						)}
					</p>
				</div>

				<div className="pt-4">
					<Button
						size="lg"
						onClick={() => router.push("/library")}
						className="gap-2"
					>
						{t("goToLibrary", "Go to Library")}
						<ArrowRight className="w-4 h-4" />
					</Button>
				</div>

				<p className="text-sm text-muted-foreground/60">
					{t("redirectingAutomatically", "Redirecting automatically...")}
				</p>
			</div>
		</div>
	);
}
