"use client";

import { useTranslation } from "@flow-like/locales";
import { CheckIcon, CopyIcon } from "lucide-react";
import { NodeApi, type TCodeBlockElement } from "platejs";
import {
	SlateElement,
	type SlateElementProps,
	SlateLeaf,
	type SlateLeafProps,
} from "platejs/static";
import { Suspense, lazy } from "react";
import * as React from "react";
import { Button } from "../../..";
import {
	useIsStreamingMarkdown,
	useRendererPresentation,
} from "../renderer-presentation";
import { useIsInsideA2UIFence } from "./a2ui-fence-context";
import { isA2UIFence } from "./a2ui-fence-detect";
import { ChartCodeBlock } from "./chart-code-block";
import { RichMediaCodeBlock } from "./rich-media-code-block";
import { isRichMediaLanguage } from "./rich-media-data";

const AdmonitionBlock = lazy(() => import("./admonition-block"));
const SpoilerBlock = lazy(() => import("./spoiler-block"));
const EmbedCodeBlock = lazy(() => import("./embed-code-block"));
const MapCodeBlock = lazy(() => import("./map-code-block"));
// Lazy on purpose: the a2ui renderer's markdown component renders this editor,
// so a static import would cycle at module evaluation.
const A2UICodeBlock = lazy(() => import("./a2ui-code-block"));

const CHART_LANGUAGES = ["nivo", "plotly"] as const;
type ChartLanguage = (typeof CHART_LANGUAGES)[number];

function isChartLanguage(lang: string | undefined): lang is ChartLanguage {
	return CHART_LANGUAGES.includes(lang as ChartLanguage);
}

function isDirectiveLanguage(lang: string | undefined): lang is string {
	return typeof lang === "string" && lang.startsWith("directive-");
}

function isCustomBlockLanguage(lang: string | undefined): lang is string {
	return lang === "embed" || lang === "map";
}

/** A fence drawn by a lazily loaded block; the Slate text stays mounted but hidden. */
function LazyFenceElement({
	block,
	...props
}: SlateElementProps<TCodeBlockElement> & { block: React.ReactNode }) {
	return (
		<SlateElement className="py-1" {...props}>
			<Suspense
				fallback={<div className="h-16 animate-pulse bg-muted/20 rounded-md" />}
			>
				{block}
			</Suspense>
			<div className="hidden">{props.children}</div>
		</SlateElement>
	);
}

export function CodeBlockElementStatic(
	props: SlateElementProps<TCodeBlockElement>,
) {
	const lang = props.element.lang;
	const isStreaming = useIsStreamingMarkdown();
	const insideA2UIFence = useIsInsideA2UIFence();
	const inChat = useRendererPresentation() !== "default";
	const content = getCodeBlockText(props.element);
	if (!insideA2UIFence && isA2UIFence(lang, content, inChat)) {
		return (
			<LazyFenceElement
				{...props}
				block={<A2UICodeBlock content={content} isStreaming={isStreaming} />}
			/>
		);
	}

	if (isRichMediaLanguage(lang)) {
		return (
			<SlateElement className="py-1" {...props}>
				<RichMediaCodeBlock
					content={content}
					language={lang}
					isStreaming={isStreaming}
				/>
				<div style={{ display: "none" }} aria-hidden="true">
					{props.children}
				</div>
			</SlateElement>
		);
	}

	// Render chart for nivo/plotly languages
	if (isChartLanguage(lang)) {
		return (
			<SlateElement className="codeblock py-1" {...props}>
				<ChartCodeBlock
					content={content}
					language={lang}
					isStreaming={isStreaming}
					className="rounded-md overflow-hidden"
				/>
				{/* Hidden children for Slate structure */}
				<div style={{ display: "none" }} aria-hidden="true">
					{props.children}
				</div>
			</SlateElement>
		);
	}

	if (isDirectiveLanguage(lang)) {
		const directiveType = lang.replace("directive-", "");
		return (
			<LazyFenceElement
				{...props}
				block={
					directiveType === "spoiler" ? (
						<SpoilerBlock content={content} />
					) : (
						<AdmonitionBlock type={directiveType} content={content} />
					)
				}
			/>
		);
	}

	if (isCustomBlockLanguage(lang)) {
		return (
			<LazyFenceElement
				{...props}
				block={
					lang === "embed" ? (
						<EmbedCodeBlock content={content} />
					) : (
						<MapCodeBlock content={content} />
					)
				}
			/>
		);
	}

	return <PlainCodeBlockElement {...props} content={content} />;
}

function PlainCodeBlockElement({
	content,
	...props
}: SlateElementProps<TCodeBlockElement> & { content: string }) {
	return (
		<SlateElement className="codeblock py-1" {...props}>
			<div className="relative rounded-md bg-muted/50">
				<pre className="overflow-x-auto p-8 pr-4 font-mono text-sm leading-[normal] [tab-size:2] print:break-inside-avoid">
					<code>{props.children}</code>
				</pre>

				<div
					className="absolute top-1 right-1 z-10 flex gap-0.5 select-none"
					contentEditable={false}
				>
					<CopyButton
						size="icon"
						variant="ghost"
						className="size-6 gap-1 text-xs text-muted-foreground"
						value={() => content}
					/>
				</div>
			</div>
		</SlateElement>
	);
}

export function CodeLineElementStatic(props: SlateElementProps) {
	return <SlateElement {...props} />;
}

export function CodeSyntaxLeafStatic(props: SlateLeafProps) {
	const tokenClassName = props.leaf.className as string;

	return <SlateLeaf className={tokenClassName} {...props} />;
}

function getCodeBlockText(element: TCodeBlockElement): string {
	const children = element?.children ?? [];
	if (!children.length) return "";
	return children.map((line) => NodeApi.string(line)).join("\n");
}

function CopyButton({
	value,
	...props
}: { value: (() => string) | string } & Omit<
	React.ComponentProps<typeof Button>,
	"value"
>) {
	const { t } = useTranslation("common");
	const [hasCopied, setHasCopied] = React.useState(false);

	React.useEffect(() => {
		if (!hasCopied) return;
		const timer = setTimeout(() => {
			setHasCopied(false);
		}, 2000);
		return () => clearTimeout(timer);
	}, [hasCopied]);

	return (
		<Button
			onClick={() => {
				void navigator.clipboard.writeText(
					typeof value === "function" ? value() : value,
				);
				setHasCopied(true);
			}}
			{...props}
		>
			<span className="sr-only">{t("copy", "Copy")}</span>
			{hasCopied ? (
				<CheckIcon className="size-3!" />
			) : (
				<CopyIcon className="size-3!" />
			)}
		</Button>
	);
}
