#!/usr/bin/env node

import { readFile } from "node:fs/promises";

const expectedBrowsersVersion = "3.2.0";
const lock = JSON.parse(
  await readFile(new URL("../package-lock.json", import.meta.url), "utf8"),
);

const packages = Object.entries(lock.packages ?? {});
const browserCopies = packages.filter(([path]) =>
  path.endsWith("node_modules/@puppeteer/browsers"),
);
const extractZipCopies = packages.filter(([path]) =>
  path.endsWith("node_modules/extract-zip"),
);

if (browserCopies.length === 0) {
  throw new Error("package-lock.json contains no @puppeteer/browsers installation");
}

const staleCopies = browserCopies.filter(
  ([, metadata]) => metadata.version !== expectedBrowsersVersion,
);
if (staleCopies.length > 0) {
  throw new Error(
    `all @puppeteer/browsers copies must resolve to ${expectedBrowsersVersion}: ${staleCopies
      .map(([path, metadata]) => `${path}=${metadata.version ?? "unknown"}`)
      .join(", ")}`,
  );
}

if (extractZipCopies.length > 0) {
  throw new Error(
    `extract-zip must not re-enter the Desktop dependency graph: ${extractZipCopies
      .map(([path, metadata]) => `${path}=${metadata.version ?? "unknown"}`)
      .join(", ")}`,
  );
}

console.log(
  `Desktop dependency security contract verified: ${browserCopies.length} ` +
    `@puppeteer/browsers copy/copies at ${expectedBrowsersVersion}; no extract-zip`,
);
