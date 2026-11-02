import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useSarSearchStore } from "../store/useSarSearchStore";
import { groupProducts } from "../model/groups";
import { revealResultRow } from "./resultListScroll";

const productKey = (group: string, product: string) => JSON.stringify([group, product]);

export function useSarResults(active: boolean) {
  const store = useSarSearchStore();
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  const [expandedProducts, setExpandedProducts] = useState<Set<string>>(new Set());
  const [expandedScenes, setExpandedScenes] = useState<Set<string>>(new Set());
  const toggleExpanded = (values: Set<string>, id: string) => {
    const next = new Set(values);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    return next;
  };
  useEffect(() => {
    setExpandedGroups(new Set());
    setExpandedProducts(new Set());
    setExpandedScenes(new Set());
  }, [store.resultRequest, store.resetVersion]);
  const groups = useMemo(() => groupProducts(store.results), [store.results]);
  const expandAll = () => {
    setExpandedGroups(new Set(groups.map((group) => group.id)));
    setExpandedProducts(new Set(groups.flatMap((group) => group.products.map((item) => productKey(group.id, item.product)))));
  };
  const collapseAll = () => { setExpandedGroups(new Set()); setExpandedProducts(new Set()); setExpandedScenes(new Set()); };
  const keysFor = (group: (typeof groups)[number]) =>
    group.products.map((p) => p.id);
  const allKeys = groups.flatMap(keysFor);
  const products = store.results.filter((p) => store.selected.includes(p.id));
  const toggleKeys = (keys: string[]) =>
    useSarSearchStore.setState({
      selected: keys.every((k) => store.selected.includes(k))
        ? store.selected.filter((k) => !keys.includes(k))
        : [...new Set([...store.selected, ...keys])],
    });
  const groupElements = useRef(new Map<string, HTMLElement>());
  const resultViewport = useRef<HTMLDivElement>(null);
  const [pendingReveal, setPendingReveal] = useState<{
    group: string;
    scene: string | null;
  } | null>(null);
  useEffect(() => {
    if (store.activeGroup) {
      if (store.activeScene) {
        setExpandedGroups((values) => new Set(values).add(store.activeGroup!));
        const product = store.results.find((item) => item.id === store.activeScene);
        if (product) setExpandedProducts((values) => new Set(values).add(productKey(store.activeGroup!, product.product)));
      }
      setPendingReveal({ group: store.activeGroup, scene: store.activeScene });
    }
  }, [store.activeGroup, store.activation]);
  useLayoutEffect(() => {
    if (!pendingReveal || !active) return;
    const viewport = resultViewport.current;
    const group = groupElements.current.get(pendingReveal.group);
    const target = pendingReveal.scene
      ? groupElements.current.get(pendingReveal.scene)
      : group?.querySelector<HTMLElement>(".sar-path-heading");
    if (!viewport || !group || !target || !viewport.clientHeight) return;
    const heading = pendingReveal.scene
      ? (group.querySelector<HTMLElement>(".sar-path-heading")?.offsetHeight ??
        36)
      : 0;
    revealResultRow(viewport, target, heading);
    setPendingReveal(null);
  }, [pendingReveal, expandedGroups, expandedProducts, active]);
  return {
    store,
    groups,
    keysFor,
    allKeys,
    products,
    toggleKeys,
    expandedGroups,
    setExpandedGroups,
    expandedProducts,
    setExpandedProducts,
    productKey,
    expandAll,
    collapseAll,
    expandedScenes,
    setExpandedScenes,
    toggleExpanded,
    groupElements,
    resultViewport,
  };
}
