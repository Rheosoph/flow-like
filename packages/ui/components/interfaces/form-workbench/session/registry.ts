/*
 * Finding a form session again after a remount (PLAN §3.7). Hosts remount the form (a Container
 * crossing 768 px, a sidebar toggle, a route key), so session controllers live in a module registry
 * keyed by the execution engine (a WeakMap: separate engines never share), the app, the event and
 * a partition (the host kind and its memory scope, so a profile or account switch never reopens
 * another person's session). A mount attaches a controller and detaches it when it goes; the
 * count of mounts is the controller's own. The first mount gets the shared controller; a second
 * mount while it is attached gets one of its own, which is never handed to another mount.
 */

/** What the registry needs of a session controller. */
export interface RegisteredController {
	attach(): void;
	detach(): void;
	isAttached(): boolean;
	isDisposed(): boolean;
	onDispose(listener: () => void): () => void;
}

const SHARED = new WeakMap<object, Map<string, RegisteredController>>();

/** The registry key of a form session. */
export const sessionKeyOf = (
	appId: string,
	eventId: string,
	partition = "",
): string => JSON.stringify([appId, eventId, partition]);

function sharedOf(engine: object) {
	let byKey = SHARED.get(engine);
	if (!byKey) {
		byKey = new Map();
		SHARED.set(engine, byKey);
	}
	return byKey;
}

/**
 * The shared controller of a form session, made with `factory` when there is none (or it ended).
 * Pure lookup apart from that: it attaches nothing, so a render may call it.
 */
export function getSessionController<C extends RegisteredController>(
	engine: object,
	appId: string,
	eventId: string,
	factory: () => C,
	partition = "",
): C {
	const byKey = sharedOf(engine);
	const key = sessionKeyOf(appId, eventId, partition);
	const existing = byKey.get(key) as C | undefined;
	if (existing && !existing.isDisposed()) return existing;
	const created = factory();
	byKey.set(key, created);
	created.onDispose(() => {
		if (byKey.get(key) === created) byKey.delete(key);
	});
	return created;
}

/**
 * The controller a mount attaches, without attaching it: the shared controller when no other mount
 * holds it, else a controller of its own (`factory`). A mount that already has its own keeps it. The
 * mount gives it its host parts before `attach()`, and never touches one another mount holds.
 */
export function claimSessionController<C extends RegisteredController>(
	engine: object,
	appId: string,
	eventId: string,
	held: C,
	factory: () => C,
	partition = "",
): C {
	const shared = getSessionController(
		engine,
		appId,
		eventId,
		factory,
		partition,
	);
	const candidate = held.isDisposed() ? shared : held;
	return candidate === shared && shared.isAttached() ? factory() : candidate;
}

/** `claimSessionController`, attached. */
export function attachSessionController<C extends RegisteredController>(
	engine: object,
	appId: string,
	eventId: string,
	held: C,
	factory: () => C,
	partition = "",
): C {
	const attached = claimSessionController(
		engine,
		appId,
		eventId,
		held,
		factory,
		partition,
	);
	attached.attach();
	return attached;
}

export function detachSessionController(
	controller: RegisteredController,
): void {
	controller.detach();
}
