"use client";

import { CheckIcon, CopyIcon } from "lucide-react";
import {
	NodeApi,
	SlateElement,
	type SlateElementProps,
	SlateLeaf,
	type SlateLeafProps,
	type TCodeBlockElement,
} from "platejs";
import { Suspense, lazy } from "react";
import * as React from "react";
import { Button } from "../../..";
import { ChartCodeBlock } from "./chart-code-block";

const AdmonitionBlock = lazy(() => import("./admonition-block"));
const SpoilerBlock = lazy(() => import("./spoiler-block"));
const EmbedCodeBlock = lazy(() => import("./embed-code-block"));
const MapCodeBlock = lazy(() => import("./map-code-block"));

const CHART_LANGUAGES = ["nivo", "plotly"] as const;
type ChartLanguage = (typeof CHART_LANGUAGES)[number];

function isChartLanguage(lang: string | undefined): lang is ChartLanguage {
	return CHART_LANGUAGES.includes(lang as ChartLanguage);
}

function isDirectiveLanguage(lang: string | undefined): boolean {
	return typeof lang === "string" && lang.startsWith("directive-");
}

function isCustomBlockLanguage(lang: string | undefined): lang is string {
	return lang === "embed" || lang === "map";
}

export function CodeBlockElementStatic(
	props: SlateElementProps<TCodeBlockElement>,
) {
	const lang = props.element.lang;
	const content = getCodeBlockText(props.element);

	// Render chart for nivo/plotly languages
	if (isChartLanguage(lang)) {
		return (
			<SlateElement className="codeblock py-1" {...props}>
				<ChartCodeBlock
					content={content}
					language={lang}
					className="rounded-md overflow-hidden"
				/>
				{/* Hidden children for Slate structure */}
				<div className="hidden">{props.children}</div>
			</SlateElement>
		);
	}

	// Render directive blocks (admonition / spoiler)
	if (isDirectiveLanguage(lang)) {
		const directiveType = lang!.replace("directive-", "");
		const isSpoiler = directiveType === "spoiler";
		return (
			<SlateElement className="py-1" {...props}>
				<Suspense fallback={<div className="h-16 animate-pulse bg-muted/20 rounded-md" />}>
					{isSpoiler ? (
						<SpoilerBlock content={content} />
					) : (
						<AdmonitionBlock type={directiveType} content={content} />
					)}
				</Suspense>
				<div className="hidden">{props.children}</div>
			</SlateElement>
		);
	}

	// Render embed/map custom blocks
	if (isCustomBlockLanguage(lang)) {
		return (
			<SlateElement className="py-1" {...props}>
				<Suspense fallback={<div className="h-16 animate-pulse bg-muted/20 rounded-md" />}>
					{lang === "embed" ? (
						<EmbedCodeBlock content={content} />
					) : (
						<MapCodeBlock content={content} />
					)}
				</Suspense>
				<div className="hidden">{props.children}</div>
			</SlateElement>
		);
	}

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
	const children = (element?.children ?? []) as any[];
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
	const [hasCopied, setHasCopied] = React.useState(false);

	React.useEffect(() => {
		const t = setTimeout(() => {
			setHasCopied(false);
		}, 2000);
		return () => clearTimeout(t);
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
			<span className="sr-only">Copy</span>
			{hasCopied ? (
				<CheckIcon className="size-3!" />
			) : (
				<CopyIcon className="size-3!" />
			)}
		</Button>
	);
}
