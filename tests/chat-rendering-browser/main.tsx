import { I18nProvider } from "@flow-like/locales";
import { ThemeProvider } from "next-themes";
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { PlanSteps } from "../../packages/ui/components/interfaces/chat-default/plan-steps";
import { StreamingTextEditor } from "../../packages/ui/components/ui/streaming-text-editor";
import { TextEditor } from "../../packages/ui/components/ui/text-editor";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import "../../packages/ui/global.css";
import "./styles.css";
import { comparisonMarkdown, wideMarkdown } from "./fixtures";
import { researchSteps } from "./research-fixtures";
import { chartsMarkdown, richMediaMarkdown } from "./rich-fixtures";

const params = new URLSearchParams(location.search);

const FIXTURES = {
	full: [comparisonMarkdown, richMediaMarkdown, chartsMarkdown].join("\n\n"),
	tables: comparisonMarkdown,
	wide: wideMarkdown,
	media: richMediaMarkdown,
	charts: chartsMarkdown,
	research: "",
};

type Fixture = keyof typeof FIXTURES;
const initialFixture = params.get("fixture");

function ChatPreview() {
	const [dark, setDark] = useState(!params.has("light"));
	const [narrow, setNarrow] = useState(params.has("narrow"));
	const [compact, setCompact] = useState(params.has("compact"));
	const [fixture, setFixture] = useState<Fixture>(
		initialFixture && initialFixture in FIXTURES
			? (initialFixture as Fixture)
			: params.has("wide")
				? "wide"
				: "full",
	);
	const [streaming, setStreaming] = useState(params.has("stream"));
	const [paused, setPaused] = useState(params.has("stream"));
	const [cursor, setCursor] = useState(
		Math.max(0, Number(params.get("stream")) || 0),
	);
	const [editing, setEditing] = useState(false);
	const [draft, setDraft] = useState<string | null>(null);
	const [researchSettled, setResearchSettled] = useState(params.has("settled"));
	const content = draft ?? FIXTURES[fixture];
	const presentation = compact ? "chat-compact" : "chat";

	useEffect(() => {
		document.documentElement.classList.toggle("dark", dark);
	}, [dark]);

	useEffect(() => {
		if (!streaming || paused) return;
		const timer = window.setInterval(() => {
			setCursor((current) => Math.min(current + 28, content.length));
		}, 45);
		return () => window.clearInterval(timer);
	}, [streaming, paused, content]);

	useEffect(() => {
		if (streaming && cursor >= content.length) setStreaming(false);
	}, [cursor, content.length, streaming]);

	return (
		<ThemeProvider
			attribute="class"
			forcedTheme={dark ? "dark" : "light"}
			enableSystem={false}
			disableTransitionOnChange
		>
			<div className="preview-workspace">
				<header className="preview-toolbar">
					<div className="preview-title">Chat rendering</div>
					<nav aria-label="Preview settings">
						<button
							type="button"
							aria-pressed={narrow}
							onClick={() => setNarrow(!narrow)}
						>
							{narrow ? "Narrow" : "Desktop"}
						</button>
						<button
							type="button"
							aria-pressed={compact}
							onClick={() => setCompact(!compact)}
						>
							{compact ? "Compact" : "Standard"}
						</button>
						<select
							aria-label="Example content"
							value={fixture}
							onChange={(event) => {
								setFixture(event.target.value as Fixture);
								setDraft(null);
								setStreaming(false);
								setCursor(0);
							}}
						>
							<option value="full">Full answer</option>
							<option value="tables">Text and tables</option>
							<option value="wide">Wide table</option>
							<option value="media">Rich media</option>
							<option value="charts">Charts</option>
							<option value="research">Live research</option>
						</select>
						{fixture === "research" && (
							<button
								type="button"
								onClick={() => setResearchSettled(!researchSettled)}
							>
								{researchSettled ? "Restart research" : "Finish research"}
							</button>
						)}
						<button
							type="button"
							aria-pressed={dark}
							onClick={() => setDark(!dark)}
						>
							{dark ? "Dark" : "Light"}
						</button>
						<button
							type="button"
							aria-pressed={editing}
							onClick={() => {
								setEditing(!editing);
								setStreaming(false);
							}}
						>
							{editing ? "Hide Markdown" : "Edit Markdown"}
						</button>
						<button
							type="button"
							onClick={() => {
								setCursor(0);
								setPaused(false);
								setStreaming(true);
							}}
						>
							Replay stream
						</button>
						{streaming && (
							<>
								<button type="button" onClick={() => setPaused(!paused)}>
									{paused ? "Resume" : "Pause"}
								</button>
								<button type="button" onClick={() => setStreaming(false)}>
									Finish
								</button>
							</>
						)}
					</nav>
				</header>
				{streaming && (
					<label className="preview-stream-position">
						<span>Stream position</span>
						<input
							type="range"
							min={0}
							max={content.length}
							value={cursor}
							onChange={(event) => {
								setPaused(true);
								setCursor(Number(event.target.value));
							}}
						/>
						<output>
							{cursor} / {content.length}
						</output>
					</label>
				)}
				{editing && (
					<section className="preview-source" aria-label="Markdown editor">
						<div className="preview-source-header">
							<label htmlFor="markdown-source">Markdown source</label>
							<button
								type="button"
								onClick={() => {
									setDraft(null);
									setStreaming(false);
								}}
							>
								Reset to example
							</button>
						</div>
						<textarea
							id="markdown-source"
							value={content}
							spellCheck={false}
							onChange={(event) => {
								setDraft(event.target.value);
								setStreaming(false);
							}}
						/>
					</section>
				)}
				<main className={`preview-canvas${narrow ? " is-narrow" : ""}`}>
					<div className="preview-question">
						{fixture === "research"
							? "Welcher Porsche ist mit einem 5er oder 7er vergleichbar? Komfort auch auf längeren Strecken."
							: "Welche Modelle passen zu Diesel, Allrad und mehr als 350 PS?"}
					</div>
					<article aria-label="Assistant response" className="preview-answer">
						{fixture === "research" ? (
							<PlanSteps
								steps={researchSteps(researchSettled)}
								currentStepId={researchSettled ? undefined : "read-5"}
								loading={!researchSettled}
							/>
						) : streaming ? (
							<StreamingTextEditor
								content={content.slice(0, cursor)}
								presentation={presentation}
							/>
						) : (
							<TextEditor
								initialContent={content}
								isMarkdown
								editable={false}
								presentation={presentation}
							/>
						)}
					</article>
					<footer className="preview-status" aria-live="polite">
						{fixture === "research"
							? "Research preview with fixed example activity"
							: streaming
								? "Antwort wird geschrieben…"
								: "Ende der Beispielantwort"}
					</footer>
				</main>
			</div>
		</ThemeProvider>
	);
}

const container = document.getElementById("root");
if (!container) throw new Error("The chat preview root is missing.");

createRoot(container).render(
	<I18nProvider language="en">
		<TooltipProvider>
			<ChatPreview />
		</TooltipProvider>
	</I18nProvider>,
);
