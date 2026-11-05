import type { BoardHitIndex, Point, WorldPin } from "./interaction";
import type { SceneEdge, SceneEndpoint } from "./scene";

export interface PinStart {
	pin: WorldPin;
	reconnect?: { edge: SceneEdge; end: "source" | "target" };
}

function insidePin(point: Point, pin: Point & Pick<WorldPin, "kind">) {
	const dx = Math.abs(point.x - pin.x);
	const dy = Math.abs(point.y - pin.y);
	return pin.kind === "reference"
		? Math.max(dx, dy) <= 6
		: pin.kind === "execution"
			? dx + dy <= 5.657
			: Math.hypot(dx, dy) <= 3.5;
}

/** Hover and press share endpoint priority and the same screen-space snap radius. */
export function resolvePinStart(
	index: BoardHitIndex,
	point: Point,
	zoom: number,
	options: {
		explicit?: SceneEndpoint;
		explicitEdgeId?: string;
		blocked?: boolean;
		nodeId?: string;
		canConnect: (pin: WorldPin) => boolean;
		canReconnect?: (edge: SceneEdge, end: "source" | "target") => boolean;
	},
): PinStart | undefined {
	if (options.blocked) return;
	const allowed = (pin: WorldPin) =>
		(!options.nodeId || pin.nodeId === options.nodeId) &&
		options.canConnect(pin);
	const node = options.nodeId
		? index.nodes.get(options.nodeId)
		: index.nodeAt(point);
	// Transparent Handle hitboxes and fuzzy radii must leave the reroute body draggable.
	if (
		node?.kind === "reroute" &&
		point.x > node.x &&
		point.x < node.x + node.width &&
		point.y >= node.y &&
		point.y <= node.y + node.height &&
		!node.pins.some((pin) =>
			insidePin(point, { ...pin, x: node.x + pin.x, y: node.y + pin.y }),
		)
	)
		return;
	if (options.explicit) {
		const pin = index.pin(options.explicit);
		return pin && allowed(pin) ? { pin } : undefined;
	}
	const visiblePin = index.closestPin(
		point,
		zoom,
		(pin) => allowed(pin) && insidePin(point, pin),
		Math.max(12, 9 * zoom),
	);
	// A pin starts a new wire regardless of selection. Reconnecting its selected wire would
	// anchor the opposite pin and reverse which direction the user can connect to.
	if (visiblePin) return { pin: visiblePin };
	// At overview zoom, a fuzzy pin radius can otherwise cover the entire node.
	if (
		node &&
		point.x > node.x &&
		point.x < node.x + node.width &&
		point.y > node.y &&
		point.y < node.y + node.height
	)
		return;
	const nearWire = index.nearEndpoint(
		point,
		zoom,
		allowed,
		options.explicitEdgeId,
		options.canReconnect,
	);
	if (nearWire) {
		const pin = index.pin(nearWire.edge[nearWire.end]);
		if (pin) return { pin, reconnect: nearWire };
	}
	const reconnect = index.selectedEndpoint(point, zoom, options.canReconnect);
	if (reconnect) {
		const pin = index.pin(reconnect.edge[reconnect.end]);
		if (pin && allowed(pin)) return { pin, reconnect };
	}
	const pin = index.closestPin(point, zoom, allowed, 12);
	return pin ? { pin } : undefined;
}
