import { createContext, use } from "react";

export const PageLayer = createContext<HTMLElement | null>(null);
export const TopPageLayer = createContext<HTMLElement | null>(null);

/** All page overlays, including Radix portals, mount in this retained layer. */
export const usePageLayer = () => use(PageLayer) ?? document.body;

/** The outermost page's layer, for overlays that cover the whole window. */
export const useTopPageLayer = () => use(TopPageLayer) ?? document.body;
