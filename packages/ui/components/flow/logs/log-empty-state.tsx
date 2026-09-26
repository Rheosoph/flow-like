import { useTranslation } from "@flow-like/locales";
import { ScrollIcon } from "lucide-react";
import { memo } from "react";

function EmptyMessage({
	title,
	detail,
	action,
	onAction,
}: Readonly<{
	title: string;
	detail?: string;
	action?: string;
	onAction?: () => void;
}>) {
	return (
		<div className="flex h-full flex-col items-center justify-center gap-1 px-6 text-center">
			<ScrollIcon aria-hidden className="size-5 text-muted-foreground/60" />
			<p className="text-sm font-medium">{title}</p>
			{detail ? (
				<p className="max-w-md text-xs text-muted-foreground wrap-anywhere">
					{detail}
				</p>
			) : null}
			{action && onAction ? (
				<button
					type="button"
					onClick={onAction}
					className="mt-1 rounded-md border px-2 py-0.5 text-xs hover:bg-secondary"
				>
					{action}
				</button>
			) : null}
		</div>
	);
}

/** Why the list is empty: the load failed, the filters hide everything, or the run logged nothing. */
export const LogEmptyState = memo(function LogEmptyState({
	error,
	filtered,
	onRetry,
	onClearFilters,
}: Readonly<{
	error?: Error;
	filtered: boolean;
	onRetry: () => void;
	onClearFilters: () => void;
}>) {
	const { t } = useTranslation("flow");
	if (error) {
		return (
			<EmptyMessage
				title={t("logViewLoadError", "Couldn't load the logs")}
				detail={error.message}
				action={t("logViewRetry", "Retry")}
				onAction={onRetry}
			/>
		);
	}
	if (filtered) {
		return (
			<EmptyMessage
				title={t("logViewNoMatches", "No logs match these filters")}
				action={t("logViewClearFilters", "Clear filters")}
				onAction={onClearFilters}
			/>
		);
	}
	return (
		<EmptyMessage
			title={t("noLogs", "No Logs")}
			detail={t(
				"noLogsFoundYetStartAnEventToSeeYourResultsHere",
				"No logs found yet, start an event to see your results here!",
			)}
		/>
	);
});
