import type { ReadyAppController } from './controller'

// A registry-backed factory: the body returns an `as ReadyAppController` cast,
// so there is NO `new` value-proof. The `: ReadyAppController` return annotation
// is the author's compiler-checked contract and the only class signal fallow
// has. Both the function-declaration and arrow forms are exercised. Issue #1744.
const registry: Record<string, unknown> = {}

export function useController(): ReadyAppController {
  return registry['controller'] as ReadyAppController
}

export const useControllerArrow = (): ReadyAppController =>
  registry['controllerArrow'] as ReadyAppController
