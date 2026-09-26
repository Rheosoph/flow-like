"use client";

import { useTranslation } from "@flow-like/locales";
import { ShieldOffIcon } from "lucide-react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useAssetUrl } from "../hooks/use-asset-url";
import { isMicroWidgetServingUrl } from "../micro-widget-host";
import type { BoundValue, IframeComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const SANDBOX_SRC_DEFAULT =
	"allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox";
const SANDBOX_SRCDOC_DEFAULT = "allow-scripts";

function pageUrl(): string | undefined {
	return typeof window === "undefined" ? undefined : window.location.href;
}

/**
 * Package widgets are consented and mounted only by the micro widget host.
 * The generic Iframe refuses their serving URLs, whether an author wrote one
 * directly or a storage path resolved to one, and anything that is not http(s).
 */
export function isBlockedIframeSrc(
	rawSrc: string | undefined,
	resolvedSrc: string | undefined,
	base: string | undefined = pageUrl(),
): boolean {
	if (rawSrc && isMicroWidgetServingUrl(rawSrc, base)) return true;
	if (!resolvedSrc) return false;
	let url: URL;
	try {
		url = new URL(resolvedSrc, base);
	} catch {
		return true;
	}
	if (url.protocol !== "http:" && url.protocol !== "https:") return true;
	return isMicroWidgetServingUrl(url.href);
}

/**
 * A `srcdoc` document inherits the host origin, where it could forge widget
 * consent or reach host internals, so it never keeps `allow-same-origin`.
 */
export function srcdocSandbox(sandbox: string | undefined): string {
	if (sandbox === undefined) return SANDBOX_SRCDOC_DEFAULT;
	return sandbox
		.split(/\s+/)
		.filter((token) => token && token.toLowerCase() !== "allow-same-origin")
		.join(" ");
}

export function A2UIIframe({
	elementRef,
	component,
	style,
}: ComponentProps<IframeComponent>) {
	const { t } = useTranslation("common");
	const rawSrc = useResolved<string>(component.src);
	const { url: src, isLoading } = useAssetUrl(rawSrc);
	const srcdoc = useResolved<string>(component.srcdoc);
	const width = useResolved<string>(component.width) ?? "100%";
	const height = useResolved<string>(component.height) ?? "400px";
	const title = useResolved<string>(component.title) ?? "Embedded content";
	const sandbox = useResolved<string>(component.sandbox);
	const allow = useResolved<string>(component.allow);
	const loading = useResolved<"lazy" | "eager">(component.loading);
	const referrerPolicy = useResolved<string>(component.referrerPolicy);
	const border = useResolved<boolean>(component.border);

	const useSrcdoc = !!srcdoc;
	const effectiveSandbox = useSrcdoc
		? srcdocSandbox(sandbox)
		: (sandbox ?? SANDBOX_SRC_DEFAULT);
	const effectiveReferrerPolicy = referrerPolicy ?? "no-referrer";
	const blocked =
		!useSrcdoc && isBlockedIframeSrc(rawSrc, isLoading ? undefined : src);

	if (blocked || (!src && !srcdoc && !isLoading)) {
		return (
			<div
				ref={elementRef}
				className={cn(
					"flex items-center justify-center gap-2 bg-muted text-muted-foreground border rounded p-4 text-center text-sm",
					resolveStyle(style),
				)}
				style={{ ...resolveInlineStyle(style), width, height }}
				data-iframe-blocked={blocked ? "true" : undefined}
			>
				{blocked ? (
					<>
						<ShieldOffIcon className="h-4 w-4 shrink-0" />
						{t(
							"widgetEmbedBlocked",
							"This address cannot be embedded here. Only http and https pages can be shown, and package widgets must be added as widgets so their permissions can be reviewed.",
						)}
					</>
				) : (
					t("noContentProvided", "No content provided")
				)}
			</div>
		);
	}

	return (
		<iframe
			ref={elementRef}
			src={useSrcdoc ? undefined : src}
			srcDoc={useSrcdoc ? srcdoc : undefined}
			title={title}
			width={width}
			height={height}
			sandbox={effectiveSandbox}
			allow={allow}
			loading={loading ?? "lazy"}
			referrerPolicy={
				effectiveReferrerPolicy as React.HTMLAttributeReferrerPolicy
			}
			className={cn(
				border ? "border rounded" : "border-0",
				resolveStyle(style),
			)}
			style={resolveInlineStyle(style)}
		/>
	);
}
