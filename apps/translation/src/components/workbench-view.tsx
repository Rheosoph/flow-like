import { CodeIcon, SearchIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { type Usage, findUsages } from "../api";
import {
	type Bundles,
	type KeyRow,
	type KeyStatus,
	type LocaleConfig,
	buildRows,
	coverageOf,
	displayKey,
	languageLabel,
	missingPlaceholders,
	placeholders,
} from "../lib/keys";
import { usageNeedles } from "../lib/usage-key";
import { Chip, Meter, STATUS_LABEL, StatusDot } from "./primitives";

const FILTERS: (KeyStatus | "all")[] = [
	"all",
	"missing",
	"broken",
	"copied",
	"orphan",
	"translated",
];

export function WorkbenchView({
	config,
	bundles,
	language,
	namespace,
	onNamespace,
	onEdit,
}: Readonly<{
	config: LocaleConfig;
	bundles: Bundles;
	language: string;
	namespace: string;
	onNamespace: (namespace: string) => void;
	onEdit: (namespace: string, key: string, value: string) => void;
}>) {
	const [filter, setFilter] = useState<KeyStatus | "all">("all");
	const [query, setQuery] = useState("");
	const [selected, setSelected] = useState<string | null>(null);
	const listRef = useRef<HTMLDivElement>(null);

	const rows = useMemo(
		() => buildRows(bundles, [namespace], config.sourceLanguage, language),
		[bundles, namespace, config.sourceLanguage, language],
	);

	const counts = useMemo(() => {
		const base: Record<string, number> = { all: rows.length };
		for (const row of rows) base[row.status] = (base[row.status] ?? 0) + 1;
		return base;
	}, [rows]);

	const visible = useMemo(() => {
		const needle = query.trim().toLowerCase();
		return rows.filter((row) => {
			if (filter !== "all" && row.status !== filter) return false;
			if (!needle) return true;
			return (
				displayKey(row.key).toLowerCase().includes(needle) ||
				row.source.toLowerCase().includes(needle) ||
				row.value.toLowerCase().includes(needle)
			);
		});
	}, [rows, filter, query]);

	// Keep a selection alive across filter and namespace changes.
	useEffect(() => {
		if (visible.length === 0) {
			setSelected(null);
			return;
		}
		if (!selected || !visible.some((row) => row.key === selected)) {
			setSelected(visible[0].key);
		}
	}, [visible, selected]);

	const active = rows.find((row) => row.key === selected) ?? null;

	function move(delta: number) {
		if (!visible.length) return;
		const index = visible.findIndex((row) => row.key === selected);
		const next =
			visible[Math.min(visible.length - 1, Math.max(0, index + delta))];
		setSelected(next.key);
		listRef.current
			?.querySelector(`[data-key="${CSS.escape(next.key)}"]`)
			?.scrollIntoView({ block: "nearest" });
	}

	return (
		<div className="grid h-0 min-h-0 min-w-0 w-full max-w-full flex-1 grid-cols-[210px_minmax(0,1fr)_320px] overflow-hidden overscroll-none max-[1100px]:grid-cols-[minmax(0,1fr)] max-[1100px]:overflow-y-auto">
			<aside className="min-h-0 min-w-0 overflow-x-hidden overflow-y-auto overscroll-none border-r border-sidebar-border bg-sidebar p-2 max-[1100px]:max-h-52 max-[1100px]:border-r-0 max-[1100px]:border-b">
				<div className="px-2 pt-2 pb-1 text-[10.5px] font-semibold tracking-[0.08em] text-muted-foreground uppercase">
					Namespaces
				</div>
				{config.namespaces.map((name) => {
					const coverage = coverageOf(
						buildRows(bundles, [name], config.sourceLanguage, language),
					);
					return (
						<button
							key={name}
							type="button"
							aria-current={name === namespace}
							onClick={() => onNamespace(name)}
							className="grid w-full gap-1.5 rounded-md border border-transparent px-2 py-1.5 text-left transition-colors hover:bg-sidebar-accent aria-current:border-sidebar-border aria-current:bg-sidebar-accent"
						>
							<span className="flex items-center gap-2 text-[12.5px] font-medium">
								{name}
								<span className="ml-auto font-mono text-[10.5px] tabular-nums text-muted-foreground">
									{coverage.total === 0
										? "empty"
										: `${coverage.complete}/${coverage.total}`}
								</span>
							</span>
							{coverage.total > 0 && <Meter value={coverage.percent} />}
						</button>
					);
				})}
			</aside>

			<main className="flex min-h-0 min-w-0 flex-col overflow-hidden max-[1100px]:min-h-105">
				<div className="flex flex-wrap items-center gap-2 border-b border-border bg-card px-3 py-2">
					<label className="flex min-w-45 flex-1 max-w-75 items-center gap-1.5 rounded-md border border-input bg-background px-2 py-1">
						<SearchIcon className="size-3.5 text-muted-foreground" />
						<input
							value={query}
							onChange={(event) => setQuery(event.target.value)}
							placeholder="Search key, source or translation…"
							aria-label="Search keys"
							className="w-full bg-transparent text-[12.5px] outline-none"
						/>
					</label>
					<div className="flex flex-wrap gap-1.5">
						{FILTERS.map((value) => (
							<Chip
								key={value}
								active={filter === value}
								onClick={() => setFilter(value)}
								count={counts[value] ?? 0}
							>
								{value !== "all" && <StatusDot status={value} />}
								{value === "all" ? "All" : STATUS_LABEL[value]}
							</Chip>
						))}
					</div>
				</div>

				<div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_112px] border-b border-border px-3 py-1.5 text-[10.5px] font-semibold tracking-[0.07em] text-muted-foreground uppercase max-[640px]:grid-cols-[minmax(0,1fr)]">
					<span>Source · {config.sourceLanguage}</span>
					<span className="max-[640px]:hidden">
						Target · {languageLabel(language)}
					</span>
					<span className="max-[640px]:hidden">State</span>
				</div>

				<div
					ref={listRef}
					className="min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-none"
					onKeyDown={(event) => {
						if (event.key === "ArrowDown") {
							event.preventDefault();
							move(1);
						}
						if (event.key === "ArrowUp") {
							event.preventDefault();
							move(-1);
						}
					}}
				>
					{visible.length === 0 ? (
						<p className="px-5 py-10 text-center text-[13px] text-muted-foreground">
							No keys match this filter.
						</p>
					) : (
						visible.map((row) => (
							<KeyRowItem
								key={row.key}
								row={row}
								selected={row.key === selected}
								onSelect={() => setSelected(row.key)}
							/>
						))
					)}
				</div>
			</main>

			<Inspector
				row={active}
				language={language}
				namespace={namespace}
				defaultNamespace={config.defaultNamespace}
				onEdit={onEdit}
				onNext={() => move(1)}
			/>
		</div>
	);
}

function KeyRowItem({
	row,
	selected,
	onSelect,
}: Readonly<{ row: KeyRow; selected: boolean; onSelect: () => void }>) {
	return (
		<button
			type="button"
			data-key={row.key}
			aria-current={selected}
			onClick={onSelect}
			className={`relative grid min-w-0 w-full max-w-full grid-cols-[minmax(0,1fr)_minmax(0,1fr)_112px] overflow-hidden border-b border-border text-left transition-colors max-[640px]:grid-cols-[minmax(0,1fr)] ${
				selected
					? "bg-muted before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-primary"
					: "hover:bg-muted/55"
			}`}
		>
			<div className="min-w-0 px-3 py-2">
				<div className="truncate font-mono text-[10px] text-muted-foreground">
					{displayKey(row.key)}
				</div>
				<div className="max-w-full [overflow-wrap:anywhere] text-[13px]">
					{row.source || "—"}
				</div>
			</div>
			<div className="min-w-0 border-l border-border px-3 py-2 max-[640px]:border-t max-[640px]:border-l-0 max-[640px]:border-dashed">
				<div className="truncate font-mono text-[10px] text-muted-foreground">
					{row.status === "orphan" ? "not in source" : " "}
				</div>
				<div
					className={`max-w-full [overflow-wrap:anywhere] text-[13px] ${row.value ? "" : "text-muted-foreground italic"}`}
				>
					{row.value || "— not translated —"}
				</div>
			</div>
			<div className="flex items-start gap-1.5 border-l border-border px-3 py-2 text-[11px] text-muted-foreground max-[640px]:border-t max-[640px]:border-l-0 max-[640px]:border-dashed">
				<span className="mt-1.5">
					<StatusDot status={row.status} />
				</span>
				<span>{STATUS_LABEL[row.status]}</span>
			</div>
		</button>
	);
}

function Inspector({
	row,
	language,
	namespace,
	defaultNamespace,
	onEdit,
	onNext,
}: Readonly<{
	row: KeyRow | null;
	language: string;
	namespace: string;
	defaultNamespace: string;
	onEdit: (namespace: string, key: string, value: string) => void;
	onNext: () => void;
}>) {
	const [usages, setUsages] = useState<Usage[] | null>(null);

	const lookup = row?.key ?? null;
	const displayLookup = lookup
		? usageNeedles(lookup, namespace, defaultNamespace)[0]
		: null;

	useEffect(() => {
		setUsages(null);
		if (!lookup) return;
		let cancelled = false;
		findUsages(usageNeedles(lookup, namespace, defaultNamespace))
			.then((hits) => {
				if (!cancelled) setUsages(hits);
			})
			.catch(() => {
				if (!cancelled) setUsages([]);
			});
		return () => {
			cancelled = true;
		};
	}, [lookup, namespace, defaultNamespace]);

	if (!row) {
		return (
			<aside className="flex items-center justify-center border-l border-border bg-card p-8 text-center text-[12.5px] text-muted-foreground max-[1100px]:border-t max-[1100px]:border-l-0">
				Select a key to edit it.
			</aside>
		);
	}

	const sourceTokens = [...new Set(placeholders(row.source))];
	const lostTokens = missingPlaceholders(row.source, row.value);
	const lostTokenSet = new Set(lostTokens);

	return (
		<aside className="flex min-h-0 min-w-0 flex-col overflow-x-hidden overflow-y-auto overscroll-none border-l border-border bg-card max-[1100px]:border-t max-[1100px]:border-l-0">
			<header className="flex items-center gap-2 border-b border-border px-3.5 py-2.5">
				<h2 className="text-[12.5px] font-semibold">Key</h2>
				<span className="ml-auto flex items-center gap-1.5 text-[11px] text-muted-foreground">
					<StatusDot status={row.status} />
					{STATUS_LABEL[row.status]}
				</span>
			</header>

			<div className="grid gap-3.5 px-3.5 pt-3 pb-6">
				<div>
					<div className="flex flex-wrap items-center gap-1.5 text-[10.5px] font-semibold tracking-[0.07em] text-muted-foreground uppercase">
						Source
						<code className="rounded-[3px] bg-muted px-1.5 py-px font-mono text-[10px] normal-case tracking-normal break-all">
							{displayLookup}
						</code>
					</div>
					<p className="mt-1.5 max-w-full [overflow-wrap:anywhere] rounded-md border border-border bg-muted px-2.5 py-2 text-[13px]">
						{row.source || (
							<em className="text-muted-foreground">not in source</em>
						)}
					</p>
				</div>

				<div>
					<div className="text-[10.5px] font-semibold tracking-[0.07em] text-muted-foreground uppercase">
						{languageLabel(language)}
					</div>
					<textarea
						value={row.value}
						spellCheck={false}
						aria-label="Translation"
						onChange={(event) => onEdit(namespace, row.key, event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
								event.preventDefault();
								onNext();
							}
						}}
						className="mt-1.5 min-h-19 w-full resize-y rounded-md border border-input bg-background px-2.5 py-2 text-[13px] outline-none transition-[border-color,box-shadow] focus:border-ring focus:ring-3 focus:ring-ring/20"
					/>
					<div className="mt-1 flex justify-between font-mono text-[10.5px] tabular-nums text-muted-foreground">
						<span>
							{row.value.length} chars · source {row.source.length}
						</span>
						<span
							className={Math.abs(row.lengthDelta) > 40 ? "text-(--warn)" : ""}
						>
							{row.value && row.source
								? `${row.lengthDelta >= 0 ? "+" : ""}${row.lengthDelta}%`
								: "—"}
						</span>
					</div>
				</div>

				{row.status === "broken" && (
					<p className="rounded-md border border-(--crit)/45 bg-(--crit)/10 px-2.5 py-2 text-[11.5px] leading-relaxed">
						<strong className="mb-0.5 block text-[12px]">
							Placeholder missing
						</strong>
						{row.lostPlaceholders.join(", ")} appears in the source but not
						here. Interpolated values, nested translations, or rendered
						components may be lost at runtime.
					</p>
				)}

				{row.status === "copied" && (
					<p className="rounded-md border border-(--warn)/45 bg-(--warn)/10 px-2.5 py-2 text-[11.5px] leading-relaxed">
						<strong className="mb-0.5 block text-[12px]">
							Identical to the source
						</strong>
						Fine for product names and codes. Otherwise this is an untranslated
						string that reads as done.
					</p>
				)}

				{row.status === "orphan" && (
					<p className="rounded-md border border-(--crit)/45 bg-(--crit)/10 px-2.5 py-2 text-[11.5px] leading-relaxed">
						<strong className="mb-0.5 block text-[12px]">
							No longer in the source
						</strong>
						Clear the field and save to drop it, or run{" "}
						<code className="font-mono">mise run i18n:extract</code> to prune
						every orphan at once.
					</p>
				)}

				{sourceTokens.length > 0 && (
					<div className="overflow-hidden rounded-md border border-border">
						<header className="border-b border-border bg-muted px-2.5 py-1.5 text-[11.5px] font-semibold">
							Placeholders
						</header>
						<div className="flex flex-wrap gap-1.5 px-2.5 py-2">
							{sourceTokens.map((token) => {
								const missing = lostTokenSet.has(token);
								return (
									<code
										key={token}
										className={`max-w-full break-all rounded-full border px-2 py-0.5 font-mono text-[10.5px] ${
											!missing || !row.value
												? "border-border text-muted-foreground"
												: "border-(--crit) text-(--crit)"
										}`}
									>
										{token}
									</code>
								);
							})}
						</div>
					</div>
				)}

				<div className="overflow-hidden rounded-md border border-border">
					<header className="flex items-center gap-1.5 border-b border-border bg-muted px-2.5 py-1.5 text-[11.5px] font-semibold">
						<CodeIcon className="size-3.5" />
						Used in
						<span className="ml-auto font-mono text-[10.5px] font-normal text-muted-foreground">
							{usages?.length ?? "…"}
						</span>
					</header>
					{usages === null ? (
						<p className="px-2.5 py-2 text-[11.5px] text-muted-foreground">
							Searching the working tree…
						</p>
					) : usages.length === 0 ? (
						<p className="px-2.5 py-2 text-[11.5px] text-muted-foreground">
							No call site found. Either the key is built dynamically, or
							nothing renders it any more.
						</p>
					) : (
						usages.map((usage) => (
							<div
								key={`${usage.file}:${usage.line}`}
								className="grid gap-0.5 border-b border-border px-2.5 py-2 last:border-b-0"
							>
								<span className="font-mono text-[10px] text-muted-foreground">
									{usage.file}:{usage.line}
								</span>
								<span className="truncate font-mono text-[10.5px]">
									{usage.text}
								</span>
							</div>
						))
					)}
				</div>
			</div>
		</aside>
	);
}
