import { type SnapshotRequest, createSnapshotEngine } from "./snapshot-engine";

const engine = createSnapshotEngine();

self.onmessage = async (
	event: MessageEvent<{ id: number; request: SnapshotRequest }>,
) => {
	const { id, request } = event.data;
	try {
		self.postMessage({ id, ok: true, response: await engine.handle(request) });
	} catch (error) {
		self.postMessage({ id, ok: false, error: String(error) });
	}
};
