<p align="center"><strong>English</strong> · <a href="README.ru.md">Русский</a></p>

![KERN — Data under control. Capture, record, recover.](assets/hero-en.svg)

# KERN · Керн

**Independent drilling measurement recording. A small Rust core, replaceable plugins and preserved measurements.**

The first headless core slice 0.1.0 is implemented: a separate process, raw-batch validation, CRC journal with disk acknowledgements, recovery, status and freshness settings. Hardware sources, the interpreter and working formats connect as separate components; the mandatory recovery journal keeps recording inputs when calculations or the interface fail.

[Plain-language core guide](docs/en/CORE_GUIDE.md) · [Validation](docs/en/CORE_VALIDATION.md) · [Architecture](docs/en/ARCHITECTURE.md) · [Stages and acceptance](docs/en/ROADMAP.md) · [Style](docs/en/BRANDING.md) · [Material provenance](docs/en/MATERIALS.md)

| Component | Responsibility |
|---|---|
| Core | Channels, parameters, quality, configuration versions, journal and supervision |
| Source plugins | Their own hardware protocol; one COM port per instance |
| Interpreter | Raw inputs → parameters using historical calibration maps |
| Recording plugins | Working files, export and an additional backup copy |
| System shell | Mandatory separate process: menus, settings and status |
| Charts | Replaceable presentation; rendering load does not set acquisition frequency |

**Status: the first pre-alpha headless slice.** 19 Rust tests and 18 end-to-end checks passed on Windows 11. The core executable is about 1 MiB; a short idle observation showed about 4.3 MiB working set. These are preliminary observations, not Windows 10, low-end hardware or 24–72-hour acceptance. Interpreter/maps/physical parameters, plugin supervision and graphical menus are not implemented.

Build: `cargo build --workspace --release --locked`. Run `target/release/kern-core.exe serve --profile runtime/first-test`, then `target/release/kern-demo.exe --profile runtime/first-test` in another window. `python tools/package_core.py` creates a local bundle with CMD launchers in `dist/Kern-Core-0.1.0/`.

At the owner's request, a detailed core guide explains the system for non-specialists; changing-UI help remains deferred until alpha. The next mandatory component is a separate menu/settings shell. The three-layer mark and Ember/Gruvbox are approved; application styling is ahead. [Dependencies and notices](docs/en/DEPENDENCIES.md).

```powershell
python tools/project_context.py start core
python tools/docs_sync.py check
python tools/project_context.py check
python -m unittest discover -s tests -v
```

The tools need Python 3.10+ and Git, with no pip dependencies. Open the repository root as an Obsidian vault. Russian originals and English translations share one branch.

[MIT](LICENSE) · [Working rules](AGENTS.md) · [Source](https://github.com/bubinbei/kern)
