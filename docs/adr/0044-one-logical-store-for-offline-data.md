# ADR-0044: One logical store for offline data on every platform

- Status: Proposed
- Date: 2026-09-23

## Context

Pilotage and Navigate use data that must be available without a network:
navigation data, terrain, imagery packages, missions, and settings. This
record uses the term **stored data** for all of this data. The same Rust core
runs on four kinds of host:

- a Linux host,
- a Windows workstation,
- the Apple app on iPadOS and visionOS,
- a browser, through WebAssembly.

Each host has its own standard place for application data. Linux uses the XDG
base directories. Windows uses `AppData`. Apple platforms use the application
container. A browser has no file system path. It gives the Origin Private File
System (OPFS) and the Cache API.

Each component chooses a location for itself. For example, the offline
aviation data work chooses a folder in Application Support on the Apple app.
It also chooses a resource-name scheme of its own. The Navigate visual tools read packages
through the `navigate-data` read port, and the browser preview keeps its
packages in OPFS.

The code does not define a common layout, a common set of data classes, or
one owner for writes in each class. A component that runs on a new platform must choose a
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

All stored data belongs to one `StorageClass`. The `Offline` class is one of
the five classes. It holds only released data that the app can download
again.

| Class | Content | Linux and Windows | Apple | Browser |
|---|---|---|---|---|
| `Data` | Data that the operator makes, such as missions and journals. Include in backup. | Child `data` of the `directories` `data_dir` (roaming on Windows) | `Application Support/Data` | OPFS `/data`, persistent storage requested |
| `Offline` | Released data that the app can download again: navigation data, terrain, imagery, map packages. Exclude from backup. Do not purge. | Child `offline` of the `directories` `data_local_dir` (local on Windows) | `Application Support/Offline`, excluded from backup | OPFS `/offline`, persistent storage requested |
| `Cache` | Derived data that the app can make again. The system can purge it. | The `directories` `cache_dir` (local on Windows) | `Library/Caches` | OPFS `/cache` |
| `Config` | Settings. | The `directories` `config_dir` (roaming on Windows) | `Application Support/Config` | OPFS `/config` |
| `Temp` | Data for one operation. | A child of `std::env::temp_dir` | `tmp/` | Memory, or OPFS `/temp` |

The `Offline` root is a sibling of the `Data` root. It is never inside the
`Data` root. On Linux, `data_dir` and `data_local_dir` are the same directory,
so the two named children keep the roots apart. A backup of the `Data` root
does not include `Offline` data.

Code names a file as a class and a logical path, for example
`(Offline, "packages/terrain/<release>/manifest.json")`. Code never builds a
platform path.

### The host resolves the roots

- The Apple host resolves the five roots in Swift at startup. Swift sets the
  backup exclusion on the `Offline` root and gives the roots to Rust.
- The Linux and Windows hosts resolve the roots with the `directories` crate
  (`ProjectDirs`), as the table gives.
- The browser host opens the OPFS directories.

### One read port, two writers

The **package store** is a proposed Pilotage component. It installs,
replaces, and removes released packages in the `Offline` class.

- **Read port.** Readers use one small interface: `open`, `len`, and
  `read_at` on a (class, logical path) name. It is asynchronous, because OPFS
  is asynchronous outside a worker. A native implementation can wrap blocking
  calls. The interface is in a neutral crate that Pilotage and Navigate both
  pin. The read port of the Navigate `navigate-data` crate is the start point.
  Its `DataUri` also names a class and a logical path.
- **Offline writer.** Only the package store writes, replaces, or removes
  data in the `Offline` class. Navigate reads only. This keeps the Navigate
  rule "consumes, never fetches".
- **Data writer.** On Apple, Linux, and Android hosts,
  `crates/pilotage-durable-storage` writes `Data`-class content, such as
  journals. It gives anchored directory handles, a `WriterLease`, and
  crash-durable writes. `pilotage-tuning-feedback` and
  `tools/flight-tune-aviate` use it for their journals. It is a native writer
  outside the read port.
- The read port does not include file locks, memory maps, symbolic links,
  file descriptors, or directory watching. Their behavior is different on
  OPFS and on Windows.

### Commit by manifest, not by rename

OPFS does not give an atomic rename in all browsers. Packages are content
addressed and written once, so the package store does not need one:

1. Write each chunk to `Offline/chunks/<sha256>`. Verify its digest.
2. Write the package manifest last. The manifest is the commit record. A
   manifest that is not present means that the install is not complete.
3. Change the selected release of a product with one small pointer file.

Releases share chunks. A refined release after new flights writes only the
chunks that changed (Navigate package lineage).

### SQLite

A native host can keep SQLite files in these classes. The workspace
`rusqlite` configuration uses the bundled C SQLite (root `Cargo.toml`). This
configuration does not build for `wasm32-unknown-unknown`. A browser host that
needs SQLite uses `sqlite-wasm-rs` with an OPFS file system in a worker.
Otherwise the browser uses the chunked package formats. Each data product
records which of these two it supports.

## Consequences

- One layout and one set of rules apply on every platform. A new platform
  needs only a root resolver and, for the browser, the OPFS reader.
- The backup and eviction rules are part of the class, so a component cannot
  forget them.
- Navigate needs a change: `DataUri` also names a class and a logical path.
- Each component that chooses its own location for stored data must move its
  data to the new layout. A migration step copies or links the installed data
  once.
- The neutral read-port crate is a new pinned dependency for Navigate and
  Pilotage. It has no I/O and no platform code.
- The store does not give POSIX semantics. Code that needs a lock or a memory
  map must use a platform-specific adapter outside the port.
