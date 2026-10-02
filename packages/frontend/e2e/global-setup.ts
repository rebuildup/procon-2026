import { mkdirSync, rmSync } from 'node:fs';

import { OUT_DIR } from './recording';

/**
 * Fresh output directory per run: artifacts from a previous recording must
 * not survive into `manifest.json` or the `target/recordings/` listing.
 */
export default function globalSetup(): void {
  rmSync(OUT_DIR, { recursive: true, force: true });
  mkdirSync(OUT_DIR, { recursive: true });
}
