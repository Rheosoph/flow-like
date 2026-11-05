import { eventEligibility } from "../../../../lib/device-management/deployment";
import type { EventRoute } from "../../../../lib/device-management/event-route";
import type {
	DeployPlan,
	PlanCheck,
	PlanTarget,
	PlanTargetService,
} from "../../../../lib/device-management/model/deploy-plan";
import type { DeployRunRow } from "../../../../lib/device-management/model/deploy-run";
import { exposureOf, serviceOrigin } from "../service/config-model";
import type { RequestOrigin } from "../service/requests-block";

/* Rollout's "Call it": each Endpoint a finished service now answers, where, and with which token. No React. */

export interface EndpointCall {
	key: string;
	target: string;
	deviceId: string;
	deviceName: string;
	serviceId: string;
	eventName: string;
	route: EventRoute;
	where: RequestOrigin;
	requiresToken: boolean;
	/** The service's new token while this page still holds it. */
	token?: string;
	/** A Docker device forwards only this port; the service's own is reached through a tunnel. */
	publishedPort?: number;
}

/** What `endpointCalls` reads of a run's result per target. */
export interface CallDetail {
	deviceName: string;
	serviceId: string;
	token?: string;
}

type Row = Pick<DeployRunRow, "target" | "deviceId" | "serviceId" | "state">;

/** The port the plan gave `service`: its own, or the target's for the first served service. */
const portOf = (target: PlanTarget, service: PlanTargetService) =>
	service.port ?? target.endpoint.port;

function serviceRoutes(plan: DeployPlan, service: PlanTargetService) {
	return service.events.flatMap((eventId) => {
		const event = plan.app?.events.find((row) => row.id === eventId);
		const route = event && eventEligibility(event).route;
		return event && route ? [{ eventId, eventName: event.name, route }] : [];
	});
}

function unpublished(
	check: Pick<PlanCheck, "exceptions"> | undefined,
	deviceId: string,
	port: number,
): number | undefined {
	const exception = check?.exceptions.find(
		(row) =>
			row.code === "port_unpublished" &&
			row.deviceId === deviceId &&
			row.params?.port === port,
	);
	const published = exception?.params?.published;
	return typeof published === "number" ? published : undefined;
}

/**
 * The calls of every finished target whose address, encryption and token the
 * plan decided. An update that keeps them is left out: the plan can't say what
 * the device has.
 */
export function endpointCalls(
	plan: DeployPlan,
	rows: readonly Row[],
	details: Readonly<Record<string, CallDetail | undefined>>,
	check?: Pick<PlanCheck, "exceptions">,
): EndpointCall[] {
	const mode = plan.draft.endpoint.token;
	if (mode === "keep") return [];
	return rows.flatMap((row) => {
		const detail = details[row.target];
		const target = plan.targets.find(
			(entry) => entry.deviceId === row.deviceId,
		);
		const service = target?.services.find(
			(entry) => entry.serviceId === (row.serviceId ?? detail?.serviceId),
		);
		const host = target?.endpoint.host;
		const certificateId = target?.endpoint.certificateId;
		const port = target && service ? portOf(target, service) : null;
		if (
			row.state !== "done" ||
			!detail ||
			!target ||
			!service ||
			host == null ||
			port === null ||
			certificateId === undefined
		)
			return [];
		const tls = certificateId !== null;
		const where: RequestOrigin = {
			origin: serviceOrigin({ host, port, exposure: exposureOf(host) }, tls),
			scheme: tls ? "https" : "http",
			port,
		};
		const publishedPort = unpublished(check, row.deviceId, port);
		return serviceRoutes(plan, service).map(
			({ eventId, eventName, route }) => ({
				key: `${row.target}/${eventId}`,
				target: row.target,
				deviceId: row.deviceId,
				deviceName: detail.deviceName,
				serviceId: service.serviceId,
				eventName,
				route,
				where,
				requiresToken: mode !== "none",
				...(mode !== "none" && detail.token ? { token: detail.token } : {}),
				...(publishedPort === undefined ? {} : { publishedPort }),
			}),
		);
	});
}
