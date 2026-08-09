module.exports = {
	apps: [
		{
			name: "signal-ws",
			script: "./server.ts",
			interpreter: "bun",
			exec_mode: "fork",
			instances: "max", // one per CPU core
			watch: false,
			// sticky keeps each client on the same worker (important for WS)
			// PM2 uses X-Forwarded-For when behind a proxy.
			// Enable sticky by setting the env var below:
			env: {
				NODE_ENV: "production",
				REDIS_URL: "redis://127.0.0.1:6379",
				SIGNAL_CHANNEL: "signal:publish",
				PM2_STICKY: "true",
			},
			// health & resiliency
			max_memory_restart: "512M",
			kill_timeout: 5000,
			listen_timeout: 5000,
			// logs (optional paths)
			out_file: "/var/log/signal-ws/out.log",
			error_file: "/var/log/signal-ws/err.log",
			merge_logs: true,
		},
	],
};
