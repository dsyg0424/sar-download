import { createContext } from "react";

// Keeps a single region controller while placing its actions beside the dates.
export const RegionActionsContext = createContext<HTMLDivElement | null>(null);
