"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import {
	LOCAL_SINK_CONSENT_REQUIRED,
	LOCAL_SINK_CONSENT_RESULT,
	type LocalSinkConsentAnswer,
	type LocalSinkConsentRememberScope,
	type LocalSinkConsentRequest,
	type LocalSinkConsentResult,
	saveLocalSinkConsent,
} from "./local-sink-consent";
import { LocalSinkConsentDialog } from "./local-sink-consent-dialog";

/** Pause between two queued requests, so a click cannot answer the next one unseen. */
const SETTLE_MS = 300;

/** Hosts the dialog behind `requestLocalSinkConsent`; requests are answered one at a time. */
export function LocalSinkConsentProvider() {
	const { t } = useTranslation("common");
	const [queue, setQueue] = useState<LocalSinkConsentRequest[]>([]);
	const [settling, setSettling] = useState(false);
	const pending = queue[0] ?? null;

	useEffect(() => {
		const onRequired = (event: Event) => {
			const request = (event as CustomEvent<LocalSinkConsentRequest>).detail;
			setQueue((current) => [...current, request]);
		};
		window.addEventListener(LOCAL_SINK_CONSENT_REQUIRED, onRequired);
		return () =>
			window.removeEventListener(LOCAL_SINK_CONSENT_REQUIRED, onRequired);
	}, []);

	useEffect(() => {
		if (!settling) return;
		const handle = window.setTimeout(() => setSettling(false), SETTLE_MS);
		return () => window.clearTimeout(handle);
	}, [settling]);

	const complete = useCallback(
		(answer: LocalSinkConsentAnswer) => {
			if (!pending) return;
			window.dispatchEvent(
				new CustomEvent<LocalSinkConsentResult>(LOCAL_SINK_CONSENT_RESULT, {
					detail: { answer, requestId: pending.requestId },
				}),
			);
			setQueue((current) => current.slice(1));
			setSettling(true);
		},
		[pending],
	);

	const confirm = useCallback(
		(rememberFor: LocalSinkConsentRememberScope) => {
			if (!pending) return;
			if (rememberFor !== "none") saveLocalSinkConsent(rememberFor, pending);
			complete("allow");
		},
		[complete, pending],
	);

	const decline = useCallback(() => {
		toast.info(
			t(
				"theEventIsSavedWithoutATriggerOnThisDevice",
				"The event is saved without a trigger on this device.",
			),
		);
		complete("decline");
	}, [complete, t]);

	return (
		<LocalSinkConsentDialog
			open={pending !== null && !settling}
			eventName={pending?.eventName}
			eventType={pending?.eventType}
			onDecline={decline}
			onDismiss={() => complete("dismiss")}
			onConfirm={confirm}
		/>
	);
}
