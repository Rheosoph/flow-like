import { mergeBoardWithLocal } from "./board-merge";

interface MergeRequest {
	id: number;
	remote: Parameters<typeof mergeBoardWithLocal>[0];
	local?: Parameters<typeof mergeBoardWithLocal>[1];
}

self.onmessage = (event: MessageEvent<MergeRequest>) => {
	const { id, remote, local } = event.data;
	try {
		const result = mergeBoardWithLocal(remote, local);
		self.postMessage({ id, ok: true, result });
	} catch (error) {
		self.postMessage({ id, ok: false, error: String(error) });
	}
};
