import type { ServerResponse } from "node:http";
import type { Plugin } from "vite";

interface FixtureRun {
	taskId: string;
	response: ServerResponse;
	closed: boolean;
	sequence: number;
}

/** An observable SSE endpoint; browser tests drive events without replacing runtime code. */
export function executionFixtureServer(): Plugin {
	const runs = new Map<string, FixtureRun>();
	const starts: string[] = [];
	const replies: unknown[] = [];
	const cancellations: string[] = [];
	const send = (run: FixtureRun, event: Record<string, unknown>) => {
		if (run.closed) throw new Error(`Stream ${run.taskId} is closed`);
		run.response.write(
			`event: ${event.event_type === "completed" ? "done" : "message"}\ndata: ${JSON.stringify({ event_id: `${run.taskId}:${run.sequence++}`, ...event })}\n\n`,
		);
		if (event.event_type === "completed") run.response.end();
	};
	return {
		name: "execution-fixture-server",
		configureServer(server) {
			server.middlewares.use(async (request, response, next) => {
				const path = request.url?.split("?")[0] ?? "";
				if (!path.startsWith("/execution-fixture/")) return next();
				try {
					const chunks: Buffer[] = [];
					for await (const chunk of request) chunks.push(Buffer.from(chunk));
					const body = chunks.length
						? JSON.parse(Buffer.concat(chunks).toString())
						: {};
					const json = (value: unknown) => {
						response.setHeader("content-type", "application/json");
						response.end(JSON.stringify(value));
					};
					if (path === "/execution-fixture/stats")
						return json({
							starts,
							replies,
							cancellations,
							runs: [...runs.values()].map(({ taskId, closed }) => ({
								taskId,
								closed,
							})),
						});
					if (path === "/execution-fixture/reset") {
						for (const run of runs.values()) run.response.end();
						runs.clear();
						starts.length = 0;
						replies.length = 0;
						cancellations.length = 0;
						return json({ ok: true });
					}
					if (path === "/execution-fixture/start") {
						const taskId = String(body.taskId);
						starts.push(taskId);
						response.writeHead(200, {
							"content-type": "text/event-stream",
							"cache-control": "no-cache",
							connection: "keep-alive",
						});
						response.flushHeaders();
						const run = { taskId, response, closed: false, sequence: 0 };
						runs.set(taskId, run);
						response.on("close", () => {
							run.closed = true;
						});
						if (!body.delayRunId)
							send(run, {
								event_type: "run_initiated",
								payload: { run_id: `run-${taskId}` },
							});
						send(run, {
							event_type: "chat_stream",
							payload: {
								message_id: `message-${taskId}`,
								response: {
									choices: [
										{
											index: 0,
											message: {
												role: "assistant",
												content: "Started processing your request.",
											},
										},
									],
								},
							},
						});
						return;
					}
					if (path === "/execution-fixture/emit") {
						const run = runs.get(body.taskId);
						if (!run) throw new Error(`Unknown run ${body.taskId}`);
						send(run, body.event);
						return json({ ok: true });
					}
					if (path === "/execution-fixture/push") {
						if (body.kind === "reply") replies.push(body);
						else if (body.kind === "cancel")
							cancellations.push(`channel:${body.channel_id}`);
						// Keep requests in flight long enough for two tabs to race their replies.
						setTimeout(() => json({ delivered: true }), 75);
						return;
					}
					if (
						path.startsWith("/execution-fixture/cancel/") &&
						request.method === "DELETE"
					) {
						cancellations.push(
							decodeURIComponent(
								path.slice("/execution-fixture/cancel/".length),
							),
						);
						return json({ ok: true });
					}
					response.statusCode = 404;
					json({ error: "Unknown fixture endpoint" });
				} catch (error) {
					response.statusCode = 500;
					response.end(JSON.stringify({ error: String(error) }));
				}
			});
		},
	};
}
