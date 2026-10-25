import { useTranslation } from "@flow-like/locales";
import { OctagonAlertIcon } from "lucide-react";
import { memo } from "react";
import { cn } from "../../lib/utils";

/**
 * The run's stored failure reason, unless the first error in its logs already
 * says the same — a rejected trigger logs its reason as that error.
 */
export function runErrorToShow(
	errorMessage: string | null | undefined,
	firstErrorMessage?: string | null,
): string | undefined {
	const reason = errorMessage?.trim();
	if (!reason) return undefined;
	return reason === firstErrorMessage?.trim() ? undefined : reason;
}

/** One truncated line for run lists; the full reason is its tooltip. */
export const RunErrorLine = memo(function RunErrorLine({
	message,
	className,
}: Readonly<{ message: string; className?: string }>) {
	return (
		<span
			title={message}
			className={cn(
				"flex min-w-0 max-w-full items-center gap-1 text-left text-xs text-destructive",
				className,
			)}
		>
			<OctagonAlertIcon aria-hidden className="size-3 shrink-0" />
			<span className="truncate">{message}</span>
		</span>
	);
});

/** The reason block above a run's logs, which may be all there is to read. */
export const RunErrorBanner = memo(function RunErrorBanner({
	message,
}: Readonly<{ message: string }>) {
	const { t } = useTranslation("flow");
	return (
		<section
			aria-label={t("runErrorReason", "Why this run failed")}
			className="shrink-0 border-b bg-destructive/5 py-1.5 pr-2 pl-3"
		>
			<div className="flex min-w-0 items-start gap-2">
				<OctagonAlertIcon
					aria-hidden
					className="mt-0.5 size-3.5 shrink-0 text-destructive"
				/>
				<div className="flex min-w-0 flex-1 flex-col gap-0.5">
					<span className="text-xs font-bold text-destructive">
						{t("runErrorReason", "Why this run failed")}
					</span>
					<p className="max-h-24 select-text overflow-y-auto whitespace-pre-wrap wrap-anywhere font-mono text-xs text-foreground">
						{message}
					</p>
				</div>
			</div>
		</section>
	);
});
