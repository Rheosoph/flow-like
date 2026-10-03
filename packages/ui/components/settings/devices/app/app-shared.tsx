"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	Copy,
	KeyRound,
	LockOpen,
	type LucideIcon,
	Stethoscope,
} from "lucide-react";
import {
	type ComponentProps,
	type MouseEvent,
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useMemo,
	useState,
} from "react";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import type {
	AppUnknown,
	AppView,
} from "../../../../lib/device-management/model/app-plan";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	AppDevicesRoute,
	DeployRoute,
	DevicesRoute,
	DevicesScope,
	GateFailure,
} from "../../../../lib/device-management/model/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { UnknownPerson, identityName } from "../access/person-name";
import { gateCopy } from "../copy/gate-copy";
import { useAreaTime } from "../primitives/area-context";
import { DvButton, type DvButtonProps } from "../primitives/dv-button";
import { type Gate, GatedAction } from "../primitives/gate-notice";
import { PersonChip } from "../primitives/person-chip";
import { cx } from "../primitives/tone";
import { useCopy } from "../primitives/use-copy";
import {
	useDevicesRoute,
	useHostLink,
	useRouteLink,
} from "../routing/use-devices-route";
import { useAttentionState, useOverlay } from "../workspace";
import { capList, capNames } from "./app-view-local";
import type { AppDevicesData } from "./use-app-devices";

export interface AppPage {
	data: AppDevicesData;
	view: AppView;
	scope: DevicesScope;
	route: AppDevicesRoute;
	/** Opens the Update everywhere sheet. */
	openUpdateAll(): void;
	/** Why Update everywhere can't run now; null when it can. */
	updateAllGate: Gate | null;
}

const AppPageContext = createContext<AppPage | null>(null);

export function AppPageProvider({
	value,
	children,
}: Readonly<{ value: AppPage; children: ReactNode }>) {
	return (
		<AppPageContext.Provider value={value}>{children}</AppPageContext.Provider>
	);
}

export function useAppPage(): AppPage {
	const page = useContext(AppPageContext);
	if (!page) throw new Error("App › Devices blocks need AppPageProvider.");
	return page;
}

/** A status chip rendered as plain inline text (group rows, list lines). */
export const PLAIN_CHIP =
	"h-auto max-w-full rounded-none border-0 bg-transparent p-0 text-xs font-normal text-ink-2 [&>span]:overflow-visible [&>span]:whitespace-normal";

export const LINK =
	"text-foreground no-underline underline-offset-2 hover:underline focus-visible:underline";

export const MENU_CONTENT =
	"border-border-strong bg-popover shadow-none backdrop-blur-none";
export const MENU_ITEM =
	"items-start gap-2 text-[13px]/[18px] focus:bg-row-hover focus:text-foreground data-[disabled]:opacity-100";

/** An id that is a readable name (an app id): shown whole, with Copy. `IdRef` cuts every id to 8 characters. */
export function NameRef({
	id,
	label,
	copyLabel,
}: Readonly<{ id: string; label?: ReactNode; copyLabel: string }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	return (
		<span
			data-idref=""
			className="inline-flex max-w-full min-w-0 items-center gap-0.5 align-middle"
		>
			{label ? (
				<span className="mr-1 text-xs whitespace-nowrap text-muted-foreground">
					{label}
				</span>
			) : null}
			<code
				title={id}
				className="min-w-0 truncate rounded-sm border border-hairline bg-surface-sunken px-1.5 py-px font-mono text-xs text-ink-2"
			>
				{id}
			</code>
			<DvButton
				variant="ghost"
				size="xs"
				iconOnly
				icon={copied ? Check : Copy}
				aria-label={copied ? t("app.id.copied", "Copied") : copyLabel}
				onClick={() => void copy(id)}
				className="size-5.5 text-muted-foreground"
			/>
		</span>
	);
}

/** R11: a list shown up to `cap` until the viewer asks for the rest. */
export function useCapped<T>(list: readonly T[], cap: number) {
	const [all, setAll] = useState(false);
	const showAll = useCallback(() => setAll(true), []);
	const { shown, rest } = all
		? { shown: [...list], rest: [] as T[] }
		: capList(list, cap);
	return { shown, rest, showAll };
}

/** "Show 12 more" (R11); nothing when the list fits. */
export function ShowMore({
	count,
	onClick,
	act,
}: Readonly<{ count: number; onClick(): void; act?: string }>) {
	const { t } = useTranslation("devices");
	if (count <= 0) return null;
	return (
		<DvButton variant="link" size="xs" data-act={act} onClick={onClick}>
			{t("app.more.show", "Show {{count, number}} more", { count })}
		</DvButton>
	);
}

/** "a, b and c", or "a, b, c and 4 more" past `cap`. */
export function useNameList(): (
	names: readonly string[],
	cap: number,
) => string {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return useCallback(
		(names, cap) => {
			const { names: shown, more } = capNames(names, cap);
			return more
				? t("app.more.names", "{{names}} and {{count, number}} more", {
						names: shown.join(", "),
						count: more,
					})
				: new Intl.ListFormat(time.locale, { type: "conjunction" }).format(
						shown,
					);
		},
		[t, time.locale],
	);
}

/** "today 13:00" for a time of today, the date and time otherwise (APP §2.9, §2.11), like the strip above. */
export function useDayTime(): (atS: number) => string {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return useCallback(
		(atS) => {
			const at = time.at(atS);
			return at === time.clock(atS, false)
				? t("app.when.today", "today {{time}}", { time: at })
				: at;
		},
		[t, time],
	);
}

/** A failed gate as the primitives take it: kind + the one-line reason (R7). */
export function useGateText(): (
	gate: GateFailure | null | undefined,
) => Gate | null {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return useCallback(
		(gate) =>
			gate ? { kind: gate.kind, reason: gateCopy(t, gate, time).inline } : null,
		[t, time],
	);
}

/** Device id → name for every device the viewer can see. */
export function useDeviceNames(): (deviceId: string) => string {
	const { data } = useAppPage();
	return useCallback(
		(deviceId: string) => {
			const view = data.devices.get(deviceId);
			return view ? deviceName(view.row) : deviceId.slice(0, 8);
		},
		[data.devices],
	);
}

interface DeployLink {
	deviceIds?: string[];
	eventId?: string;
	serviceId?: string;
	mode?: "new" | "update";
	step?: "what" | "settings" | "copy_upload";
}

export interface AppLinks {
	device(deviceId: string): DevicesRoute;
	service(deviceId: string, serviceId: string): DevicesRoute;
	logs(deviceId: string, serviceId: string): DevicesRoute;
	deploy(link?: DeployLink): DevicesRoute;
}

/** Into the deploy flow: an update of one service, or a new deploy. */
function deployRoute(link: DeployLink = {}): DeployRoute {
	const route: DeployRoute = {
		screen: "deploy",
		deviceIds: link.deviceIds ?? [],
	};
	if (link.serviceId) route.serviceId = link.serviceId;
	else route.mode = link.mode ?? "new";
	if (link.eventId) route.eventId = link.eventId;
	if (link.step) route.step = link.step;
	return route;
}

/** APP §6.1: every link of the page opens in this app's context. */
export const APP_LINKS: AppLinks = {
	device(deviceId) {
		return { screen: "device", deviceId, tab: "services" };
	},
	service(deviceId, serviceId) {
		return { screen: "service", deviceId, serviceId, tab: "status" };
	},
	logs(deviceId, serviceId) {
		return { screen: "service", deviceId, serviceId, tab: "activity" };
	},
	deploy: deployRoute,
};

export interface MenuEntry {
	id: string;
	label: string;
	icon?: LucideIcon;
	route?: DevicesRoute;
	scope?: DevicesScope;
	href?: string;
	onSelect?(): void;
	/** The reason the entry can't run now; it stays visible (R7). */
	blocked?: string | null;
	note?: string;
	danger?: boolean;
	separated?: boolean;
}

function MenuLabel({ entry }: Readonly<{ entry: MenuEntry }>) {
	const Icon = entry.icon;
	const sub = entry.blocked ?? entry.note;
	return (
		<>
			{Icon ? <Icon aria-hidden className="mt-0.5 size-3.5 shrink-0" /> : null}
			<span className="flex min-w-0 flex-col">
				<span>{entry.label}</span>
				{sub ? (
					<span className="max-w-[36ch] text-xs whitespace-normal text-muted-foreground">
						{sub}
					</span>
				) : null}
			</span>
		</>
	);
}

/**
 * A link to another page of the host app (its Events page): a plain click goes
 * through the host's router, so the page isn't loaded anew and the key
 * sessions stay open.
 */
export function HostLink({
	href,
	onClick,
	...props
}: Readonly<ComponentProps<"a"> & { href: string }>) {
	const anchor = useHostLink()(href);
	return (
		<a
			{...props}
			href={anchor.href}
			onClick={(event) => {
				onClick?.(event);
				if (!event.defaultPrevented) anchor.onClick(event);
			}}
		/>
	);
}

/** Where an entry leads inside the area; a blocked entry leads nowhere. */
function entryAnchor(entry: MenuEntry, link: ReturnType<typeof useRouteLink>) {
	if (entry.blocked || !entry.route) return null;
	return link(entry.route, entry.scope ? { scope: entry.scope } : undefined);
}

function MenuEntryItem({ entry }: Readonly<{ entry: MenuEntry }>) {
	const link = useRouteLink();
	const className = cx(
		MENU_ITEM,
		entry.danger && "text-danger focus:text-danger",
	);
	const anchor = entryAnchor(entry, link);
	if (anchor)
		return (
			<DropdownMenuItem asChild className={className}>
				<a data-menu-item={entry.id} {...anchor}>
					<MenuLabel entry={entry} />
				</a>
			</DropdownMenuItem>
		);
	if (entry.href && !entry.blocked)
		return (
			<DropdownMenuItem asChild className={className}>
				<HostLink data-menu-item={entry.id} href={entry.href}>
					<MenuLabel entry={entry} />
				</HostLink>
			</DropdownMenuItem>
		);
	return (
		<DropdownMenuItem
			data-menu-item={entry.id}
			disabled={!!entry.blocked}
			onSelect={entry.blocked ? undefined : entry.onSelect}
			className={className}
		>
			<MenuLabel entry={entry} />
		</DropdownMenuItem>
	);
}

function MenuEntries({ entries }: Readonly<{ entries: readonly MenuEntry[] }>) {
	return entries.map((entry) => (
		<span key={entry.id} className="contents">
			{entry.separated ? <DropdownMenuSeparator /> : null}
			<MenuEntryItem entry={entry} />
		</span>
	));
}

/** A menu whose gated entries stay visible with their reason (R7); the entries mount only while it is open. */
export function AppMenu({
	trigger,
	entries,
	align = "end",
	className,
}: Readonly<{
	trigger: ReactNode;
	entries: () => readonly MenuEntry[];
	align?: "start" | "end";
	className?: string;
}>) {
	return (
		<DropdownMenu modal={false}>
			<DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
			<DropdownMenuContent
				align={align}
				className={cx(MENU_CONTENT, "min-w-56", className)}
			>
				<LazyEntries entries={entries} />
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function LazyEntries({
	entries,
}: Readonly<{ entries: () => readonly MenuEntry[] }>) {
	const list = useMemo(() => entries(), [entries]);
	return <MenuEntries entries={list} />;
}

type LinkButtonProps = Omit<DvButtonProps, "asChild" | "onClick"> & {
	route: DevicesRoute;
	scope?: DevicesScope;
	gate?: Gate | null;
	/** The reason is rendered by the caller next to the row. */
	reasonShown?: boolean;
	/** `data-act` of the control. */
	act?: string;
};

/**
 * A button that is a link into another screen. Gated, it is a disabled button
 * without the link (R7: a click sends nothing), with its reason underneath
 * unless the row already states it.
 */
export function LinkButton(props: Readonly<LinkButtonProps>) {
	const { route, scope, gate, reasonShown, act, children, ...button } = props;
	const link = useRouteLink();
	if (gate) {
		const disabled = (
			<DvButton {...button} data-act={act}>
				{children}
			</DvButton>
		);
		return reasonShown ? (
			<DvButton {...button} data-act={act} data-gated={gate.kind} aria-disabled>
				{children}
			</DvButton>
		) : (
			<GatedAction gate={gate}>{disabled}</GatedAction>
		);
	}
	return (
		<DvButton {...button} asChild>
			<a data-act={act} {...link(route, scope ? { scope } : undefined)}>
				{children}
			</a>
		</DvButton>
	);
}

/**
 * Primitives that take a plain `href` would reload the page (and lock every
 * key with it): a click on one of the known routes navigates inside the area.
 */
export function useLinkCapture(
	routes: readonly DevicesRoute[],
): (event: MouseEvent<HTMLElement>) => void {
	const { href, navigate } = useDevicesRoute();
	const known = useMemo(
		() => new Map(routes.map((route) => [href(route), route])),
		[routes, href],
	);
	return useCallback(
		(event) => {
			if (
				event.defaultPrevented ||
				event.button !== 0 ||
				event.metaKey ||
				event.ctrlKey ||
				event.shiftKey ||
				event.altKey
			)
				return;
			const anchor = (event.target as HTMLElement).closest?.("a[href]");
			const route = anchor
				? known.get(anchor.getAttribute("href") ?? "")
				: undefined;
			if (!route) return;
			event.preventDefault();
			navigate(route);
		},
		[known, navigate],
	);
}

export { EventTile, eventIcon } from "../primitives/event-cell";

/**
 * A person by account id: "You" for the viewer, the directory name otherwise;
 * never the account id as the name. `showId` puts it on a second line for an
 * account without a name, where it is the only way to tell who it is.
 */
export function Person({
	userId,
	showId = false,
	className,
}: Readonly<{ userId: string; showId?: boolean; className?: string }>) {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const identity = useUserIdentity(userId);
	const you = userId === input.me;
	const name = identityName(identity, userId);
	if (!name && !you)
		return (
			<UnknownPerson
				pending={identity.isPending}
				unreachable={!identity.isResolved}
				className={className}
				{...(showId ? { userId } : {})}
			/>
		);
	return (
		<PersonChip
			name={name ?? t("app.person.you", "You")}
			you={you}
			{...(identity.subtitle ? { email: identity.subtitle } : {})}
			{...(identity.avatarUrl ? { avatarUrl: identity.avatarUrl } : {})}
			className={className}
		/>
	);
}

/** The one action that makes an unknown device readable: Unlock…, Restore keys…, Diagnose. */
export function UnknownAction({
	deviceId,
	unknown,
	size = "sm",
}: Readonly<{
	deviceId: string;
	unknown: Pick<AppUnknown, "kind"> | { kind: "snapshot" };
	size?: "sm" | "xs";
}>) {
	const { t } = useTranslation("devices");
	const overlay = useOverlay();
	const link = useRouteLink();
	if (unknown.kind === "locked")
		return (
			<DvButton
				size={size}
				icon={LockOpen}
				data-act="unlock"
				onClick={() => overlay.openUnlock(deviceId)}
			>
				{t("app.unknown.unlock", "Unlock…")}
			</DvButton>
		);
	if (unknown.kind === "nokeys")
		return (
			<DvButton size={size} icon={KeyRound} asChild>
				<a
					data-act="restore-keys"
					{...link({ screen: "keys", focusDeviceId: deviceId })}
				>
					{t("app.unknown.restoreKeys", "Restore keys…")}
				</a>
			</DvButton>
		);
	if (unknown.kind === "noaccess" || unknown.kind === "never") return null;
	return (
		<DvButton
			size={size}
			icon={Stethoscope}
			data-act="diagnose"
			onClick={() => overlay.openDiagnose(deviceId)}
		>
			{t("app.unknown.diagnose", "Diagnose")}
		</DvButton>
	);
}
