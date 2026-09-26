"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

export function AuditHash({
	value,
	className,
}: Readonly<{ value?: string | null; className?: string }>) {
	if (!value) return <span className="text-muted-foreground">—</span>;
	const short =
		value.length > 18 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<code
					className={cn(
						"rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground",
						className,
					)}
				>
					{short}
				</code>
			</TooltipTrigger>
			<TooltipContent className="max-w-sm">
				<code className="break-all font-mono text-[11px]">{value}</code>
			</TooltipContent>
		</Tooltip>
	);
}

export function AuditCopyButton({
	value,
	label,
	className,
	variant = "outline",
}: Readonly<{
	value: string;
	label: string;
	className?: string;
	variant?: "outline" | "ghost" | "secondary";
}>) {
	const { t } = useTranslation("audit");
	const [copied, setCopied] = useState(false);

	useEffect(() => {
		if (!copied) return;
		const timer = setTimeout(() => setCopied(false), 1500);
		return () => clearTimeout(timer);
	}, [copied]);

	const copy = useCallback(async () => {
		try {
			await navigator.clipboard.writeText(value);
			setCopied(true);
		} catch {
			toast.error(t("copyFailed", "Could not copy to the clipboard."));
		}
	}, [value, t]);

	const Icon = copied ? Check : Copy;
	return (
		<Button
			type="button"
			size="sm"
			variant={variant}
			className={cn("gap-1.5", className)}
			onClick={copy}
		>
			<Icon className="h-3.5 w-3.5" />
			{copied ? t("copied", "Copied") : label}
		</Button>
	);
}
