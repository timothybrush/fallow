// Side-effect import: runs the target module at runtime. Symbol carries
// is_type_only: false, so the edge is NOT all-type-only. Must fire.
import '../db/runtime';
import { helper } from '../shared/utils';

export const sideEffected = (): string => helper();
