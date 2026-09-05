import { defineConfig, mergeConfig } from "vitest/config";
import fixture from "./vite.config";

export default mergeConfig(
	fixture,
	defineConfig({
		test: {
			environment: "happy-dom",
			include: ["*.dom.test.tsx"],
			pool: "forks",
			maxWorkers: 1,
			testTimeout: 15000,
		},
	}),
);
