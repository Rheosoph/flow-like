"use client";

import { type Event, type UnlistenFn, listen } from "@tauri-apps/api/event";
import type { IIntercomEvent } from "@flow-like/flow-like-ui";
import {
	type ProgressToastData,
	finishAllProgressToasts,
	showProgressToast,
} from "@flow-like/flow-like-ui";
import { useEffect } from "react";
import { toast } from "sonner";

interface IToastEvent {
	message: string;
	level: "success" | "error" | "info" | "warning";
}

export default function ToastProvider() {
	useEffect(() => {
		const subscriptions: (Promise<UnlistenFn> | undefined)[] = [];

		const unlistenToast = listen("toast", (events: Event<IIntercomEvent[]>) => {
			const messages: IToastEvent[] = events.payload.map(
				(event) => event.payload,
			);
			for (const message of messages) {
				if (message.level === "success") {
					toast.success(message.message);
				} else if (message.level === "error") {
					toast.error(message.message);
				} else if (message.level === "warning") {
					toast.warning(message.message);
				} else {
					toast.info(message.message);
				}
			}
		});

		const unlistenProgress = listen(
			"progress",
			(events: Event<IIntercomEvent[]>) => {
				const progressEvents: ProgressToastData[] = events.payload.map(
					(event) => event.payload,
				);
				for (const event of progressEvents) {
					showProgressToast(event);
				}
			},
		);

		const unlistenCompleted = listen(
			"completed",
			(_events: Event<IIntercomEvent[]>) => {
				finishAllProgressToasts(true);
			},
		);

		const unlistenError = listen(
			"error",
			(_events: Event<IIntercomEvent[]>) => {
				finishAllProgressToasts(false);
			},
		);

		subscriptions.push(
			unlistenToast,
			unlistenProgress,
			unlistenCompleted,
			unlistenError,
		);

		return () => {
			(async () => {
				for await (const subscription of subscriptions) {
					if (subscription) subscription();
				}
			})();
		};
	}, []);

	return null;
}
