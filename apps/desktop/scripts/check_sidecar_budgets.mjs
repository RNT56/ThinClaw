#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function pathInside(root, candidate, label) {
  if (typeof candidate !== 'string' || candidate.length === 0 || candidate.includes('\0')) {
    fail(`invalid ${label} path`);
  }
  const resolvedRoot = path.resolve(root);
  const resolved = path.resolve(resolvedRoot, candidate);
  if (resolved !== resolvedRoot && !resolved.startsWith(`${resolvedRoot}${path.sep}`)) {
    fail(`${label} escapes ${resolvedRoot}: ${candidate}`);
  }
  return resolved;
}

function fail(message) {
  console.error(`Sidecar budget check failed: ${message}`);
  process.exit(1);
}

function argument(name, fallback) {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : process.argv[index + 1];
}

function targetTriple() {
  let triple;
  if (process.env.TAURI_TARGET_TRIPLE || process.env.TARGET) {
    triple = process.env.TAURI_TARGET_TRIPLE || process.env.TARGET;
  } else {
    const arch = process.arch === 'arm64' ? 'aarch64' : 'x86_64';
    if (process.platform === 'darwin') triple = `${arch}-apple-darwin`;
    else if (process.platform === 'win32') triple = `${arch}-pc-windows-msvc`;
    else triple = `${arch}-unknown-linux-gnu`;
  }
  if (!/^[A-Za-z0-9_.-]+$/.test(triple)) fail(`invalid target triple: ${triple}`);
  return triple;
}

function filesUnder(entry) {
  if (!fs.existsSync(entry)) return [];
  const stat = fs.lstatSync(entry);
  if (stat.isSymbolicLink()) return [entry];
  if (stat.isFile()) return [entry];
  return fs.readdirSync(entry, { withFileTypes: true }).flatMap((child) =>
    filesUnder(path.join(entry, child.name)),
  );
}

function formatBytes(bytes) {
  return `${(bytes / 1024 / 1024).toFixed(1)} MiB`;
}

const configPath = pathInside(
  desktopRoot,
  argument('--config', 'backend/tauri.override.json'),
  'config',
);
const budgetPath = pathInside(
  desktopRoot,
  argument('--budgets', 'sidecar-budgets.json'),
  'budgets',
);
if (!configPath || !budgetPath || !fs.existsSync(configPath) || !fs.existsSync(budgetPath)) {
  fail(`missing config (${configPath}) or budgets (${budgetPath})`);
}

const config = JSON.parse(fs.readFileSync(configPath, 'utf8'));
const budgets = JSON.parse(fs.readFileSync(budgetPath, 'utf8')).limitsBytes;
for (const key of [
  'individualNativeArtifact',
  'nativeSidecarsAndLibraries',
  'chromium',
  'totalBundledRuntime',
]) {
  if (!Number.isSafeInteger(budgets?.[key]) || budgets[key] <= 0) {
    fail(`invalid positive integer limit: ${key}`);
  }
}

const triple = targetTriple();
const bundle = config.bundle ?? {};
const nativeFiles = new Set();
for (const externalBin of bundle.externalBin ?? []) {
  if (typeof externalBin !== 'string' || !/^bin\/[A-Za-z0-9_.-]+$/.test(externalBin)) {
    fail(`invalid external binary declaration: ${externalBin}`);
  }
  const base = path.join(desktopRoot, 'backend', 'bin', path.basename(externalBin));
  const candidates = [`${base}-${triple}`, `${base}-${triple}.exe`];
  const selected = candidates.find((candidate) => fs.existsSync(candidate));
  if (!selected) fail(`declared sidecar is missing for ${triple}: ${externalBin}`);
  nativeFiles.add(selected);
}

const binDir = path.join(desktopRoot, 'backend', 'bin');
if (fs.existsSync(binDir)) {
  for (const file of fs.readdirSync(binDir)) {
    const nativeLibrary = triple.includes('apple-darwin')
      ? file.endsWith('.dylib') || file.endsWith('.metal')
      : triple.includes('windows')
        ? file.endsWith('.dll')
        : file.endsWith('.so') || file.includes('.so.');
    if (nativeLibrary) nativeFiles.add(path.join(binDir, file));
  }
}

const chromiumDeclared = (bundle.resources ?? []).some((resource) =>
  resource === 'resources/chromium' || resource.startsWith('resources/chromium/'),
);
const chromiumFiles = chromiumDeclared
  ? filesUnder(path.join(desktopRoot, 'backend', 'resources', 'chromium'))
  : [];
if (chromiumDeclared && chromiumFiles.length === 0) {
  fail('Chromium is declared but backend/resources/chromium is empty or missing');
}

let nativeTotal = 0;
for (const file of nativeFiles) {
  const size = fs.statSync(file).size;
  if (size > budgets.individualNativeArtifact) {
    fail(`${file} is ${formatBytes(size)}; individual limit is ${formatBytes(budgets.individualNativeArtifact)}`);
  }
  nativeTotal += size;
}
const chromiumTotal = chromiumFiles.reduce((total, file) => total + fs.lstatSync(file).size, 0);
const total = nativeTotal + chromiumTotal;

if (nativeTotal > budgets.nativeSidecarsAndLibraries) {
  fail(`native runtime is ${formatBytes(nativeTotal)}; limit is ${formatBytes(budgets.nativeSidecarsAndLibraries)}`);
}
if (chromiumTotal > budgets.chromium) {
  fail(`Chromium is ${formatBytes(chromiumTotal)}; limit is ${formatBytes(budgets.chromium)}`);
}
if (total > budgets.totalBundledRuntime) {
  fail(`bundled runtime is ${formatBytes(total)}; limit is ${formatBytes(budgets.totalBundledRuntime)}`);
}

console.log(
  `Sidecar budgets passed for ${triple}: native=${formatBytes(nativeTotal)}, Chromium=${formatBytes(chromiumTotal)}, total=${formatBytes(total)}.`,
);
