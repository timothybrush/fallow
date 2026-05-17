// Mixed specifiers: at least one value symbol (`runQuery`), so the edge is
// NOT all-type-only. A violation must still fire because the runtime
// dependency on `runQuery` is real.
import { type Query, runQuery } from '../db/runtime';
import { helper } from '../shared/utils';

export const formatMixed = (q: Query): string => helper() + runQuery(q);
