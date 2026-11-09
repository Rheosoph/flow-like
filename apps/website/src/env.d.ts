/// <reference path="../.astro/types.d.ts" />

interface ImportMetaEnv {
	readonly REGISTRY_PAT: string;
	readonly REGISTRY_API_URL: string;
	readonly ENABLE_STORE_SEO: string;
}

interface ImportMeta {
	readonly env: ImportMetaEnv;
}

declare module "cloudflare:workers" {
	export const env: {
		readonly ASSETS: { fetch(request: Request): Promise<Response> };
	};
}
