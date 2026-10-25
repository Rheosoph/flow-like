"use client";

import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, Copy, ExternalLink, LogIn } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

const COPIED_RESET_MS = 2_000;

interface CopyState {
	readonly url: string;
	readonly status: "copied" | "failed";
}

/**
 * The opener reports success once xdg-open spawns, even when no browser
 * appears, so the authorize link is offered on every sign-in.
 */
export function SignInLinkDialog({
	url,
	onClose,
}: Readonly<{ url?: string; onClose: () => void }>) {
	const { t } = useTranslation("common");
	const inputRef = useRef<HTMLInputElement>(null);
	const [copyState, setCopyState] = useState<CopyState>();
	const copyStatus = copyState?.url === url ? copyState?.status : undefined;

	useEffect(() => {
		if (copyState?.status !== "copied") return;
		const timer = setTimeout(() => setCopyState(undefined), COPIED_RESET_MS);
		return () => clearTimeout(timer);
	}, [copyState]);

	const copyLink = useCallback(async () => {
		if (!url) return;
		try {
			await writeText(url);
			setCopyState({ url, status: "copied" });
		} catch (error) {
			console.error("Failed to copy the sign-in link:", error);
			setCopyState({ url, status: "failed" });
			inputRef.current?.select();
		}
	}, [url]);

	const openAgain = useCallback(() => {
		if (!url) return;
		openUrl(url).catch((error) =>
			console.error("Failed to open the sign-in link:", error),
		);
	}, [url]);

	return (
		<Dialog open={Boolean(url)} onOpenChange={(open) => !open && onClose()}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<div className="flex items-center gap-2">
						<LogIn className="h-5 w-5 text-primary" />
						<DialogTitle>
							{t("signInLink.title", "Continue in your browser")}
						</DialogTitle>
					</div>
					<DialogDescription>
						{t(
							"signInLink.description",
							"The sign-in page should have opened in your browser. If it didn't, copy this link and open it in a browser on this device.",
						)}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-2">
					<div className="flex items-center gap-2">
						<Input
							ref={inputRef}
							readOnly
							value={url ?? ""}
							onFocus={(event) => event.currentTarget.select()}
							aria-label={t("signInLink.linkLabel", "Sign-in link")}
							className="font-mono text-xs md:text-xs"
						/>
						<Button
							variant="outline"
							size="icon"
							onClick={copyLink}
							aria-label={t("copyLink", "Copy Link")}
							title={t("copyLink", "Copy Link")}
						>
							{copyStatus === "copied" ? (
								<Check className="h-4 w-4" />
							) : (
								<Copy className="h-4 w-4" />
							)}
						</Button>
					</div>
					{copyStatus === "failed" && (
						<p className="text-destructive text-xs" role="alert">
							{t(
								"signInLink.copyFailed",
								"Copying failed. The link is selected, so you can copy it with your keyboard.",
							)}
						</p>
					)}
				</div>

				<DialogFooter>
					<Button variant="outline" onClick={onClose}>
						{t("close", "Close")}
					</Button>
					<Button onClick={openAgain}>
						<ExternalLink className="h-4 w-4" />
						{t("signInLink.openAgain", "Open browser again")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
