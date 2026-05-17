// Namespace type import: whole declaration is type-only.
import type * as Db from '../db/types';
import { helper } from '../shared/utils';

export const sumIds = (rows: ReadonlyArray<Db.RowId>): string =>
  helper() + rows.length.toString();
