// Two import statements to the SAME target: line 4 is type-only, line 5
// is the real runtime import. Fallow groups these into ONE Edge with
// two ImportedSymbols. The boundary violation must anchor on line 5
// (the value import), not line 4 (the type-only import), so that
// `// fallow-ignore-next-line` placed above the runtime line works.
import type { Query } from '../db/runtime';
import { runQuery } from '../db/runtime';
import { helper } from '../shared/utils';

export const formatSibling = (q: Query): string => helper() + runQuery(q);
