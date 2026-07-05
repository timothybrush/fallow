// Internal controller class (exported from a non-entry module, never re-exported
// by the entry), so its public members are subject to unused-class-member
// detection. The members are reached cross-module ONLY through the typed
// factory/hook wrappers in use-controller.ts, whose bodies contain NO `new`
// expression (the reporter's exact shape: `return registry.get() as Ctrl`), so
// the ONLY class signal is the wrapper's explicit `: ReadyAppController` return
// annotation. Issue #1744.
export class ReadyAppController {
  getServices(): number {
    return 1
  }

  createEstimate(): number {
    return 2
  }

  cloneEstimate(): number {
    return 3
  }

  neverUsedAnywhere(): number {
    // No call site anywhere: must STAY flagged (no blanket over-credit).
    return 4
  }
}
