# ThinClaw security backport

This local `0.18.6` package is the crates.io `glib` 0.18.5 source with the
upstream `VariantStrIter::impl_get` mutability fix backported from
`gtk-rs/gtk-rs-core#1343` (merge commit
`05dff0ee696f9bcd8617cd48c4b812d046d440cb`).

The source change passes the C out-argument as `&mut p`, closing
RUSTSEC-2024-0429 / GHSA-wrw7-89jp-8q8g while ThinClaw remains on Tauri's
GTK3-compatible glib 0.18 line. Remove this patch once Tauri moves its Linux
stack to glib 0.20 or newer.
