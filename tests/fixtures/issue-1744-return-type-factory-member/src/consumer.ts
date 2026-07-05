import { useController, useControllerArrow } from './use-controller'

export function run(): number {
  const c = useController()
  const d = useControllerArrow()
  // getServices + createEstimate via the fn-decl factory; cloneEstimate via the
  // arrow factory. All three must be credited across the module boundary.
  return c.getServices() + c.createEstimate() + d.cloneEstimate()
}
