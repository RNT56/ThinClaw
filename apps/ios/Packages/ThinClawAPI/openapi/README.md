# Vendored OpenAPI spec

`scripts/generate-api.sh` copies the committed gateway contract from
`clients/openapi/thinclaw-gateway.openapi.json` (repo root) into this
directory and runs Apple's `swift-openapi-generator` against it. The
generator output is committed under `../Sources/ThinClawAPI/Generated/`.

Do not hand-edit either the vendored spec or the generated sources —
regenerate instead. CI enforces freshness via
`scripts/check-generated-drift.sh`.
