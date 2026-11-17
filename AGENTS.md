# AGENTS.md

Style rules and workflows for agents and contributors working in this repository.

## Project context

`usbpd` is a `no_std` USB Power Delivery 3.2 crate. The spec text lives in the repo root.
Workspace members: `usbpd` and `usbpd-traits`. Firmware builds live in `examples/` (excluded from the workspace).
Edition 2024, MSRV 1.88. The `epr` feature is enabled by default. Building without it must stay green
(CI gap today: run `cargo build -p usbpd --no-default-features` locally).

## Spec references

Cite the spec canonically: `PD 3.2 Sec. 6.2.1.2.5.2`, `Sec. 8.3.3.2.3`, `Tab. 6.50`, `Fig. 6.9`.
Use section numbers, not page numbers. Page numbers only where the section reference is ambiguous.
This applies to doc-comments, comments, CHANGELOG entries and PR descriptions.

## Comments and doc-comments

Full sentences. Sentence case. Terminal period.
Keep docs brief. Short sentences. No semicolons. No em-dashes. ASCII letters only (no smart quotes).
State purpose and pitfalls. Skip the obvious.
Explain why and how it works only where that is not obvious from the code. Never narrate what a line does.

Doc-comments: one-line summary, optional spec citation, optional pitfall note. No signature repetition.
All public items need doc-comments (`#![warn(missing_docs)]`).
Error `Display` strings are exempt from sentence style: lowercase, no trailing period,
data via interpolation (e.g. `"protocol error: {0}"`).

`FIXME`/`TODO` comments must state the blocker, not the wish.

## Panic policy

Nothing reachable from partner-supplied wire data or driver callbacks may panic.
Parse paths use `TryFrom` or total fallbacks. Validate untrusted lengths before slicing.
`unwrap`/`expect` are reserved for: test code, fixtures in `dummy/`, static invariants
(`unreachable!`, `debug_assert!`), and builders with statically valid arguments.

Behavior fixes land test-first: failing test, then fix, then green.

## Tests

Unit tests inline in `#[cfg(test)] mod tests`.
Policy engine state-machine suites live in a sibling `tests.rs`.
Feature-gated suites live in sibling files gated `#[cfg(test)] #[cfg(feature = "epr")]`.
Code shared between lib and tests gates as `cfg(any(feature = "epr", test))`.
Test names follow `test_<subject>_<outcome>`.

## Naming

Standard Rust naming. PE states mirror spec state names (`SendCapabilities`, `WaitForVconn`).
Message types mirror spec message names (EPR_Mode becomes `EprMode`).
Acronyms capitalize the first letter only in identifiers (`Epr`, not `EPR`).

## Logging

Use the facade macros in `fmt.rs` (`trace!`, `debug!`, `warn!`, `error!`) only.
Never call `defmt` or `log` directly. Enabling both backends together is a compile error.
Levels: `trace`/`debug` for protocol and PE step events, `warn` for recoverable anomalies,
`error` for escalation or terminal conditions. Messages: lowercase start, no trailing period.

## Formatting

Run `cargo +nightly fmt` (the rustfmt.toml options are nightly-only).
Imports group as std, external crates, then crate (`group_imports = "StdExternalCrate"`),
one `use` per module (`imports_granularity = "Module"`), max width 120.

## Verification loop

Run before declaring work done. Quick loop:

```
cargo +nightly fmt --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
cargo test --workspace
```

Feature-sensitive work adds `cargo test -p usbpd --no-default-features` and
`cargo build -p usbpd --features serde,log,defmt`.
The full CI matrix (host plus all firmware targets) is `bash .github/ci/build.sh`.

## Commit style

Conventional Commits with the types used in this history: `fix:`, `feat:`, `chore:`, `ci:`, `docs:`.
Imperative summary of at most 72 characters. Scope only when it clarifies.
Never commit without an explicit request.
