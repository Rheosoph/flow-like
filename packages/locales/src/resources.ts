import admin from "../locales/en/admin.json";
import chat from "../locales/en/chat.json";
import common from "../locales/en/common.json";
import flow from "../locales/en/flow.json";
import interfaces from "../locales/en/interfaces.json";
import nav from "../locales/en/nav.json";
import settings from "../locales/en/settings.json";
import store from "../locales/en/store.json";

/**
 * The source language ships in the main bundle. Every other language is fetched
 * lazily by the backend in `create-i18n.ts`, so a first paint never renders raw
 * keys and a missing translation always has something to fall back to.
 */
export const SOURCE_RESOURCES = {
	common,
	nav,
	settings,
	flow,
	store,
	chat,
	interfaces,
	admin,
} as const;

export type SourceResources = typeof SOURCE_RESOURCES;
