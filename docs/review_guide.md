# Review Guide - `spec_compliance_fixes` branch

Reading companion for reviewing the uncommitted changes on this branch, with pointers into
`USB_PD_R3_2 V1.1 2024-10.pdf` (printed page = PDF page; the PDF has 1047 pages).

For the full per-item audit table see `docs/spec_check.md`; this guide is the condensed
review path, ordered so the spec reading builds up.

---

## 0. Orientation

| Doc | Role |
|---|---|
| `docs/spec_check.md` | Full audit table (62 items), spec refs, and the API-evolution roadmap |
| `CHANGELOG.md` | Release notes for `4.0.0` (Breaking / Added / Fixed / Known limitations) |
| `usbpd/src/protocol_layer/mod.rs` | The largest diff: chunked TX, chunk-request serving, bounded hard reset |
| `usbpd/src/{sink,source}/policy_engine/mod.rs` | PE state-machine fixes + de-duplication |
| `usbpd/src/{swap,swap_reply}.rs` | New shared DRP state types + shared swap-reply await |
| `usbpd/src/timers/mod.rs` | Dead timer variant deletions |
| `usbpd/src/protocol_layer/message/data/{epr_mode,vendor_defined}.rs` | No-panic parse hardening |

Test discipline: every fix was written test-first (failing test -> fix -> green), plus a
subagent audit that strengthened the weak ones. During the final
simplification pass the noisy `eprintln!` scaffolding (79 lines) was stripped and a
pre-existing no-op assertion in the source PE's `run_test_step` helper was repaired
(`matches!(&state, _new_state)` bound instead of compared - it could never fail); all
43 tests still pass with the real assertion in place. Final suite after the follow-up
hardening commits: 46 tests.

---

## 1. Protocol layer - chunked extended messages

**Spec to read:** Sec. 6.2.1.2.5.2 (p. 124) - the chunked example sequence in Figure 6.7 shows the
whole conversation. Then Sec. 6.12.2.1.2.2 `RCH_Pass_Up_Message` (p. 278) and
Sec. 6.12.2.1.2.4 `RCH_Requesting_Chunk` (p. 279) for the receiver side. Chunk timers:
Sec. 6.6.18.3 (p. 259).

**Code:**
- `Message::to_bytes` (`message/mod.rs`): payloads > 26 bytes now emit a partial chunk-0
  frame instead of one oversized "chunked" frame (the wire-visible spec violation).
  One unified code path: serialize the full payload, truncate the returned size to
  `min(len, 26)` - the <=26-byte and >26-byte cases are identical on the wire.
- `ProtocolLayer::transmit` retains the payload (`ChunkedTxState`), copying it out of the
  already-serialized frame buffer (no second serialization pass).
- `receive_message_inner` answers incoming `Chunk Request` frames from the retained
  payload via `transmit_chunk_response` (chunk number, full data size, <= 26 B data).
- New named state structs: `ChunkedRxState` (assembly) / `ChunkedTxState` (retained TX
  payload), `Option`-wrapped - the old parallel buffer+tuple fields are gone.
- The old `extended/chunked.rs` module (standalone `ChunkedMessageAssembler`/
  `ChunkedMessageSender` helpers, 542 lines) was deleted: chunking is handled entirely
  by the protocol layer, and nothing consumed the helpers. Chunk assembly now parses via
  the shared `Message::parse_extended_payload` (one implementation instead of three
 copies).

**Tests:** `test_oversized_extended_tx_is_chunked` (asserts the frame is exactly
header + ext-header + 26 bytes - fails pre-fix where it was 64), 
`test_chunk_request_is_served_from_retained_payload` (asserts served chunk bytes, length,
and that a pre-fix layer errors instead of serving). The captured-frame assembly test
(`epr_messages_test.rs`) now concatenates chunk payloads directly instead of using the
deleted assembler.

---

## 2. Hard reset + timers

**Spec to read:** Sec. 6.7.1.1 Transmitter Usage (p. 265) - retry/GoodCRC machinery;
`tHardResetComplete` in Tab. 6-68 (pp. 262-263). For the NoResponseTimer escalation,
Sec. 8.3.3.2.3 (p. 829) - read it before this section's code.

**Code:**
- `hard_reset()` (`protocol_layer/mod.rs`): the retry loop on `DriverTxError::Discarded`
  is bounded by `TimerType::HardResetComplete` (5 ms) and now returns
  `Err(ProtocolError::TxError(...))` on expiry - previously a wedged driver hung forever.
- `timers/mod.rs`: deleted 5 unused variants (`BISTContMode`, `ChunkingNotSupported`,
  `DataResetFail`, `DataResetFailUFP`, `SinkTx`) - features not implemented (BIST,
  Data Reset, non-chunking ports, SOP'' comms). Breaking change -> 4.0.0.

**Tests:** `test_hard_reset_bounded_when_driver_discards` (mock driver always discards ->
must error, hangs forever pre-fix).

---

## 3. Source policy engine - CapsCounter + NoResponseTimer

**Spec to read:** Sec. 8.3.3.2.3 `PE_SRC_Send_Capabilities` (p. 829) - the single most
relevant page for both fixes here. Key clauses:
- "The Policy Engine Shall then increment the CapsCounter"
- "If a GoodCRC Message is received... Reset the HardResetCounter and CapsCounter to zero"
- Bottom half: the NoResponseTimer AND HardResetCounter > nHardResetCount escalation table
  (not-connected -> Discovery/Disabled; previously connected -> ErrorRecovery).
Also Sec. 8.3.3.28.1 `ErrorRecovery` (p. 965) to understand the escalation target.

**Code:**
- `SendCapabilities` (`source/policy_engine/mod.rs`): CapsCounter increment added in the
  send path (the Discovery increment stays for the fail loop); both counters reset after a
  successful (GoodCRC-acked) send. Pre-fix, a long-lived renegotiating port could
  spuriously hit the 50-cap limit -> Disabled.
- With-contract silent partner: arms `NoResponseTimer` (5 s) then transitions to
  `State::ErrorRecovery` -> `run_step` returns `Err(Error::ReconnectionRequired)`.
  Deliberate spec-strict choice (was a livelock in Disabled); *not-connected* tx-failure
  still goes to `Discovery`.

**Tests:** `test_caps_counter_resets_on_goodcrc` (60 alternating fail/success cycles; probe
asserts counter == 0 after each GoodCRC-acked send), 
`test_no_response_timer_contract_silent_partner_error_recovers` (asserts the exact error).

---

## 4. Sink policy engine - swap fixes

**Spec to read:**
- Sec. 8.3.3.19.1.5 `PE_DRS_DFP_UFP_Send_Swap` (p. 889): SenderResponseTimer timeout -> Ready.
- Sec. 8.3.3.20.5 `PE_VCS_Wait_for_VCONN` (p. 921): VCONNOnTimer timeout -> Hard Reset.

**Code (`sink/policy_engine/mod.rs` error-mapping match):**
- `Data(Send)` timeout -> `Ready(false)` (pre-fix: generic fallback left the state in place,
  re-sending Data_Swap forever).
- `WaitForVconn` timeout -> `HardReset` (pre-fix: re-wait forever; the source PE already had
  this arm).

**De-duplication (same section, mechanical):**
- New `usbpd/src/swap.rs`: `DataRoleSwap`, `VcsState` and the PE `Error` enum are now
  defined once and shared by both policy engines (they previously existed twice with
  drifted naming, e.g. `VcsForceVconn` vs `ForceVconn`). `PowerRoleSwap`/`FastPowerRoleSwap`
  stay per-role on purpose - the spec defines *different* state machines per side
  (PE_PRS_SNK_SRC vs PE_PRS_SRC_SNK, Sec. 8.3.3.19.4 vs Sec. 8.3.3.19.3).
- New `usbpd/src/swap_reply.rs`: the "await Accept/Reject/Wait (+ Not_Supported) on
  SenderResponseTimer" pattern shared by 5 swap executors (DRS sink+source, PRS source,
  VCS sink+source) collapsed into `await_swap_reply`. Behavioral notes: the source VCS's
  ReceiveTimeout->Ready mapping and both sides' Wait->Ready handling are preserved exactly.
- The dead sink `FastPowerRoleSwap::Start` state was folded into the DPM event arm that
  jumped straight to `Send` anyway (its only reachable successor).

**Tests:** `test_data_role_swap_send_timeout_to_ready`, `test_vconn_swap_wait_for_vconn_timeout_hard_resets`
(both verified to fail pre-fix), plus `test_data_role_swap_send_transmits_request`
(happy-path: the transmitted frame is a `DrSwap`).

---

## 5. Sink FRS + PRS-Accept wiring

**Spec to read:**
- Sec. 8.3.3.19.4.3 `PE_PRS_SNK_SRC_Accept_Swap` (p. 899): "disable the Fast Role Swap receiver".
- Sec. 8.3.3.19.6.2-6.6 (pp. 907-909): full sink->source FRS flow. Note 6.4 `VBUS_Applied` waits
  for the DPM's "vSafe5V applied" indication and 6.6 `Source_on` only sends PS_RDY on exit.

**Code:**
- `PowerRoleSwap::Accept` calls `<DPM as DrpDevicePolicyManager>::set_fr_swap_detect(false)`
  (the hook previously had zero call sites).
- FRS `VbusApplied` keeps `source_on()` (the DPM hook returns once vSafe5V is applied  - 
  matches 6.4's wait); FRS `SourceOn`'s contradictory commented-out call replaced with a
  comment explaining the split. Non-DRP sink FRS event -> ignored, not ErrorRecovery.

**Tests:** `test_pr_swap_accept_disables_frs_receiver` (instrumented DPM + atomic; asserts
Accept TX + state + `set_fr_swap_detect(false)`), `test_frs_source_on_sends_ps_rdy`
(characterization: PS_RDY on exit).

---

## 6. Parse hardening - no panics from the wire

**Spec to read:** Sec. 6.4.9 `EPR_Request Message` (p. 205) for EPRMDO context; Sec. 6.4.10
(p. 206ff) - Tab. 6.50 marks reserved values "Shall not be used" (receiving them is a
partner bug; a receiver must not crash). Sec. 6.4.4 (p. 162) for the VDM header.

**Code:**
- `epr_mode.rs`: `From<u8> for Action` now total - reserved values map to
  `Action::Reserved(u8)`; `DataEnterFailed` keeps a total `From<u8>` where reserved values
  map to `UnknownCause` (Tab. 6.50 binds senders; the receiver tolerates them).
- `vendor_defined.rs`: all 12 panicking `From<u8>` impls converted to `TryFrom<u8>`; the
  identity-header bitfields expose raw (`u8`) fields with `try_*` fallible converters.
  (The typed getters were previously reachable from wire data.)

**Tests:** 12 new unit tests covering exhaustive reserved ranges (e.g. all VDM commands
0x00-0x1f except the 8 defined ones).

---

## 7. Source EPR_Request validation

**Spec to read:** Sec. 6.4.9 (p. 205) - RDO + PDO copy structure and the object-position rule;
Sec. 6.5.15.1 `EPR Capabilities Message Construction` (p. 245) - EPR PDOs live at object
positions 8..=14; Sec. 6.4.10.3.3 `Exits due to errors` (p. 211) - invalid EPR Requests are a
protocol error.

**Code:** `match_message_to_state` EPR_Request arm (`source/policy_engine/mod.rs`):
- Still in SPR mode -> `State::HardReset` (initiated), per Sec. 6.4.9 ("An EPR Source
  operating in SPR Mode that receives a EPR_Request Message Shall initiate a Hard Reset").
- In EPR mode: the RDO's object position must reference an EPR PDO in the DPM's advertised
  `epr_source_capabilities()` and the message's PDO copy must match the advertised one;
  the RDO must set the EPRCapable bit (Tab. 6.23 B22). Any failure -> `State::HardReset`
  (initiated), per Sec. 6.4.10.3.3. Previously *no* validation: any RDO went straight into
  negotiation.
- A `Request` Message received in EPR mode -> `State::HardReset`, per Sec. 6.4.2
  (was a passive `RxError::HardReset` mapped to `HardResetReceived`).

**Tests:** `test_invalid_epr_request_is_not_negotiated` (asserts `HardReset`),
`test_epr_request_pdo_copy_mismatch_not_negotiated` (asserts `HardReset`),
`test_epr_request_missing_epr_capable_bit_hard_resets` (asserts `HardReset` with a
test-local DPM advertising a valid EPR AVS PDO, isolating the bit check).

---

## 8. Test suite slimming (no coverage removed)

The PE test files had grown scaffolding noise: 79 `eprintln!` debug lines, repeated
GoodCRC-probe boilerplate, and dead drain-parsing loops. All removed mechanically;
every assertion was kept. Two things worth genuine reviewer attention:
- **`run_test_step` (source PE) was a no-op checker**: `matches!(&policy_engine.state, _new_state)`
  binds `_new_state` instead of comparing (leading underscore => fresh binding), so the
  state-transition guard could never fire, for any input. Replaced with a real
  `discriminant` comparison + `assert!`. All 43 tests still pass - the expected states
  were correct, but nothing would have caught a regression there before.
- GoodCRC probing is now one helper per PE (`assert_transmitted_good_crc`) instead of
  7 copied blocks (sink) / inline duplicates (source).

## 9. Other cleanups (mechanical)

- `MSG_HEADER_SIZE` was defined twice (`protocol_layer/mod.rs` + `message/mod.rs`).
  Single source now: `message::MSG_HEADER_SIZE = Header::SIZE`; `ExtendedHeader::SIZE`
  added as the analogous constant for the extended header (replaces the protocol-layer
  `EXT_HEADER_SIZE` copy).
- The tripled extended-payload parse match (`from_bytes`, the assembler path, and the
  test-only `parse_extended_payload`) is unified on `parse_extended_payload`.
- `dummy::get_source_capability_request` (zero callers) deleted.
- `docs/spec_check.md` rows A7/L9 updated for the deleted chunked module.

---

## 10. Docs & versioning review points

- `CHANGELOG.md [4.0.0]`: honest `### Breaking` block (timer variants, `Action::Reserved`,
  VDM getters, ErrorRecovery behavior change). One-liner style to match the rest of the
  file. **`3.1.0` was never published to crates.io** - verified - so the fold to 4.0.0 is
  safe; 3.0.0 remains published (not yanked, per decision).
- `usbpd-traits` untouched (2.0.0).
- `usbpd/src/lib.rs` + `README.md`: CC/TCPM scope statement.
- `docs/spec_check.md`: verdict rows updated per fix + the "Planned API evolution" roadmap
  (Tier 1 additive -> minors; Tier 2 breaking bundled into a future major, cable work
  expected to move to a companion crate; Tier 3 internal).

---

## Suggested review order

1. CHANGELOG (what changed at release level) - 5 min
2. Section 3 code+spec (p. 829 page printed next to you) - the most instructive pairing
3. Section 1 (chunking) diff on `protocol_layer/mod.rs` + `message/mod.rs`
4. Sections 4-5 (swaps + shared swap modules) - mechanical after the spec skim
5. Section 6-7 (parse hardening + EPR validation)
6. Sections 8-9 (test slimming, cleanups) - mechanical, but check the no-op-assert fix
7. `docs/spec_check.md` roadmap - forward API planning

## Reading-plan shortcuts (spec PDF pages)

| Purpose | Page(s) |
|---|---|
| PE source state diagram entry point (Sec. 8.3.3.2.x) | pp. 828-860 |
| PE sink state diagram entrance (Sec. 8.3.3.3.x) | pp. 831-846 |
| Swaps & FRS (Sec. 8.3.3.19.x) | pp. 885-909 |
| VCS (Sec. 8.3.3.20.x) | pp. 917-924 |
| EPR mode (Sec. 8.3.3.26.x) | pp. 950-962 |
| Message/data formats (Sec. 6.4.x) | pp. 120-260 |
| PRL chunking (Sec. 6.12.2.x) | pp. 274-286 |