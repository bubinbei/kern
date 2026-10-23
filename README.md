<p align="center"><strong>English</strong> · <a href="README.ru.md">Русский</a></p>

![KERN — Data under control. Capture, record, recover.](assets/hero-en.svg)

# KERN · Керн

**Independent drilling measurement recording. A small Rust core, replaceable plugins and preserved measurements.**

We begin with a resilient headless core. Hardware sources, the interpreter and working formats connect as separate components; the mandatory recovery journal keeps recording inputs when calculations or the interface fail.

[Architecture](docs/en/ARCHITECTURE.md) · [Stages and acceptance](docs/en/ROADMAP.md) · [Style](docs/en/BRANDING.md) · [Material provenance](docs/en/MATERIALS.md)

| Component | Responsibility |
|---|---|
| Core | Channels, parameters, quality, configuration versions, journal and supervision |
| Source plugins | Their own hardware protocol; one COM port per instance |
| Interpreter | Raw inputs → parameters using historical calibration maps |
| Recording plugins | Working files, export and an additional backup copy |
| System shell | Mandatory separate process: menus, settings and status |
| Charts | Replaceable presentation; rendering load does not set acquisition frequency |

**Status: pre-alpha, architecture agreed, core implementation has not started.** This repository contains no ready application or accepted performance measurements. Windows 10 x64 is the first target. Testing on a low-end Intel “4125” laptop and later on a 2012 Mac running macOS is planned; support is not claimed.

Before alpha, the priority is working, verifiable increments. Detailed user help follows stable behavior. The three-layer mark and amber Ember / Gruvbox palette are approved; application integration is ahead.

```powershell
python tools/project_context.py start core
python tools/docs_sync.py check
python tools/project_context.py check
python -m unittest discover -s tests -v
```

The tools need Python 3.10+ and Git, with no pip dependencies. Open the repository root as an Obsidian vault. Russian originals and English translations share one branch.

[MIT](LICENSE) · [Working rules](AGENTS.md) · [Source](https://github.com/bubinbei/kern)
