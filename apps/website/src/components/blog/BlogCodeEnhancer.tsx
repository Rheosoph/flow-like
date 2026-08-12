"use client";

import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import { Maximize2, Minimize2 } from "lucide-react";
import {
	type KeyboardEvent as ReactKeyboardEvent,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { createPortal } from "react-dom";

const MIN_EDITOR_HEIGHT = 132;
const MAX_EDITOR_HEIGHT = 520;
const EDITOR_LINE_HEIGHT = 18;
const EDITOR_VERTICAL_PADDING = 20;

interface EnhancedCodeBlock {
	id: string;
	host: HTMLDivElement;
	pre: HTMLPreElement;
	source: string;
	languageLabel: string;
	wasHidden: boolean;
	previousAriaHidden: string | null;
}

function extractLanguage(pre: HTMLPreElement, code: HTMLElement): string {
	const explicit = pre.dataset.language ?? code.dataset.language;
	if (explicit) return explicit.toLowerCase();

	for (const element of [pre, code]) {
		const match = Array.from(element.classList)
			.map((className) => className.match(/^language-(.+)$/i)?.[1])
			.find(Boolean);
		if (match) return match.toLowerCase();
	}

	return "text";
}

function EditableCodeBlock({ block }: Readonly<{ block: EnhancedCodeBlock }>) {
	const [value, setValue] = useState(block.source);
	const [fullscreen, setFullscreen] = useState(false);
	const gutterRef = useRef<HTMLDivElement | null>(null);

	const lines = useMemo(() => value.split(/\r\n|\r|\n/), [value]);
	const editorHeight = useMemo(() => {
		const height = Math.min(
			MAX_EDITOR_HEIGHT,
			Math.max(
				MIN_EDITOR_HEIGHT,
				lines.length * EDITOR_LINE_HEIGHT + EDITOR_VERTICAL_PADDING,
			),
		);
		return `${height}px`;
	}, [lines.length]);

	useEffect(() => {
		if (!fullscreen) return;
		const handleKeyDown = (event: KeyboardEvent) => {
			if (event.key === "Escape") setFullscreen(false);
		};
		document.addEventListener("keydown", handleKeyDown);
		return () => document.removeEventListener("keydown", handleKeyDown);
	}, [fullscreen]);

	const handleEditorKeyDown = useCallback(
		(event: ReactKeyboardEvent<HTMLTextAreaElement>) => {
			if (event.key !== "Tab") return;
			event.preventDefault();
			const target = event.currentTarget;
			const start = target.selectionStart;
			const end = target.selectionEnd;
			const nextValue = `${value.slice(0, start)}  ${value.slice(end)}`;
			setValue(nextValue);
			window.requestAnimationFrame(() => {
				target.selectionStart = start + 2;
				target.selectionEnd = start + 2;
			});
		},
		[value],
	);

	const height = fullscreen ? "100vh" : editorHeight;

	const editor = (
		<section
			aria-label={`Editable ${block.languageLabel} code example`}
			className={`monaco-code-editor-wrapper relative overflow-hidden rounded-md border bg-muted/30 ${
				fullscreen
					? "fixed inset-0 z-[100] h-screen w-screen rounded-none bg-background"
					: ""
			}`}
			data-product-ui-source="packages/ui/components/ui/monaco-code-editor.tsx"
		>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="absolute right-2 top-2 z-10 h-7 w-7 bg-background/80 hover:bg-background"
				onClick={() => setFullscreen((current) => !current)}
				aria-label={
					fullscreen ? "Exit full screen editor" : "Open full screen editor"
				}
			>
				{fullscreen ? (
					<Minimize2 className="h-4 w-4" />
				) : (
					<Maximize2 className="h-4 w-4" />
				)}
			</Button>
			<div
				className="grid min-h-0 grid-cols-[3rem_minmax(0,1fr)] overflow-hidden bg-[#fffffe] dark:bg-[#1e1e1e]"
				style={{ height }}
			>
				<div
					ref={gutterRef}
					aria-hidden="true"
					className="overflow-hidden border-r border-black/10 bg-[#f5f5f5] py-2 text-right font-mono text-xs leading-[18px] text-[#8a8a8a] dark:border-white/10 dark:bg-[#181818] dark:text-[#858585]"
				>
					{lines.map((_, index) => (
						<div key={`${block.id}-line-${index}`} className="pr-3">
							{index + 1}
						</div>
					))}
				</div>
				<textarea
					value={value}
					onChange={(event) => setValue(event.target.value)}
					onKeyDown={handleEditorKeyDown}
					onScroll={(event) => {
						if (gutterRef.current) {
							gutterRef.current.scrollTop = event.currentTarget.scrollTop;
						}
					}}
					wrap="off"
					spellCheck={false}
					aria-label={`${block.languageLabel} source code`}
					className="h-full w-full resize-none overflow-auto bg-transparent px-3 py-2 pr-12 font-mono text-xs leading-[18px] text-[#1f1f1f] caret-primary outline-none selection:bg-primary/20 dark:text-[#d4d4d4]"
				/>
			</div>
		</section>
	);

	if (fullscreen && typeof document !== "undefined") {
		return (
			<>
				<div
					className="rounded-md border bg-muted/30"
					style={{ height: editorHeight }}
				/>
				{createPortal(editor, document.body)}
			</>
		);
	}

	return editor;
}

export function BlogCodeEnhancer() {
	const [blocks, setBlocks] = useState<EnhancedCodeBlock[]>([]);

	useEffect(() => {
		const article = document.getElementById("post-content");
		if (!article) return;

		const discovered: EnhancedCodeBlock[] = [];
		const preBlocks = article.querySelectorAll<HTMLPreElement>("pre");

		for (const [index, pre] of Array.from(preBlocks).entries()) {
			if (pre.dataset.blogCodeEnhanced === "true") continue;
			const code = pre.querySelector<HTMLElement>(":scope > code");
			const isFencedCode = Boolean(
				code &&
					(pre.dataset.language ||
						pre.classList.contains("astro-code") ||
						Array.from(code.classList).some((name) =>
							name.startsWith("language-"),
						)),
			);
			if (!code || !isFencedCode) continue;

			const id = `block-${index}`;
			const host = document.createElement("div");
			host.className = "not-prose my-5";
			host.dataset.blogCodeEnhancerHost = id;
			const block: EnhancedCodeBlock = {
				id,
				host,
				pre,
				source: code.textContent ?? "",
				languageLabel: extractLanguage(pre, code),
				wasHidden: pre.hidden,
				previousAriaHidden: pre.getAttribute("aria-hidden"),
			};

			pre.insertAdjacentElement("afterend", host);
			pre.hidden = true;
			pre.setAttribute("aria-hidden", "true");
			pre.dataset.blogCodeEnhanced = "true";
			discovered.push(block);
		}

		setBlocks(discovered);
		return () => {
			for (const block of discovered) {
				block.host.remove();
				block.pre.hidden = block.wasHidden;
				if (block.previousAriaHidden === null) {
					block.pre.removeAttribute("aria-hidden");
				} else {
					block.pre.setAttribute("aria-hidden", block.previousAriaHidden);
				}
				delete block.pre.dataset.blogCodeEnhanced;
			}
		};
	}, []);

	return (
		<>
			{blocks.map((block) =>
				createPortal(<EditableCodeBlock block={block} />, block.host, block.id),
			)}
		</>
	);
}
