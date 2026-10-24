# Parameter catalog, presets and calculations

Status: target design dated 2026-10-24, partially implemented by catalogs/presets and graph-v1. Current capabilities and precise limits: [PARAMETERS](PARAMETERS.md), with 81 definitions, seven presets, up to 128 parameters/48 KiB, predefined calculations and bounded rate windows. Further requirements here (SDK, arbitrary expressions, automatic depth/operation accounting and third-party checkpoints) are not implemented. Existing external profiles and implementations are not imported.

## Three operator concepts

| Concept | Contents | Action |
|---|---|---|
| Parameter catalog | Definitions: name, unit, meaning, allowed value providers, dependencies and required algorithms | Search/grouped Add Parameter; custom definitions allowed |
| Settings preset | Named, versioned parameter list with selected providers/settings; optional calibration, hardware binding, screen and recording sections | Preview, merge into working configuration, save a custom preset |
| Working configuration | Concrete parameter instances, inputs, verified calibrations, calculations and recorded operator inputs | Draft → validation → core-persisted version → activation |

Selecting a preset fills a draft. It does not start COM, install executable code or automatically change the working configuration. Editing the working configuration does not overwrite its source preset. Operators may add any catalog parameter or a custom definition.

## Preset selection

The left pane lists/searches built-in and custom presets by purpose, name and version. The right pane shows the complete selected contents grouped by purpose. Each row shows parameter, unit, provider, required inputs and status. Filters cover all, sensor, calculated, operator input and needs configuration.

A separate section lists included categories: parameters, algorithms/calculation settings, calibrations, hardware bindings, screens and recording rules. Missing categories are explicit. Before applying, show exact counts and changes: add/update/keep/disable, conflicts and missing dependencies. Counts come from actual preset contents.

The default is Add Missing, preserving existing bindings and calibrations. Updating selected rows and full replacement are explicit modes with a comparison. Conflicting definitions of one stable ID require resolution; equal names do not imply equal parameters. Deleting a preset does not delete working parameters or history. Switching presets does not erase recorded measurements.

Initial preset directions: basic drilling technology, depth/tripping, pumps, tanks/mud, gas monitoring and diagnostics. A complete working set combines these groups without duplicating shared dependencies. Exact contents require separate definition; this list does not claim implemented calculations. Missing algorithms/plugins leave visible unavailable rows without numeric output.

## Value providers

| Provider | Configuration and behavior |
|---|---|
| Source channel | Source instance + permanent channel + calibration map; code/V for applicable analog inputs |
| Calculation | Algorithm ID/version, input parameters, coefficients, time/depth window and applicability conditions |
| Operator input | Value, unit, effective time/boundary and a core change event; not a fabricated sensor sample |

One physical quantity may have alternative providers. Input flow, for example, may use a flowmeter or pump calculation. Exactly one provider is explicitly selected; automatic fallback requires a separate policy and event. Names/colors may change without changing stable IDs. Repeated objects, such as tanks, have separate instance IDs.

Group the catalog by task (technology, depth/speeds, pumps, mud/tanks, gas, custom), with provider as a separate field. Measured and calculated quantities can appear together. The settings table needs Provider, Inputs and Readiness columns; editing a calculated quantity opens dependencies/settings, while editing a sensor opens calibration.

## Dependencies and examples

```mermaid
flowchart LR
    Raw[Core raw channels] --> Cal[Interpreter calibrations]
    Cal --> Graph[Interpreter calculation graph]
    Manual[Recorded operator actions] --> Graph
    Cal --> Params[Core parameter table]
    Graph --> Params
    Params --> View[Charts and writers]
```

These are logical stages of one processing pipeline. No additional mandatory table between calibration and formulas is required. Several algorithms can run in one Rust interpreter process; a separate process per parameter is unnecessary.

- Weight on bit: a first variant to agree upon is free-hanging string weight minus current hook load, with an explicit free-weight setting event and applicability conditions. This is a preliminary model, not a universal field procedure. Reference updates, tool state and input quality need defined rules.
- Speeds: distinguish movement speed, averaged penetration rate and rate over a specified drilled interval. They require different inputs, windows and drilling/pause-time rules; a single Speed label is insufficient.
- Pump flow: each pump's stroke rate, volume per stroke and delivery coefficient; units and coefficients must be explicit. Do not also sum a flowmeter of the same flow without an explicit result definition.
- Tank volume: level and the tank's own calibration table; total/active volume selects participating tanks. Adding a tank must not silently change the sum membership.

Adding a calculation shows its dependency tree and offers to add missing definitions. Adding a definition does not create a missing sensor or insert an arbitrary calibration. Examples: Load: free weight not set; Speed: depth and history required. Distinguish missing inputs, configuration needed, window warmup, stale data and calculation errors. Missing data, invalid expressions and division by zero must not become 0.

## Core/plugin boundary

Plugins provide catalogs/algorithms; the settings screen edits drafts. The core owns working definitions, immutable configuration versions and activation boundaries. Presets reference definition IDs and compatible versions; application captures the complete selected definitions in the working version. Catalog updates do not change ongoing registration. Removing a required algorithm is blocked for an active calculation or makes its unavailability explicit; history remains readable.

The Rust interpreter executes calculations. Validate the graph for missing inputs, cycles, unit compatibility, versions and limits. Ordinary expressions have no implicit previous-value self-reference; accumulation/averaging are explicit stateful algorithms. Start with tested predefined algorithms; later add a bounded expression editor with preview and complexity limits, without arbitrary-code eval.

Dependencies determine evaluation order. Different sources require an explicit temporal alignment/maximum-age policy, rather than table-read order. Validate/prepare the graph at version activation; each cycle recalculates the affected subset without reparsing formulas or loading the entire catalog. History windows are bounded. Results carry configuration/algorithm version, lineage and quality; replaying old history does not make it fresh. Slow/failing calculations do not block raw intake.

Stateful algorithms require committed checkpoints/positions and window-restoration rules. Well settings, free weight, initial depth and other result-affecting changes must be journaled calculation inputs. Backup obtains working versions, events and necessary checkpoints through the core. Replay separately checks executable algorithm-version availability: a JSON formula alone cannot restore every calculation. This extends the current stateless linear-v1 contract and requires separate implementation.

## Saving custom presets

Save as Preset requests a name, description, version, groups, selected sections and file preview. Default contents are definitions/calculations without specific COM ports, working bindings, initial depths, free weight or measured calibrations. Moving verified bindings/calibrations requires explicit sections, provenance and validation on the destination bench. Measurements, secrets and accumulator state are excluded.

The proposed format is declarative JSON with a schema version and ASCII IDs/paths, for example parameter-presets/drilling-basic/preset.json; Russian display names are allowed inside. Built-in presets resist accidental overwrite; editing creates a custom profile copy. Saving adds it to the selection list. Import validates schema/size/versions and previews contents; a missing plugin does not trigger installation. A preset is not an executable plugin. This is a future structure unsupported by the current loader.

## Implementation and acceptance order

1. Catalog/library, preview/comparison, parameter addition and custom preset saving. Initially support only implemented types; unavailable calculations are explicit. Opening/cancelling leaves the working profile unchanged.
2. Versioned providers/dependencies and recorded operator inputs. Preserve old maps, journals and replay. Expand the 32-parameter limit consistently across contract/IPC/storage/GUI after budget measurement; an oversized preset must not apply partially.
3. Stateless calculations: difference/sum, pump flow and table-based tank volume. Independent numerical oracles and quality checks.
4. Depth/load/speeds after agreeing field conditions, resets/transitions and checkpoints. Forced restart/backlog must match continuous calculation.

Required scenarios: exact preview/applied-content match; merge without calibration loss; ID/version conflicts; dependency addition without duplication; custom preset across restart/export/import; missing plugin; cycles/units/division by zero; asynchronous/stale inputs; consistent activation during acquisition; restoration from backup with versions and manual events. Measure update time/memory on weak hardware at realistic active-parameter counts; total catalog size must not determine per-cycle calculation cost.
