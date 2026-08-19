"use client";

import { useTranslation } from "@flow-like/locales";
import { captureTelemetryError } from "@flow-like/flow-like-ui/lib/telemetry/errors";
import * as Sentry from "@sentry/nextjs";
import { useEffect } from "react";

export default function GlobalError({
	error,
}: {
	error: Error & { digest?: string };
}) {
	const { t } = useTranslation("common");
	useEffect(() => {
		Sentry.captureException(error);
		captureTelemetryError(error, {
			level: "fatal",
			culprit: "app/global-error-boundary",
		});
	}, [error]);

	return (
		<html lang="en">
			<body>
				<div
					style={{
						display: "flex",
						flexDirection: "column",
						alignItems: "center",
						justifyContent: "center",
						minHeight: "100vh",
						gap: "24px",
						padding: "24px",
						fontFamily: "system-ui, sans-serif",
						background: "#0a0a0a",
						color: "#fafafa",
					}}
				>
					<h2 style={{ fontSize: "1.5rem", fontWeight: 600 }}>
						{t('somethingWentWrong', 'Something went wrong')}
					</h2>
					<p style={{ fontSize: "0.875rem", color: "#a1a1aa" }}>
						{t('anUnexpectedErrorOccurred', 'An unexpected error occurred.')}
					</p>
					<div style={{ display: "flex", gap: "12px" }}>
						<button
							type="button"
							onClick={() => window.history.back()}
							style={{
								padding: "8px 16px",
								fontSize: "0.875rem",
								borderRadius: "6px",
								border: "1px solid #27272a",
								background: "transparent",
								color: "#fafafa",
								cursor: "pointer",
							}}
						>
							{t('goBack', 'Go Back')}
						</button>
						<button
							type="button"
							onClick={() => window.location.reload()}
							style={{
								padding: "8px 16px",
								fontSize: "0.875rem",
								borderRadius: "6px",
								border: "none",
								background: "#fafafa",
								color: "#0a0a0a",
								cursor: "pointer",
							}}
						>
							{t('reload', 'Reload')}
						</button>
					</div>
				</div>
			</body>
		</html>
	);
}
