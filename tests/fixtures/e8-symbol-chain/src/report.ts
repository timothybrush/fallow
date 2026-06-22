import { formatDate } from "./format";

// A same-module local helper: it is a callee that does NOT resolve to an
// import-symbol edge, so the trace must REPORT it as an unresolved callee
// (never silently drop it).
function localHelper(n: number): number {
  return n + 1;
}

export function buildReport(ts: number): string {
  const adjusted = localHelper(ts);
  // `parseInt` is a global: another unresolved (LocalOrGlobal) callee.
  const base = parseInt("0", 10);
  return formatDate(adjusted + base);
}
