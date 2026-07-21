# ADR-001: Supported Apple Platforms

Accepted: 2026-07-10

ThinClaw supports iOS 18+ and watchOS 11+. Features introduced in iOS/watchOS
26, including Liquid Glass presentation, are progressive enhancements guarded by
availability checks. Semantic materials, strokes, and standard controls remain
the complete functional and accessible fallback.

This keeps the beta installable on still-supported hardware without allowing a
new visual API to become the sole source of hierarchy, contrast, or interaction.
