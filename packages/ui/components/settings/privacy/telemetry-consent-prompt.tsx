"use client";

import { useTranslation } from "@flow-like/locales";
import Link from "next/link";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardFooter,
	CardHeader,
	CardTitle,
} from "../../ui/card";

export interface TelemetryConsentPromptProps {
	onDecision: (enabled: boolean) => void;
	privacyHref: string;
}

export function TelemetryConsentPrompt({
	onDecision,
	privacyHref,
}: Readonly<TelemetryConsentPromptProps>) {
	const { t } = useTranslation("settings");
	return (
		<Card className="fixed bottom-4 right-4 z-50 max-w-sm shadow-lg">
			<CardHeader>
				<CardTitle className="text-base">{t('helpImproveFlowlike', 'Help improve Flow-Like')}</CardTitle>
				<CardDescription>
					{t('shareAnonymousOptinUsageCountersMdashNeverPromptsBoardContentOrPersonalDataAnonymousCrashReportsAreASeparateSettingThatIsOnByDefaultAndCanBeTurnedOffAnyTime', "Share anonymous, opt-in usage counters — never prompts, board content, or personal data. Anonymous crash reports are a separate setting that is on by default and can be turned off any time.")}
				</CardDescription>
			</CardHeader>
			<CardContent className="flex flex-wrap items-center gap-2">
				<Button size="sm" onClick={() => onDecision(true)}>
					{t('shareAnonymousUsage', 'Share anonymous usage')}
				</Button>
				<Button size="sm" variant="outline" onClick={() => onDecision(false)}>
					{t('noThanks', 'No thanks')}
				</Button>
			</CardContent>
			<CardFooter>
				<Link
					href={privacyHref}
					className="text-sm text-muted-foreground underline underline-offset-4 hover:text-foreground"
				>
					{t('learnMore', 'Learn more')}
				</Link>
			</CardFooter>
		</Card>
	);
}
