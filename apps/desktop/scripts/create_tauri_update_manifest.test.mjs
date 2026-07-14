import assert from 'node:assert/strict';
import test from 'node:test';

import { createUpdateManifest } from './create_tauri_update_manifest.mjs';

test('creates the exact static Tauri updater contract for Apple Silicon', () => {
  const manifest = createUpdateManifest({
    version: '0.16.0',
    tag: 'v0.16.0',
    repository: 'RNT56/ThinClaw',
    arch: 'aarch64',
    updaterArtifact: '/tmp/ThinClaw Desktop.app.tar.gz',
    signature: 'trusted-signature\n',
    notes: 'Release notes',
    publishedAt: '2026-07-14T00:00:00.000Z',
  });

  assert.deepEqual(manifest, {
    version: '0.16.0',
    notes: 'Release notes',
    pub_date: '2026-07-14T00:00:00.000Z',
    platforms: {
      'darwin-aarch64': {
        signature: 'trusted-signature',
        url: 'https://github.com/RNT56/ThinClaw/releases/download/v0.16.0/ThinClaw%20Desktop.app.tar.gz',
      },
    },
  });
});

test('rejects drift between the tag, version, artifact, and signature', () => {
  const valid = {
    version: '0.16.0',
    tag: 'v0.16.0',
    repository: 'RNT56/ThinClaw',
    arch: 'aarch64',
    updaterArtifact: 'ThinClaw Desktop.app.tar.gz',
    signature: 'signature',
  };

  assert.throws(() => createUpdateManifest({ ...valid, tag: 'v0.16.1' }), /must equal/);
  assert.throws(() => createUpdateManifest({ ...valid, updaterArtifact: 'ThinClaw.dmg' }), /unexpected/);
  assert.throws(() => createUpdateManifest({ ...valid, signature: ' ' }), /empty/);
});
