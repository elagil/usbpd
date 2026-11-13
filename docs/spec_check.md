# USB PD R3.2 Spec-Compliance Checklist - `usbpd` crate

Spec: `USB_PD_R3_2 V1.1 2024-10.md` (repo root).
Verdicts: **OK** (matches spec) / **DEVIATION** (differs from spec) / **GAP** (spec-required, not implemented) / **N/A** (intentionally out of scope).
Audit performed by subagent sweep. Each row verified against code and spec text. Code refs are `usbpd/src/...:line`.

| # | Item | Code | Spec Sec. | Verdict | Evidence / Notes |
|---|------|------|--------|---------|------------------|
| A1 | GoodCRC reply to every received non-reserved message, correct MessageID | `protocol_layer/mod.rs:452-473` (`handle_rx_ack`) | 8.3.2.1.1, 6.12.2.3.3 | DEVIATION (minor) | GoodCRC w/ correct echoed ID sent for all normal rx (incl. parked). Reserved types are dropped *before* acking; spec PRL acks first, PE replies Not_Supported. Retransmission case correctly re-acks without re-parking. |
| A2 | GoodCRC expected after each transmit; CRCReceive wait; mismatched MessageID -> retry | `protocol_layer/mod.rs:245-285` (`wait_for_good_crc`), `:397-425` | 8.3.2.1.1 Tab 8.1/8.2, 6.7.1.1 | OK | Timer armed on TX, GoodCRC ID verified, RetryCounter reset + MessageIDCounter increment only on match; mismatch -> retransmit with same ID (test `mod.rs:1184`). |
| A3 | nMessageID counters, wrap, duplicate ID -> no ack, drop | `counters.rs:46`, `protocol_layer/mod.rs:662-685` | 6.7.1.2, Tab 6.70 | OK | Duplicate -> ack re-sent, message dropped. One counter per SOP instance; layer handles a single SOP type (per-SOP*-type copies out of scope). |
| A4 | nRetryCount, NoResponseTimer | `counters.rs:47`, `protocol_layer/mod.rs:397-419`, `timers/mod.rs:103` | 6.7.2 Tab 6.70, Tab 6.68 | OK | 2 retries max (nRetryCount=2) + original = 3 TXs, then error; NoResponseTimer 5000 ms in range; HardResetCounter=3 (1+nHardResetCount) correct. |
| A5 | Parked inbound during GoodCRC wait; in-order; overflow | `protocol_layer/mod.rs:130-159, 294-311, 489-504` | 8.3.2.1.1 | DEVIATION (minor) | Parking + re-ack + FIFO of 2 messages is spec-aligned (tests `:1103-1244`). Overflow silently drops an already-acked message; no spec rule sanctions losing acked rx. |
| A6 | Extended RX reassembly, ChunkSenderResponseTimer, missing chunks | `protocol_layer/mod.rs:518-608` | 6.12.2.1.2 (RCH_*), 6.6.18 | DEVIATION | In-order reassembly + chunk requests match RCH_Requesting_Chunk, but `ChunkSenderRequestTimer` defined (`timers/mod.rs:21,96`) and never started - a missing next chunk can block forever on bare `receive_message()`. Out-of-order chunk aborts with error instead of RCH error flow. Chunk-request frames can be mis-folded into reassembly while `extended_rx_expected` is set. |
| A7 | Extended TX chunked, chunk-request serving | `message/mod.rs`, `transmit_chunk_response` | 6.12.2.1.3 (TCH_*), 6.2.1.2.5.2 | OK (fixed) | `Message::to_bytes` emits chunk 0 truncated to 26 B; the protocol layer retains the full payload and serves `Chunk Request` frames from it. Previously one oversize "chunk" frame was emitted and requests went unanswered. |
| A8 | Outgoing message validation | `protocol_layer/mod.rs:321-343`, `epr.rs:136-152` | 6.2.1.1 | DEVIATION (minor) | Validates unchunked-ext bit + AVS alignment; no oversized-payload/num-objects rejection (overflow would hit serialization bound, not a clean TxError). |
| B1 | All timer values vs Tab. 6-68 | `timers/mod.rs` (5 dead variants removed) | Tab 6-68 | OK (dead variants pruned) | All ~28 checked individually, incl. SPR/EPR splits: PSSourceOff 835/1260, PSTransition 500/925, PSSourceOn 435, SinkEPREnter 500, SinkEPRKeepAlive 375, SourceEPRKeepAlive 875, SenderResponse 30 (matches newer 27-33), SinkWaitCap 465, SourceCapability 150. All within range; EPR PSSourceOn variant correctly absent. |
| C1 | Source-initiated hard reset | `source/policy_engine/mod.rs:607-619`, `protocol_layer/mod.rs` (bounded) | 8.3.3.2.9, 7.3.3.1 | OK (fixed) | Counter, `hard_reset()`, PSHardResetTimer 30 ms. GAP: NoResponseTimer never started (see C-issue 1); HardResetCompleteTimer not enforced - driver `transmit_hard_reset` can loop forever on `Discarded`. |
| C2 | Sink-initiated hard reset | `sink/policy_engine/mod.rs:711-765` | 8.3.3.3.8/9, 7.3.3.2 | OK | Signaling + counter (saturation -> PortPartnerUnresponsive), DPM `hard_reset()` contract covers vSafe0V->vSafe5V. |
| C3 | Hard reset received (source & sink) | `sink/mod.rs:254-255`, `source/mod.rs:275, 621-627` | 8.3.3.2.10, 8.3.3.3.9 | OK | Source: RxError::HardReset -> HardResetReceived (PSHardResetTimer -> default). Sink: -> TransitionToDefault directly, matching spec. |
| C4 | Reset to defaults after hard reset | `source/mod.rs:629-649`, `sink/mod.rs:754-765` | 8.3.3.2.11, 8.3.3.3.9 | OK | VCONN cycle, DPM reset, protocol layer reset (message IDs), EPR mode -> Spr, contract cleared, -> Startup. |
| E1 | Source send Soft_Reset | `source/policy_engine/mod.rs:770-786` | 8.3.3.4.1.1 | OK | Layer reset, send, SenderResponse wait for Accept -> SendCapabilities; timeout -> HardReset. |
| E2 | Sink send Soft_Reset | `sink/policy_engine/mod.rs:685-699` | 8.3.3.4.2.1 | OK | Identical flow -> WaitForCapabilities. |
| E3 | Soft_Reset received | `source/mod.rs:788-794`, `sink/mod.rs:701-709` | 8.3.3.4.1.2/4.2.2, 6.8.1 | OK | Reset layer, send Accept; source -> Send_Capabilities, sink -> Wait_for_Capabilities. |
| F1 | Source Not_Supported | `source/policy_engine/mod.rs:799-809, 1222-1344` | 8.3.3.6.1.1/1.2 | OK | Unsupported data/ext messages route -> SendNotSupported -> Ready. |
| F2 | Sink Not_Supported | `sink/policy_engine/mod.rs:666-684, 561-627` | 8.3.3.6.2.1/2.2 | OK (minor issue) | Same routing. Bug: DPM inform requires cached caps - incoming Not_Supported during discovery aborts with UnexpectedMessage (C-issue 4). |
| F3 | Chunk_Received PE states | `protocol_layer/mod.rs:475-481` | 8.3.3.6.1.3/2.3 | N/A | Chunking implemented at protocol layer, so PE states apply only to non-chunking ports (spec note). No misbehavior route if partner stalls mid-chunk (ties into A6). |
| D1 | PE_SNK_Startup | `sink/policy_engine/mod.rs:202,219,409-421` | 8.3.3.3.1 | OK (minor) | Starts at Discovery; spec starts at Startup on power-up but reset is a no-op on fresh layer. Vconn/CC delegated to driver. |
| D2 | PE_SNK_Discovery | `sink/policy_engine/mod.rs:416-421` | 8.3.3.3.2 | OK | Waits VBUS only; tTypeCSendSourceCap is a source-side timer (covered sink-side by SinkWaitCap in D3). |
| D3 | PE_SNK_Wait_for_Capabilities | `sink/mod.rs:422-424, 391-405`; `protocol_layer/mod.rs:925-938` | 8.3.3.3.3, 6.6.3.2 | OK | SinkWaitCapTimer 465 ms via protocol-layer receive timeout; timeout -> HardReset; EPR variant too. |
| D4 | PE_SNK_Evaluate_Capability | `sink/mod.rs:425-437` | 8.3.3.3.4 | OK (minor) | DPM evaluation; mismatch flag auto-derived in request build - DPM can't signal mismatch independently. `hard_reset_counter.reset()` on each evaluation is more lenient than spec ("power cycle or Detach only"). |
| D5 | PE_SNK_Select_Capability | `sink/mod.rs:438-472, 313-316, 345` | 8.3.3.3.5, 8.3.2.2.1.3, 8.3.2.2.1.1.3 | OK | SenderResponse wait; Accept/Reject/Wait paths incl. contract/no-contract distinction; Wait -> `Ready(true)` gating re-request; timeout -> HardReset; tx retry exhaustion -> SoftReset (Tab 6.72). |
| D6 | PE_SNK_Transition_Sink | `sink/mod.rs:473-489` | 8.3.3.3.6 | OK (minor) | PS_RDY -> contract set -> DPM -> Ready, PSTransition timer armed. Deviation: DPM "reduce current draw at Accept" hook missing; `transition_power` called after PS_RDY only. |
| D7 | PE_SNK_Ready dispatch | `sink/mod.rs:490-664` | 8.3.3.3.7, 8.3.2.2.1.1.3 | OK (documented gaps) | Caps re-eval in Ready, swap dispatch, SinkPPSPeriodic, SinkRequestTimer-after-Wait (tSinkRequest 100 ms). CHANGELOG's "Wait handling incomplete" = (a) Wait to *sink-initiated* DR/PR/VCS swaps silently drops to `Ready(false)` without DPM inform (FIXMEs `:957,1047,1157,1187`), (b) Ready-entry TODOs `:491-493` (DiscoverIdentityTimer, Get_Sink_Cap-for-FRS, AMS notify). |
| D8 | Contract invalidation / caps change | `sink/mod.rs:541-563, 784-876` | 8.3.3.3.7 | OK | Unrequested Source_Caps in Ready -> Evaluate; old contract preserved through Reject/Wait. Comment bug at `:556` only. |
| D9 | Transition_Sink timeout / give-up | `sink/mod.rs:318-321, 711-734, 896-900`, `counters.rs:41-44` | 8.3.3.3.6/3.8, 8.3.3.28.1 | OK | PSTransition timeout -> HardReset; HardResetCounter caps at 3 (1+nHardResetCount) -> PortPartnerUnresponsive. NoResponseTimer is source-side (N/A). |
| G1 | Sink-initiated data role swap | `sink/policy_engine/mod.rs` (Send timeout arm) | 8.3.3.19.1.5/2.5 | OK (fixed) | Send/await/Change flow correct, but SenderResponse timeout during `Data(Send)` falls to generic error handler -> re-sends DR_Swap indefinitely (spec: -> Ready). No Wait path. |
| G1b | Sink receives Data_Swap (as UFP) | `sink/mod.rs:594-597, 911-932, 967-973` | 8.3.3.19.2.2-2.4 | OK (minor) | Evaluate/Accept/Change/Reject correct; Reject can't send Wait (FIXME); modal-operation -> HardReset rule absent. |
| G3 | Sink power role swap (both directions) | `sink/mod.rs:590-592, 980-1051, 1021, 294-306` | 8.3.3.19.4.1-4.8, 8.3.2.7.2 | OK (minor) | Ordering (Accept->off->wait PS_RDY->Rp->vSafe5V->PS_RDY) + timers conform; ends in `PrSwapToSourceStartup` handoff. Deviations: FRS receiver never disabled on Accept (DPM `set_fr_swap_detect` has zero call sites); no Wait response in Reject. |
| G4 | Sink FRS | `sink/mod.rs:122-131, 641-647, 1090-1120, 247-251` | 8.3.3.19.6.x, 8.3.2.8, 7.1.13 | GAP | Signalling half implemented (FR_Swap, SenderResponse, tPSSourceOff, ErrorRecovery). No FRS detection/arming: preconditions (FRS bits in Sink_Caps, power reserve, CC detection) skipped; TODO on entry. `source_on()` called in `VbusApplied` but FIXME'd in `SourceOn` - one of the two is wrong. |
| G5 | Sink Vconn swap | `sink/mod.rs` (WaitForVconn arm) | 8.3.3.20.1-20.9 | OK (fixed) | 9-state flow + vconn gating matches; VCONNOn timer 150 ms. Bug: VCONNOnTimer timeout in `WaitForVconn` -> generic handler, must -> HardReset (spec 8.3.3.20.5); *source* PE implements this, sink doesn't. `ForceVconn` unreachable; Wait -> Ready w/o DPM inform (FIXME). |
| H1 | Sink EPR entry | `sink/policy_engine/epr.rs:50-111, 57-62`; `mod.rs:285-290` | 8.3.3.26.2.1/2.2 | OK (minor dev.) | Enter + operational PDP, SenderResponseTimer + SinkEPREnterTimer; EnterFailed -> SoftReset. Deviation: SinkEPREnterTimer restarted fresh in Wait_For_Response instead of running continuously (aggregate approx. 530 ms, within 550 max; documented). Exit-during-entry routes to exit handling instead of mandated SoftReset. |
| H2 | Sink EPR exit (send + received) | `epr.rs:159-188`, `sink/mod.rs:585-589, 309-311` | 8.3.3.26.4.1/4.2, 8.3.2.2.2.4 | OK | Send exit -> mode=Spr -> WaitForCaps; source-initiated exit: SPR contract -> WaitForCapabilities, EPR contract -> HardReset. |
| H3 | Sink EPR keep-alive | `epr.rs:189-222`, `sink/mod.rs:514-533, 657-658` | 8.3.3.3.11, 6.6.21.2 | OK (minor dev.) | Ack/timeout paths correct, tSinkEPRKeepAlive 375 ms. Deviations: timer not stopped on arbitrary message transmissions (6.6.21.2 requires stop); non-Ack ExtendedControl -> Not_Supported instead of spec's Ack/timeout-only. |
| H4 | Sink EPR caps + EPR_Request | `epr.rs:139-157`, `sink/mod.rs:391-405, 847-886, 425-439`, `request/epr.rs:52-91` | 6.4.9, 8.3.2.2.2 | OK | Chunked EPR_Source_Capabilities parsed to PDOs; DPM-driven EPR_Request with RDO + PDO copy, obj position <=14. |
| H5 | EPR integration in Ready | `sink/mod.rs:634-638, 541-589, 388-405` | 6.4.10.2/3.3 | OK | Enter from Ready via DPM event; unrequested SPR caps in EPR mode -> HardReset; EPR PDO in positions 1-7 -> HardReset; EprMode -> ExitReceived. |
| I1 | PE_SRC_Startup | `source/policy_engine/mod.rs:385-400` | 8.3.3.2.1 | OK (minor) | Reset contract/mode/layer/counter; `Startup{role_swap}` runs SwapSourceStart (20 ms) before wait_for_vbus. Rp/Vconn via DPM/driver. |
| I2 | Discovery / Send_Capabilities | `source/mod.rs` (CapsCounter in SendCapabilities) | 8.3.3.2.2/2.3 | OK (fixed) | Retransmit loop Discovery<->SendCapabilities, saturation -> Disabled. Bugs: CapsCounter incremented in Discovery + never reset on GoodCRC (spec requires reset in Send_Capabilities on GoodCRC) -> long-lived renegotiation can spuriously exhaust 50-cap budget; counter reset is unconditional, not GoodCRC-gated. |
| I3 | PE_SRC_Negotiate_Capability | `mod.rs:439-472, 1352-1379, 290, 337` | 8.3.3.2.4 | OK | SenderResponse wait; timeout -> HardReset; malformed Request -> SoftReset path; per-field validation delegated to DPM (Accept/Reject/Wait). |
| I4 | PE_SRC_Transition_Supply | `mod.rs:474-491, 332` | 8.3.3.2.5 | DEVIATION (minor) | Accept -> transition_power -> PS_RDY -> Ready matches. GAP: no PSTransitionTimer bound on DPM `transition_power` - hung DPM blocks PE forever. |
| I5 | PE_SRC_Ready dispatch | `mod.rs:493-565, 1218-1348` | 8.3.3.2.6 | OK (gaps) | Full dispatch: Request, SoftReset, GetSourceCap->resend caps, swaps, GetSinkCap, Not_Supported, DPM events. Gaps: AMS end notification FIXME (`:494`); DiscoverIdentityTimer stub -> cable-plug discovery never runs, HardReset path at `:558` dead code. |
| I6 | PE_SRC_Capability_Response | `mod.rs:591-605` | 8.3.3.2.8, 8.3.2.2.1.1.3 | OK | Reject/Wait per DPM; contract-invalid + Reject -> HardReset, Wait -> WaitNewCapabilities; explicit contract -> Ready. Matches Tab 8.34. |
| I7 | PE_SRC_Disabled / ErrorRecovery | `mod.rs` (NoResponseTimer with-contract path -> ErrorRecovery) | 8.3.3.2.3, 8.3.3.2.7, 8.3.3.28.1 | OK (fixed) | Caps saturation -> Disabled matches Sec. 8.3.3.2.7. With-contract silent partner: NoResponseTimer (5 s) -> ErrorRecovery -> `Err(ReconnectionRequired)` per Sec. 8.3.3.2.3 (previously-connected escalation, spec-strict). Not-connected tx-fail -> Discovery per Sec. 8.3.3.2.3. Escalation gated on transmit failure, not on NoResponseTimer AND HardResetCounter>n conjunction (HardResetCounter check happens via caps-saturation/PE giveup paths) - documented nuance. |
| I8 | PE_SRC_Get_Sink_Cap | `mod.rs:652-709` | 8.3.3.2.12 | OK | GetSinkCap/EPR variant; Sink_Caps awaited under SenderResponseTimer; timeout -> DPM inform(None) -> Ready. |
| I9 | PE_SRC_Wait_New_Capabilities | `mod.rs:711-738` | 8.3.3.2.13 | OK (minor) | Waits DPM UpdatedSourceCapabilities -> SendCapabilities. Bug: deaf to incoming Hard/Soft Reset signaling while waiting (no receive-message select). |
| J1 | Source data role swap | `source/policy_engine/mod.rs:881-939, 1272-1287, 552` | 8.3.3.19.1-2.x, 8.3.2.9.2/3 | OK (dev.) | Shared flow covers DFP-as-source & UFP-as-source directions; role change derived from header. Deviations: DPM-initiated `Send` bypasses `dual_role` gating; Reject lacks Wait (FIXME `:932`). |
| J2 | Source power role swap | `mod.rs:942-1019, 1290-1296, 856-858, 324-329` | 8.3.3.19.3.x, 8.3.2.7.1 | OK (dev.) | Accept->off->AssertRd->PS_RDY->`PrSwapToSinkStartup` handoff; PSSourceOn timer on WaitSourceOn; errors -> ErrorRecovery. Deviation: `TransitionToOff` has no tPSSourceOff wait / vSafe0V verification - delegated entirely to DPM `disable()`. |
| J3 | Source FRS | `mod.rs:1022-1066, 1299-1305, 142-148` | 8.3.3.19.5.x, 8.3.2.8 | OK (dev.) | Received FR_Swap -> DPM `fr_swap_signaled()` -> Accept -> discharge_vbus -> cc_sink -> SwapToSink handoff. Deviation: FRS never self-triggers from CC-only signaling (no DPM event for it); default DPM => any FR_Swap -> HardReset. |
| J4 | Source Vconn swap | `mod.rs:1069-1210, 1152-1162, 303-311` | 8.3.3.20.1-20.9 | OK (gaps) | All reachable states; WaitForVconn timeout -> HardReset. Gaps: `ForceVconn` coded but unreachable (Not_Supported in SendSwap routes to NotSupportedReceived; spec: -> Force_Vconn when not VCONN source); VCONN drive failure -> abrupt `ReconnectionRequired` instead of spec recovery. |
| K1 | PE_SRC_Evaluate_EPR_Mode_Entry | `source/policy_engine/epr.rs:51-56` | 8.3.3.26.1.1 | OK | DPM `epr_capable()` gates ack/EnterFailed(`SourceUnableToEnterEprMode`). |
| K2 | EPR entry ack + cable discovery | `epr.rs:58-94, 76-78` | 8.3.3.26.1.2-1.5 | DEVIATION | EnterAcknowledged -> DiscoverCable -> EvaluateCable -> Succeeded/Failed structure present, but cable discovery is **stubbed** (DPM `epr_cable_good()` default false; no SOP'/' Discover Identity VDM, no tEPRSourceCableDiscovery). Bug: with `vconn_source==true` + unknown cable, code sends Vconn_Swap - spec sends Swap only when *not* VCONN source; `vconn_source` also never updated after VCS, so branch works only accidentally. FIXMEs: EPR headers not set/cleared (`:101,119,126`). |
| K3 | Source EPR exit send/received | `epr.rs:114-145` | 8.3.3.26.3.1/3.2, 6.4.10.3 | OK (minor) | Exit -> mode=Spr -> SendCapabilities; received exit: SPR contract -> SendCapabilities, EPR contract -> HardReset. Odd: HardReset rule checks *own* caps for epr PDOs in SPR positions (rule targets the sink-side check). Implicit exit on PRS/FRS via Startup `mode` reset. |
| K4 | PE_SRC_EPR_Keep_Alive | `mod.rs:739-749, 1333, 556-563`, `timers/mod.rs:122` | 8.3.3.2.14, 6.6.21.3, Tab 6-68 | OK (minor) | Responds EPR_KeepAlive_Ack, timeout -> HardReset; 875 ms = spec nominal. Timer restart approximated (not per-GoodCRC). |
| K5 | PE_SRC_Give_Source_Cap | `mod.rs:750-766, 1261-1265, 1323-1326` | 8.3.3.2.15 | DEVIATION | Response keyed on own mode: Get_Source_Cap-in-EPR -> EPR caps (defensible); EPR_Get_SourceCap-in-SPR -> plain Source_Capabilities (mismatch - should be rejected/Not_Supported). |
| K6 | Source EPR_Request validation + caps TX | `match_message_to_state` EprRequest arm (source PE) | 8.3.2.2.2, 6.4.10.3.3 | OK (fixed) | Source validates incoming EPR_Request: EPR mode required, object position must reference an EPR PDO (8..=14) of the DPM's EPR caps, PDO copy must match, EPRCapable bit (Tab. 6.23 B22) must be set; any failure initiates Hard Reset (Sec. 6.4.9/6.4.10.3.3). Request-in-EPR-mode also Hard Resets (Sec. 6.4.2). Remaining: EPR caps TX zero-fill of SPR positions 1-7 TBD. |
| L1 | Header bit layout + round-trip | `message/header.rs:21-38, 107-120, 123-132, 148-158` | Tab 6.1, 6.2.1.1 | OK | Extended@15, nObj@12-14, ID@9-11, roles/rev per spec; SpecRev=11b rejected; type dispatch by nDataObjs. |
| L2 | Request RDO fields (fixed/PPS/AVS/battery) | `data/request.rs:34-49, 70-91, 112-131, 153-174, 366-430` | Tab 6.23-6.26 | OK | All bit positions match; max-operating-current set; GiveBack deprecated-never-set; PPS current clamped to 7 bits. Bug (hardcoded flags): `no_usb_suspend=true`, `usb_communications_capable=true` always set (`request.rs:387-388`, FIXME in code). |
| L3 | Fixed PDO layout (source & sink) | `data/source_capabilities/mod.rs:90-113`, `data/sink_capabilities.rs:50-71` | Tab 6.9, 6.17 | OK | Fields incl. EPR b23, Unchunked b24, FRS b23-24 (sink) match; reserved bits zeroed. |
| L4 | PPS APDO layout | `source_capabilities/mod.rs:245-265` | Tab 6.13 | OK | PPS power-limited b27, V/MinV 100 mV, MaxI 50 mA fields match. |
| L5 | EPR AVS APDO layout | `source_capabilities/epr.rs:16-30` | Tab 6.15 | OK | 9-bit max voltage, 8-bit PDP, supply=01b match. |
| L6 | EPR_Mode message field | `data/epr_mode.rs` (Reserved variant) | Tab 6.50 | OK (fixed) | Action codes + EnterFailed reasons match. `From<u8>` for `Action` maps reserved -> `Action::Reserved(u8)`; `DataEnterFailed` maps reserved -> `UnknownCause` (total `From<u8>`, no panic; Tab. 6.50 binds senders). Operational-PDP data semantics not validated. |
| L7 | EPR_Request RDO | `data/request/epr.rs:52-102, 106-113` | 6.4.9, 6.4.2 | OK | RDO per Tab 6.26 + PDO copy, objPos 8-11 mapping via epr_pdos() matches Sec.6.5.15.1. (Source-side request validation is K6's GAP.) |
| L8 | VDM header | `data/vendor_defined.rs` (TryFrom + raw getters) | Tab 6.28/6.29 | OK (fixed) | SVDM layout exact. **Bug: `From<u8> for VdmCommand` panics on all SVID-specific commands except DP 0x10/0x11 and reserved 7-15 (`:112`) - parse-time crash on hostile input.** |
| L9 | Extended header | `extended/mod.rs:74-84` | Tab 6.3, 6.2.1.2.x | OK | Bit layout exact; 26 B chunk limit (`MAX_EXTENDED_MSG_CHUNK_LEN`). Chunk assembly lives solely in the protocol layer (the standalone assembler/sender helpers were removed as unused). |
| L10 | Reserved/unknown message types | `header.rs:241, 283, 310`, `data/mod.rs:183-185`, `mod.rs:243` | 6.2.1.1.8 | OK | Reserved -> `Reserved`/`Unknown` payloads, parseable -> PE can send Not_Supported. Numbering matches Tables 6.5/6.6/6.53. |
| M1 | DRP PRS handoff | `dual_role.rs:58-79`, `sink/mod.rs:1016-1022, 892-894`, `source/mod.rs:979, 856-858, 385-399` | 8.3.2.7, 8.3.3.19.3/4 | OK | PE exits with SwapToSource/SwapToSink; `dual_role.rs` restarts opposite role with `role_swapped` latched; `Startup{role_swap}` + tSwapSourceStart. |
| M2 | Error propagation to DRP wrapper | `dual_role.rs:86-89, 102-105`, `sink/mod.rs:897-899` | 8.3.3.28.1 | OK (minor) | Errors bubble up; caller must reconstruct driver (documented). Bug: `Ok(_)` -> `unreachable!()` panic if a PE exits with non-swap RunResult - panic-in-embedded undesirable. |
| M3 | Sink GiveSinkCap | `sink/mod.rs:599, 614, 767-782` | 8.3.3.3.10 | OK | Sink_Capabilities / EPR_Sink_Capabilities per mode; no response timer (spec: none). |
| M4 | Sink GetSourceCap | `sink/mod.rs:632-634, 784-876` | 8.3.3.3.12, 8.3.2.11.3.x | OK | SenderResponseTimer, timeout -> DPM inform + Ready, mode matching, `get_source_cap_pending` guard. |
| M5 | Source DrpGetSourceCap / DrpGiveSinkCap | `source/mod.rs:1307-1313, 1327-1331, 819-852` | 8.3.3.19.7.1/8.1, 8.3.3.2.12 | OK | Correctly gated on `dual_role`; per-mode messages; SenderResponseTimer; DPM informed incl. timeouts. |

## Aggregate findings

*Post-fix status (this branch, "spec_compliance_fixes"): items A7, C1, G1, G5, I2, L6, L8, K6
and the chunk-serving hardening closed; verdict rows above updated. High-impact list from the
original audit:*

### High-impact bugs - RESOLVED in this branch
1. ~~Panic on hostile input~~ (L6, L8): `Action` now maps reserved values to `Reserved(u8)`;
   all VDM enums use `TryFrom` + raw getters with regression tests.
2. ~~Chunked TX broken~~ (A7): oversized payloads sent with a correct chunk-0 header
   (including the object count per Sec. 6.2.1.1.2); incoming chunk requests are acked and
   served from the retained payload (`transmit_chunk_response`); duplicates re-serve the
   chunk; out-of-range chunk numbers are ignored instead of panicking.
3. ~~DR_Swap send timeout loop~~ (G1): timeout -> Ready(false) per 8.3.3.19.1.5.
4. ~~Sink VCONNOnTimer~~ (G5): timeout -> HardReset per 8.3.3.20.5.
5. ~~Source-side EPR_Request validation~~ (K6): EPR mode, position/PDO-copy/EPRCapable-bit
   validated against DPM EPR caps; invalid requests initiate Hard Reset per 6.4.10.3.3.
6. ~~CapsCounter never reset on GoodCRC~~ (I2): increment added in Send_Capabilities, reset on GoodCRC.

### Remaining known deviations (documented, intentionally not addressed)
- **Wait message handling incomplete** on the responding side (DPM hooks are bool) - documented
  in CHANGELOG Known limitations + trait rustdoc; receiving Wait works.
- **Timers defined but never started**: `ChunkSenderRequest`/`ChunkSenderResponse` are available
  for the chunk-request serving flow but the PE-level awaiting loop is simplified (no per-request
  timer guard yet); `DiscoverIdentity` still stubbed (cable-plug discovery not implemented).
- **DPM-delegated correctness**: tPSSourceOff/vSafe0V (J2), PSTransition bound (I4), EPR cable
  discovery (K2) rely on DPM promises rather than enforced timeouts.
- **FRS**: message-based half complete + `set_fr_swap_detect` wired; CC-only detection external
  (out of crate scope, documented in lib.rs/README).
- **EPR mode entry**: SinkEPREnterTimer restarted per state (H1); Exit-during-entry routed to
  exit handling instead of SoftReset (H1); `vconn_source` staleness after VCS (K2) remains.
- **EPR caps TX** for >26 B payloads goes through the chunked TX path (A7 fix) but zero-fill of
  SPR positions 1-7 is not performed (see K6 row).

### Verified-good areas (unchanged)
- Timer values (B1), header/message bit layouts (L1-L5, L7, L9, L10), GoodCRC/retry/counters core
  (A2-A4), hard/soft reset core flows (C2-C4, E1-E3), SPR negotiation flows both roles
  (D3-D9, I1-I9), PRS handoff + DRP wrapper (J2, M1-M5).

## Planned API evolution (roadmap)

*Added 2026-11: for transparency with downstream users; also mirrored in the 4.0.0 CHANGELOG.*

### Tier 1 - additive trait methods (minor bumps; non-breaking)

| Feature | Spec Sec. | New DPM surface (default methods) | Status |
|---|---|---|---|
| Status/Alert (sink & source incl. Get_PPS_Status) | 8.3.3.10, 8.3.3.7, 8.3.3.10.3/.4 | `give_status()`, `alert_received()`, `get_status()` | message types parse, dropped today |
| Battery Status/Capabilities + `new_battery` request helper | 8.3.3.11, 8.3.3.12, 6.4.2 | battery hooks | niche; implement on demand |
| Enter_USB | 8.3.3.16 | `enter_usb_requested()` | mechanical |
| Revision / Source_Info / Country* gives | 8.3.3.15, 8.3.3.9, 8.3.3.14 | `give_*` hooks | mechanical |

### Tier 2 - breaking (bundle into one coherent major, expected `5.0.0`)

| Feature | Spec Sec. | API impact | Notes |
|---|---|---|---|
| Wait tri-state for swap decisions | 8.3.2.2.1.1.3, swap Reject states | `evaluate_swap_request`/`evaluate_vconn_swap_request`: `bool` -> 3-state enum | deferred from 4.0.0; the responding side only |
| Vendor Defined Messages, both directions | 6.4.4 | new `vendor_message_received()` hook + TX types/API | unblocks DP alt-mode; TX side currently `unimplemented!()` |
| EPR cable discovery | 8.3.3.26.1.2-1.4 | `cable_identity()`-style hook, real SOP'/' exchange | **expected to live in a separate Type-C companion crate (cable/CC handling)**; `usbpd` keeps the delegation hooks only (`epr_cable_good` today, `cable_identity()` later) - K2 row stays DPM-delegated until that crate exists |
| AMS notification to protocol layer | 8.3.3.2.6 (FIXME at source `Ready` entry/exit) | possibly `usbpd-traits` `Driver` addition (minor if defaulted) or internal-only | unblocks strict AMS arbitration |
| Request RDO flag control | 6.4.2.4-6.4.2.7 (hardcoded `no_usb_suspend`/`usb_communications_capable` at `request.rs:387-388`) | `request()` hook signature or builder setters | small break; correctness gap today |

### Tier 3 - internal only (no API impact)

- DiscoverIdentityTimer stub (cable-plug PD connection establishment) - depends on Tier-2 VDM/cable crate.
- ChunkSenderRequest/Response timer guards around chunk-request serving (serving works; timer hygiene pending).
- Oversized-payload rejection in `validate_outgoing_message` (clean TxError instead of buffer-bound truncation).
- `vconn_source` tracking after VCS (K2 accidental-correctness note); EPR caps zero-fill of SPR positions 1-7.
- SinkEPRKeepAliveTimer stop-on-Tx semantics (6.6.21.2); SinkEPREnterTimer continuity across entry states (H1 note).

## Test coverage notes

- `cargo test -p usbpd`: **43 passed, 0 failed** on the fix branch. Regression tests: reserved-value parse (11), VconnSwap/DR_Swap timeout transitions, CapsCounter GoodCRC reset, chunked TX + chunk-request serving, hard_reset bound, NoResponseTimer Disabled escalation, FRS wiring. (Count lowered from 52 by removing self-test-only unit tests of the deleted standalone chunking helpers; all wire/PE-level regression coverage retained.)
- Tests are host-based (`sink/policy_engine/tests.rs`, `source/policy_engine/tests.rs`) with mock driver/timer. Known coverage gaps (per audit): no tests for peer-initiated Wait responses, FRS, chunked TX serving, EPR_Request validation, or reserved-value parse paths (the panic bugs in L6/L8 would not be caught).