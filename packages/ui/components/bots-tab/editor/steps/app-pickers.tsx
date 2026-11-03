"use client";

import { useTranslation } from "@flow-like/locales";
import { AppWindow, Search } from "lucide-react";
import {
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { useBotApps, useSetBotApp } from "../../../../hooks/use-bots";
import { useInvoke } from "../../../../hooks/use-invoke";
import { IAppVisibility } from "../../../../lib/schema/app/app";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import { Checkbox } from "../../../ui/checkbox";
import { Input } from "../../../ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { Skeleton } from "../../../ui/skeleton";
import { useBotErrorMessage } from "../bot-editor-copy";

interface AppOption {
	id: string;
	name: string;
	/** Edit mode only: the owner's role in the app, or null once they left it. */
	role?: string | null;
}

function profileOptions(profiles: readonly IProfile[] | undefined) {
	const options: { id: string; name: string; apps: number }[] = [];
	for (const profile of profiles ?? []) {
		if (profile.id)
			options.push({
				id: profile.id,
				name: profile.name,
				apps: profile.apps?.length ?? 0,
			});
	}
	return options;
}

export function BotProfilePicker({
	id,
	value,
	onChange,
}: Readonly<{
	id?: string;
	value: string | null;
	onChange: (profileId: string) => void;
}>) {
	const { t } = useTranslation("bots");
	const backend = useBackend();
	const profiles = useInvoke(
		backend.userState.getProfiles,
		backend.userState,
		[],
	);
	const options = profileOptions(profiles.data);
	const onlyProfileId = options.length === 1 ? options[0].id : null;
	useEffect(() => {
		if (!value && onlyProfileId) onChange(onlyProfileId);
	}, [onChange, onlyProfileId, value]);
	if (profiles.isLoading) return <Skeleton className="h-9 w-full" />;
	return (
		<Select value={value ?? ""} onValueChange={onChange}>
			<SelectTrigger id={id} className="w-full">
				<SelectValue placeholder={t("editor.abilities.profilePlaceholder")} />
			</SelectTrigger>
			<SelectContent>
				{options.map((profile) => (
					<SelectItem key={profile.id} value={profile.id}>
						{profile.name}
						<span className="text-muted-foreground">
							{" · "}
							{t("editor.abilities.profileApps", { count: profile.apps })}
						</span>
					</SelectItem>
				))}
				{options.length === 0 && (
					<p className="px-2 py-3 text-xs text-muted-foreground">
						{t("editor.abilities.profileEmpty")}
					</p>
				)}
			</SelectContent>
		</Select>
	);
}

function useLibraryApps(): { apps: AppOption[]; isLoading: boolean } {
	const backend = useBackend();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	return useMemo(
		() => ({
			isLoading: apps.isLoading,
			apps: (apps.data ?? [])
				.filter(([app]) => app.visibility !== IAppVisibility.Offline)
				.map(([app, metadata]) => ({ id: app.id, name: metadata?.name ?? "" })),
		}),
		[apps.data, apps.isLoading],
	);
}

/** Apps for a bot that doesn't exist yet: the picks are attached right after it is created. */
export function PendingAppPicker({
	selected,
	onChange,
}: Readonly<{
	selected: readonly string[];
	onChange: (ids: string[]) => void;
}>) {
	const { apps, isLoading } = useLibraryApps();
	const isChecked = useCallback(
		(appId: string) => selected.includes(appId),
		[selected],
	);
	const toggle = useCallback(
		(appId: string, checked: boolean) =>
			onChange(
				checked
					? [...selected, appId]
					: selected.filter((existing) => existing !== appId),
			),
		[onChange, selected],
	);
	return (
		<AppChecklist
			apps={apps}
			isLoading={isLoading}
			isChecked={isChecked}
			onToggle={toggle}
		/>
	);
}

/** A tick that is still being saved; `ticket` tells a newer tick of the same app from an older one. */
interface PendingAppChange {
	checked: boolean;
	ticket: number;
}

/** Apps of an existing bot: every change is saved right away. */
export function AttachedAppPicker({ botId }: Readonly<{ botId: string }>) {
	const { t } = useTranslation("bots");
	const errorMessage = useBotErrorMessage();
	const library = useLibraryApps();
	const attached = useBotApps(botId);
	const { mutateAsync: setApp } = useSetBotApp();
	const [pending, setPending] = useState<Record<string, PendingAppChange>>({});
	const tickets = useRef(0);
	const apps = useMemo(() => {
		const roles = new Map(
			(attached.data ?? []).map((entry) => [entry.app_id, entry.owner_role]),
		);
		const listed = library.apps.map((app) => ({
			...app,
			role: roles.get(app.id),
		}));
		const missing = [...roles.keys()]
			.filter((appId) => !library.apps.some((app) => app.id === appId))
			.map((appId) => ({ id: appId, name: "", role: roles.get(appId) }));
		return [...listed, ...missing];
	}, [attached.data, library.apps]);
	const isChecked = useCallback(
		(appId: string) =>
			pending[appId]?.checked ??
			(attached.data ?? []).some((entry) => entry.app_id === appId),
		[attached.data, pending],
	);
	const toggle = useCallback(
		async (appId: string, checked: boolean) => {
			tickets.current += 1;
			const ticket = tickets.current;
			setPending((current) => ({ ...current, [appId]: { checked, ticket } }));
			try {
				await setApp({ botId, appId, attached: checked });
			} catch (error) {
				toast.error(errorMessage(error, t("editor.abilities.appsFailed")));
			} finally {
				setPending((current) => {
					if (current[appId]?.ticket !== ticket) return current;
					const { [appId]: _, ...rest } = current;
					return rest;
				});
			}
		},
		[botId, errorMessage, setApp, t],
	);
	return (
		<AppChecklist
			apps={apps}
			isLoading={library.isLoading || attached.isLoading}
			isChecked={isChecked}
			onToggle={toggle}
		/>
	);
}

function AppChecklist({
	apps,
	isLoading,
	isChecked,
	onToggle,
}: Readonly<{
	apps: readonly AppOption[];
	isLoading: boolean;
	isChecked: (appId: string) => boolean;
	onToggle: (appId: string, checked: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	const [search, setSearch] = useState("");
	const query = search.trim().toLocaleLowerCase();
	const visible = apps
		.filter((app) => app.name.toLocaleLowerCase().includes(query))
		.sort((a, b) => a.name.localeCompare(b.name));
	const count = apps.filter((app) => isChecked(app.id)).length;
	if (isLoading) return <Skeleton className="h-28 w-full" />;
	if (!apps.length)
		return (
			<p className="text-xs text-muted-foreground">
				{t("editor.abilities.appsEmpty")}
			</p>
		);
	return (
		<div className="flex flex-col gap-2">
			<div className="flex items-center gap-2">
				<div className="relative flex-1">
					<Search
						className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground"
						aria-hidden
					/>
					<Input
						type="search"
						value={search}
						aria-label={t("editor.abilities.appsSearch")}
						placeholder={t("editor.abilities.appsSearch")}
						onChange={(event) => setSearch(event.target.value)}
						className="h-8 pl-8 text-xs"
					/>
				</div>
				<span
					aria-live="polite"
					className="text-xs whitespace-nowrap text-muted-foreground"
				>
					{t("editor.abilities.appsCount", { count })}
				</span>
			</div>
			<ul className="max-h-56 divide-y divide-border overflow-y-auto rounded-lg border border-border">
				{visible.map((app) => (
					<AppRow
						key={app.id}
						app={app}
						checked={isChecked(app.id)}
						onToggle={onToggle}
					/>
				))}
				{visible.length === 0 && (
					<li className="px-3 py-3 text-xs text-muted-foreground">
						{t("editor.abilities.appsNoMatch")}
					</li>
				)}
			</ul>
		</div>
	);
}

function AppRow({
	app,
	checked,
	onToggle,
}: Readonly<{
	app: AppOption;
	checked: boolean;
	onToggle: (appId: string, checked: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	const id = useId();
	return (
		<li className="flex items-center gap-3 px-3 py-2 text-sm hover:bg-muted/60">
			<Checkbox
				id={id}
				checked={checked}
				onCheckedChange={(next) => onToggle(app.id, next === true)}
			/>
			<label
				htmlFor={id}
				className="flex min-w-0 flex-1 cursor-pointer items-center gap-3"
			>
				<AppWindow
					className="size-4 shrink-0 text-muted-foreground"
					aria-hidden
				/>
				<span className="min-w-0 flex-1 truncate">
					{app.name || t("editor.abilities.appsUntitled")}
				</span>
				{app.role === null && checked && (
					<span className="text-[0.6875rem] text-warning">
						{t("editor.abilities.appsLeft")}
					</span>
				)}
				{app.role && (
					<span className="text-[0.6875rem] text-muted-foreground">
						{t("editor.abilities.appsRole", { role: app.role })}
					</span>
				)}
			</label>
		</li>
	);
}
