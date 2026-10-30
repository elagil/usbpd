# Changelog

All notable changes to the `usbpd` and `usbpd-traits` crates are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.0.0] — `usbpd`

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

### [2.0.0] — 2026-02-09

### Changed

- `Driver::wait_for_vbus` takes `&mut self`.

### Added

- `Driver::HAS_AUTO_RETRY`: skip the protocol-layer retry loop when the PHY
  retries transmission automatically.

## `usbpd`

### [2.0.0] — 2026-02-09

### Added

- EPR sink mode: EPR messages, chunked extended messages, EPR mode entry/exit,
  EPR source capability handling.
- `serde` derives on sink capabilities.

### Changed

- Policy engine restructured into a module with dedicated host tests, plus many
  spec compliance fixes.
- Extended messages are sent using chunked mode.
- Requires `usbpd-traits` 2.0.

## [1.1.3] — 2025-08-14

- Refactor of PDO request naming and flexibility.

## [1.1.2] — 2025-08-10

- Documentation updates.

## [1.1.1] — 2025-08-03

- Version bump for `usbpd-traits` 1.1.0.

## [1.1.0] — 2025-08-03

- Transition from `futures` to `embassy-futures`.
- PPS periodic request as keep-alive.
- Fallible message parsing (parsing error type).
- `thiserror` error types.

## [1.0.3] — 2025-07-03

- Added `serde` support.

## [1.0.2] — 2025-06-23

- Made message types public, fixed `log` feature compilation.

## [1.0.1] — 2025-06-15

- CI-only change. Published alongside `usbpd-traits` 1.0.1, which adds the
  optional `Driver::HAS_AUTO_GOOD_CRC` constant.

## [1.0.0] — 2025-05-10

- First stable release.
- Split into two crates: `usbpd` and `usbpd-traits`.
