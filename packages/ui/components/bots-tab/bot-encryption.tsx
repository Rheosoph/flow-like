"use client";

import { useTranslation } from "@flow-like/locales";
import { LockIcon, RefreshCwIcon } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useBackend } from "../../state/backend-state";
import type {
	IBot,
	IBotEncryptionStatus,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";

/** Whether the Details tab shows encryption for this bot. */
export function offersEncryption(bot: IBot): boolean {
	return !bot.local && bot.capabilities.includes("encryption");
}

/** End-to-end chats: the key fingerprint and the host holding the private key. */
export function BotEncryption({ bot }: Readonly<{ bot: IBot }>) {
	const { t } = useTranslation("bots");
	const host = useBackend().botState?.encryption;
	const [status, setStatus] = useState<IBotEncryptionStatus | null>(null);
	const [busy, setBusy] = useState(false);

	useEffect(() => {
		if (!host) return;
		let active = true;
		host
			.status(bot.id)
			.then((next) => active && setStatus(next))
			.catch((cause) => console.warn("Reading the bot's key failed", cause));
		return () => {
			active = false;
		};
	}, [host, bot.id]);

	const rotate = useCallback(async () => {
		if (!host) return;
		setBusy(true);
		try {
			setStatus(await host.rotate(bot.id));
			toast.success(t("panel.encryption.rotated", "New key published"));
		} catch (cause) {
			console.warn("Rotating the bot's key failed", cause);
			toast.error(
				t("panel.encryption.rotateFailed", "The key could not be rotated"),
			);
		} finally {
			setBusy(false);
		}
	}, [host, bot.id, t]);

	let hint = t(
		"panel.encryption.waiting",
		"Waiting for the host to make the key.",
	);
	if (status?.fingerprint) {
		hint = t(
			"panel.encryption.fingerprint",
			"Key fingerprint {{fingerprint}}",
			{
				fingerprint: status.fingerprint,
			},
		);
	}
	return (
		<section
			data-slot="bot-encryption"
			className="flex flex-col gap-2 rounded-xl border bg-card p-3"
		>
			<div className="flex items-start gap-3">
				<span className="grid size-8 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">
					<LockIcon className="size-4" aria-hidden />
				</span>
				<div className="min-w-0 flex-1">
					<p className="text-xs text-muted-foreground">
						{t("panel.encryption.label", "End-to-end encrypted chats")}
					</p>
					<p className="text-sm font-medium">
						{t("panel.encryption.on", "On")}
					</p>
					<p className="mt-0.5 break-all font-mono text-xs text-muted-foreground">
						{hint}
					</p>
					{status && (
						<p className="mt-0.5 text-xs text-muted-foreground">
							{t("panel.encryption.host", "Key held by {{host}}", {
								host: status.host_id,
							})}
						</p>
					)}
					{status && !status.published && (
						<p className="mt-0.5 text-xs text-muted-foreground">
							{t(
								"panel.encryption.unpublished",
								"The key isn't published yet; it is published when the host is online.",
							)}
						</p>
					)}
				</div>
				{host && (
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={busy || !status?.enabled}
						onClick={rotate}
					>
						<RefreshCwIcon aria-hidden />
						{t("panel.encryption.rotate", "Rotate key")}
					</Button>
				)}
			</div>
		</section>
	);
}
