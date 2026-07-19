"use client";

import { createContext } from "react";

/** App scope forwarded by editable surfaces for hosted-model usage attribution. */
export const AIUsageAppContext = createContext<string | undefined>(undefined);
