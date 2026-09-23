# ADR-0044: One logical store for offline data on every platform

- Status: Proposed
- Date: 2026-09-23

## Context

Pilotage and Navigate use data that must be available without a network:
navigation data, terrain, imagery packages, missions, and settings. The same
Rust core runs on four kinds of host:

- a Linux host,
- a Windows workstation,
- the Apple app on iPadOS and visionOS,
- a browser, through WebAssembly.

Each host has its own standard place for application data. Linux uses the XDG
base directories. Windows uses `AppData`. Apple platforms use the application
container. A browser has no file system path. It gives the Origin Private File
System (OPFS) and the Cache API.

Today each component chooses a location for itself:

- The Apple app resolves `Application Support/AviationData`, excludes it from
  backup, and gives the path to `AviationDataSession`
  (`clients/apple/App/AviationDataWorker.swift`).
- `pilotage-map-archives` resolves `pilotage://` resource names to installed
  files (`ResourceBinding`, `ResourceSet`).
- The Navigate visual tools read packages through the `navigate-data` read
  port. The browser preview keeps its packages in OPFS.

The code does not define a common layout, a common set of data classes, or
one owner for writes. A component that runs on a new platform must choose a
location again. Two rules are easy to break:

1. **Backup.** On Apple platforms, data in Application Support is included in
   device backups. Large data that the app can download again must be
   excluded from backup. App Review can refuse an app that does not do this.
   `Library/Caches` is also wrong for this data, because the system can delete
   it when storage is low.
2. **Eviction.** A browser can delete OPFS data when storage is low, unless the
   origin has persistent storage (`navigator.storage.persist()`).

## Decision

### Five storage classes

All offline data belongs to one `StorageClass`:

| Class | Content | Linux | Windows | Apple | Browser |
|---|---|---|---|---|---|
| `Data` | Data that the operator makes, such as missions and journals. Include in backup. | `$XDG_DATA_HOME/pilotage` | `%APPDATA%\Pilotage` | `Application Support/Data` | OPFS `/data`, persistent storage requested |
| `Offline` | Released data that the app can download again: navigation data, terrain, imagery, map packages. Exclude from backup. Do not purge. | `$XDG_DATA_HOME/pilotage/offline` | `%LOCALAPPDATA%\Pilotage\Offline` | `Application Support/Offline`, excluded from backup | OPFS `/offline`, persistent storage requested |
| `Cache` | Derived data that the app can make again. The system can purge it. | `$XDG_CACHE_HOME/pilotage` | `%LOCALAPPDATA%\Pilotage\Cache` | `Library/Caches` | OPFS `/cache` |
| `Config` | Settings. | `$XDG_CONFIG_HOME/pilotage` | `%APPDATA%\Pilotage\Config` | `Application Support/Config` | OPFS `/config` |
| `Temp` | Data for one operation. | `$TMPDIR` | `%TEMP%` | `tmp/` | Memory, or OPFS `/temp` |

Code names a file as a class and a logical path, for example
`(Offline, "packages/terrain/<release>/manifest.json")`. Code never builds a
platform path.

### The host resolves the roots

- The Apple host resolves the five roots in Swift at startup. Swift sets the
  backup exclusion on the `Offline` root and gives the roots to Rust. This is
  the same pattern that `AviationDataWorker` uses now.
- The Linux and Windows hosts resolve the roots with the `directories` crate.
- The browser host opens the OPFS directories.

### One read port, one writer

- **Read port.** Readers use one small interface: `open`, `len`, and
  `read_at` on a (class, logical path) name. It is asynchronous, because OPFS
  is asynchronous outside a worker. A native implementation can wrap blocking
  calls. The interface is in a neutral crate that Pilotage and Navigate both
  pin. The `navigate-data` read port is the start point. Its `DataUri` becomes
  a class and a logical path.
- **Writer.** Only the Pilotage package store writes, replaces, or removes
  data in the `Offline` class. Navigate reads only. This keeps the Navigate
  rule "consumes, never fetches".
- The interface does not include file locks, memory maps, symbolic links,
  file descriptors, or directory watching. Their behavior is different on
  OPFS and on Windows.

### Commit by manifest, not by rename

OPFS does not give an atomic rename in all browsers. Packages are content
addressed and written once, so the store does not need one:

1. Write each chunk to `Offline/chunks/<sha256>`. Verify its digest.
2. Write the package manifest last. The manifest is the commit record. A
   manifest that is not present means that the install is not complete.
3. Change the selected release of a product with one small pointer file.

Releases share chunks. A refined release after new flights writes only the
chunks that changed (Navigate package lineage).

### SQLite

A native host can keep SQLite files in these classes. In the browser,
`rusqlite` does not build for `wasm32-unknown-unknown`. A browser host that
needs SQLite must use a SQLite build with an OPFS file system, which runs in a
worker. Otherwise the browser uses the chunked package formats. Each data
product records which of these two it supports.

## Consequences

- One layout and one set of rules apply on every platform. A new platform
  needs only a root resolver and, for the browser, the OPFS reader.
- The backup and eviction rules are part of the class, so a component cannot
  forget them.
- Navigate needs a change: `DataUri` becomes a class and a logical path. This
  change follows the Navigate data port pull requests.
- Existing `pilotage://` resource bindings and the `AviationData` folder must
  move to the new layout. A migration step copies or links the installed data
  once.
- The neutral read-port crate is a new pinned dependency for Navigate and
  Pilotage. It has no I/O and no platform code.
- The store does not give POSIX semantics. Code that needs a lock or a memory
  map must use a platform-specific adapter outside the port.
