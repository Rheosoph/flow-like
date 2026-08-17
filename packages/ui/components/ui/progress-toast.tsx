"use client";

import { useTranslation } from "@flow-like/locales";
import { toast } from "sonner";

export interface ProgressToastData {
	id: string;
	message: string;
	progress?: number;
	done: boolean;
	success: boolean;
}

const activeProgressToasts = new Map<string, ProgressToastData>();

function ProgressBarDescription({ progress }: { progress?: number }) {
	const { t } = useTranslation("common");
	const isIndeterminate = progress === undefined;
	const clampedProgress =
		progress !== undefined ? Math.max(0, Math.min(100, progress)) : 0;

	return (
		<div className="mt-1" style={{ width: "100%" }}>
			<div
				className="relative h-1.5 overflow-hidden rounded-full bg-primary/20"
				style={{ width: "100%" }}
			>
				{isIndeterminate ? (
					<div className="absolute h-full w-1/3 animate-[indeterminate_1.5s_ease-in-out_infinite] rounded-full bg-primary" />
				) : (
					<div
						className="h-full rounded-full bg-primary transition-all"
						style={{ width: `${clampedProgress}%` }}
					/>
				)}
			</div>
			{!isIndeterminate && (
				<span className="block text-right text-xs text-muted-foreground mt-1">{`${clampedProgress}%`}</span>
			)}
		</div>
	);
}

export function showProgressToast(data: ProgressToastData): void {
	const { id, message, progress, done, success } = data;

	if (done) {
		activeProgressToasts.delete(id);
		if (success) {
			toast.success(message, { id, duration: 3000 });
		} else {
			toast.error(message, { id, duration: 3000 });
		}
	} else {
		activeProgressToasts.set(id, data);
		toast.loading(message, {
			id,
			duration: Number.POSITIVE_INFINITY,
			description: <ProgressBarDescription progress={progress} />,
		});
	}
}

export function finishAllProgressToasts(success = true): void {
	for (const [id, data] of activeProgressToasts) {
		if (success) {
			toast.success(data.message, { id, duration: 3000 });
		} else {
			toast.dismiss(id);
		}
	}
	activeProgressToasts.clear();
}

export function getActiveProgressToasts(): Map<string, ProgressToastData> {
	return activeProgressToasts;
}
