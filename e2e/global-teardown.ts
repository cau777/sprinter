import { rm } from "node:fs/promises";

export default async function globalTeardown() {
  const dataDir = process.env.SPRINTER_E2E_DATA_DIR;
  if (dataDir) await rm(dataDir, { recursive: true, force: true });
}
