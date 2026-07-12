import { readdir, stat } from "node:fs/promises";
import { join } from "node:path";

const assetsDir = join(process.cwd(), "dist", "assets");
const maxChunkBytes = 500 * 1024;
const files = (await readdir(assetsDir)).filter((name) => name.endsWith(".js"));

if (files.length === 0) {
  throw new Error(`No JavaScript chunks found in ${assetsDir}`);
}

const chunks = await Promise.all(
  files.map(async (name) => ({ name, bytes: (await stat(join(assetsDir, name))).size })),
);
const oversized = chunks.filter((chunk) => chunk.bytes > maxChunkBytes);
const largest = chunks.reduce((left, right) => (left.bytes >= right.bytes ? left : right));

if (oversized.length > 0) {
  const details = oversized
    .sort((left, right) => right.bytes - left.bytes)
    .map((chunk) => `${chunk.name}: ${(chunk.bytes / 1024).toFixed(1)} KiB`)
    .join("\n");
  throw new Error(`Frontend chunk budget exceeded (500 KiB):\n${details}`);
}

console.log(
  `Frontend chunk budget passed: ${chunks.length} chunks; largest ${largest.name} ` +
    `${(largest.bytes / 1024).toFixed(1)} KiB`,
);
