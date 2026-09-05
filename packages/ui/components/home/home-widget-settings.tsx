"use client";

import { useQuery } from "@tanstack/react-query";
import { Loader2, Plus, Search, Trash2, X } from "lucide-react";
import { type ReactNode, useEffect, useId, useMemo, useState } from "react";
import { useAppCategoryLabel } from "../../lib/app-category";
import { APP_CATEGORY_ORDER } from "../../lib/category-meta";
import { useBackend } from "../../state/backend-state";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
import { useHomeLibrary } from "./home-content/collections";
import { numberConfig, stringList, textConfig } from "./home-content/config";
import {
	HOME_QUICK_ACTIONS,
	homeLinks,
	informationItems,
} from "./home-content/personal-content";
import { useHomeScope } from "./home-content/shared";
import type { IHomeWidget } from "./types";

interface SettingsProps {
	widget: IHomeWidget;
	onChange: (config: Record<string, unknown>) => void;
}
const selectClass =
	"h-9 w-full rounded-md border border-input bg-background px-3 text-sm shadow-xs outline-none focus-visible:ring-2 focus-visible:ring-ring";

function Field({
	label,
	hint,
	children,
}: { label: string; hint?: string; children: (id: string) => ReactNode }) {
	const id = useId();
	return (
		<div className="space-y-1.5">
			<label htmlFor={id} className="text-xs font-medium">
				{label}
			</label>
			{children(id)}
			{hint && (
				<p className="text-[11px] leading-relaxed text-muted-foreground">
					{hint}
				</p>
			)}
		</div>
	);
}

function Choice({
	label,
	value,
	onChange,
	choices,
	hint,
}: {
	label: string;
	value: string;
	onChange: (value: string) => void;
	choices: [string, string][];
	hint?: string;
}) {
	return (
		<Field label={label} hint={hint}>
			{(id) => (
				<select
					id={id}
					value={value}
					onChange={(event) => onChange(event.target.value)}
					className={selectClass}
				>
					{choices.map(([key, title]) => (
						<option key={key} value={key}>
							{title}
						</option>
					))}
				</select>
			)}
		</Field>
	);
}

function Count({
	config,
	update,
}: {
	config: Record<string, unknown>;
	update: (key: string, value: unknown) => void;
}) {
	return (
		<Field label="Number of items">
			{(id) => (
				<Input
					id={id}
					type="number"
					min={1}
					max={50}
					value={numberConfig(config, "limit", 8)}
					onChange={(event) => update("limit", Number(event.target.value))}
				/>
			)}
		</Field>
	);
}

export function HomeAppPicker({
	value,
	onChange,
	multiple = false,
	label = "App",
	allowExplore = true,
}: {
	value: string[];
	onChange: (ids: string[]) => void;
	multiple?: boolean;
	label?: string;
	allowExplore?: boolean;
}) {
	const backend = useBackend();
	const scope = useHomeScope();
	const library = useHomeLibrary();
	const [query, setQuery] = useState("");
	const [search, setSearch] = useState("");
	useEffect(() => {
		const timer = setTimeout(() => setSearch(query.trim()), 300);
		return () => clearTimeout(timer);
	}, [query]);
	const results = useQuery({
		queryKey: ["home", ...scope, "app-picker", search],
		queryFn: () =>
			backend.appState.searchApps(
				undefined,
				search,
				undefined,
				undefined,
				undefined,
				undefined,
				undefined,
				0,
				20,
			),
		enabled: allowExplore && search.length >= 2,
	});
	const apps = useMemo(() => {
		const combined = new Map(
			(library.data ?? []).map(([app, metadata]) => [
				app.id,
				{ id: app.id, name: metadata?.name ?? app.id },
			]),
		);
		for (const [app, metadata] of results.data ?? [])
			combined.set(app.id, { id: app.id, name: metadata?.name ?? app.id });
		return [...combined.values()];
	}, [library.data, results.data]);
	const filtered = apps.filter(
		(app) =>
			!query ||
			`${app.name} ${app.id}`.toLowerCase().includes(query.toLowerCase()),
	);
	const select = (id: string) => {
		if (multiple)
			onChange(
				value.includes(id)
					? value.filter((current) => current !== id)
					: [...value, id],
			);
		else onChange(value[0] === id ? [] : [id]);
	};
	return (
		<div className="space-y-2">
			<Field
				label={label}
				hint={
					allowExplore
						? "Search your library or find an app by name."
						: "Select from this profile's apps."
				}
			>
				{(id) => (
					<div className="relative">
						<Search className="absolute left-2.5 top-2.5 size-4 text-muted-foreground" />
						<Input
							id={id}
							className="pl-8"
							value={query}
							onChange={(event) => setQuery(event.target.value)}
							placeholder="Search apps…"
						/>
					</div>
				)}
			</Field>
			{value.length > 0 && (
				<div className="flex flex-wrap gap-1.5">
					{value.map((id) => (
						<span
							key={id}
							className="flex max-w-full items-center gap-1 rounded-md bg-primary/10 px-2 py-1 text-[11px]"
						>
							<span className="truncate">
								{apps.find((app) => app.id === id)?.name ?? id}
							</span>
							<button
								type="button"
								aria-label={`Remove ${apps.find((app) => app.id === id)?.name ?? "app"}`}
								onClick={() =>
									onChange(value.filter((current) => current !== id))
								}
								className="shrink-0 rounded hover:bg-primary/10"
							>
								<X className="size-3" />
							</button>
						</span>
					))}
				</div>
			)}
			<div className="max-h-44 space-y-0.5 overflow-y-auto rounded-md border p-1">
				{library.isLoading ? (
					<div className="flex items-center gap-2 p-2 text-xs text-muted-foreground">
						<Loader2 className="size-3 animate-spin" />
						Loading apps…
					</div>
				) : filtered.length ? (
					filtered.map((app) => (
						<label
							key={app.id}
							className="flex cursor-pointer items-center gap-2 rounded p-2 hover:bg-muted"
						>
							<input
								type="checkbox"
								checked={value.includes(app.id)}
								onChange={() => select(app.id)}
								className="size-3.5 accent-primary"
							/>
							<span className="min-w-0 truncate text-xs">{app.name}</span>
						</label>
					))
				) : (
					<p className="p-2 text-xs text-muted-foreground">
						{results.isFetching
							? "Searching apps…"
							: "No apps match this search."}
					</p>
				)}
			</div>
			{library.isError && (
				<output className="block text-xs text-destructive">
					Your library could not load.{" "}
					<button
						type="button"
						onClick={() => void library.refetch()}
						className="underline"
					>
						Try again
					</button>
				</output>
			)}
			{results.isError && (
				<output className="block text-xs text-muted-foreground">
					App search is unavailable. Your loaded library is still shown.
				</output>
			)}
		</div>
	);
}

function EmbedSettings({ widget, onChange }: SettingsProps) {
	const backend = useBackend();
	const scope = useHomeScope();
	const { config } = widget;
	const appId = textConfig(config, "appId");
	const target = textConfig(config, "target", "landing");
	const events = useQuery({
		queryKey: ["home", ...scope, "app-interfaces", appId],
		queryFn: () => backend.eventState.getEvents(appId),
		enabled: Boolean(appId),
	});
	const update = (key: string, value: unknown) =>
		onChange({ ...config, [key]: value });
	const interfaces = (events.data ?? []).filter(
		(event) =>
			event.default_page_id ||
			["simple_chat", "generic_form", "quick_action"].includes(
				event.event_type,
			),
	);
	const routes = (events.data ?? []).filter((event) => event.route);
	return (
		<div className="space-y-4">
			<HomeAppPicker
				value={appId ? [appId] : []}
				onChange={(ids) =>
					onChange({ ...config, appId: ids[0] ?? "", eventId: "", route: "/" })
				}
			/>
			<Choice
				label="Open"
				value={target}
				onChange={(value) => update("target", value)}
				choices={[
					["landing", "App landing page"],
					["route", "A specific page or route"],
					["event", "A chat or other interface"],
				]}
			/>
			{target === "route" && (
				<>
					<Field
						label="Page route"
						hint="The app's route path, for example /reports. Query values can be configured below."
					>
						{(id) => (
							<>
								<Input
									id={id}
									list={`${id}-routes`}
									value={textConfig(config, "route", "/")}
									placeholder="/reports"
									onChange={(event) => update("route", event.target.value)}
								/>
								<datalist id={`${id}-routes`}>
									{routes.map((event) => (
										<option key={event.id} value={event.route ?? "/"}>
											{event.name}
										</option>
									))}
								</datalist>
							</>
						)}
					</Field>
					{routes.length > 0 && (
						<Choice
							label="Available pages"
							value={textConfig(config, "route", "/")}
							onChange={(value) => update("route", value)}
							choices={[
								["/", "Landing page /"],
								...routes
									.filter((event) => event.route !== "/")
									.map(
										(event) =>
											[
												event.route ?? "/",
												`${event.name} (${event.route})`,
											] as [string, string],
									),
							]}
						/>
					)}
				</>
			)}
			{target === "event" && (
				<Choice
					label="App interface"
					value={textConfig(config, "eventId")}
					onChange={(value) => update("eventId", value)}
					choices={[
						[
							"",
							events.isLoading ? "Loading interfaces…" : "Choose an interface",
						],
						...interfaces.map(
							(event) =>
								[
									event.id,
									`${event.name} · ${event.event_type === "simple_chat" ? "Chat" : event.default_page_id ? "Page" : "Form"}`,
								] as [string, string],
						),
					]}
					hint={
						!events.isLoading && appId && !interfaces.length
							? "This app does not expose a page, chat, or form to embed."
							: undefined
					}
				/>
			)}
			{events.isError && (
				<p className="text-xs text-destructive">
					App interfaces could not load.{" "}
					<button
						type="button"
						className="underline"
						onClick={() => void events.refetch()}
					>
						Try again
					</button>
				</p>
			)}
			<Field
				label="Query parameters"
				hint="For example: period=month&team=sales. Parameters belong to this widget and do not change your home URL. The names id, route, and eventId are reserved."
			>
				{(id) => (
					<Textarea
						id={id}
						rows={2}
						value={textConfig(config, "query")}
						onChange={(event) => update("query", event.target.value)}
						placeholder="period=month&team=sales"
						className="font-mono text-xs"
					/>
				)}
			</Field>
		</div>
	);
}

function InformationSettings({ widget, onChange }: SettingsProps) {
	const { config } = widget;
	const update = (key: string, value: unknown) =>
		onChange({ ...config, [key]: value });
	const mode = textConfig(config, "mode", "markdown");
	const items = informationItems(config);
	const itemLabel =
		mode === "faq"
			? "Question"
			: ["steps", "checklist"].includes(mode)
				? "Step"
				: "Entry";
	return (
		<div className="space-y-4">
			<Choice
				label="Content format"
				value={mode}
				onChange={(value) => update("mode", value)}
				choices={[
					["markdown", "Notes and rich text"],
					["banner", "Feature banner"],
					["announcement", "Announcement"],
					["faq", "Questions and answers"],
					["checklist", "Checklist"],
					["countdown", "Milestone countdown"],
					["story", "Editorial story"],
					["image", "Image and caption"],
					["quote", "Quote and attribution"],
					["feed", "Updates and changelog"],
					["steps", "Guided steps"],
					["resources", "Resource directory"],
					["facts", "Facts and highlights"],
				]}
			/>
			{mode === "countdown" && (
				<Field label="Milestone date">
					{(id) => (
						<Input
							id={id}
							type="date"
							value={textConfig(config, "date").slice(0, 10)}
							onChange={(event) =>
								update(
									"date",
									event.target.value ? `${event.target.value}T12:00:00` : "",
								)
							}
						/>
					)}
				</Field>
			)}
			{["faq", "checklist", "feed", "steps", "resources", "facts"].includes(
				mode,
			) ? (
				<div className="space-y-3">
					{items.map((item, index) => (
						<div key={item.id} className="space-y-2 rounded-xl border p-3">
							<div className="flex items-center justify-between">
								<span className="text-xs font-medium">
									{itemLabel} {index + 1}
								</span>
								<Button
									type="button"
									variant="ghost"
									size="icon"
									className="size-6"
									aria-label={`Remove item ${index + 1}`}
									onClick={() =>
										update(
											"items",
											items.filter((_, itemIndex) => itemIndex !== index),
										)
									}
								>
									<Trash2 className="size-3" />
								</Button>
							</div>
							<Input
								aria-label={`${itemLabel} ${index + 1}`}
								value={item.title}
								onChange={(event) =>
									update(
										"items",
										items.map((entry, itemIndex) =>
											itemIndex === index
												? { ...entry, title: event.target.value }
												: entry,
										),
									)
								}
							/>
							{mode !== "checklist" && (
								<Textarea
									aria-label={`${mode === "faq" ? "Answer" : "Description"} ${index + 1}`}
									rows={3}
									value={item.body ?? ""}
									onChange={(event) =>
										update(
											"items",
											items.map((entry, itemIndex) =>
												itemIndex === index
													? { ...entry, body: event.target.value }
													: entry,
											),
										)
									}
									placeholder="Add context. Markdown is supported."
								/>
							)}
							{["feed", "steps", "resources", "facts"].includes(mode) && (
								<>
									<Input
										aria-label={`Label ${index + 1}`}
										value={item.label ?? ""}
										placeholder="Label, date, or category"
										onChange={(event) =>
											update(
												"items",
												items.map((entry, i) =>
													i === index
														? { ...entry, label: event.target.value }
														: entry,
												),
											)
										}
									/>
									<Input
										aria-label={`Destination ${index + 1}`}
										value={item.href ?? ""}
										placeholder="Optional link or app destination"
										onChange={(event) =>
											update(
												"items",
												items.map((entry, i) =>
													i === index
														? { ...entry, href: event.target.value }
														: entry,
												),
											)
										}
									/>
								</>
							)}
						</div>
					))}
					<Button
						type="button"
						variant="outline"
						size="sm"
						onClick={() =>
							update("items", [
								...items,
								{
									id: crypto.randomUUID(),
									title: "",
									body: "",
									checked: false,
								},
							])
						}
					>
						<Plus className="mr-1.5 size-3.5" />
						Add {itemLabel.toLowerCase()}
					</Button>
				</div>
			) : (
				<Field
					label={mode === "countdown" ? "Description" : "Content"}
					hint="Use Markdown for headings, links, lists, and emphasis."
				>
					{(id) => (
						<Textarea
							id={id}
							rows={9}
							value={textConfig(config, "body")}
							onChange={(event) => update("body", event.target.value)}
							placeholder="Write something useful…"
						/>
					)}
				</Field>
			)}
			{[
				"banner",
				"announcement",
				"story",
				"image",
				"quote",
				"markdown",
			].includes(mode) && (
				<>
					{[
						["eyebrow", "Label"],
						["imageUrl", "Image URL"],
						["imageAlt", "Image description"],
						["actionLabel", "Button label"],
						["actionHref", "Button destination"],
					].map(([key, label]) => (
						<Field key={key} label={label}>
							{(id) => (
								<Input
									id={id}
									value={textConfig(config, key)}
									onChange={(event) => update(key, event.target.value)}
								/>
							)}
						</Field>
					))}
					{mode === "quote" && (
						<Field label="Attribution">
							{(id) => (
								<Input
									id={id}
									value={textConfig(config, "attribution")}
									onChange={(event) =>
										update("attribution", event.target.value)
									}
								/>
							)}
						</Field>
					)}
				</>
			)}
		</div>
	);
}

export function HomeWidgetSettings({ widget, onChange }: SettingsProps) {
	const config = widget.config;
	const update = (key: string, value: unknown) =>
		onChange({ ...config, [key]: value });
	const categoryLabel = useAppCategoryLabel();
	if (widget.type === "app-embed")
		return <EmbedSettings widget={widget} onChange={onChange} />;
	if (widget.type === "information")
		return <InformationSettings widget={widget} onChange={onChange} />;
	if (widget.type === "flowpilot")
		return (
			<div className="space-y-4">
				<Choice
					label="FlowPilot shape"
					value={textConfig(config, "mode", "bar")}
					onChange={(value) => update("mode", value)}
					choices={[
						["orb", "Round orb"],
						["bar", "Wide prompt bar"],
						["card", "Composer card"],
						["hero", "Full animated hero"],
					]}
				/>
				<Field label="Prompt placeholder">
					{(id) => (
						<Input
							id={id}
							value={textConfig(config, "placeholder")}
							onChange={(event) => update("placeholder", event.target.value)}
							placeholder="Ask FlowPilot to build, explore, or explain…"
						/>
					)}
				</Field>
			</div>
		);
	if (widget.type === "greeting")
		return (
			<div className="space-y-4">
				<Field
					label="Name"
					hint="Leave blank to use your account's first name."
				>
					{(id) => (
						<Input
							id={id}
							value={textConfig(config, "name")}
							onChange={(event) => update("name", event.target.value)}
						/>
					)}
				</Field>
				<Field label="Welcome message">
					{(id) => (
						<Textarea
							id={id}
							value={textConfig(config, "subtitle")}
							onChange={(event) => update("subtitle", event.target.value)}
							rows={2}
						/>
					)}
				</Field>
			</div>
		);
	if (widget.type === "app-collection")
		return (
			<div className="space-y-4">
				<Choice
					label="Apps to show"
					value={textConfig(config, "source", "library")}
					onChange={(value) => update("source", value)}
					choices={[
						["library", "This profile's library"],
						["recent", "Recently updated in this library"],
						["favorites", "This profile's favorites"],
						["manual", "A handpicked collection"],
						["new", "New community apps"],
						["popular", "Popular community apps"],
					]}
				/>
				{textConfig(config, "source") === "manual" && (
					<HomeAppPicker
						label="Apps in this collection"
						value={stringList(config, "appIds")}
						onChange={(ids) => update("appIds", ids)}
						multiple
					/>
				)}
				<Field label="Search filter">
					{(id) => (
						<Input
							id={id}
							value={textConfig(config, "query")}
							onChange={(event) => update("query", event.target.value)}
							placeholder="For example, documents"
						/>
					)}
				</Field>
				<Choice
					label="Category"
					value={textConfig(config, "category")}
					onChange={(value) => update("category", value)}
					choices={[
						["", "All categories"],
						...APP_CATEGORY_ORDER.map(
							(category) =>
								[category, categoryLabel(category)] as [string, string],
						),
					]}
				/>
				<Field label="Tag filter">
					{(id) => (
						<Input
							id={id}
							value={textConfig(config, "tag")}
							onChange={(event) => update("tag", event.target.value)}
							placeholder="Optional tag"
						/>
					)}
				</Field>
				<Count config={config} update={update} />
			</div>
		);
	if (widget.type === "categories")
		return (
			<div className="space-y-2">
				<p className="text-xs text-muted-foreground">
					Choose categories, or leave all unchecked to show every category.
				</p>
				{APP_CATEGORY_ORDER.map((category) => (
					<label
						key={category}
						className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 hover:bg-muted"
					>
						<input
							type="checkbox"
							className="accent-primary"
							checked={stringList(config, "categories").includes(category)}
							onChange={(event) =>
								update(
									"categories",
									event.target.checked
										? [...stringList(config, "categories"), category]
										: stringList(config, "categories").filter(
												(item) => item !== category,
											),
								)
							}
						/>
						<span className="text-xs">{categoryLabel(category)}</span>
					</label>
				))}
			</div>
		);
	if (widget.type === "packages" || widget.type === "models")
		return (
			<div className="space-y-4">
				{widget.type === "models" && (
					<Choice
						label="Model source"
						value={textConfig(config, "source", "profile")}
						onChange={(value) => update("source", value)}
						choices={[
							["profile", "Models in this profile"],
							["explore", "Explore available models"],
						]}
					/>
				)}
				<Field label="Search">
					{(id) => (
						<Input
							id={id}
							value={textConfig(config, "query")}
							onChange={(event) => update("query", event.target.value)}
							placeholder={
								widget.type === "models" ? "Search models…" : "Search packages…"
							}
						/>
					)}
				</Field>
				{widget.type === "packages" && (
					<Choice
						label="Sort packages"
						value={textConfig(config, "sort", "downloads")}
						onChange={(value) => update("sort", value)}
						choices={[
							["downloads", "Most downloaded"],
							["created_at", "Newest"],
							["updated_at", "Recently updated"],
							["relevance", "Search relevance"],
						]}
					/>
				)}
				<Count config={config} update={update} />
			</div>
		);
	if (widget.type === "quick-actions")
		return (
			<div className="space-y-2">
				{Object.entries(HOME_QUICK_ACTIONS).map(([id, action]) => (
					<label
						key={id}
						className="flex cursor-pointer items-start gap-2 rounded p-2 hover:bg-muted"
					>
						<input
							type="checkbox"
							className="mt-0.5 accent-primary"
							checked={stringList(config, "actions").includes(id)}
							onChange={(event) =>
								update(
									"actions",
									event.target.checked
										? [...stringList(config, "actions"), id]
										: stringList(config, "actions").filter(
												(value) => value !== id,
											),
								)
							}
						/>
						<span>
							<span className="block text-xs font-medium">{action.title}</span>
							<span className="text-[11px] text-muted-foreground">
								{action.description}
							</span>
						</span>
					</label>
				))}
			</div>
		);
	if (widget.type === "quick-links") {
		const links = homeLinks(config);
		return (
			<div className="space-y-3">
				{links.map((link, index) => (
					<div key={link.id} className="space-y-2 rounded-xl border p-3">
						<div className="flex items-center justify-between">
							<span className="text-xs font-medium">Link {index + 1}</span>
							<Button
								type="button"
								size="icon"
								variant="ghost"
								className="size-6"
								aria-label={`Remove link ${index + 1}`}
								onClick={() =>
									update(
										"links",
										links.filter((_, itemIndex) => itemIndex !== index),
									)
								}
							>
								<Trash2 className="size-3" />
							</Button>
						</div>
						{(["title", "description", "href"] as const).map((key) => (
							<Field
								key={key}
								label={
									key === "href"
										? "Destination"
										: key === "title"
											? "Title"
											: "Description"
								}
							>
								{(id) => (
									<Input
										id={id}
										value={link[key] ?? ""}
										placeholder={
											key === "href" ? "/use?id=… or https://…" : undefined
										}
										onChange={(event) =>
											update(
												"links",
												links.map((entry, itemIndex) =>
													itemIndex === index
														? { ...entry, [key]: event.target.value }
														: entry,
												),
											)
										}
									/>
								)}
							</Field>
						))}
					</div>
				))}
				<Button
					type="button"
					variant="outline"
					size="sm"
					onClick={() =>
						update("links", [
							...links,
							{
								id: crypto.randomUUID(),
								title: "New link",
								description: "",
								href: "",
							},
						])
					}
				>
					<Plus className="mr-1.5 size-3.5" />
					Add link
				</Button>
			</div>
		);
	}
	if (widget.type === "notifications" || widget.type === "needs-attention")
		return (
			<div className="space-y-4">
				{widget.type === "notifications" && (
					<label className="flex items-center gap-2 text-xs">
						<input
							type="checkbox"
							className="accent-primary"
							checked={config.unread === true}
							onChange={(event) => update("unread", event.target.checked)}
						/>
						Unread notifications only
					</label>
				)}
				<Choice
					label="Notification type"
					value={textConfig(
						config,
						"notificationType",
						widget.type === "needs-attention" ? "WORKFLOW" : "all",
					)}
					onChange={(value) => update("notificationType", value)}
					choices={[
						["all", "All personal notifications"],
						["WORKFLOW", "Workflow notifications"],
						["SYSTEM", "System notifications"],
					]}
				/>
				<Count config={config} update={update} />
				<p className="text-[11px] text-muted-foreground">
					These notifications belong to your account. Type filters search the
					latest 100 matching read/unread records.
				</p>
			</div>
		);
	if (["run-activity", "executions-by-app"].includes(widget.type))
		return (
			<div className="space-y-4">
				<Choice
					label="Time window"
					value={String(config.days ?? 7)}
					onChange={(value) => update("days", Number(value))}
					choices={[
						["1", "Today (UTC)"],
						["7", "Last 7 days (UTC)"],
						["30", "Last 30 days (UTC)"],
					]}
				/>
				<HomeAppPicker
					value={
						textConfig(config, "appId") ? [textConfig(config, "appId")] : []
					}
					onChange={(ids) => update("appId", ids[0] ?? "")}
					label="App (optional)"
					allowExplore={false}
				/>
				{widget.type === "executions-by-app" && (
					<Count config={config} update={update} />
				)}
				<p className="text-[11px] leading-relaxed text-muted-foreground">
					Uses your latest 100 recorded executions. Coverage is shown with the
					results so partial history stays visible.
				</p>
			</div>
		);
	if (widget.type === "ai-usage")
		return (
			<p className="text-xs leading-relaxed text-muted-foreground">
				Shows your account's recorded AI and embedding request totals and
				reported costs on this backend.
			</p>
		);

	if (widget.type === "run-stats")
		return (
			<div className="space-y-4">
				<Choice
					label="Measure"
					value={textConfig(config, "metric", "overview")}
					onChange={(value) => update("metric", value)}
					choices={[
						["overview", "Recorded usage overview"],
						["executions", "Total recorded executions"],
						["ai", "Total AI requests"],
						["embeddings", "Total embedding requests"],
						["errors", "Error-severity executions in latest 100"],
						["duration", "Average duration in latest 100"],
					]}
				/>
				{["errors", "duration"].includes(textConfig(config, "metric")) && (
					<HomeAppPicker
						value={
							textConfig(config, "appId") ? [textConfig(config, "appId")] : []
						}
						onChange={(ids) => update("appId", ids[0] ?? "")}
						label="Limit sample to an app (optional)"
						allowExplore={false}
					/>
				)}
				<p className="text-[11px] leading-relaxed text-muted-foreground">
					Totals cover your account's recorded history on this backend. Sample
					measures use your latest 100 execution records. Log severity does not
					establish a workflow success rate.
				</p>
			</div>
		);
	if (widget.type === "recent-runs")
		return (
			<div className="space-y-4">
				<HomeAppPicker
					value={
						textConfig(config, "appId") ? [textConfig(config, "appId")] : []
					}
					onChange={(ids) => update("appId", ids[0] ?? "")}
					label="App (optional)"
					allowExplore={false}
				/>
				<Count config={config} update={update} />
			</div>
		);
	if (widget.type === "schedules")
		return (
			<div className="space-y-4">
				<HomeAppPicker
					value={stringList(config, "appIds")}
					onChange={(ids) => update("appIds", ids)}
					label="Apps (leave empty for all in this profile)"
					multiple
					allowExplore={false}
				/>
				<Count config={config} update={update} />
				<p className="text-[11px] leading-relaxed text-muted-foreground">
					Upcoming times are calculated from active recurring and one-time
					events. A device or runner must be available for the event to execute.
				</p>
			</div>
		);
	return (
		<p className="text-sm text-muted-foreground">
			This widget has no additional content settings.
		</p>
	);
}
