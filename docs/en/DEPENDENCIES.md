# Core 0.1 dependencies

Cargo.lock pins versions. The [inventory](../dependencies.json) is generated from cargo metadata --locked; original notices are retained under third_party/licenses and included in the local bundle. Only public official dependency-license files are copied.

| Group | Purpose |
|---|---|
| serde / serde_json | Typed messages and our journal events |
| crc32fast | Accidental record-damage detection |
| getrandom | Fresh local IPC token from OS randomness |
| Others | Transitive/build/target dependencies of these libraries |

MIT is selected where explicitly offered as a permissive alternative. This includes r-efi: MIT OR Apache-2.0 OR LGPL-2.1-or-later does not require selecting LGPL. unicode-ident additionally requires Unicode-3.0; that notice is retained too. The inventory includes packages for other targets, not only those linked into Windows executables.

New KERN code is MIT. The Rust standard library/toolchain have separate terms; the compiler is not bundled. External binary distribution needs a separate composition review, including standard-library notices. This is a local test bundle; no public binary release is published.

Official `windows-sys`/`windows-link` bindings (MIT alternative) were added for the separate native shell; they are not linked into the headless core. No web runtime/async framework was added. The absence of a browser engine does not replace CPU/RAM/GPU/disk measurements or Windows 10 validation.
