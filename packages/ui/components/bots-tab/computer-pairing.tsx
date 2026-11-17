"use client";

import { useTranslation } from "@flow-like/locales";
import { LinkIcon, Loader2Icon, QrCodeIcon, ScanLineIcon } from "lucide-react";
import { type FormEvent, useCallback, useMemo, useState } from "react";
import { PAIR_LINK_PREFIX } from "../../lib/bot-screens/screen-pairing";
import type { BarcodeDetect } from "../interfaces/chat-default/chatbox/barcode-frames";
import { BarcodeScannerDialog } from "../interfaces/chat-default/chatbox/barcode-scanner-dialog";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { browserBarcodeDetect } from "./browser-barcode-detect";

const errorText = (cause: unknown) =>
	cause instanceof Error ? cause.message : String(cause);

/**
 * The stage of a bot whose computer this device has not paired with: scan the QR code the
 * desktop shows or paste its pairing link.
 */
export function ComputerPairingStage({
	name,
	hostLabel,
	detect,
	onPair,
}: Readonly<{
	name: string;
	hostLabel?: string;
	detect?: BarcodeDetect;
	onPair: (link: string) => Promise<void>;
}>) {
	const { t } = useTranslation("bots");
	const scanner = useMemo(() => detect ?? browserBarcodeDetect(), [detect]);
	const [link, setLink] = useState("");
	const [scanning, setScanning] = useState(false);
	const [pending, setPending] = useState(false);
	const [error, setError] = useState<string>();

	const pair = useCallback(
		async (text: string) => {
			setPending(true);
			setError(undefined);
			try {
				await onPair(text);
				setLink("");
			} catch (cause) {
				setError(errorText(cause));
			} finally {
				setPending(false);
			}
		},
		[onPair],
	);

	const onCapture = useCallback(
		({ barcodes }: { barcodes: readonly { text: string }[] }) => {
			setScanning(false);
			const code = barcodes.find((barcode) =>
				barcode.text.trim().startsWith(PAIR_LINK_PREFIX),
			);
			if (code) void pair(code.text);
			else
				setError(
					t(
						"computer.pairing.noCode",
						"No pairing code found in the photo. Show the code the desktop app displays under Watch on another device.",
					),
				);
		},
		[pair, t],
	);

	const onCode = useCallback(
		(text: string) => {
			if (!text.trim().startsWith(PAIR_LINK_PREFIX)) return false;
			setScanning(false);
			void pair(text);
			return true;
		},
		[pair],
	);

	const submit = (event: FormEvent) => {
		event.preventDefault();
		if (link.trim()) void pair(link);
	};

	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 py-10 text-center">
			<span className="flex size-11 items-center justify-center rounded-2xl bg-muted text-muted-foreground [&_svg]:size-5">
				<QrCodeIcon aria-hidden />
			</span>
			<p className="max-w-72 text-sm font-medium">
				{hostLabel
					? t("computer.pairing.title", "Pair this device with {{host}}", {
							host: hostLabel,
						})
					: t(
							"computer.pairing.titleUnknown",
							"Pair this device with the computer that runs {{name}}",
							{ name },
						)}
			</p>
			<p className="max-w-72 text-xs text-muted-foreground">
				{t(
					"computer.pairing.hint",
					"On that computer, open this bot's Computer tab and choose Watch on another device. The screen stays end-to-end encrypted between your devices.",
				)}
			</p>
			{scanner && (
				<Button
					type="button"
					size="sm"
					disabled={pending}
					onClick={() => setScanning(true)}
				>
					<ScanLineIcon className="size-4" aria-hidden />
					{t("computer.pairing.scan", "Scan QR code")}
				</Button>
			)}
			<form className="flex w-full max-w-80 gap-2" onSubmit={submit}>
				<Input
					value={link}
					onChange={(event) => setLink(event.target.value)}
					placeholder={t(
						"computer.pairing.linkPlaceholder",
						"Paste pairing link",
					)}
					aria-label={t("computer.pairing.linkLabel", "Pairing link")}
					className="h-8 text-xs"
				/>
				<Button
					type="submit"
					variant="outline"
					size="sm"
					disabled={pending || link.trim() === ""}
				>
					{pending ? (
						<Loader2Icon className="size-4 animate-spin" aria-hidden />
					) : (
						<LinkIcon className="size-4" aria-hidden />
					)}
					{t("computer.pairing.pair", "Pair")}
				</Button>
			</form>
			{error && (
				<p role="alert" className="max-w-80 text-xs text-destructive">
					{error}
				</p>
			)}
			{scanner && (
				<BarcodeScannerDialog
					open={scanning}
					detect={scanner}
					onCapture={onCapture}
					onCancel={() => setScanning(false)}
					onCode={onCode}
					title={t("computer.pairing.scanTitle", "Scan the pairing code")}
					hint={t(
						"computer.pairing.scanHint",
						"Point the camera at the code under Watch on another device on the computer that runs {{name}}. It pairs as soon as the code is read.",
						{ name },
					)}
				/>
			)}
		</div>
	);
}
