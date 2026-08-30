"use client";

import { Button, Card, CardContent, CardHeader } from "@flow-like/flow-like-ui";
import { i18n as i18next } from "@flow-like/locales";
import { PartyPopper } from "lucide-react";
import { useRouter } from "next/navigation";
import Crossfire from "react-canvas-confetti/dist/presets/crossfire";

const CongratsHeader = () => (
	<CardHeader className="text-center space-y-2">
		<h2 className="text-2xl sm:text-3xl font-semibold">
			{i18next.t("congratulations", "🎉 Congratulations!")}
		</h2>
		<p className="text-muted-foreground">
			{i18next.t(
				"youHaveSuccessfullyCompletedTheOnboardingProcess",
				"You have successfully completed the onboarding process.",
			)}
		</p>
	</CardHeader>
);

const FinishSetupButton: React.FC<{ onFinish: () => void }> = ({
	onFinish,
}) => (
	<Button className="gap-2 w-full mt-6" onClick={onFinish}>
		<PartyPopper className="h-4 w-4" aria-hidden="true" />
		{i18next.t("finishSetup", "Finish Setup")}
	</Button>
);

export default function DonePage() {
	const router = useRouter();

	return (
		<main className="relative min-h-dvh w-full overflow-hidden z-10">
			<div className="relative z-10 flex min-h-dvh flex-col items-center justify-center py-8 sm:py-12">
				<Crossfire autorun={{ speed: 1 }} />
				<Card className="w-full max-w-md sm:max-w-lg md:max-w-2xl">
					<CardContent className="pt-6">
						<CongratsHeader />
						<FinishSetupButton onFinish={() => router.push("/library")} />
					</CardContent>
				</Card>
			</div>
		</main>
	);
}
