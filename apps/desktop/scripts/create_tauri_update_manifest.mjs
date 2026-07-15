#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const SEMVER = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;

export function createUpdateManifest({
  version,
  tag,
  repository,
  arch,
  updaterArtifact,
  signature,
  notes = '',
  publishedAt = new Date().toISOString(),
}) {
  if (!SEMVER.test(version)) throw new Error(`invalid SemVer: ${version}`);
  if (tag !== `v${version}`) throw new Error(`release tag ${tag} must equal v${version}`);
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    throw new Error(`invalid GitHub repository: ${repository}`);
  }
  if (!['aarch64', 'x86_64'].includes(arch)) throw new Error(`unsupported macOS architecture: ${arch}`);
  if (!updaterArtifact.endsWith('.app.tar.gz')) {
    throw new Error(`unexpected macOS updater artifact: ${updaterArtifact}`);
  }
  if (!signature.trim()) throw new Error('updater signature is empty');
  if (Number.isNaN(Date.parse(publishedAt))) throw new Error(`invalid publication timestamp: ${publishedAt}`);

  const asset = path.basename(updaterArtifact);
  const url = `https://github.com/${repository}/releases/download/${tag}/${encodeURIComponent(asset)}`;
  const target = {
    signature: signature.trim(),
    url,
  };
  const platforms = arch === 'aarch64'
    ? { 'darwin-aarch64': target }
    : { 'darwin-x86_64': target };
  return {
    version,
    notes,
    pub_date: new Date(publishedAt).toISOString(),
    platforms,
  };
}

export function parseArguments(argv) {
  const allowed = new Set([
    'version', 'tag', 'repository', 'arch', 'artifact', 'signature', 'notes', 'output',
  ]);
  const values = Object.create(null);
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith('--') || value === undefined) throw new Error(`invalid argument near ${key ?? '<end>'}`);
    const name = key.slice(2);
    if (!allowed.has(name)) throw new Error(`unknown argument --${name}`);
    if (Object.hasOwn(values, name)) throw new Error(`duplicate argument --${name}`);
    values[name] = value;
  }
  return values;
}

function main() {
  const args = parseArguments(process.argv.slice(2));
  for (const required of ['version', 'tag', 'repository', 'arch', 'artifact', 'signature', 'output']) {
    if (!args[required]) throw new Error(`missing --${required}`);
  }
  if (!fs.existsSync(args.artifact)) throw new Error(`updater artifact does not exist: ${args.artifact}`);
  if (!fs.existsSync(args.signature)) throw new Error(`signature file does not exist: ${args.signature}`);

  const manifest = createUpdateManifest({
    version: args.version,
    tag: args.tag,
    repository: args.repository,
    arch: args.arch,
    updaterArtifact: args.artifact,
    signature: fs.readFileSync(args.signature, 'utf8'),
    notes: args.notes ?? '',
    publishedAt: process.env.SOURCE_DATE_EPOCH
      ? new Date(Number(process.env.SOURCE_DATE_EPOCH) * 1000).toISOString()
      : new Date().toISOString(),
  });
  fs.mkdirSync(path.dirname(args.output), { recursive: true });
  fs.writeFileSync(args.output, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`Wrote signed updater manifest for darwin-${args.arch}: ${args.output}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`Updater manifest generation failed: ${error.message}`);
    process.exit(1);
  }
}
