/// <reference types="astro/client" />

declare module "virtual:starlight/pagefind-config" {
	export const pagefindUserConfig: Record<string, unknown>;
}

declare module "@pagefind/default-ui" {
	export class PagefindUI {
		constructor(options: Record<string, unknown>);
	}
}
