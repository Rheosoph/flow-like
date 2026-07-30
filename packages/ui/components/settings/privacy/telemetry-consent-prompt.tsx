"use client";

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
	return (
		<Card className="fixed bottom-4 right-4 z-50 max-w-sm shadow-lg">
			<CardHeader>
				<CardTitle className="text-base">Help improve Flow-Like</CardTitle>
				<CardDescription>
					Share anonymous, opt-in usage counters &mdash; never prompts, board
					content, or personal data. Anonymous crash reports are a separate
					setting that is on by default and can be turned off any time.
				</CardDescription>
			</CardHeader>
			<CardContent className="flex flex-wrap items-center gap-2">
				<Button size="sm" onClick={() => onDecision(true)}>
					Share anonymous usage
				</Button>
				<Button size="sm" variant="outline" onClick={() => onDecision(false)}>
					No thanks
				</Button>
			</CardContent>
			<CardFooter>
				<Link
					href={privacyHref}
					className="text-sm text-muted-foreground underline underline-offset-4 hover:text-foreground"
				>
					Learn more
				</Link>
			</CardFooter>
		</Card>
	);
}
