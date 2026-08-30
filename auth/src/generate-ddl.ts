/**
 * Emit the DDL better-auth's configuration implies, as the Kysely generator
 * behind `auth generate` does: `getMigrations(...).compileMigrations()` is the
 * whole of that generator (`packages/cli/src/generators/kysely.ts`). Running it
 * here keeps the CLI's ~20 further dependencies out of this service's tree
 * while producing the same SQL.
 *
 * This compiles rather than applies. The emitted statements are reviewed by
 * hand, qualified with a search_path, and landed as a numbered file under
 * `db/auth/`; nothing here writes to the database.
 */
import { getMigrations } from 'better-auth/db/migration';
import { auth, pool } from './auth.ts';

const { compileMigrations, unsafeChanges } = await getMigrations(auth.options, {
  throwOnUnsafe: false,
});

for (const change of unsafeChanges) {
  console.error(`unsafe change: ${change}`);
}

const sql = await compileMigrations();
process.stdout.write(sql.trim() === ';' ? '' : sql);

await pool.end();
