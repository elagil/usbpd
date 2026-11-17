# Changelog

All notable changes to the `usbpd` and `usbpd-traits` crates are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [4.0.0] - `usbpd`

### Breaking

- The policy engine `Error` type moved to the crate root (`usbpd::Error`). The sink and
  source policy engines re-export it under their prior paths
  (`sink::policy_engine::Error`, `source::policy_engine::Error`), so existing imports
  keep compiling.
- Removed unused `TimerType` variants: `BISTContMode`, `ChunkingNotSupported`,
  `DataResetFail`, `DataResetFailUFP`, `SinkTx`. The remaining variants are renumbered:
  numeric casts (`TimerType as u8`) no longer yield the old values.
- Removed `ParseError::ChunkOverflow` and `ParseError::ParserReuse`; chunked frames
  surface as `ParseError::ChunkedExtendedMessage`.
- Removed the unused `ChunkedMessageAssembler`/`ChunkedMessageSender` chunking helpers,
  the `extended::chunked` module and `Message::parse_extended_chunk`; chunking is handled
  entirely by the protocol layer. The constants `MAX_EXTENDED_MSG_LEN`, `MAX_CHUNKS` and
  `MAX_EXTENDED_MSG_CHUNK_LEN` moved accordingly (`MAX_EXTENDED_MSG_CHUNK_LEN` now lives
  at `protocol_layer::message::extended`).
- Reserved `EPR_Mode` actions map to a new `Action::Reserved(u8)` variant instead of panicking.
- Reserved `EnterFailed` reasons map to `UnknownCause` instead of panicking; the
  conversion stays a total `From<u8>`.
- VDM identity-header bitfields expose raw fields with fallible `try_*` converters
  instead of the previously panicking typed getters.
- A silent port partner under an explicit contract escalates via the NoResponseTimer to
  ErrorRecovery: `run()` returns `Error::ReconnectionRequired` instead of idling in `Disabled`
  (PD 3.2 Sec. 8.3.3.2.3).

### Added

- `usbpd-traits`: thiserror-based `Display`/`core::error::Error` impls on `DriverRxError`
  and `DriverTxError`; all remaining `usbpd` error types (`counters::Error`,
  `request::Error`, `dual_role::Error`) now implement them as well, simplifying error
  nesting with `#[from]`/`source()`.
- `epr` feature gate (enabled by default): EPR protocol paths compile out when disabled.
- Chunked extended message transmission plus chunk-request serving per PD 3.2
  Sec. 6.2.1.2.5.2: oversized payloads (e.g. `EPR_Source_Capabilities`) are fragmented,
  incoming chunk requests are acknowledged and served from the retained payload, and
  duplicate requests re-serve the chunk as the response retry. Chunked frames carry the
  Number of Data Objects count per Sec. 6.2.1.1.2.
- CC-hardware boundary documentation: CC detection, VBUS/VCONN control and FRS CC
  signaling are out of scope (companion crate or DPM implementations, see crate docs).

### Fixed

- Reserved `EPR_Mode` actions and unknown/reserved VDM fields no longer panic on parse.
- Sink-initiated `Data_Role_Swap` returns to `Ready` on SenderResponseTimer timeout
  instead of re-sending indefinitely (Sec. 8.3.3.19.1.5).
- Sink Vconn swap: VCONNOnTimer timeout triggers a Hard Reset (Sec. 8.3.3.20.5).
- Source `CapsCounter` handling per Sec. 8.3.3.2.3: increment per send, reset on GoodCRC;
  previously spurious saturation could disable the port.
- `hard_reset` is bounded by tHardResetComplete and errors on a persistently wedged
  driver instead of looping forever.
- Sink FRS: `source_on` placement corrected (Sec. 8.3.3.19.6.4/.6.6); the FRS receiver is
  disabled in `Accept` via `set_fr_swap_detect` (Sec. 8.3.3.19.4.3).
- Incoming `EPR_Request` is validated (EPR mode, object position 8..=14, PDO copy,
  EPRCapable bit) and triggers a Hard Reset when invalid, instead of being answered with
  `Not_Supported` (Sec. 6.4.9, 6.4.10.3.3, Tab. 6.23). A `Request` Message received in EPR
  mode initiates a Hard Reset as well (Sec. 6.4.2).
- Out-of-range chunk request numbers are ignored instead of indexing beyond the retained
  payload.

### Known limitations

- `Wait` message handling is incomplete on the responding side: swap requests
  (Data/Power/Vconn Swap) can only be answered with `Accept` or `Reject`, because the
  `DevicePolicyManager` decision hooks return `bool`. *Receiving* a `Wait` response to an
  initiated swap is handled correctly.
- Message-type coverage is incomplete.
- Fast Role Swap: message-based FRS (sending/receiving `FR_Swap`) is supported; CC-signaled
  FRS detection must be provided by an external Type-C layer through the DPM hooks
  (`fr_swap_signaled`, `set_fr_swap_detect`).
- The NoResponseTimer is armed only for the with-contract silent-partner path: without an
  explicit contract, escalation to `Disabled` happens via the HardResetCounter
  (spec-equivalent per Sec. 8.3.3.2.3 / 8.3.3.3.8).
- EPR cable discovery during mode entry is delegated to the DPM (`epr_cable_good`); no
  SOP'/' Discover Identity exchange is performed by the policy engine itself.
- Planned follow-up work (Wait tri-state, VDM forwarding, EPR cable discovery, status
  messages) is tracked with an API-evolution roadmap in `docs/spec_check.md`; additive
  trait methods land in minor releases, breaking changes are bundled into the next major.

## [3.0.0] - `usbpd`

### Added

- SPR and EPR source mode (`usbpd/src/source/`), first public release with it.
- Dual-role port (DRP) support (`usbpd/src/dual_role.rs`), with a DRP example.
- `uom-ng` for compile-time evaluation of unit conversion factors. Quantities from
  the original `uom` crate are distinct types. Note: `uom-ng` is a fork used for an
  important feature for now, and may be replaced in the future.

### Known limitations

- `Wait` message handling is incomplete.
- Message-type coverage is incomplete.
- The SPR/EPR source-mode policy engine is new in this release and is covered by
  host tests only.

## `usbpd-traits`

### [2.1.0] - `usbpd-traits`

### Added

- thiserror-based `Display`/`core::error::Error` impls on `DriverRxError` and
  `DriverTxError`, so driver errors nest cleanly in higher-level error types.

### [2.0.0] - 2026-02-09

### Changed

- `Driver::wait_for_vbus` takes `&mut self`.

### Added

- `Driver::HAS_AUTO_RETRY`: skip the protocol-layer retry loop when the PHY
  retries transmission automatically.

## `usbpd`

### [2.0.0] - 2026-02-09

### Added

- EPR sink mode: EPR messages, chunked extended messages, EPR mode entry/exit,
  EPR source capability handling.
- `serde` derives on sink capabilities.

### Changed

- Policy engine restructured into a module with dedicated host tests, plus many
  spec compliance fixes.
- Extended messages are sent using chunked mode.
- Requires `usbpd-traits` 2.0.

## [1.1.3] - 2025-08-14

- Refactor of PDO request naming and flexibility.

## [1.1.2] - 2025-08-10

- Documentation updates.

## [1.1.1] - 2025-08-03

- Version bump for `usbpd-traits` 1.1.0.

## [1.1.0] - 2025-08-03

- Transition from `futures` to `embassy-futures`.
- PPS periodic request as keep-alive.
- Fallible message parsing (parsing error type).
- `thiserror` error types.

## [1.0.3] - 2025-07-03

- Added `serde` support.

## [1.0.2] - 2025-06-23

- Made message types public, fixed `log` feature compilation.

## [1.0.1] - 2025-06-15

- CI-only change. Published alongside `usbpd-traits` 1.0.1, which adds the
  optional `Driver::HAS_AUTO_GOOD_CRC` constant.

## [1.0.0] - 2025-05-10

- First stable release.
- Split into two crates: `usbpd` and `usbpd-traits`.
