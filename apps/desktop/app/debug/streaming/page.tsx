"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { StreamingTextEditor } from "@flow-like/flow-like-ui";
import { useCallback, useEffect, useRef, useState } from "react";

const SAMPLE_MD = `# Hello World

This is a **streaming** test with some markdown features.

## Code Example

\`\`\`typescript
function fibonacci(n: number): number {
  if (n <= 1) return n;
  return fibonacci(n - 1) + fibonacci(n - 2);
}

console.log(fibonacci(10));
\`\`\`

## Lists

- First item
- Second item with \`inline code\`
- Third item

## Table

| Feature | Status |
|---------|--------|
| Streaming | ✅ Working |
| Markdown | ✅ Parsed |
| Code blocks | ✅ Highlighted |

## Math

Inline math: $E = mc^2$

> This is a blockquote with **bold** and *italic* text.

And some final text to end the stream.
`;

export default function StreamingDebugPage() {
	const { t } = useTranslation("common");
	const [content, setContent] = useState("");
	const [isStreaming, setIsStreaming] = useState(false);
	const [speed, setSpeed] = useState(20);
	const [chunkSize, setChunkSize] = useState(3);
	const [customMd, setCustomMd] = useState(SAMPLE_MD);
	const [useCustom, setUseCustom] = useState(false);
	const [renderCount, setRenderCount] = useState(0);
	const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
	const posRef = useRef(0);

	const stop = useCallback(() => {
		if (intervalRef.current) {
			clearInterval(intervalRef.current);
			intervalRef.current = null;
		}
		setIsStreaming(false);
	}, []);

	const start = useCallback(() => {
		stop();
		const source = useCustom ? customMd : SAMPLE_MD;
		setContent("");
		posRef.current = 0;
		setIsStreaming(true);
		setRenderCount(0);

		intervalRef.current = setInterval(() => {
			const nextPos = Math.min(posRef.current + chunkSize, source.length);
			const newContent = source.substring(0, nextPos);
			posRef.current = nextPos;
			setContent(newContent);
			setRenderCount((c) => c + 1);

			if (nextPos >= source.length) {
				clearInterval(intervalRef.current!);
				intervalRef.current = null;
				setIsStreaming(false);
			}
		}, speed);
	}, [stop, speed, chunkSize, customMd, useCustom]);

	const startRAF = useCallback(() => {
		stop();
		const source = useCustom ? customMd : SAMPLE_MD;
		setContent("");
		posRef.current = 0;
		setIsStreaming(true);
		setRenderCount(0);

		let lastTime = 0;
		const tick = (time: number) => {
			if (time - lastTime < speed) {
				intervalRef.current = requestAnimationFrame(tick) as any;
				return;
			}
			lastTime = time;

			const nextPos = Math.min(posRef.current + chunkSize, source.length);
			posRef.current = nextPos;
			setContent(source.substring(0, nextPos));
			setRenderCount((c) => c + 1);

			if (nextPos >= source.length) {
				intervalRef.current = null;
				setIsStreaming(false);
				return;
			}
			intervalRef.current = requestAnimationFrame(tick) as any;
		};
		intervalRef.current = requestAnimationFrame(tick) as any;
	}, [stop, speed, chunkSize, customMd, useCustom]);

	useEffect(() => {
		return () => {
			if (intervalRef.current) {
				clearInterval(intervalRef.current);
			}
		};
	}, []);

	return (
		<div className="flex h-full flex-col overflow-hidden">
			<div className="flex items-center gap-3 border-b px-4 py-2 bg-muted/30 flex-wrap">
				<span className="text-sm font-semibold">{t('streamingDebug', 'Streaming Debug')}</span>

				<button
					type="button"
					onClick={start}
					disabled={isStreaming}
					className="rounded bg-primary px-3 py-1 text-xs text-primary-foreground disabled:opacity-50"
				>
					{t('streamSetinterval', 'Stream (setInterval)')}
				</button>
				<button
					type="button"
					onClick={startRAF}
					disabled={isStreaming}
					className="rounded bg-primary px-3 py-1 text-xs text-primary-foreground disabled:opacity-50"
				>
					{t('streamRaf', 'Stream (RAF)')}
				</button>
				<button
					type="button"
					onClick={stop}
					disabled={!isStreaming}
					className="rounded bg-destructive px-3 py-1 text-xs text-destructive-foreground disabled:opacity-50"
				>
					{t('stop', 'Stop')}
				</button>
				<button
					type="button"
					onClick={() => {
						setContent("");
						posRef.current = 0;
						setRenderCount(0);
					}}
					className="rounded bg-muted px-3 py-1 text-xs"
				>
					{t('reset', 'Reset')}
				</button>

				<label className="flex items-center gap-1 text-xs"><Trans i18nKey="speedMsInputTypenumberValuespeedOnchangeeSetspeednumberetargetvalueClassnamew16RoundedBorderPx1Py05TextxsBgbackgroundMin1">Speed (ms):
					<input
						type="number"
						value={speed}
						onChange={(e) => setSpeed(Number(e.target.value))}
						className="w-16 rounded border px-1 py-0.5 text-xs bg-background"
						min={1}
					/></Trans></label>
				<label className="flex items-center gap-1 text-xs"><Trans i18nKey="chunkCharsInputTypenumberValuechunksizeOnchangeeSetchunksizenumberetargetvalueClassnamew16RoundedBorderPx1Py05TextxsBgbackgroundMin1">Chunk (chars):
					<input
						type="number"
						value={chunkSize}
						onChange={(e) => setChunkSize(Number(e.target.value))}
						className="w-16 rounded border px-1 py-0.5 text-xs bg-background"
						min={1}
					/></Trans></label>
				<label className="flex items-center gap-1 text-xs"><Trans i18nKey="inputTypecheckboxCheckedusecustomOnchangeeSetusecustometargetcheckedCustomMd"><input
						type="checkbox"
						checked={useCustom}
						onChange={(e) => setUseCustom(e.target.checked)}
					/>
					Custom MD</Trans></label>

				<span className="ml-auto text-xs text-muted-foreground">{t('rendersRendercountCharsLengthLength2', "renders: {{renderCount}} | chars: {{length}}/ {{length2}}", { renderCount, length: content.length, length2: (useCustom ? customMd : SAMPLE_MD).length })}{isStreaming && t('streaming2', "| streaming...")}
				</span>
			</div>

			<div
				className="grid flex-1 overflow-hidden"
				style={{ gridTemplateColumns: useCustom ? "1fr 1fr" : "1fr" }}
			>
				{useCustom && (
					<div className="border-r overflow-auto">
						<textarea
							value={customMd}
							onChange={(e) => setCustomMd(e.target.value)}
							className="h-full w-full resize-none bg-muted/50 p-3 font-mono text-xs outline-none"
							placeholder={t('enterCustomMarkdown', 'Enter custom markdown...')}
						/>
					</div>
				)}
				<div className="overflow-auto p-4">
					<div className="rounded-lg border p-4 bg-background">
						<StreamingTextEditor content={content} />
					</div>
					{content && (
						<details className="mt-4 text-xs">
							<summary className="cursor-pointer text-muted-foreground">{t('rawContentLengthChars', 'Raw content ({{length}} chars)', { length: content.length })}</summary>
							<pre className="mt-2 max-h-48 overflow-auto rounded bg-muted p-2 font-mono text-xs whitespace-pre-wrap">
								{content}
							</pre>
						</details>
					)}
				</div>
			</div>
		</div>
	);
}
