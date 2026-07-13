# Graph Report - kds-windows  (2026-07-13)

## Corpus Check
- 15 files · ~30,751 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 335 nodes · 585 edges · 16 communities
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- lib.rs
- properties
- definitions
- properties
- definitions
- tauri.conf.json
- package.json
- compilerOptions
- AppData
- main.ts
- .new
- default.json
- KDS Guard for Windows

## God Nodes (most connected - your core abstractions)
1. `StateInner` - 25 edges
2. `snapshot()` - 16 edges
3. `compilerOptions` - 15 edges
4. `AppSnapshot` - 14 edges
5. `save_settings()` - 13 edges
6. `start_monitor_inner()` - 13 edges
7. `emit_snapshot()` - 13 edges
8. `Settings` - 12 edges
9. `AppState` - 12 edges
10. `poll_once()` - 12 edges

## Surprising Connections (you probably didn't know these)
- `AppData` --references--> `Settings`  [EXTRACTED]
  src-tauri/src/lib.rs → src-tauri/src/lib.rs  _Bridges community 0 → community 8_
- `StateInner` --references--> `AppHandle`  [EXTRACTED]
  src-tauri/src/lib.rs →   _Bridges community 0 → community 10_

## Import Cycles
- None detected.

## Communities (16 total, 0 thin omitted)

### Community 0 - "lib.rs"
Cohesion: 0.19
Nodes (47): Arc, AtomicBool, Client, Default, JoinHandle, Mutex, Result, alarm_severity_has_priority_over_gentle() (+39 more)

### Community 1 - "properties"
Cohesion: 0.05
Nodes (45): description, properties, required, type, Capability, Identifier, default, description (+37 more)

### Community 2 - "definitions"
Cohesion: 0.05
Nodes (38): anyOf, anyOf, description, description, required, type, description, properties (+30 more)

### Community 3 - "properties"
Cohesion: 0.06
Nodes (36): properties, default, description, type, type, $ref, type, array (+28 more)

### Community 4 - "definitions"
Cohesion: 0.07
Nodes (29): anyOf, anyOf, description, description, properties, required, type, definitions (+21 more)

### Community 5 - "tauri.conf.json"
Cohesion: 0.07
Nodes (27): icons/128x128@2x.png, icons/128x128.png, icons/32x32.png, icons/icon.icns, icons/icon.ico, resources/sounds/alarm.ogg, resources/sounds/gentle.ogg, resources/sounds/notif.ogg (+19 more)

### Community 6 - "package.json"
Cohesion: 0.08
Nodes (23): dependencies, @tauri-apps/api, @tauri-apps/plugin-autostart, @tauri-apps/plugin-opener, devDependencies, @tauri-apps/cli, typescript, vite (+15 more)

### Community 7 - "compilerOptions"
Cohesion: 0.10
Nodes (20): DOM, DOM.Iterable, ES2020, src, compilerOptions, allowImportingTsExtensions, isolatedModules, lib (+12 more)

### Community 8 - "AppData"
Cohesion: 0.22
Nodes (17): DateTime, Option, active_mute_suppresses_audio_and_expired_mute_does_not(), alarm(), AlarmDecision, AlarmFingerprint, app_data(), AppData (+9 more)

### Community 9 - "main.ts"
Cohesion: 0.18
Nodes (11): AppSnapshot, call(), els, escapeHtml(), fillForm(), fmtDate(), LogEntry, render() (+3 more)

### Community 10 - ".new"
Cohesion: 0.33
Nodes (9): AppHandle, PathBuf, Self, resolve_resource(), resolve_sound_paths(), run(), setup_tray(), show_main_window() (+1 more)

### Community 11 - "default.json"
Cohesion: 0.20
Nodes (9): autostart:default, core:default, main, opener:default, description, identifier, permissions, $schema (+1 more)

### Community 12 - "KDS Guard for Windows"
Cohesion: 0.40
Nodes (4): Current limitations, KDS Guard for Windows, Run on Windows, What works in the first MVP

## Knowledge Gaps
- **153 isolated node(s):** `name`, `private`, `version`, `type`, `dev` (+148 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `properties` connect `properties` to `definitions`?**
  _High betweenness centrality (0.032) - this node is a cross-community bridge._
- **Why does `definitions` connect `definitions` to `properties`?**
  _High betweenness centrality (0.029) - this node is a cross-community bridge._
- **What connects `name`, `private`, `version` to the rest of the system?**
  _153 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `properties` be split into smaller, more focused modules?**
  _Cohesion score 0.046464646464646465 - nodes in this community are weakly interconnected._
- **Should `definitions` be split into smaller, more focused modules?**
  _Cohesion score 0.05128205128205128 - nodes in this community are weakly interconnected._
- **Should `properties` be split into smaller, more focused modules?**
  _Cohesion score 0.05873015873015873 - nodes in this community are weakly interconnected._
- **Should `definitions` be split into smaller, more focused modules?**
  _Cohesion score 0.06666666666666667 - nodes in this community are weakly interconnected._