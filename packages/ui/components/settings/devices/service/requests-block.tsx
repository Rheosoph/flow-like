"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy, Network } from "lucide-react";
import type { PlacementConfiguration } from "../../../../lib/device-management/deployment";
import {
	type EventRoute,
	ROUTE_EVENT_TYPES,
} from "../../../../lib/device-management/event-route";
import type { DevicesT } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { useCopy } from "../primitives/use-copy";
import { useAppView } from "../workspace";
import { ConfigStamp, Mono } from "./config-parts";
import type { AppEventRow } from "./service-events";
import {
	type DeployedRoute,
	type DeployedRoutesRead,
	type ServiceConfigRead,
	useDeployedRoutes,
} from "./use-service-config";

/* Service › Endpoint: the requests the service's Endpoints answer, as the version it runs has them, never as Events has them now. */

const BODY_METHODS = new Set(["POST", "PUT", "PATCH"]);

/** A `curl` line for the route, with a token placeholder when one is required. */
export function curlLine(method: string, url: string, token?: string): string {
	const auth = token ? ` -H "Authorization: Bearer ${token}"` : "";
	const body = BODY_METHODS.has(method)
		? ` -H "Content-Type: application/json" -d '{}'`
		: "";
	return `curl -X ${method}${auth}${body} "${url}"`;
}

/** An `http` or `api` event without a Page: a device answers its route. */
export const isEndpoint = (row: Pick<AppEventRow, "hasPage" | "eventType">) =>
	!row.hasPage &&
	(ROUTE_EVENT_TYPES as readonly string[]).includes(row.eventType);

/** Where the service is reached: `https://host:port`, or how much of it is known without the name people use. */
export interface RequestOrigin {
	/** Undefined while the address people use isn't known (a service on all networks). */
	origin: string | undefined;
	scheme: "http" | "https";
	port: number;
}

function CopyValue({
	value,
	label,
	act,
}: Readonly<{ value: string; label: string; act: string }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	return (
		<DvButton
			size="xs"
			variant="ghost"
			icon={copied ? Check : Copy}
			data-act={act}
			onClick={() => void copy(value)}
		>
			{copied ? t("serviceConfig.endpoint.copied", "Copied") : label}
		</DvButton>
	);
}

function unknownReason(
	t: DevicesT,
	reason: "live" | "failed" | "role",
	device: string,
) {
	const reasons = {
		live: () =>
			t(
				"devices:serviceEndpoint.requests.why.live",
				"connect live to read them from {{device}}",
				{ device },
			),
		failed: () =>
			t(
				"devices:serviceEndpoint.requests.why.failed",
				"reading them failed. Open this tab again to retry",
			),
		role: () =>
			t(
				"devices:serviceEndpoint.requests.why.role",
				"your role can't read this app's events",
			),
	};
	return t(
		"devices:serviceEndpoint.requests.unknown",
		"The paths of the deployed version can't be read here: {{reason}}",
		{ reason: reasons[reason]() },
	);
}

const sameRoute = (a: EventRoute | undefined, b: EventRoute | undefined) =>
	a?.method === b?.method && a?.path === b?.path;

const ITEM =
	"flex min-w-0 flex-col gap-1.5 border-t border-hairline py-2.5 text-ui first:border-t-0 first:pt-0 last:pb-0";
const VALUE =
	"rounded-md border border-border bg-surface-sunken px-2 py-1 font-mono text-xs";

/** The address a route is called at; the host stays "…" while the name people use isn't known. */
const addressOf = (where: RequestOrigin, path: string) =>
	(where.origin ?? [where.scheme, "://…:", where.port].join("")) + path;

/** The full address with Copy, once the host is known. */
function AddressLine({
	address,
	copyable,
}: Readonly<{ address: string; copyable: boolean }>) {
	const { t } = useTranslation("devices");
	return (
		<div className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
			<span
				data-request-address=""
				title={address}
				className={`min-w-0 max-w-full truncate ${VALUE}`}
			>
				{address}
			</span>
			{copyable ? (
				<CopyValue
					value={address}
					act="request-copy-address"
					label={t("serviceEndpoint.requests.copyAddress", "Copy address")}
				/>
			) : null}
		</div>
	);
}

/** A `curl` line that calls the route, with Copy. */
function CurlLine({
	route,
	address,
	requiresToken,
}: Readonly<{ route: EventRoute; address: string; requiresToken: boolean }>) {
	const { t } = useTranslation("devices");
	const command = curlLine(
		route.method,
		address,
		requiresToken
			? t("serviceEndpoint.requests.tokenPlaceholder", "<access token>")
			: undefined,
	);
	return (
		<div className="flex min-w-0 items-start gap-2">
			<code
				data-request-curl=""
				className={`min-w-0 flex-1 wrap-anywhere whitespace-pre-wrap ${VALUE}`}
			>
				{command}
			</code>
			<CopyValue
				value={command}
				act="request-copy-curl"
				label={t("serviceEndpoint.requests.copyCurl", "Copy command")}
			/>
		</div>
	);
}

function RequestItem({
	row,
	name,
	live,
	where,
	serviceId,
	requiresToken,
}: Readonly<{
	row: DeployedRoute;
	name: string;
	/** The route the event has in Events now, when that can be read. */
	live: EventRoute | undefined;
	where: RequestOrigin;
	serviceId: string;
	requiresToken: boolean;
}>) {
	const { t } = useTranslation("devices");
	const { route } = row;
	const changed = !!live && !!route && !sameRoute(live, route);
	return (
		<li data-request={row.eventId} className={ITEM}>
			<p className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5">
				<b className="font-semibold">{name}</b>
				{route ? (
					<span data-request-route="">
						<Mono>{`${route.method} ${route.path}`}</Mono>
					</span>
				) : null}
			</p>
			{route ? (
				<>
					<AddressLine
						address={addressOf(where, route.path)}
						copyable={!!where.origin}
					/>
					<CurlLine
						route={route}
						address={addressOf(where, route.path)}
						requiresToken={requiresToken}
					/>
				</>
			) : (
				<p className="text-ink-2">
					{t(
						"serviceEndpoint.requests.noRoute",
						"Its deployed version has no path a device answers.",
					)}
				</p>
			)}
			{changed ? (
				<p data-request-changed="" className="text-xs text-muted-foreground">
					{t(
						"serviceEndpoint.requests.changed",
						"Events has {{method}} {{path}} now. Update {{service}} to answer that instead.",
						{ method: live?.method, path: live?.path, service: serviceId },
					)}
				</p>
			) : null}
		</li>
	);
}

/** The service's Endpoints by the app's events: ids, names and the routes Events has now; null while the app's events can't be read. */
function useAppEndpoints(configuration: PlacementConfiguration) {
	const { view, loading } = useAppView(configuration.project_id);
	const rows = new Map(
		view
			? [...view.events.rows, ...view.events.ineligible].map((row) => [
					row.eventId,
					row,
				])
			: [],
	);
	const served = configuration.config.events.map((event) => event.event_id);
	return {
		rows,
		readable: !!view,
		loading,
		endpoints: served.filter(endpointOf(rows)),
	};
}

/** Whether an event id names an Endpoint in the app's list. */
const endpointOf =
	(rows: ReadonlyMap<string, AppEventRow>) =>
	(eventId: string): boolean => {
		const row = rows.get(eventId);
		return !!row && isEndpoint(row);
	};

/** What the block can say: the deployed routes, a reason they can't be read, or nothing for a service without Endpoints. */
type RequestList =
	| { routes: DeployedRoute[] }
	| { reason: "live" | "failed" | "role" }
	| null;

function listOf(
	deployed: DeployedRoutesRead,
	app: ReturnType<typeof useAppEndpoints>,
	online: boolean,
) {
	const none: RequestList = null;
	if (deployed.state === "known")
		return deployed.routes.length ? { routes: deployed.routes } : none;
	// Without the app's events an online service's pinned events can't be read either.
	if (online && !app.readable && !app.loading)
		return { reason: "role" as const };
	if (!app.endpoints.length) return none;
	return deployed.state === "unknown" ? { reason: deployed.reason } : none;
}

/**
 * Requests: one row per Endpoint the service answers, with the full address,
 * a `curl` line and who may call it. A route that Events has changed since
 * reads as the deployed one, with the new one beside it.
 */
export function RequestsBlock({
	deviceId,
	serviceId,
	read,
	configuration,
	where,
}: Readonly<{
	deviceId: string;
	serviceId: string;
	read: ServiceConfigRead;
	configuration: PlacementConfiguration;
	where: RequestOrigin;
}>) {
	const { t } = useTranslation("devices");
	const deployed = useDeployedRoutes(
		deviceId,
		configuration,
		read.gate === null,
	);
	const app = useAppEndpoints(configuration);
	const online = configuration.config.source === "online";
	const requiresToken = configuration.config.hosting?.authentication !== "none";
	const list: RequestList = listOf(deployed, app, online);
	if (!list) return null;
	const nameOf = (eventId: string) =>
		app.rows.get(eventId)?.name ??
		t("service.serves.removed", "A removed event");
	return (
		<Block
			id="svc-requests"
			icon={Network}
			title={t("serviceEndpoint.requests.title", "Requests")}
			{...("routes" in list ? { count: list.routes.length } : {})}
			stamp={<ConfigStamp read={read} />}
			foot={
				requiresToken
					? t(
							"serviceEndpoint.requests.token",
							"Every request needs this service's access token. The token set in Events is not used on a device.",
						)
					: t(
							"serviceEndpoint.requests.noToken",
							"Anyone who can reach this service can call these requests without a token. The token set in Events is not used on a device.",
						)
			}
		>
			{"routes" in list ? (
				<ul className="flex min-w-0 flex-col">
					{list.routes.map((row) => (
						<RequestItem
							key={row.eventId}
							row={row}
							name={nameOf(row.eventId)}
							live={app.rows.get(row.eventId)?.eligibility.route}
							where={where}
							serviceId={serviceId}
							requiresToken={requiresToken}
						/>
					))}
				</ul>
			) : (
				<p data-requests-unknown="" className="text-ui text-ink-2">
					{unknownReason(t, list.reason, read.deviceLabel)}
				</p>
			)}
			{"routes" in list && !where.origin ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"serviceEndpoint.requests.noAddress",
						"Enter the address people use under Service page to copy full addresses.",
					)}
				</p>
			) : null}
		</Block>
	);
}

/** The service's served events are all Endpoints: it is called by programs, not opened in a browser. */
export function useEndpointsOnly(configuration: PlacementConfiguration) {
	const { view } = useAppView(configuration.project_id);
	if (!view) return false;
	const rows = new Map(
		[...view.events.rows, ...view.events.ineligible].map((row) => [
			row.eventId,
			row,
		]),
	);
	const served = configuration.config.events.flatMap((event) => {
		const row = rows.get(event.event_id);
		return row?.eligibility.kind === "served" ? [row] : [];
	});
	return served.length > 0 && served.every(isEndpoint);
}
