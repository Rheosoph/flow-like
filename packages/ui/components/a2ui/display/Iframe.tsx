"use client";

import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, IframeComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const SANDBOX_SRC_DEFAULT =
	"allow-scripts allow-same-origin allow-forms allow-popups allow-popups-to-escape-sandbox";
const SANDBOX_SRCDOC_DEFAULT = "allow-scripts";

export function A2UIIframe({
	component,
	style,
}: ComponentProps<IframeComponent>) {
	const src = useResolved<string>(component.src);
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
	const effectiveSandbox =
		sandbox ?? (useSrcdoc ? SANDBOX_SRCDOC_DEFAULT : SANDBOX_SRC_DEFAULT);
	const effectiveReferrerPolicy = referrerPolicy ?? "no-referrer";

	if (!src && !srcdoc) {
		return (
			<div
				className={cn(
					"flex items-center justify-center bg-muted text-muted-foreground border rounded",
					resolveStyle(style),
				)}
				style={{ ...resolveInlineStyle(style), width, height }}
			>
				No content provided
			</div>
		);
	}

	return (
		<iframe
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
