"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useState } from "react";
import {
	type PendingAsk,
	formatPendingElapsed,
	pendingAskStage,
} from "./pending-ask";

const TICK_MS = 1_000;

function useElapsedSince(since: number) {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		const timer = setInterval(() => setNow(Date.now()), TICK_MS);
		return () => clearInterval(timer);
	}, []);
	return Math.max(0, now - since);
}

/** "· Waiting 4s" beside the question's label until its answer shows a first block. */
export function PendingAskStatus({
	pending,
}: Readonly<{ pending: PendingAsk }>) {
	const { t } = useTranslation("chat");
	const elapsed = useElapsedSince(pending.since);
	const time = formatPendingElapsed(elapsed);
	const labels = {
		sending: t("askSending", "Sending"),
		waiting: t("askWaiting", "Waiting"),
		stillWaiting: t("askStillWaiting", "Still waiting"),
		slow: t("askTakingLonger", "Taking longer than usual"),
	};

	return (
		<span
			className="inline-flex min-w-0 items-baseline gap-1.5 text-[11px] font-normal normal-case tracking-normal text-muted-foreground animate-in fade-in duration-300"
			data-fl-chat-ask-status={pending.phase}
		>
			<span aria-hidden="true">·</span>
			<output className="truncate">
				{labels[pendingAskStage(pending.phase, elapsed)]}
			</output>
			{time && (
				<span aria-hidden="true" className="font-mono tabular-nums">
					{time}
				</span>
			)}
		</span>
	);
}

/** A light running down the question's rule while its answer has not started. */
export function PendingAskRule() {
	return (
		<span
			aria-hidden="true"
			className="pointer-events-none absolute inset-y-0 -left-0.5 w-0.5 bg-linear-to-b from-transparent via-primary-foreground/90 to-transparent bg-size-[100%_2.5rem] bg-no-repeat animate-ask-scan motion-reduce:hidden"
		/>
	);
}
