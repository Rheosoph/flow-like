export interface NativeAutoModeRequest {
	runId: string;
	generation: string;
	expectedRevision: number;
	autoApprove: boolean;
}

interface NativeAutoModeResponse {
	applied: boolean;
	revision: number;
}

interface NativeAutoModeIdentity {
	generation: string;
	revision: number;
}

/** Keep the owning window's choice live across tools and delegated runs. */
export function createNativeAutoModeControl(options: {
	runId: string;
	read: () => boolean;
	subscribe: (changed: () => void) => () => void;
	update: (request: NativeAutoModeRequest) => Promise<NativeAutoModeResponse>;
}) {
	let control: NativeAutoModeIdentity | undefined;
	let active = true;
	let busy = false;
	let dirty = false;
	let retry: ReturnType<typeof setTimeout> | undefined;
	const synchronize = async () => {
		dirty = true;
		if (!active || busy || !control) return;
		busy = true;
		try {
			while (active && dirty && control) {
				dirty = false;
				const current: NativeAutoModeIdentity = control;
				const enabled = options.read();
				try {
					const response = await options.update({
						runId: options.runId,
						generation: current.generation,
						expectedRevision: current.revision,
						autoApprove: enabled,
					});
					if (!active) return;
					if (current !== control) {
						dirty = true;
						continue;
					}
					if (
						!Number.isSafeInteger(response.revision) ||
						response.revision < 0
					) {
						throw new Error("Invalid native approval mode revision");
					}
					control.revision = Math.max(control.revision, response.revision);
					dirty ||= !response.applied || enabled !== options.read();
				} catch {
					if (active) {
						dirty = true;
						retry = setTimeout(() => void synchronize(), 250);
					}
					return;
				}
			}
		} finally {
			busy = false;
		}
	};
	const unsubscribe = options.subscribe(() => void synchronize());
	return {
		accept(chunk: string): boolean {
			let frame: Record<string, unknown>;
			try {
				frame = JSON.parse(chunk);
			} catch {
				return false;
			}
			if (!frame || frame.type !== "native_run_control") return false;
			if (
				!active ||
				frame.run_id !== options.runId ||
				typeof frame.generation !== "string" ||
				!frame.generation ||
				typeof frame.auto_mode_revision !== "number" ||
				!Number.isSafeInteger(frame.auto_mode_revision) ||
				frame.auto_mode_revision < 0
			)
				return true;
			if (control?.generation === frame.generation) {
				control.revision = Math.max(control.revision, frame.auto_mode_revision);
			} else {
				control = {
					generation: frame.generation,
					revision: frame.auto_mode_revision,
				};
			}
			void synchronize();
			return true;
		},
		dispose() {
			active = false;
			unsubscribe();
			if (retry) clearTimeout(retry);
		},
	};
}
