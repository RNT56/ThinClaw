# ThinClaw source mirror

This directory is a source-equivalent mirror of the yanked crates.io
`glass_pumpkin` 1.10.0 archive (crate checksum
`f09b0eef9941bda7cc263c23c3977d437a9aa19abb6a58ed0234d145d9c024bc`,
upstream revision `8a8e056a9f3e7f13d4cec7af61dc3782b7f73e55`). It carries no ThinClaw
runtime source changes; CI enforces that with `patches/manifest.json` and
`scripts/ci/check-vendored-patches.py`.

`grammers-crypto` 0.8 accepts `glass_pumpkin >=1.7,<2`. Its normal resolver
path is not reproducible from a fresh lock because the 1.7–1.9 releases use
the yanked `core2` 0.4, while the compatible 1.10 bridge itself was yanked.
`grammers-crypto` 0.10 has moved to unyanked `glass_pumpkin` 2.x.

Owner, upstream references, review date, security-update procedure, and the
removal gate are canonical in `patches/manifest.json`. Remove this mirror by
upgrading all Telegram `grammers-*` dependencies together and proving login,
SRP 2FA, session persistence, transport, and reproducible WASM bundles.
