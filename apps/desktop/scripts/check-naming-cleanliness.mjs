#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';

const repoRoot = path.resolve(import.meta.dirname, '../../..');
const desktopRoot = path.join(repoRoot, 'apps/desktop');

const sourceRoots = [
  'apps/desktop/backend/src/thinclaw',
  'apps/desktop/frontend/src',
  'apps/desktop/documentation',
  'apps/desktop/README.md',
].map(p => path.join(repoRoot, p));

const directWorkbenchRoots = [
  'apps/desktop/frontend/src/components/chat',
  'apps/desktop/frontend/src/components/imagine',
  'apps/desktop/frontend/src/components/voice',
  'apps/desktop/frontend/src/hooks/use-auto-start.ts',
  'apps/desktop/frontend/src/hooks/use-chat.ts',
  'apps/desktop/frontend/src/hooks/use-cloud-models.ts',
  'apps/desktop/frontend/src/hooks/use-engine-setup.ts',
  'apps/desktop/frontend/src/hooks/use-inference-backends.ts',
  'apps/desktop/frontend/src/lib/imagine.ts',
  'apps/desktop/frontend/src/lib/prompt-enhancer.ts',
].map(p => path.join(repoRoot, p));

const agentCockpitRoots = [
  'apps/desktop/frontend/src/components/thinclaw',
  'apps/desktop/frontend/src/lib/thinclaw.ts',
].map(p => path.join(repoRoot, p));

const allowedLegacyFiles = new Set([
  'apps/desktop/backend/src/thinclaw/config/keychain.rs',
  'apps/desktop/backend/src/thinclaw/config/types.rs',
  'apps/desktop/frontend/src/lib/local-storage-migration.ts',
  'apps/desktop/frontend/src/lib/status-tags.ts',
  'apps/desktop/frontend/src/lib/syntax-themes.ts',
  'apps/desktop/frontend/src/tests/lib/status-tags.test.ts',
  'apps/desktop/frontend/src/tests/lib/syntax-themes.test.ts',
  'apps/desktop/frontend/src/tests/lib/event-bus-migration.test.ts',
  'apps/desktop/documentation/bridge-contract.md',
  'apps/desktop/documentation/packaging-platform-readiness.md',
  'apps/desktop/documentation/runtime-parity-checklist.md',
  'apps/desktop/documentation/secrets-policy.md',
  'apps/desktop/documentation/setup.md',
]);

const forbidden = [
  { pattern: /\bIronClaw(State|Inner)?\b/g, reason: 'active IronClaw type/name' },
  { pattern: /\bOpenClaw\b/g, reason: 'active OpenClaw name' },
  { pattern: /openclaw_/g, reason: 'legacy IPC command prefix' },
  { pattern: /openclaw-event/g, reason: 'legacy event bus' },
  { pattern: /ironclaw_(bridge|builder|channel|types|secrets)/g, reason: 'legacy Desktop module name' },
  { pattern: /\[ironclaw\]/g, reason: 'legacy runtime log target' },
  { pattern: /embedded:\/\/ironclaw/g, reason: 'legacy embedded runtime URL' },
  { pattern: /id:\s*["']scrappy-(dark|light)["']/g, reason: 'legacy syntax theme id' },
  { pattern: /format!\("scrappy-/g, reason: 'legacy generated device id prefix' },
];

function walk(filePath) {
  const stat = fs.statSync(filePath);
  if (stat.isDirectory()) {
    return fs.readdirSync(filePath)
      .filter(name => !['node_modules', 'dist', 'target'].includes(name))
      .flatMap(name => walk(path.join(filePath, name)));
  }
  return [filePath];
}

function relative(filePath) {
  return path.relative(repoRoot, filePath).split(path.sep).join('/');
}

const files = sourceRoots.flatMap(root => fs.existsSync(root) ? walk(root) : []);
const failures = [];

for (const file of files) {
  const rel = relative(file);
  if (allowedLegacyFiles.has(rel)) continue;
  if (!/\.(rs|ts|tsx|md)$/.test(file)) continue;

  const text = fs.readFileSync(file, 'utf8');
  for (const rule of forbidden) {
    rule.pattern.lastIndex = 0;
    let match;
    while ((match = rule.pattern.exec(text))) {
      const line = text.slice(0, match.index).split('\n').length;
      failures.push(`${rel}:${line}: ${rule.reason}: ${match[0]}`);
    }
  }
}

const legacyModuleFiles = walk(path.join(desktopRoot, 'backend/src/thinclaw'))
  .map(relative)
  .filter(rel => /\/ironclaw_[^/]+\.rs$/.test(rel));
for (const file of legacyModuleFiles) {
  failures.push(`${file}:1: legacy Desktop module filename`);
}

function scanBoundary(roots, rules) {
  const filesToScan = roots
    .filter(root => fs.existsSync(root))
    .flatMap(root => walk(root))
    .filter(file => /\.(ts|tsx)$/.test(file));

  for (const file of filesToScan) {
    const rel = relative(file);
    const text = fs.readFileSync(file, 'utf8');
    for (const rule of rules) {
      rule.pattern.lastIndex = 0;
      let match;
      while ((match = rule.pattern.exec(text))) {
        const line = text.slice(0, match.index).split('\n').length;
        failures.push(`${rel}:${line}: ${rule.reason}: ${match[0]}`);
      }
    }
  }
}

scanBoundary(directWorkbenchRoots, [
  {
    pattern: /\bcommands\.direct(?:Chat|History|Runtime|Rag|Assets|Media|Imagine|Inference)[A-Z]/g,
    reason: 'Direct Workbench UI must import the scoped directCommands surface',
  },
  {
    pattern: /\bcommands\.thinclaw[A-Z]/g,
    reason: 'Direct Workbench UI must not call ThinClaw Agent commands',
  },
  {
    pattern: /["']thinclaw_[a-z0-9_]+["']/g,
    reason: 'Direct Workbench UI must not invoke ThinClaw Agent commands',
  },
]);

scanBoundary(agentCockpitRoots, [
  {
    pattern: /\bcommands\.thinclaw[A-Z]/g,
    reason: 'ThinClaw Agent UI must import the scoped thinclawCommands surface',
  },
  {
    pattern: /\bcommands\.direct(?:Chat|History|Runtime|Rag|Assets|Media|Imagine|Inference)[A-Z]/g,
    reason: 'ThinClaw Agent UI must not call Direct Workbench state APIs',
  },
  {
    pattern: /["']direct_(?:chat|history|runtime|rag|assets|media|imagine|inference)_[a-z0-9_]+["']/g,
    reason: 'ThinClaw Agent UI must not invoke Direct Workbench state APIs',
  },
]);

scanBoundary(sourceRoots, [
  {
    pattern: /["'`](?:chat_stream|start_chat_server|get_inference_backends|upload_image|imagine_generate|discover_hf_models|get_model_files|download_hf_model_files|discover_embedding_dimension)["'`]/g,
    reason: 'old unprefixed Direct command name',
  },
  {
    pattern: /\b(?:chatStream|startChatServer|getInferenceBackends|uploadImage|imagineGenerate|discoverHfModels|getModelFiles|downloadHfModelFiles|discoverEmbeddingDimension)\b/g,
    reason: 'old generated Direct command binding name',
  },
]);

for (const rel of [
  'apps/desktop/frontend/src/components/settings/SecretsTab.tsx',
  'apps/desktop/frontend/src/components/settings/ModelBrowser.tsx',
  'apps/desktop/frontend/src/components/thinclaw/CloudBrainConfigModal.tsx',
]) {
  const file = path.join(repoRoot, rel);
  if (!fs.existsSync(file)) continue;
  const text = fs.readFileSync(file, 'utf8');
  if (/\bxiaomi\b/i.test(text)) {
    failures.push(`${rel}:1: stale non-registry provider must not be selectable: xiaomi`);
  }
}

if (failures.length > 0) {
  console.error('Naming cleanliness check failed:');
  for (const failure of failures) {
    console.error(`- ${failure}`);
  }
  process.exit(1);
}

console.log('Naming cleanliness check passed.');
