"use client";

import { useTranslation } from "@flow-like/locales";
import type { TFunction } from "i18next";
import { useRef } from "react";
import type { Announcement } from "../contracts";
import { formatClock, formatTook } from "../run/format";

type InterfacesT = TFunction<"interfaces">;
type Ended = Exclude<Announcement, { kind: "capReached" }>;
type CapReached = Extract<Announcement, { kind: "capReached" }>;

const tookOf = (announcement: Ended) => formatTook(announcement.seconds * 1000);

const doneText = (t: InterfacesT, announcement: Ended) =>
	t("interfaces:workbench.shell.live.done", "Run {{n}} done in {{took}}.", {
		n: announcement.n,
		took: tookOf(announcement),
	});

const failedText = (t: InterfacesT, announcement: Ended) =>
	announcement.step
		? t(
				"interfaces:workbench.shell.live.failedAt",
				"Run {{n}} failed after {{took}} at step {{step}}: {{title}}.",
				{
					n: announcement.n,
					took: tookOf(announcement),
					step: announcement.step.number,
					title: announcement.step.title,
				},
			)
		: t(
				"interfaces:workbench.shell.live.failed",
				"Run {{n}} failed after {{took}}.",
				{ n: announcement.n, took: tookOf(announcement) },
			);

const stoppedText = (t: InterfacesT, announcement: Ended) =>
	t(
		"interfaces:workbench.shell.live.stopped",
		"Run {{n}} stopped at {{clock}}.",
		{
			n: announcement.n,
			clock: formatClock(announcement.seconds * 1000),
		},
	);

const lostText = (t: InterfacesT, announcement: Ended) =>
	t(
		"interfaces:workbench.shell.live.lost",
		"Run {{n}}: the connection ended before it reported a result.",
		{ n: announcement.n },
	);

const capText = (t: InterfacesT, announcement: CapReached) =>
	t(
		"interfaces:workbench.shell.live.capReached",
		"{{count}} runs are going. You can run again when one ends.",
		{
			count: announcement.running,
			defaultValue_one:
				"{{count}} run is going. You can run again when one ends.",
		},
	);

const ENDED_TEXT: Readonly<
	Record<Ended["kind"], (t: InterfacesT, announcement: Ended) => string>
> = {
	done: doneText,
	empty: doneText,
	failed: failedText,
	stopped: stoppedText,
	unknown: lostText,
};

/** What the polite live region says: run ends and a refused zero-field press, never the ticking clock. */
export function announcementText(t: InterfacesT, announcement: Announcement) {
	return announcement.kind === "capReached"
		? capText(t, announcement)
		: ENDED_TEXT[announcement.kind](t, announcement);
}

/**
 * One polite live region for the whole interface. An announcement that was already there when the
 * form mounted is not read again; a new one is, even when its words repeat (its node is replaced).
 */
export function LiveRegion({
	announcement,
}: Readonly<{ announcement: Announcement | null }>) {
	const { t } = useTranslation("interfaces");
	const mountedWith = useRef(announcement?.seq ?? null);
	const fresh =
		announcement && announcement.seq !== mountedWith.current
			? announcement
			: null;
	return (
		<output
			aria-live="polite"
			aria-atomic="true"
			className="sr-only"
			data-fw-live=""
		>
			{fresh ? <span key={fresh.seq}>{announcementText(t, fresh)}</span> : null}
		</output>
	);
}
