"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CopyIcon,
	Loader2Icon,
	RefreshCwIcon,
	SmartphoneIcon,
	Trash2Icon,
} from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { useCallback, useEffect, useState } from "react";
import type {
	IBotPairedScreenDevice,
	IBotScreenPairing,
	IBotScreenPairingCode,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "../ui/dialog";

const errorText = (cause: unknown) =>
	cause instanceof Error ? cause.message : String(cause);

function useSecondsLeft(expiresAt: number | undefined): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		if (expiresAt === undefined) return;
		setNow(Date.now());
		const timer = setInterval(() => setNow(Date.now()), 1000);
		return () => clearInterval(timer);
	}, [expiresAt]);
	return expiresAt === undefined
		? 0
		: Math.max(0, Math.ceil(expiresAt - now / 1000));
}

function PairedDevices({
	devices,
	onRemove,
}: Readonly<{
	devices: IBotPairedScreenDevice[];
	onRemove: (keyId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	if (devices.length === 0)
		return (
			<p className="text-xs text-muted-foreground">
				{t("computer.pairing.noDevices", "No device is paired yet.")}
			</p>
		);
	return (
		<ul className="flex flex-col gap-1.5">
			{devices.map((device) => (
				<li
					key={device.key_id}
					className="flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-xs"
				>
					<SmartphoneIcon
						className="size-3.5 shrink-0 text-muted-foreground"
						aria-hidden
					/>
					<span className="min-w-0 flex-1">
						<span className="block truncate font-medium">{device.label}</span>
						<span className="block truncate font-mono text-[11px] text-muted-foreground">
							{device.fingerprint}
						</span>
					</span>
					<Button
						type="button"
						variant="ghost"
						size="icon"
						className="size-7"
						aria-label={t("computer.pairing.remove", "Remove {{label}}", {
							label: device.label,
						})}
						onClick={() => onRemove(device.key_id)}
					>
						<Trash2Icon className="size-3.5" aria-hidden />
					</Button>
				</li>
			))}
		</ul>
	);
}

function PairingBody({ pairing }: Readonly<{ pairing: IBotScreenPairing }>) {
	const { t } = useTranslation("bots");
	const [code, setCode] = useState<IBotScreenPairingCode>();
	const [devices, setDevices] = useState<IBotPairedScreenDevice[]>([]);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string>();
	const [notice, setNotice] = useState<string>();
	const secondsLeft = useSecondsLeft(code?.expires_at);

	const refreshDevices = useCallback(async () => {
		try {
			setDevices(await pairing.list());
		} catch (cause) {
			setError(errorText(cause));
		}
	}, [pairing]);

	const newCode = useCallback(async () => {
		setLoading(true);
		setError(undefined);
		try {
			setCode(await pairing.start());
		} catch (cause) {
			setCode(undefined);
			setError(errorText(cause));
		} finally {
			setLoading(false);
		}
	}, [pairing]);

	useEffect(() => {
		void newCode();
		void refreshDevices();
		return pairing.onPaired((device) => {
			setNotice(
				t("computer.pairing.paired", "{{label}} is paired.", {
					label: device.label,
				}),
			);
			void refreshDevices();
			void newCode();
		});
	}, [pairing, newCode, refreshDevices, t]);

	const copy = async () => {
		if (!code) return;
		try {
			await navigator.clipboard.writeText(code.link);
			setNotice(t("computer.pairing.copied", "Pairing link copied."));
		} catch (cause) {
			setError(errorText(cause));
		}
	};

	const remove = async (keyId: string) => {
		try {
			await pairing.remove(keyId);
			await refreshDevices();
		} catch (cause) {
			setError(errorText(cause));
		}
	};

	const expired = code !== undefined && secondsLeft === 0;
	return (
		<div className="flex flex-col gap-4">
			<div className="flex flex-col items-center gap-2">
				{code && !expired ? (
					<div className="rounded-lg border bg-white p-3 text-black">
						<QRCodeSVG
							value={code.link}
							size={176}
							bgColor="transparent"
							fgColor="currentColor"
							role="img"
							aria-label={t(
								"computer.pairing.qrLabel",
								"Pairing code for {{host}}",
								{ host: code.host_label },
							)}
						/>
					</div>
				) : (
					<div className="flex size-[200px] items-center justify-center rounded-lg border bg-muted/40">
						{loading && (
							<Loader2Icon className="size-5 animate-spin" aria-hidden />
						)}
					</div>
				)}
				{code && !expired && (
					<p className="text-xs text-muted-foreground">
						{t(
							"computer.pairing.expiresIn",
							"Code valid for {{minutes}}:{{seconds}}",
							{
								minutes: Math.floor(secondsLeft / 60),
								seconds: String(secondsLeft % 60).padStart(2, "0"),
							},
						)}
					</p>
				)}
				{expired && (
					<p className="text-xs text-muted-foreground">
						{t("computer.pairing.expired", "This code expired.")}
					</p>
				)}
				<div className="flex gap-2">
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={loading}
						onClick={() => void newCode()}
					>
						<RefreshCwIcon className="size-3.5" aria-hidden />
						{t("computer.pairing.newCode", "New code")}
					</Button>
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={!code || expired}
						onClick={() => void copy()}
					>
						<CopyIcon className="size-3.5" aria-hidden />
						{t("computer.pairing.copy", "Copy pairing link")}
					</Button>
				</div>
			</div>
			{error && (
				<p role="alert" className="text-xs text-destructive">
					{error}
				</p>
			)}
			{notice && (
				<output className="text-xs text-foreground" aria-live="polite">
					{notice}
				</output>
			)}
			<div className="flex flex-col gap-2">
				<h4 className="text-xs font-medium">
					{t("computer.pairing.devices", "Paired devices")}
				</h4>
				<PairedDevices devices={devices} onRemove={(id) => void remove(id)} />
			</div>
		</div>
	);
}

/** "Watch on another device": the QR code another device scans to see this computer's bots. */
export function ComputerPairingDialog({
	pairing,
}: Readonly<{ pairing: IBotScreenPairing }>) {
	const { t } = useTranslation("bots");
	return (
		<Dialog>
			<DialogTrigger asChild>
				<Button type="button" variant="outline" size="sm" className="gap-1.5">
					<SmartphoneIcon className="size-3.5" aria-hidden />
					{t("computer.pairing.open", "Watch on another device")}
				</Button>
			</DialogTrigger>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle>
						{t("computer.pairing.dialogTitle", "Watch on another device")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"computer.pairing.dialogHint",
							"Open this bot's Computer tab on your phone or another computer, choose Scan QR code, and point the camera here. Only devices you pair can open the screen.",
						)}
					</DialogDescription>
				</DialogHeader>
				<PairingBody pairing={pairing} />
			</DialogContent>
		</Dialog>
	);
}
