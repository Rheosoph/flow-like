"use client";

import type { SlateElementProps, TEquationElement } from "platejs";

import { getEquationHtml } from "@platejs/math";
import DOMPurify from "dompurify";
import { RadicalIcon } from "lucide-react";
import { SlateElement } from "platejs";

import { cn } from "../../../lib/utils";

// MathML tags and attributes needed by KaTeX
const KATEX_ALLOWED_TAGS = [
	"annotation",
	"annotation-xml",
	"math",
	"maction",
	"maligngroup",
	"malignmark",
	"menclose",
	"merror",
	"mfenced",
	"mfrac",
	"mi",
	"mlongdiv",
	"mmultiscripts",
	"mn",
	"mo",
	"mover",
	"mpadded",
	"mphantom",
	"mroot",
	"mrow",
	"ms",
	"mscarries",
	"mscarry",
	"msgroup",
	"msline",
	"mspacer",
	"msqrt",
	"msrow",
	"mstack",
	"mstyle",
	"msub",
	"msubsup",
	"msup",
	"mtable",
	"mtd",
	"mtext",
	"mtr",
	"munder",
	"munderover",
	"semantics",
];

const KATEX_ALLOWED_ATTR = [
	"columnalign",
	"columnlines",
	"columnspacing",
	"displaystyle",
	"encoding",
	"fence",
	"lspace",
	"mathvariant",
	"rowalign",
	"rowlines",
	"rowspacing",
	"rspace",
	"separator",
	"stretchy",
];

function sanitizeKatexHtml(html: string): string {
	return DOMPurify.sanitize(html, {
		ADD_TAGS: KATEX_ALLOWED_TAGS,
		ADD_ATTR: KATEX_ALLOWED_ATTR,
		FORCE_BODY: true,
	});
}

export function EquationElementStatic(
	props: SlateElementProps<TEquationElement>,
) {
	const { element } = props;

	const html = getEquationHtml({
		element,
		options: {
			displayMode: true,
			errorColor: "#cc0000",
			fleqn: false,
			leqno: false,
			macros: { "\\f": "#1f(#2)" },
			output: "htmlAndMathml",
			strict: "warn",
			throwOnError: false,
			trust: false,
		},
	});

	return (
		<SlateElement className="my-1" {...props}>
			<div
				className={cn(
					"group flex items-center justify-center rounded-sm select-none hover:bg-primary/10 data-[selected=true]:bg-primary/10",
					element.texExpression.length === 0
						? "bg-muted p-3 pr-9"
						: "px-2 py-1",
				)}
			>
				{element.texExpression.length > 0 ? (
					<span
						dangerouslySetInnerHTML={{
							__html: sanitizeKatexHtml(html),
						}}
					/>
				) : (
					<div className="flex h-7 w-full items-center gap-2 text-sm whitespace-nowrap text-muted-foreground">
						<RadicalIcon className="size-6 text-muted-foreground/80" />
						<div>Add a Tex equation</div>
					</div>
				)}
			</div>
			{props.children}
		</SlateElement>
	);
}

export function InlineEquationElementStatic(
	props: SlateElementProps<TEquationElement>,
) {
	const html = getEquationHtml({
		element: props.element,
		options: {
			displayMode: true,
			errorColor: "#cc0000",
			fleqn: false,
			leqno: false,
			macros: { "\\f": "#1f(#2)" },
			output: "htmlAndMathml",
			strict: "warn",
			throwOnError: false,
			trust: false,
		},
	});

	return (
		<SlateElement
			{...props}
			className="inline-block rounded-sm select-none [&_.katex-display]:my-0"
		>
			<div
				className={cn(
					'after:absolute after:inset-0 after:-top-0.5 after:-left-1 after:z-1 after:h-[calc(100%)+4px] after:w-[calc(100%+8px)] after:rounded-sm after:content-[""]',
					"h-6",
					props.element.texExpression.length === 0 &&
						"text-muted-foreground after:bg-neutral-500/10",
				)}
			>
				<span
					className={cn(
						props.element.texExpression.length === 0 && "hidden",
						"font-mono leading-none",
					)}
					dangerouslySetInnerHTML={{ __html: sanitizeKatexHtml(html) }}
				/>
			</div>
			{props.children}
		</SlateElement>
	);
}
