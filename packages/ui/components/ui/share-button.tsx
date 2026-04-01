"use client";

import { Check, Copy, Link2, Monitor, Share2 } from "lucide-react";
import { useCallback, useState } from "react";
import { toast } from "sonner";
import { cn } from "../../lib/utils";
import { Button } from "./button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "./dropdown-menu";
import { Tooltip, TooltipContent, TooltipTrigger } from "./tooltip";

export interface ShareButtonProps {
	appId: string;
	appName?: string;
	webBaseUrl?: string;
	className?: string;
	variant?: "default" | "outline" | "ghost" | "icon";
	size?: "default" | "sm" | "lg" | "icon";
	inviteToken?: string;
}

function isTauri(): boolean {
	return typeof window !== "undefined" && "__TAURI__" in window;
}

function generateWebUrl(baseUrl: string, appId: string): string {
	return `${baseUrl}/store?id=${appId}`;
}

function generateDeepLink(appId: string): string {
	return `flow-like://store?id=${appId}`;
}

function generateInviteWebUrl(
	baseUrl: string,
	appId: string,
	token: string,
): string {
	return `${baseUrl}/join?appId=${appId}&token=${token}`;
}

function generateInviteDeepLink(appId: string, token: string): string {
	return `flow-like://join?appId=${appId}&token=${token}`;
}

export function ShareButton({
	appId,
	appName,
	webBaseUrl = "https://app.flow-like.com",
	className,
	variant = "outline",
	size,
	inviteToken,
}: ShareButtonProps) {
	const [copied, setCopied] = useState<string | null>(null);

	const copyToClipboard = useCallback(async (text: string, type: string) => {
		try {
			await navigator.clipboard.writeText(text);
			setCopied(type);
			toast.success("Link copied to clipboard!");
			setTimeout(() => setCopied(null), 2000);
		} catch {
			toast.error("Failed to copy link");
		}
	}, []);

	const handleShare = useCallback(async () => {
		const webUrl = generateWebUrl(webBaseUrl, appId);
		const shareData = {
			title: appName || "Flow Like App",
			text: `Check out ${appName || "this app"} on Flow Like!`,
			url: webUrl,
		};

		if (navigator.share) {
			try {
				await navigator.share(shareData);
				return;
			} catch {
				// User cancelled or share failed, fall back to copy
			}
		}
		await copyToClipboard(webUrl, "web");
	}, [appId, appName, webBaseUrl, copyToClipboard]);

	const isDesktop = isTauri();

	const openInDesktopApp = useCallback(() => {
		const deepLink = generateDeepLink(appId);
		window.location.href = deepLink;
	}, [appId]);

	if (variant === "icon") {
		return (
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						onClick={handleShare}
						className={cn("h-8 w-8", className)}
					>
						{copied ? (
							<Check className="h-4 w-4 text-green-500" />
						) : (
							<Share2 className="h-4 w-4" />
						)}
					</Button>
				</TooltipTrigger>
				<TooltipContent>Share App</TooltipContent>
			</Tooltip>
		);
	}

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant={variant}
					size={size}
					className={cn("gap-2", className)}
				>
					<Share2 className="h-4 w-4" />
					Share
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-56">
				{!isDesktop && (
					<>
						<DropdownMenuItem onClick={openInDesktopApp}>
							<Monitor className="h-4 w-4 mr-2" />
							Open in Desktop App
						</DropdownMenuItem>
						<DropdownMenuSeparator />
					</>
				)}
				<DropdownMenuItem onClick={handleShare}>
					<Share2 className="h-4 w-4 mr-2" />
					Share via System
				</DropdownMenuItem>
				<DropdownMenuSeparator />
				<DropdownMenuItem
					onClick={() =>
						copyToClipboard(generateWebUrl(webBaseUrl, appId), "web")
					}
				>
					{copied === "web" ? (
						<Check className="h-4 w-4 mr-2 text-green-500" />
					) : (
						<Link2 className="h-4 w-4 mr-2" />
					)}
					Copy Web Link
				</DropdownMenuItem>
				<DropdownMenuItem
					onClick={() => copyToClipboard(generateDeepLink(appId), "deep")}
				>
					{copied === "deep" ? (
						<Check className="h-4 w-4 mr-2 text-green-500" />
					) : (
						<Copy className="h-4 w-4 mr-2" />
					)}
					Copy Desktop Link
				</DropdownMenuItem>
				{inviteToken && (
					<>
						<DropdownMenuSeparator />
						<DropdownMenuItem
							onClick={() =>
								copyToClipboard(
									generateInviteWebUrl(webBaseUrl, appId, inviteToken),
									"invite-web",
								)
							}
						>
							{copied === "invite-web" ? (
								<Check className="h-4 w-4 mr-2 text-green-500" />
							) : (
								<Link2 className="h-4 w-4 mr-2" />
							)}
							Copy Invite Link (Web)
						</DropdownMenuItem>
						<DropdownMenuItem
							onClick={() =>
								copyToClipboard(
									generateInviteDeepLink(appId, inviteToken),
									"invite-deep",
								)
							}
						>
							{copied === "invite-deep" ? (
								<Check className="h-4 w-4 mr-2 text-green-500" />
							) : (
								<Copy className="h-4 w-4 mr-2" />
							)}
							Copy Invite Link (Desktop)
						</DropdownMenuItem>
					</>
				)}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

export function QuickShareButton({
	url,
	title = "Share",
	className,
}: {
	url: string;
	title?: string;
	className?: string;
}) {
	const [copied, setCopied] = useState(false);

	const handleCopy = useCallback(async () => {
		try {
			await navigator.clipboard.writeText(url);
			setCopied(true);
			toast.success("Link copied!");
			setTimeout(() => setCopied(false), 2000);
		} catch {
			toast.error("Failed to copy link");
		}
	}, [url]);

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					variant="outline"
					size="sm"
					onClick={handleCopy}
					className={cn("gap-2", className)}
				>
					{copied ? (
						<Check className="h-4 w-4 text-green-500" />
					) : (
						<Copy className="h-4 w-4" />
					)}
					{title}
				</Button>
			</TooltipTrigger>
			<TooltipContent>{copied ? "Copied!" : `Copy ${title}`}</TooltipContent>
		</Tooltip>
	);
}
