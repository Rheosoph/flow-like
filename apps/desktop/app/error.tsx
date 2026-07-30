"use client";

import { captureTelemetryError } from "@flow-like/flow-like-ui";
import * as Sentry from "@sentry/nextjs";
import { AlertTriangle, ArrowLeft, RotateCcw } from "lucide-react";
import { useRouter } from "next/navigation";
import { useEffect } from "react";

export default function Error({
	error,
	reset,
}: Readonly<{
	error: Error & { digest?: string };
	reset: () => void;
}>) {
	const router = useRouter();

	useEffect(() => {
		Sentry.captureException(error);
		captureTelemetryError(error, { culprit: "app/error-boundary" });
	}, [error]);

	return (
		<div className="flex flex-col items-center justify-center min-h-[60vh] gap-6 p-6">
			<div className="flex flex-col items-center gap-3 text-center">
				<AlertTriangle className="h-12 w-12 text-destructive" />
				<h2 className="text-2xl font-semibold tracking-tight">
					Something went wrong
				</h2>
				<p className="text-sm text-muted-foreground max-w-md">
					An unexpected error occurred. You can try reloading the page or going
					back.
				</p>
			</div>
			<div className="flex items-center gap-3">
				<button
					type="button"
					onClick={() => router.back()}
					className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-md border border-border bg-background hover:bg-muted transition-colors"
				>
					<ArrowLeft className="h-4 w-4" />
					Go Back
				</button>
				<button
					type="button"
					onClick={reset}
					className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-md bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
				>
					<RotateCcw className="h-4 w-4" />
					Reload
				</button>
			</div>
		</div>
	);
}
