// Loading the compiled core under node, which is the one place the asset URL
// cannot serve it: there is no fetch, so the bytes are read from disk and
// `initSync` takes them directly.
//
// Kept out of `index.ts` so `node:fs` never reaches a browser bundle, and kept
// in one file so the path to the generated module is written once.

import { readFileSync } from 'node:fs';
import { loadCoreFrom, type Core } from './index';

/** The module, initialised from the bytes `just web-wasm` produced. */
export function loadCoreForTest(): Core {
	return loadCoreFrom(readFileSync(new URL('./generated/core_bg.wasm', import.meta.url)));
}
