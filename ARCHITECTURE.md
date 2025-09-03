# GLOBALSEND Architecture

A cross‑platform, seamless, unobtrusive tool for sharing and syncing anything, anywhere — on local networks or over the internet.

This document describes the system architecture, guiding principles, and major components. It’s intended for contributors and maintainers.

## Goals

- Share and sync files/folders easily across devices with minimal setup.
- Work well on LAN and across the internet, with automatic discovery where possible.
- Privacy and security by default: end‑to‑end protection and explicit trust establishment.
- Resilient and fast transfers: resume, deduplicate, and avoid re‑sending existing data.
- Cross‑platform (Linux, macOS, Windows; later mobile) and low‑friction CLI first.

## Non‑Goals (for now)

- Full cloud storage product (we don’t host user data; optional relay is stateless for data content).
- Multi‑tenant account system. Pairing/trust is device‑centric.
- Complex ACLs beyond simple device authorization and session‑scoped permissions.

## High‑Level Overview

Globalsend splits responsibilities into a control plane and a data plane:

- Control plane: session establishment, discovery, authentication, capability negotiation, sync planning.
- Data plane: high‑throughput, encrypted content transfer and delta sync.

On LAN, devices discover peers via mDNS/Bonjour and establish a direct connection.
Over the internet, devices use a small rendezvous service to exchange connection info and attempt direct NAT traversal; fallback to an encrypted relay when direct paths fail.

```
+--------------------+        (LAN)         +--------------------+
|   Device A (CLI)   | <---- mDNS/QUIC ----> |   Device B (CLI)   |
+--------------------+                       +--------------------+
        |  (Internet)
        v
+--------------------+     WebSocket/TLS     +--------------------+
| Rendezvous Service | <--------------------> |  Device C (CLI)    |
|  (control only)    |       (control)       |  (NAT traversal)    |
+--------------------+                        +--------------------+
        ^                                              |
        |                QUIC (direct or relay)        v
        |                                       +------------------+
        +--------------- Optional Relay --------|   Relay (data)   |
                                                +------------------+
```

## Component Breakdown

- CLI (binary): user interface, commands (send, recv, sync, connect, pair, discover).
- Core engine (lib): sessions, state machine, job orchestration, config, and persistence.
- Discovery: mDNS/Bonjour on LAN; code/URL‑based rendezvous on Internet.
- Transport: QUIC for data (UDP) with TLS 1.3; TCP/TLS fallback for control where needed.
- NAT traversal: UDP hole‑punching attempts; relay fallback for stubborn NATs.
- Sync engine: content‑defined chunking, hashing, delta computation, resume, deduplication.
- Crypto & identity: device keys, short authentication strings (SAS), end‑to‑end encryption.
- Protocol & schemas: versioned messages for control and sync negotiation.
- Telemetry & logging: structured logging for debugging; privacy‑sensitive by default.

## Proposed Crate Layout (Cargo workspace)

To keep the codebase modular, we plan a workspace with these crates:

- globalsend-cli (bin): CLI UX, argument parsing, TUI progress.
- globalsend-core (lib): sessions, job orchestration, config, state persistence.
- globalsend-transport (lib): QUIC/TCP, TLS, NAT traversal, relay client.
- globalsend-discovery (lib): mDNS, rendezvous client.
- globalsend-sync (lib): chunking, hashing, diff/merge, manifests, resume.
- globalsend-crypto (lib): identity, SAS verification, key management.
- globalsend-proto (lib): versioned message schemas and encoding (serde‑based).
- globalsend-relay (bin/lib): optional rendezvous + relay server (can live here or in a separate repo).

The current repo will evolve into a workspace; the initial `globalsend` bin can delegate to `globalsend-core` as features land.

## Data Flows

### 1) Send a file on LAN

1. Sender invokes `globalsend send <path>`.
2. Discovery advertises a one‑time session (mDNS service record with ephemeral ID + capabilities).
3. Receiver runs `globalsend recv` and discovers the sender.
4. Control handshake: device identity exchange, SAS code display on both ends; user confirms.
5. Data plane: QUIC session established; metadata and chunk inventory exchanged; missing chunks sent.
6. Receiver writes file(s) atomically and verifies hashes.

### 2) Sync a folder over the Internet

1. Both devices run `globalsend sync <folder>`; user provides a rendezvous code/URL.
2. Both connect to the rendezvous service via WebSocket/TLS; exchange offers (endpoints, fingerprints).
3. Attempt direct QUIC connection (UDP hole punching); else use the relay.
4. Control plane negotiates manifests (FastCDC chunk map + BLAKE3 hashes).
5. Sender streams only missing chunks; receiver reconstructs and applies rename/move ops.
6. Periodic watches keep the folder in sync (notify‑based file watching).

## Protocols & Formats

- Control protocol: serde‑based (CBOR or bincode) messages over QUIC stream or WebSocket (for rendezvous). Versioned with semantic negotiation.
- Data protocol: chunked streams with length‑prefix framing; integrity via per‑chunk BLAKE3; end‑to‑end via QUIC TLS.
- Chunking: FastCDC (content‑defined) for robust delta detection; target chunk size ~1MB (configurable).
- Hashing: BLAKE3 for chunk IDs and file digests; file digest is a rolling hash over chunk digests.
- Manifests: lists of files -> list of chunks (id, size, offsets, permissions, mtime, xattrs where supported).

## Security Model

- Device identity: Ed25519 keypair generated on first run, stored in OS‑appropriate config dir.
- Session authentication: SAS (short authentication string) or code phrase; user visually compares.
- Transport security: QUIC with TLS 1.3 (rustls). Self‑signed, ephemeral certs bound to device key via SAS confirmation.
- Authorization: explicit accept on the receiving side with scope (e.g., chosen folder) and operation type (send/recv/sync).
- Replay protection: nonces and session IDs; manifests include timestamps and versioning.
- Metadata minimization: only necessary metadata is exchanged; filenames protected where feasible.

Threats considered:
- MITM on first contact (mitigated by SAS / code verification).
- Relay/rendezvous compromise (no content visibility; limited metadata exposure; rate‑limiting).
- Directory traversal (path normalization and sandboxing targets).

## Discovery

- LAN: mDNS/Bonjour (advertise `_globalsend._udp` and `_globalsend._quic` service records). Include capabilities and a session ID.
- Internet: user‑provided rendezvous code or link; server mediates peer introduction only. Control messages are authenticated and limited.

## NAT Traversal & Relay

- Attempt UDP hole punching for QUIC. If both peers behind symmetric NATs or blocked UDP, fallback to:
  - QUIC over relay (user‑configurable relay URI) or
  - TCP/TLS fallback for control plane, relay for data plane.
- Relay is stateless for content; rate limiting and auth tokens prevent abuse.

## Filesystem Semantics

- Preserve file permissions and mtimes where supported; configurable symlink handling.
- Cross‑platform path normalization; Windows path edge cases handled (reserved names, long paths).
- Exclusions via `.globalsendignore` (gitignore syntax) and CLI flags.
- Atomic writes: download to temp file, fsync, rename; partial downloads resume.

## Performance Considerations

- Asynchronous I/O (Tokio) with backpressure and pipelining.
- QUIC streams for parallel file/chunk transfers.
- Zero‑copy where possible; large read buffers; zstd compression optional.
- Bloom filters or hash summaries to reduce manifest exchange overhead.

## Telemetry & Logging

- `tracing` crate with human‑readable default subscriber; structured logs via env flag.
- Redact sensitive data by default. No silent network beacons.

## Configuration & Paths

- XDG on Linux: `$XDG_CONFIG_HOME/globalsend/` (else `~/.config/globalsend/`).
- macOS: `~/Library/Application Support/globalsend/`.
- Windows: `%APPDATA%\globalsend\`.
- Config file: `config.toml`; device key(s): `keys.json` or `keys.bin` with OS keychain integration later.

## External Dependencies (planned)

- Async runtime: `tokio`.
- QUIC: `quinn` (rustls under the hood).
- TLS: `rustls`.
- Discovery: `mdns`/`libmdns`/`bonjour` compatible crate.
- Chunking: `fastcdc`.
- Hashing: `blake3`.
- Compression: `zstd`.
- File watching: `notify`.
- Serialization: `serde`, `serde_cbor` or `bincode`.
- CLI: `clap`.

## Versioning & Compatibility

- Protocol versions negotiated at session start. Backward‑compatible minor changes; breaking changes bump major.
- Crate versions follow semver. `globalsend-proto` drives wire compatibility.

## Testing Strategy

- Unit tests for chunking, hashing, manifest diff, SAS verification.
- Integration tests: simulated peers over loopback; NAT scenarios with containers.
- Property‑based testing for chunk boundaries and hash maps.
- Performance benchmarks for large file and many small files scenarios.

## Roadmap (Phases)

1. MVP (LAN only)
   - CLI send/recv single files and simple folders via mDNS + QUIC.
   - SAS pairing; progress bars; resume on interruption.
2. Sync Engine
   - FastCDC manifests, delta sync, metadata preservation, ignore patterns.
3. Internet Mode
   - Rendezvous service (WS/TLS), NAT traversal, relay fallback, auth tokens.
4. Polishing & GUI
   - TUI enhancements; optional GUI (Tauri/egui); richer error reporting; packaging.

## Open Questions / Decisions To Validate

- Use CBOR vs bincode for control plane? (debuggability vs speed)
- Default chunk size and FastCDC parameters per platform.
- Built‑in relay vs separate deployment guide.
- Optional additional payload‑level encryption beyond QUIC.

## ASCII Sequence Diagrams

Send (LAN) with SAS verification:

```
Sender                     Receiver
  |    mDNS advertise         |
  |-------------------------->|
  |     discover              |
  |<--------------------------|
  |   QUIC handshake (TLS)    |
  |<=========================>|
  |   exchange SAS (6‑words)  |
  |<------------------------->|
  |   user compares & OK      |
  |<------------------------->|
  |   manifest -> missing     |
  |-------------------------->|
  |   send chunks             |
  |==========================>|
  |   verify & commit         |
  |<--------------------------|
```

Sync (Internet) with rendezvous and relay fallback:

```
A                         Rendezvous                      B
|-- WS/TLS connect  ------------------------------->  |  (code 9‑words)
|<-- peer info / offer -----------------------------  |
|-- attempt QUIC (UDP punch) ======================>  |
|        [fails?]                                     |
|-- QUIC via Relay =================================> |
|<-- manifests / plan ==============================> |
|== delta chunks ===================================>|
|<== acks / verify ================================= |
```

## Contribution Guidance

- Keep crates cohesive and dependency graph minimal.
- Favor small, versioned protocol changes and clear feature flags.
- Wire changes require updating `globalsend-proto` and compatibility notes here.

---

Last updated: 2025‑09‑03.
