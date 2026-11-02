import { create } from "zustand";
export const useMapStore = create<{
  drawing: boolean; focus: number;
  setDrawing: (drawing: boolean) => void; focusRegion: () => void;
}>((set) => ({
  drawing: false, focus: 0,
  setDrawing: (drawing) => set({ drawing }),
  focusRegion: () => set((s) => ({ focus: s.focus + 1 })),
}));
