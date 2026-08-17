import { AlertTriangleIcon, LanguagesIcon, PlusIcon } from "lucide-react";
import { useMemo, useState } from "react";
import {
	type Bundles,
	type LocaleConfig,
	buildRows,
	coverageOf,
	flatten,
	languageLabel,
} from "../lib/keys";
import { GhostButton, Meter, Panel, PrimaryButton } from "./primitives";

function heatStep(percent: number): number {
	if (percent === 0) return 0;
	if (percent < 25) return 1;
	if (percent < 50) return 2;
	if (percent < 75) return 3;
	if (percent < 95) return 4;
	return 5;
}

export function CoverageView({
	config,
	bundles,
	onOpen,
	onAddLanguage,
}: Readonly<{
	config: LocaleConfig;
	bundles: Bundles;
	onOpen: (language: string, namespace: string) => void;
	onAddLanguage: (code: string) => Promise<void>;
}>) {
	const [adding, setAdding] = useState(false);
	const [submitting, setSubmitting] = useState(false);
	const [draftCode, setDraftCode] = useState("");
	const [error, setError] = useState<string | null>(null);

	const targets = config.languages.filter(
		(code) => code !== config.sourceLanguage,
	);

	const grid = useMemo(() => {
		const cells: Record<
			string,
			Record<string, ReturnType<typeof coverageOf>>
		> = {};
		for (const namespace of config.namespaces) {
			cells[namespace] = {};
			for (const language of targets) {
				cells[namespace][language] = coverageOf(
					buildRows(bundles, [namespace], config.sourceLanguage, language),
				);
			}
		}
		return cells;
	}, [bundles, config.namespaces, config.sourceLanguage, targets]);

	const totals = useMemo(
		() =>
			Object.fromEntries(
				targets.map((language) => [
					language,
					coverageOf(
						buildRows(
							bundles,
							config.namespaces,
							config.sourceLanguage,
							language,
						),
					),
				]),
			),
		[bundles, config.namespaces, config.sourceLanguage, targets],
	);

	const sourceKeyCounts = useMemo(
		() =>
			Object.fromEntries(
				config.namespaces.map((namespace) => [
					namespace,
					Object.keys(
						flatten(bundles[config.sourceLanguage]?.[namespace] ?? {}),
					).length,
				]),
			),
		[bundles, config.namespaces, config.sourceLanguage],
	);
	const sourceKeys = Object.values(sourceKeyCounts).reduce(
		(sum, count) => sum + count,
		0,
	);

	const outstanding = targets.reduce(
		(sum, language) =>
			sum +
			(totals[language]?.missing ?? 0) +
			(totals[language]?.problems ?? 0),
		0,
	);

	async function submitLanguage() {
		if (submitting) return;
		setError(null);
		setSubmitting(true);
		try {
			await onAddLanguage(draftCode.trim());
			setDraftCode("");
			setAdding(false);
		} catch (cause) {
			setError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			setSubmitting(false);
		}
	}

	return (
		<div className="mx-auto grid max-w-350 gap-5 p-5">
			<section className="grid grid-cols-[repeat(auto-fit,minmax(180px,1fr))] gap-3">
				<Tile
					label="Source keys"
					value={String(sourceKeys)}
					hint={`${config.namespaces.length} namespaces`}
				/>
				<Tile
					label="Target locales"
					value={String(targets.length)}
					hint={`source is ${config.sourceLanguage}`}
				/>
				<Tile
					label="Needs attention"
					value={String(outstanding)}
					hint="missing, broken, copied, or orphaned"
					tone={outstanding > 0 ? "crit" : undefined}
				/>
			</section>

			<Panel
				title="Coverage"
				subtitle="namespace × locale, straight off the JSON files"
				action={
					adding ? (
						<div className="flex items-center gap-1.5">
							<input
								// The field only exists after the user clicks "Add language",
								// so focusing it is continuing their action, not hijacking the
								// page on load.
								ref={(node) => node?.focus()}
								value={draftCode}
								onChange={(event) => setDraftCode(event.target.value)}
								onKeyDown={(event) => {
									if (event.key === "Enter") void submitLanguage();
									if (event.key === "Escape" && !submitting) setAdding(false);
								}}
								placeholder="fr"
								aria-label="New language code"
								className="w-24 rounded-md border border-input bg-background px-2 py-1 text-[12px] outline-none focus:border-ring"
							/>
							<PrimaryButton
								onClick={() => void submitLanguage()}
								disabled={submitting || draftCode.trim() === ""}
							>
								{submitting ? "Creating…" : "Create"}
							</PrimaryButton>
						</div>
					) : (
						<GhostButton onClick={() => setAdding(true)}>
							<PlusIcon className="size-3.5" />
							Add language
						</GhostButton>
					)
				}
			>
				{error && (
					<p className="border-b border-border bg-(--crit)/10 px-3.5 py-2 text-[12px] text-(--crit)">
						{error}
					</p>
				)}
				<div className="overflow-x-auto">
					<table className="h-px w-full min-w-170 border-separate border-spacing-0">
						<thead>
							<tr>
								<th className="sticky left-0 z-10 border-b border-border bg-card px-3.5 py-2 text-left text-[11px] font-semibold text-muted-foreground">
									Namespace
								</th>
								{targets.map((language) => (
									<th
										key={language}
										title={languageLabel(language)}
										// Fixed-width locale columns: with one target language an
										// auto-sized column would stretch the heat fill across
										// half the table and read as an alarm.
										className="w-26 border-b border-border bg-card px-2 py-2 text-center text-[11px] font-semibold text-muted-foreground"
									>
										{language.toUpperCase()}
									</th>
								))}
								<th className="w-full border-b border-border bg-card" />
							</tr>
						</thead>
						<tbody>
							{config.namespaces.map((namespace) => (
								<tr key={namespace}>
									<th className="sticky left-0 z-10 h-full w-55 border-r border-b border-border bg-card px-3.5 py-2 text-left align-middle text-[12.5px] font-medium">
										{namespace}
										<span className="block font-mono text-[9.5px] font-normal text-muted-foreground">
											{sourceKeyCounts[namespace]} keys
										</span>
									</th>
									{targets.map((language) => {
										const cell = grid[namespace][language];
										// An empty namespace is not 100% done, it is undefined.
										// Rendering it green would claim work that never existed.
										const empty = cell.total === 0;
										return (
											<td
												key={language}
												className="h-full border-r border-b border-border p-0"
											>
												<button
													type="button"
													onClick={() => onOpen(language, namespace)}
													style={{
														background: empty
															? undefined
															: `var(--heat-${heatStep(cell.percent)})`,
													}}
													className="grid h-full w-full content-center gap-1 px-2 py-2 text-center transition-[filter,box-shadow] hover:brightness-105 hover:inset-ring-1 hover:inset-ring-foreground"
													aria-label={`${namespace} in ${languageLabel(language)}: ${empty ? "no keys" : `${cell.percent} percent`}`}
												>
													<span
														className={`font-mono text-[12px] font-semibold tabular-nums ${
															empty || cell.percent === 0
																? "font-normal text-muted-foreground"
																: ""
														}`}
													>
														{empty ? "—" : `${cell.percent}%`}
													</span>
													{!empty && <Meter value={cell.percent} />}
													{cell.problems > 0 && (
														<span className="inline-flex items-center justify-center gap-1 text-[9.5px] font-semibold tracking-wide text-(--crit) uppercase">
															<AlertTriangleIcon className="size-2.5" />
															{cell.problems}
														</span>
													)}
												</button>
											</td>
										);
									})}
									<td className="border-b border-border" />
								</tr>
							))}
						</tbody>
					</table>
				</div>
				<footer className="flex flex-wrap items-center gap-3 border-t border-border px-3.5 py-2 text-[11px] text-muted-foreground">
					<span className="inline-flex items-center gap-1">
						0%
						{[0, 1, 2, 3, 4, 5].map((step) => (
							<i
								key={step}
								className="inline-block h-2.5 w-5 rounded-[2px] border border-border"
								style={{ background: `var(--heat-${step})` }}
							/>
						))}
						100%
					</span>
					<span>Click a cell to open it in the workbench.</span>
				</footer>
			</Panel>

			<Panel title="Locales" subtitle="overall progress across every namespace">
				<div className="grid">
					{targets.map((language) => {
						const total = totals[language];
						return (
							<div
								key={language}
								className="grid grid-cols-[minmax(0,1fr)_140px_auto] items-center gap-4 border-b border-border px-3.5 py-2.5 last:border-b-0"
							>
								<div className="min-w-0">
									<div className="flex items-baseline gap-2 text-[13px] font-medium">
										<LanguagesIcon className="size-3.5 text-muted-foreground" />
										{languageLabel(language)}
										<span className="font-mono text-[10.5px] text-muted-foreground uppercase">
											{language}
										</span>
									</div>
									<div className="text-[11px] text-muted-foreground">
										{total.complete} complete · {total.missing} missing
										{total.problems > 0 && ` · ${total.problems} to check`}
									</div>
								</div>
								<Meter value={total.percent} />
								<span className="w-10 text-right font-mono text-[12px] tabular-nums">
									{total.percent}%
								</span>
							</div>
						);
					})}
				</div>
			</Panel>
		</div>
	);
}

function Tile({
	label,
	value,
	hint,
	tone,
}: Readonly<{ label: string; value: string; hint: string; tone?: "crit" }>) {
	return (
		<div className="relative grid gap-1.5 overflow-hidden rounded-md border border-border bg-card px-3.5 py-3">
			{tone === "crit" && (
				<span className="absolute inset-y-0 left-0 w-0.75 bg-(--crit)" />
			)}
			<span className="text-[10.5px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
				{label}
			</span>
			<span
				className={`text-[26px] leading-none font-semibold tabular-nums ${
					tone === "crit" ? "text-(--crit)" : ""
				}`}
			>
				{value}
			</span>
			<span className="text-[11.5px] text-muted-foreground">{hint}</span>
		</div>
	);
}
