"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy, Download, SlidersHorizontal } from "lucide-react";
import { useState } from "react";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { type Gate, GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { useCopy } from "../primitives/use-copy";
import { useDeviceWorkspace } from "../workspace";
import { downloadText } from "./observe-data";
import { readerRequestOf } from "./reader-request";
import {
	type HistoryRead,
	type HistoryStream,
	ownRecipient,
} from "./use-history";
import { type ObserveTarget, usePeople } from "./use-observe-target";

/** The owner by name, "the owner" while the name isn't known. */
function useOwnerName(target: ObserveTarget): string {
	const { t } = useTranslation("devices");
	const ownerId = target.view?.row.owner_id;
	const people = usePeople(ownerId ? [ownerId] : []);
	return (
		(ownerId ? people(ownerId) : undefined) ??
		t("observe.access.theOwner", "the owner")
	);
}

/** The readers lists that name this computer's reader key. */
function readingOf(
	streams: readonly HistoryStream[],
	recipientId: string | undefined,
): HistoryStream[] {
	if (!recipientId) return [];
	return streams.filter((stream) =>
		stream.roster?.recipients.some(
			(reader) => reader.recipient_id === recipientId,
		),
	);
}

function statusText(
	t: DevicesT,
	time: ReturnType<typeof useAreaTime>,
	/** Undefined until the readers lists were read from the device. */
	reading: readonly HistoryStream[] | undefined,
	device: string,
	owner: string,
): string {
	if (!reading)
		return t(
			"observe.access.unknown",
			"Whether this computer is a reader of the retained history of {{device}} is read from the device over a live connection. To become one, send {{owner}} a reader request; they approve it on their computer.",
			{ device, owner },
		);
	if (!reading.length)
		return t(
			"observe.access.notReader",
			"You aren't a reader of this device's retained history. Ask {{owner}} to add you: send a reader request, and they approve it on their computer.",
			{ owner },
		);
	const kinds = new Set(
		reading.map((stream) =>
			enumLabel(t, "archiveKind", stream.kind).toLowerCase(),
		),
	);
	const until = Math.min(
		...reading.map((stream) => stream.roster?.expires_at ?? 0),
	);
	return t(
		"observe.access.reader",
		"This computer is a reader of the retained {{kinds}} of {{device}} until {{date}}. To read history on another computer, send {{owner}} a reader request from there.",
		{
			kinds: new Intl.ListFormat(time.locale, { type: "conjunction" }).format([
				...kinds,
			]),
			device,
			date: time.at(until),
			owner,
		},
	);
}

/** A request carries this computer's reader key, so it needs the unlocked keys. */
function requestGate(t: DevicesT, target: ObserveTarget): Gate {
	const device = target.name;
	return target.hasKeys
		? {
				kind: "locked",
				reason: t(
					"observe.access.locked",
					"Unlock {{device}} to create a reader request.",
					{ device },
				),
			}
		: {
				kind: "nokeys",
				reason: t(
					"observe.access.noKeys",
					"This computer has no keys for {{device}}.",
					{ device },
				),
			};
}

/**
 * SPEC §5.2 "History access" (not the owner): whether this computer is a
 * reader, and the request to send the owner to become one.
 */
export function HistoryAccess({
	target,
	history,
}: Readonly<{ target: ObserveTarget; history: HistoryRead }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const { copied, copy } = useCopy();
	const [saved, setSaved] = useState<string | null>(null);
	const { deviceId, name } = target;
	const owner = useOwnerName(target);

	const controller = target.locked
		? undefined
		: workspace.keys.controller(deviceId);
	const mine = controller ? ownRecipient(controller, target.me) : undefined;
	const request = mine
		? JSON.stringify(readerRequestOf(deviceId, mine), null, 2)
		: undefined;
	const file = `reader-request-${deviceId.slice(0, 8)}.json`;
	const reading = history.streams
		? readingOf(history.streams, mine?.recipient_id)
		: undefined;

	return (
		<Block
			id="observe-history-access"
			icon={SlidersHorizontal}
			title={t("observe.access.title", "History access")}
			stamp={
				history.readAt === undefined ? (
					<FreshnessStamp source="live" age="notloaded" />
				) : (
					<FreshnessStamp
						source="live"
						age="live"
						observedAt={history.readAt}
						cadenceSec={60}
					/>
				)
			}
		>
			<p className="max-w-[72ch] text-ui">
				{statusText(t, time, reading, name, owner)}
			</p>
			<div className="flex flex-wrap items-start gap-2">
				<GatedAction gate={request ? null : requestGate(t, target)}>
					<DvButton
						size="sm"
						icon={copied ? Check : Copy}
						onClick={() => {
							if (request) void copy(request);
						}}
					>
						{copied
							? t("observe.access.copied", "Copied")
							: t("observe.access.copy", "Copy reader request")}
					</DvButton>
				</GatedAction>
				{request ? (
					<DvButton
						size="sm"
						icon={Download}
						onClick={() => {
							if (downloadText(file, request, "application/json"))
								setSaved(file);
						}}
					>
						{t("observe.access.download", "Download reader request")}
					</DvButton>
				) : null}
			</div>
			{saved ? (
				<InlineResult tone="good" onDismiss={() => setSaved(null)}>
					{t(
						"observe.access.saved",
						"Saved as {{file}}. It holds no secret: send it to {{owner}}.",
						{ file: saved, owner },
					)}
				</InlineResult>
			) : null}
		</Block>
	);
}
