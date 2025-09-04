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

## Use Cases

These use cases capture the real product scenarios that drive the architecture:

1. Two communication channels: local (LAN) and global (internet). Global mode requires the user to be authenticated (logged in) to the global service.
2. The application must be able to function entirely on the local network (no login or external services), or operate in both local and global modes ("global mode").
3. Automatic channel switching: the system should automatically prefer direct LAN paths and fall back to global/internet relay paths without user intervention.
4. Devices on a LAN can share resources called "sendlets". Sendlets are typed payloads (see below).
5. Sendlets include multiple kinds of data: clipboard items, files (media, documents, applications, archives), URLs/links, and small structured data. The system must treat types appropriately (e.g., file streaming vs short clipboard push).
6. Sendlets can be exchanged across both local and global networks with the same UX and guarantees.
7. Everything transferred is end‑to‑end encrypted by default and non‑negotiable.
8. Simple pairing: all that is required is scanning a code shown on a device (or vice versa). This must work both on local and global paths.
9. All sendlet transfers use per‑session encryption keys derived automatically from device passkeys; the user is only prompted for a PIN/passkey when necessary.
10. Storage/backends: local mode uses SQLite for transient state; global mode uses Supabase only as a transient relay/discovery aid (no persistent user data). Files uploaded to the global relay are deleted once delivered and are encrypted end‑to‑end.
11. Native frontends (macOS, Windows, Linux, Android, iOS via Flutter) can operate in local and global modes. The web frontend (TypeScript) works in global mode only.
12. Minimal user interaction: the app must minimize prompts and make channel switching and pairing seamless.

These use cases are authoritative and drive the rest of this document.

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
 - Sendlet manager: typed sendlet handling (clipboard, file stream, URL, structured data) and policies for how each type is serialized, chunked, and encrypted.
 - Crypto & identity: device keys, short authentication strings (SAS), end‑to‑end encryption, and device passkey handling. Encryption is a core component (payload‑level AEAD) using ChaCha20‑Poly1305 with X25519 key agreement and HKDF key derivation. Passkey APIs automatically derive session keys and gate user confirmation when required.
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
- globalsend-crypto (lib): identity, SAS verification, key management, X25519 ECDH, HKDF, and ChaCha20‑Poly1305 AEAD helpers.
 - globalsend-crypto (lib): identity, SAS verification, key management, passkey support (PIN/biometric integration), X25519 ECDH, HKDF, and ChaCha20‑Poly1305 AEAD helpers.
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
- Data protocol: chunked streams with length‑prefix framing; integrity via per‑chunk BLAKE3. In addition to transport security (QUIC/TLS), every data payload (chunk stream) is encrypted at the payload level using ChaCha20‑Poly1305 AEAD. This provides end‑to‑end confidentiality even when relays/transport endpoints are used.
 - Data protocol: chunked streams with length‑prefix framing; integrity via per‑chunk BLAKE3. In addition to transport security (QUIC/TLS), every data payload (sendlet payload, chunk stream, or small item) is encrypted at the payload level using ChaCha20‑Poly1305 AEAD. This provides end‑to‑end confidentiality even when relays/transport endpoints are used. Payload encryption is derived from device passkeys and session ECDH.
- Chunking: FastCDC (content‑defined) for robust delta detection; target chunk size ~1MB (configurable).
- Hashing: BLAKE3 for chunk IDs and file digests; file digest is a rolling hash over chunk digests.
- Manifests: lists of files -> list of chunks (id, size, offsets, permissions, mtime, xattrs where supported).

## Security Model

- Device identity: Ed25519 keypair generated on first run, stored in OS‑appropriate config dir.
- Session authentication: SAS (short authentication string) or code phrase; user visually compares.
- Transport security: QUIC with TLS 1.3 (rustls). Self‑signed, ephemeral certs bound to device key via SAS confirmation.
- Transport security: QUIC with TLS 1.3 (rustls). Self‑signed, ephemeral certs bound to device key via SAS confirmation.
- Payload encryption (core): ChaCha20‑Poly1305 AEAD encrypts each chunk stream. Keys are derived per‑session using X25519 ECDH between device ephemeral/static keys and HKDF (HKDF‑SHA256) to produce AEAD keys and nonces. This design defends against relay/server compromise and provides an explicit, auditable separation between transport layer security and end‑to‑end payload confidentiality.
 - Device identity & passkey: each device has an Ed25519 identity key and an associated device passkey (user PIN or biometric-backed OS passkey where available). The passkey unlocks long‑term private material or authorizes ephemeral key use. The passkey model minimizes user prompts (unlock once per session or use OS biometric).
 - Session authentication: SAS (short authentication string) or code phrase; user visually compares, or scans a QR / enters a short code displayed by the other device. The scanned code contains the rendezvous token and the session fingerprint.
 - Transport security: QUIC with TLS 1.3 (rustls). Self‑signed, ephemeral certs bound to device key via SAS confirmation.
 - Payload encryption (core): ChaCha20‑Poly1305 AEAD encrypts each sendlet payload or chunk stream. Keys are derived per‑session using X25519 ECDH between device ephemeral/static keys and HKDF (HKDF‑SHA256) to produce AEAD keys and nonces. Passkeys are used to unlock or derive device private keys as needed. This design defends against relay/server compromise and provides an explicit, auditable separation between transport layer security and end‑to‑end payload confidentiality.
- Authorization: explicit accept on the receiving side with scope (e.g., chosen folder) and operation type (send/recv/sync).
- Replay protection: nonces and session IDs; manifests include timestamps and versioning.
- Replay protection: nonces and session IDs; manifests include timestamps and versioning. AEAD nonces use a session nonce + per‑chunk counter to avoid reuse; keys are rotated per session.
- Metadata minimization: only necessary metadata is exchanged; filenames protected where feasible.

## Storage & Backend

- Local mode (LAN only): runtime state and small manifests are stored in a local SQLite database. No global service is required for discovery or transfer.
- Global mode (internet): Supabase is used as the authenticated rendezvous and transient relay. Supabase is only used to aid transfers — all payloads are end‑to‑end encrypted and Supabase should not be able to read data. Uploaded blobs are ephemeral and deleted once delivered; the architecture documents policies to ensure intransience and automatic cleanup.
- Privacy guarantees: all payload content remains encrypted end‑to‑end; the global backend only sees encrypted blobs and minimal metadata required for routing (sizes, encrypted identifiers). The system is designed so the server is never a data controller — it's a transient router.

Threats considered:
- MITM on first contact (mitigated by SAS / code verification).
- Relay/rendezvous compromise (no content visibility; limited metadata exposure; rate‑limiting).
- Directory traversal (path normalization and sandboxing targets).

Additional threat mitigations from payload encryption:
- Relay/rendezvous compromise: relays never see plaintext file/chunk contents; only encrypted frames and minimal metadata (sizes, approximate counts) are exposed.
- Compromised transport endpoints: TLS termination points do not have AEAD keys; only endpoints that complete X25519 handshake and SAS verification can decrypt payloads.

## Passkeys, WebAuthn, and Cross‑Device Authentication

Passkeys are central to globalsend's UX and security model. They are used both to protect device private material and — when the user chooses — to authenticate with the global service (Supabase) to enable global mode. Key points:

- Platform support: native frontends should use each platform's biometric/passkey APIs where available. The web frontend uses WebAuthn for platform authenticators. All platforms expose an API in `globalsend-crypto` to derive or unlock local key material from passkeys.
- Data protection role: a passkey unlocks the device's long‑term identity private key or is used to derive a key‑encryption‑key (KEK). Private keys stored on disk are encrypted with the KEK; the user only enters a PIN or uses biometric once per session (or per policy) to unlock the KEK.
- Authentication role: if a user opts into global mode, the passkey can be used to authenticate to Supabase (WebAuthn or platform flow). Authentication is distinct from payload encryption — but the same passkey can serve both roles to provide a simpler UX.
- Cross‑device provisioning: when a user wants to add a new device (e.g., phone → desktop), the following flow is used:
   1. Initiator (desktop) displays a QR or short numeric code representing a one‑time session offer and a short session fingerprint.
 2. Responder (mobile) scans the QR and performs a rendezvous handshake with the initiator via LAN or the global rendezvous.
 3. Devices perform an authenticated ECDH (ephemeral X25519) and the initiator wraps the device private material (or a per‑session AEAD key) under a KEK derived from the responder's public key and the initiator's passkey‑authorized secret.
 4. The wrapped key is sent over the established encrypted channel. The responder uses its passkey to unwrap and persist the private material, optionally adding its own biometric/webauthn method as an additional unlocking method.

This model allows a user to provision new devices with a single scan and minimal prompts while preserving end‑to‑end encryption guarantees. Each device may add multiple authenticators (biometric, PIN, platform authenticator via WebAuthn) that can unlock the same private material via key wrapping (encrypted blobs) without exposing raw private keys.

- WebAuthn specifics: the web client registers a credential via WebAuthn and derives or obtains a wrapping/unlocker credential. The web flow is global‑mode oriented (because browsers normally cannot act as arbitrary LAN peers without special handling), but a scoped code + WebAuthn pairing flow can allow the web client to participate in cross‑device provisioning.

- Recovery & multi‑auth: users can add multiple authenticators on different devices. Recovery strategies and backup/restore flows are explicitly documented and require explicit user consent because they increase attack surface. Device removal revokes the stored wrapped keys and updates server‑side rendezvous records when applicable.

### Public links and shareable tokens

Global mode supports optional public links: the user can create a public share link for a sendlet. By default all sendlets are end‑to‑end encrypted and private; a public link is an explicit, deliberate action.

- Public link design: to preserve the principle that content is never leaked to the relay in plaintext, a public link contains a reference to the encrypted object plus an embedded decryption token (a symmetric AEAD key or token). The public link encodes both the object ID (on the relay) and the AEAD key required to decrypt. Anyone with the link can download and decrypt the object.
- Tradeoffs: public links weaken confidentiality (anyone with link can access). They are opt‑in and should be rate‑limited, TTL‑limited, and revocable. The UI must warn users about the security implications.
- Local public sharing: technically possible (e.g., a device could start a short‑lived HTTP share on LAN), but this is not surfaced by default since LAN devices already have implicit reachability; if implemented it will follow the same encrypted‑token design.

These passkey and cross‑device patterns are designed to keep user interaction minimal while enabling secure cross‑device key transfer and flexible authenticators.

## Discovery

- LAN: mDNS/Bonjour (advertise `_globalsend._udp` and `_globalsend._quic` service records). Include capabilities and a session ID.
- Internet: user‑provided rendezvous code or link; server mediates peer introduction only. Control messages are authenticated and limited.

- Internet (global mode): a logged‑in user uses Supabase auth. The rendezvous flow exchanges encrypted offers and endpoints; users scan codes or exchange short links. The web frontend (global mode) authenticates via Supabase and uses the same passkey‑derived session flow for payload encryption.

## NAT Traversal & Relay

- Attempt UDP hole punching for QUIC. If both peers behind symmetric NATs or blocked UDP, fallback to:
  - QUIC over relay (user‑configurable relay URI) or
  - TCP/TLS fallback for control plane, relay for data plane.
- Relay is stateless for content; rate limiting and auth tokens prevent abuse.

Notes on Supabase relay behavior:
- The Supabase relay acts as an authenticated, transient object store and WebSocket rendezvous layer when direct connections fail. Blobs uploaded to the relay remain encrypted and are deleted immediately after successful delivery or after a short TTL. The relay should never be treated as long‑term storage.

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

- Config file: `config.toml`; device key(s): `keys.json` or `keys.bin`. Long‑term keys should integrate with OS keychain/keyring where possible; ephemeral keys derived from passkeys are stored encrypted on disk.

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
   - Supabase integration for authenticated rendezvous and transient relay; enforce encryption, TTL, and automatic deletion policies.
4. Polishing & GUI
   - TUI enhancements; optional GUI (Tauri/egui); richer error reporting; packaging.

## Open Questions / Decisions To Validate

- Use CBOR vs bincode for control plane? (debuggability vs speed)
- Default chunk size and FastCDC parameters per platform.
- Built‑in relay vs separate deployment guide.
- AEAD choices: ChaCha20‑Poly1305 is chosen for lightweight high‑performance payload encryption; confirm if additional AEAD (e.g., AES‑GCM) must be supported for hardware acceleration on some platforms.
- Key storage and OS keychain integration strategy for long‑term device keys vs ephemeral keys.

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
