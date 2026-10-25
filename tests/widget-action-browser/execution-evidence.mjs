/** Parses only execution metadata. Event bodies are never returned or retained in reports. */
export function createExecutionParser() {
	const state = {
		terminalObserved: false,
		errorEvent: false,
		errorPayload: false,
		runIdMismatch: false,
		status: null,
		logLevel: null,
		runId: null,
	};
	let buffer = "";
	let data = [];
	const consume = (raw) => {
		let event;
		try {
			event = JSON.parse(raw);
		} catch {
			return;
		}
		if (!event || typeof event !== "object") return;
		const payload = event.payload ?? event;
		if (!payload || typeof payload !== "object") return;
		const type = event.event_type ?? event.type;
		if (
			!["run_initiated", "completed", "execution_complete", "error"].includes(
				type,
			)
		)
			return;
		const runId = payload.run_id ?? payload.runId;
		if (typeof runId === "string" && /^[\w-]{1,256}$/.test(runId)) {
			if (state.runId && state.runId !== runId) state.runIdMismatch = true;
			state.runId = runId;
		}
		if (type === "error") {
			state.errorEvent = state.terminalObserved = true;
			state.status ??= "error";
		}
		if (type === "completed" || type === "execution_complete") {
			state.terminalObserved = true;
			if (typeof payload.status === "string") {
				const status = payload.status.toLowerCase();
				state.status = [
					"completed",
					"success",
					"succeeded",
					"failed",
					"error",
					"cancelled",
					"canceled",
					"running",
				].includes(status)
					? status
					: "unknown";
			}
			if (Number.isInteger(payload.log_level))
				state.logLevel = payload.log_level;
			if (
				payload.error !== undefined &&
				payload.error !== null &&
				payload.error !== false &&
				payload.error !== ""
			)
				state.errorPayload = true;
		}
	};
	const flush = () => {
		if (data.length) consume(data.join("\n"));
		data = [];
	};
	return {
		feed(chunk) {
			buffer += chunk;
			let newline;
			while ((newline = buffer.indexOf("\n")) !== -1) {
				const line = buffer.slice(0, newline).replace(/\r$/, "");
				buffer = buffer.slice(newline + 1);
				if (!line) flush();
				else if (line.startsWith("data:"))
					data.push(line.slice(5).replace(/^ /, ""));
				else if (line.trimStart().startsWith("{")) consume(line);
			}
			if (
				buffer.length > 16 * 1024 * 1024 ||
				data.reduce((size, part) => size + part.length, 0) > 16 * 1024 * 1024
			) {
				buffer = "";
				data = [];
				state.observationError = "event_size_limit";
			}
		},
		end() {
			if (buffer.trimStart().startsWith("{")) consume(buffer);
			else if (buffer.startsWith("data:"))
				data.push(buffer.slice(5).replace(/^ /, ""));
			buffer = "";
			flush();
		},
		snapshot() {
			return {
				...state,
				success:
					state.terminalObserved &&
					["completed", "success", "succeeded"].includes(state.status) &&
					!state.errorEvent &&
					!state.errorPayload &&
					!state.runIdMismatch &&
					!state.observationError &&
					!(state.logLevel >= 3),
			};
		},
	};
}

/** CDP observes the real response; it never intercepts or replaces application traffic. */
export function observeExecution(cdp, requestId, record) {
	const parser = createExecutionParser();
	const decoder = new TextDecoder();
	let queued = [];
	let streaming = false;
	let enabled;
	let finish;
	let resolveEvidence;
	const evidence = new Promise((resolve) => {
		resolveEvidence = resolve;
	});
	const update = (parsed, source) => {
		record.execution = { ...parsed, source };
		if (parsed.terminalObserved) resolveEvidence();
	};
	const feed = (encoded) => {
		if (!encoded) return;
		parser.feed(
			decoder.decode(Buffer.from(encoded, "base64"), { stream: true }),
		);
		update(parser.snapshot(), "cdp-stream");
	};
	return {
		start() {
			enabled = cdp
				.send("Network.streamResourceContent", { requestId })
				.then(({ bufferedData }) => {
					feed(bufferedData);
					for (const chunk of queued) feed(chunk);
					queued = [];
					streaming = true;
				})
				.catch(() => {
					queued = [];
					record.streamObservationAvailable = false;
				});
		},
		data(encoded) {
			if (streaming) feed(encoded);
			else if (encoded) queued.push(encoded);
		},
		finish() {
			if (finish) return finish;
			finish = (async () => {
				await enabled;
				try {
					const response = await cdp.send("Network.getResponseBody", {
						requestId,
					});
					const bodyParser = createExecutionParser();
					bodyParser.feed(
						response.base64Encoded
							? Buffer.from(response.body, "base64").toString("utf8")
							: response.body,
					);
					bodyParser.end();
					if (bodyParser.snapshot().terminalObserved) {
						update(bodyParser.snapshot(), "cdp-response-body");
						return;
					}
				} catch {
					record.responseBodyAvailable = false;
				}
				parser.feed(decoder.decode());
				parser.end();
				update(parser.snapshot(), "cdp-stream");
			})();
			return finish;
		},
		async wait(timeoutMs) {
			if (record.execution?.terminalObserved) return;
			let timer;
			try {
				await Promise.race([
					evidence,
					new Promise((resolve) => {
						timer = setTimeout(resolve, timeoutMs);
					}),
				]);
			} finally {
				clearTimeout(timer);
			}
		},
	};
}
