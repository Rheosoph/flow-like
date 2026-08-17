import {
	CheckIcon,
	LoaderIcon,
	MoonIcon,
	SunIcon,
	TriangleAlertIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { addLanguage, loadLocales, saveNamespace } from "./api";
import { CoverageView } from "./components/coverage-view";
import { GhostButton, PrimaryButton } from "./components/primitives";
import { WorkbenchView } from "./components/workbench-view";
import {
	type Bundles,
	type LocaleConfig,
	flatten,
	languageLabel,
	unflatten,
} from "./lib/keys";
import { dirtyAfterSave } from "./lib/save-state";

type View = "coverage" | "workbench";
type SaveState = "idle" | "saving" | "saved" | "error";

export function App() {
	const [config, setConfig] = useState<LocaleConfig | null>(null);
	const [bundles, setBundles] = useState<Bundles>({});
	const [loadError, setLoadError] = useState<string | null>(null);
	const [view, setView] = useState<View>("coverage");
	const [language, setLanguage] = useState<string>("");
	const [namespace, setNamespace] = useState<string>("");
	// `${language}/${namespace}` for every bundle edited since the last save.
	const [dirty, setDirty] = useState<Set<string>>(new Set());
	// The version lets an async save clear only the exact edits it persisted.
	const editVersionsRef = useRef<Record<string, number>>({});
	const [saveState, setSaveState] = useState<SaveState>("idle");
	const [saveError, setSaveError] = useState<string | null>(null);
	const savingRef = useRef(false);
	const [dark, setDark] = useState(
		() =>
			typeof window !== "undefined" &&
			window.matchMedia("(prefers-color-scheme: dark)").matches,
	);

	useEffect(() => {
		document.documentElement.classList.toggle("dark", dark);
	}, [dark]);

	useEffect(() => {
		loadLocales()
			.then(({ config: loaded, bundles: files }) => {
				setConfig(loaded);
				setBundles(files);
				setLanguage(
					loaded.languages.find((code) => code !== loaded.sourceLanguage) ??
						loaded.sourceLanguage,
				);
				setNamespace(loaded.defaultNamespace);
			})
			.catch((cause) =>
				setLoadError(cause instanceof Error ? cause.message : String(cause)),
			);
	}, []);

	const edit = useCallback(
		(ns: string, key: string, value: string) => {
			const entry = `${language}/${ns}`;
			const nextVersion = (editVersionsRef.current[entry] ?? 0) + 1;
			editVersionsRef.current = {
				...editVersionsRef.current,
				[entry]: nextVersion,
			};
			setBundles((previous) => {
				const flat = flatten(previous[language]?.[ns] ?? {});
				const source = flatten(
					previous[config?.sourceLanguage ?? ""]?.[ns] ?? {},
				);
				// Empty source keys are the canonical marker for untranslated work.
				// An empty orphan, however, should disappear immediately after clear.
				if (value === "" && !Object.hasOwn(source, key)) delete flat[key];
				else flat[key] = value;
				return {
					...previous,
					[language]: { ...previous[language], [ns]: unflatten(flat) },
				};
			});
			setDirty((previous) => new Set(previous).add(entry));
			setSaveState("idle");
		},
		[config?.sourceLanguage, language],
	);

	const save = useCallback(async () => {
		if (dirty.size === 0 || savingRef.current) return;
		savingRef.current = true;
		const pending = [...dirty].map((entry) => {
			const [lang, ns] = entry.split("/");
			return {
				entry,
				version: editVersionsRef.current[entry] ?? 0,
				// Capture the exact tree paired with the version. A later edit may
				// replace the live bundle while this request is still in flight.
				tree: bundles[lang]?.[ns] ?? {},
			};
		});
		setSaveState("saving");
		setSaveError(null);
		try {
			for (const { entry, tree } of pending) {
				const [lang, ns] = entry.split("/");
				await saveNamespace(lang, ns, tree);
			}
			setDirty((current) => {
				return dirtyAfterSave(current, pending, editVersionsRef.current);
			});
			setSaveState("saved");
		} catch (cause) {
			setSaveState("error");
			setSaveError(cause instanceof Error ? cause.message : String(cause));
		} finally {
			savingRef.current = false;
		}
	}, [bundles, dirty]);

	useEffect(() => {
		if (dirty.size === 0) return;
		function beforeUnload(event: BeforeUnloadEvent) {
			event.preventDefault();
		}
		window.addEventListener("beforeunload", beforeUnload);
		return () => window.removeEventListener("beforeunload", beforeUnload);
	}, [dirty.size]);

	useEffect(() => {
		function onKey(event: KeyboardEvent) {
			if ((event.metaKey || event.ctrlKey) && event.key === "s") {
				event.preventDefault();
				void save();
			}
		}
		window.addEventListener("keydown", onKey);
		return () => window.removeEventListener("keydown", onKey);
	}, [save]);

	const createLanguage = useCallback(async (code: string) => {
		const next = await addLanguage(code);
		setConfig(next);
		setBundles((previous) => {
			if (previous[code]) return previous;
			return {
				...previous,
				[code]: Object.fromEntries(
					next.namespaces.map((name) => [
						name,
						unflatten(
							Object.fromEntries(
								Object.keys(
									flatten(previous[next.sourceLanguage]?.[name] ?? {}),
								).map((key) => [key, ""]),
							),
						),
					]),
				),
			};
		});
		setLanguage(code);
	}, []);

	const targets = useMemo(
		() =>
			config?.languages.filter((code) => code !== config.sourceLanguage) ?? [],
		[config],
	);

	if (loadError) {
		return (
			<div className="grid min-h-dvh place-items-center p-8">
				<div className="max-w-md rounded-md border border-(--crit)/45 bg-(--crit)/10 px-4 py-3 text-[13px]">
					<strong className="mb-1 block">
						Could not read the locale files
					</strong>
					{loadError}
					<p className="mt-2 text-muted-foreground">
						The studio talks to the Vite dev server that serves it. Start it
						with <code className="font-mono">mise run i18n:studio</code>.
					</p>
				</div>
			</div>
		);
	}

	if (!config) {
		return (
			<div className="grid min-h-dvh place-items-center text-[13px] text-muted-foreground">
				Reading packages/locales…
			</div>
		);
	}

	return (
		<div className="flex h-dvh min-h-0 flex-col overflow-hidden">
			<header className="flex flex-wrap items-center gap-3 border-b border-border bg-card px-3.5 py-2">
				<div className="flex items-center gap-2 font-semibold tracking-[-0.012em]">
					<span className="grid size-[22px] place-items-center rounded-[6px] bg-linear-145 from-primary to-accent text-[12px] font-bold text-primary-foreground">
						F
					</span>
					Translation Studio
				</div>

				<div className="inline-flex gap-0.5 rounded-md bg-muted p-0.5">
					{(["coverage", "workbench"] as const).map((value) => (
						<button
							key={value}
							type="button"
							aria-pressed={view === value}
							onClick={() => setView(value)}
							className="rounded-[calc(var(--radius)-1px)] px-2.5 py-1 text-[12.5px] text-muted-foreground capitalize transition-colors aria-[pressed=true]:bg-card aria-[pressed=true]:font-medium aria-[pressed=true]:text-foreground"
						>
							{value}
						</button>
					))}
				</div>

				{view === "workbench" && targets.length > 0 && (
					<label className="flex items-center gap-1.5 rounded-md border border-border bg-muted py-0.5 pr-1 pl-2 text-[12.5px]">
						<span className="text-[11px] tracking-[0.06em] text-muted-foreground uppercase">
							Target
						</span>
						<select
							value={language}
							onChange={(event) => setLanguage(event.target.value)}
							className="cursor-pointer bg-transparent py-1 text-[12.5px] font-medium outline-none"
						>
							{targets.map((code) => (
								<option key={code} value={code}>
									{languageLabel(code)} · {code}
								</option>
							))}
						</select>
					</label>
				)}

				<div className="ml-auto flex items-center gap-2">
					<SaveIndicator
						state={saveState}
						dirty={dirty.size}
						error={saveError}
					/>
					<PrimaryButton
						onClick={() => void save()}
						disabled={dirty.size === 0 || saveState === "saving"}
					>
						Save {dirty.size > 0 ? `(${dirty.size})` : ""}
					</PrimaryButton>
					<GhostButton
						onClick={() => setDark((value) => !value)}
						title="Toggle theme"
					>
						{dark ? (
							<SunIcon className="size-3.5" />
						) : (
							<MoonIcon className="size-3.5" />
						)}
					</GhostButton>
				</div>
			</header>

			{view === "coverage" ? (
				<div className="min-h-0 flex-1 overflow-y-auto">
					<CoverageView
						config={config}
						bundles={bundles}
						onAddLanguage={createLanguage}
						onOpen={(lang, ns) => {
							setLanguage(lang);
							setNamespace(ns);
							setView("workbench");
						}}
					/>
				</div>
			) : (
				<WorkbenchView
					config={config}
					bundles={bundles}
					language={language}
					namespace={namespace}
					onNamespace={setNamespace}
					onEdit={edit}
				/>
			)}

			<footer className="flex h-8 shrink-0 items-center gap-3 overflow-x-auto border-t border-border bg-card px-3.5 text-[11.5px] whitespace-nowrap text-muted-foreground">
				<span>
					Editing <code className="font-mono">packages/locales/locales</code>{" "}
					directly
				</span>
				<span className="ml-auto">
					<kbd className="rounded border border-b-2 border-border bg-muted px-1 py-px font-mono text-[10px] text-foreground">
						↑
					</kbd>
					<kbd className="ml-0.5 rounded border border-b-2 border-border bg-muted px-1 py-px font-mono text-[10px] text-foreground">
						↓
					</kbd>{" "}
					move ·{" "}
					<kbd className="rounded border border-b-2 border-border bg-muted px-1 py-px font-mono text-[10px] text-foreground">
						⌘S
					</kbd>{" "}
					save
				</span>
			</footer>
		</div>
	);
}

function SaveIndicator({
	state,
	dirty,
	error,
}: Readonly<{ state: SaveState; dirty: number; error: string | null }>) {
	if (state === "saving") {
		return (
			<span className="flex items-center gap-1.5 text-[11.5px] text-muted-foreground">
				<LoaderIcon className="size-3.5 animate-spin" />
				Saving…
			</span>
		);
	}
	if (state === "error") {
		return (
			<span
				title={error ?? undefined}
				className="flex items-center gap-1.5 text-[11.5px] text-(--crit)"
			>
				<TriangleAlertIcon className="size-3.5" />
				{error ?? "Save failed"}
			</span>
		);
	}
	if (state === "saved" && dirty === 0) {
		return (
			<span className="flex items-center gap-1.5 text-[11.5px] text-(--ok)">
				<CheckIcon className="size-3.5" />
				Written to disk
			</span>
		);
	}
	if (dirty > 0) {
		return (
			<span className="text-[11.5px] text-(--warn)">
				{dirty} unsaved {dirty === 1 ? "file" : "files"}
			</span>
		);
	}
	return null;
}
