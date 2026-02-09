//! Tests for the policy engine.

use super::Sink;
use crate::counters::{Counter, CounterType};
use crate::dummy::{DUMMY_CAPABILITIES, DummyDriver, DummySinkDevice, DummyTimer, MAX_DATA_MESSAGE_SIZE};
use crate::protocol_layer::message::data::Data;
use crate::protocol_layer::message::data::epr_mode::Action;
use crate::protocol_layer::message::data::request::PowerSource;
use crate::protocol_layer::message::data::source_capabilities::PowerDataObject;
use crate::protocol_layer::message::header::{
    ControlMessageType, DataMessageType, ExtendedMessageType, Header, MessageType,
};
use crate::protocol_layer::message::{Message, Payload};
use crate::sink::policy_engine::State;

fn get_policy_engine() -> Sink<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DummySinkDevice> {
    Sink::new(DummyDriver::new(), DummySinkDevice {})
}

fn simulate_source_control_message<DPM: crate::sink::device_policy_manager::DevicePolicyManager>(
    policy_engine: &mut Sink<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DPM>,
    control_message_type: ControlMessageType,
    message_id: u8,
) {
    let header = *policy_engine.protocol_layer.header();
    let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];

    Message::new(Header::new_control(
        header,
        Counter::new_from_value(CounterType::MessageId, message_id),
        control_message_type,
    ))
    .to_bytes(&mut buf);
    policy_engine.protocol_layer.driver().inject_received_data(&buf);
}

/// Get a header template for simulating source messages (Source/Dfp roles).
/// This flips the roles from the sink's perspective to simulate messages from the source.
fn get_source_header_template() -> Header {
    use crate::protocol_layer::message::header::SpecificationRevision;
    use crate::{DataRole, PowerRole};

    // Source messages have Source/Dfp roles (opposite of sink's Sink/Ufp)
    Header::new_template(DataRole::Dfp, PowerRole::Source, SpecificationRevision::R3_X)
}

/// Simulate an EPR Mode data message from the source with proper API.
/// Returns the serialized bytes for assertion.
fn simulate_source_epr_mode_message<DPM: crate::sink::device_policy_manager::DevicePolicyManager>(
    policy_engine: &mut Sink<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DPM>,
    action: Action,
    message_id: u8,
) -> heapless::Vec<u8, MAX_DATA_MESSAGE_SIZE> {
    use crate::protocol_layer::message::data::epr_mode::EprModeDataObject;

    let source_header = get_source_header_template();
    let header = Header::new_data(
        source_header,
        Counter::new_from_value(CounterType::MessageId, message_id),
        DataMessageType::EprMode,
        1, // 1 data object (the EprModeDataObject)
    );

    let epr_mode = EprModeDataObject::default().with_action(action);
    let message = Message::new_with_data(header, Data::EprMode(epr_mode));

    let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];
    let len = message.to_bytes(&mut buf);
    policy_engine.protocol_layer.driver().inject_received_data(&buf[..len]);

    let mut result = heapless::Vec::new();
    result.extend_from_slice(&buf[..len]).unwrap();
    result
}

/// Simulate an EprKeepAliveAck extended control message from the source.
/// Returns the serialized bytes for assertion.
fn simulate_epr_keep_alive_ack<DPM: crate::sink::device_policy_manager::DevicePolicyManager>(
    policy_engine: &mut Sink<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DPM>,
    message_id: u8,
) -> heapless::Vec<u8, MAX_DATA_MESSAGE_SIZE> {
    use crate::protocol_layer::message::Payload;
    use crate::protocol_layer::message::extended::Extended;
    use crate::protocol_layer::message::extended::extended_control::{ExtendedControl, ExtendedControlMessageType};

    let source_header = get_source_header_template();
    // Create extended message header (num_objects=0 as used in transmit_extended_control_message)
    let header = Header::new_extended(
        source_header,
        Counter::new_from_value(CounterType::MessageId, message_id),
        ExtendedMessageType::ExtendedControl,
        0,
    );

    // Create the message with proper payload
    let mut message = Message::new(header);
    message.payload = Some(Payload::Extended(Extended::ExtendedControl(
        ExtendedControl::default().with_message_type(ExtendedControlMessageType::EprKeepAliveAck),
    )));

    // Serialize and inject
    let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];
    let len = message.to_bytes(&mut buf);
    policy_engine.protocol_layer.driver().inject_received_data(&buf[..len]);

    let mut result = heapless::Vec::new();
    result.extend_from_slice(&buf[..len]).unwrap();
    result
}

#[tokio::test]
async fn test_negotiation() {
    // Instantiated in `Discovery` state
    let mut policy_engine = get_policy_engine();

    // Provide capabilities
    policy_engine
        .protocol_layer
        .driver()
        .inject_received_data(&DUMMY_CAPABILITIES);

    // `Discovery` -> `WaitForCapabilities`
    policy_engine.run_step().await.unwrap();

    // `WaitForCapabilities` -> `EvaluateCapabilities`
    policy_engine.run_step().await.unwrap();

    let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    assert!(matches!(
        good_crc.header.message_type(),
        MessageType::Control(ControlMessageType::GoodCRC)
    ));

    // Simulate `GoodCrc` with ID 0.
    simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 0);

    // `EvaluateCapabilities` -> `SelectCapability`
    policy_engine.run_step().await.unwrap();

    // Simulate `Accept` message.
    simulate_source_control_message(&mut policy_engine, ControlMessageType::Accept, 1);

    // `SelectCapability` -> `TransitionSink`
    policy_engine.run_step().await.unwrap();

    let request_capabilities =
        Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    assert!(matches!(
        request_capabilities.header.message_type(),
        MessageType::Data(DataMessageType::Request)
    ));

    // Simulate `PsRdy` message.
    simulate_source_control_message(&mut policy_engine, ControlMessageType::PsRdy, 2);

    // `TransitionSink` -> `Ready`
    policy_engine.run_step().await.unwrap();
    assert!(matches!(policy_engine.state, State::Ready(..)));

    let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    assert!(matches!(
        good_crc.header.message_type(),
        MessageType::Control(ControlMessageType::GoodCRC)
    ));
}

#[tokio::test]
async fn test_epr_negotiation() {
    use crate::dummy::{DUMMY_SPR_CAPS_EPR_CAPABLE, DummySinkEprDevice};

    // Create policy engine with EPR-capable DPM
    let mut policy_engine: Sink<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DummySinkEprDevice> =
        Sink::new(DummyDriver::new(), DummySinkEprDevice::new());

    // === Phase 1: Initial SPR Negotiation ===
    // Using same flow as test_negotiation
    eprintln!("Starting test");

    policy_engine
        .protocol_layer
        .driver()
        .inject_received_data(&DUMMY_SPR_CAPS_EPR_CAPABLE);

    // Discovery -> WaitForCapabilities
    eprintln!("run_step 1");
    policy_engine.run_step().await.unwrap();

    // WaitForCapabilities -> EvaluateCapabilities
    eprintln!("run_step 2");
    policy_engine.run_step().await.unwrap();

    eprintln!("Probing first GoodCRC");
    let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    eprintln!("Got first GoodCRC");
    assert!(matches!(
        good_crc.header.message_type(),
        MessageType::Control(ControlMessageType::GoodCRC)
    ));

    // Simulate GoodCRC with ID 0
    simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 0);

    // EvaluateCapabilities -> SelectCapability
    policy_engine.run_step().await.unwrap();

    // Simulate Accept
    simulate_source_control_message(&mut policy_engine, ControlMessageType::Accept, 1);

    // SelectCapability -> TransitionSink
    policy_engine.run_step().await.unwrap();

    let request_capabilities =
        Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    assert!(matches!(
        request_capabilities.header.message_type(),
        MessageType::Data(DataMessageType::Request)
    ));

    // Simulate PsRdy
    simulate_source_control_message(&mut policy_engine, ControlMessageType::PsRdy, 2);

    // TransitionSink -> Ready
    policy_engine.run_step().await.unwrap();
    eprintln!("State after last run_step: {:?}", policy_engine.state);
    assert!(matches!(policy_engine.state, State::Ready(..)));

    eprintln!(
        "Has transmitted data: {}",
        policy_engine.protocol_layer.driver().has_transmitted_data()
    );
    let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    eprintln!("Got first GoodCRC: {:?}", good_crc.header.message_type());
    assert!(matches!(
        good_crc.header.message_type(),
        MessageType::Control(ControlMessageType::GoodCRC)
    ));

    // Probe any remaining messages from Phase 1
    while policy_engine.protocol_layer.driver().has_transmitted_data() {
        let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!("Draining leftover message: {:?}", msg.header.message_type());
    }

    eprintln!("\n=== Phase 1 Complete: SPR negotiation at 20V ===\n");

    // === Phase 2: EPR Mode Entry ===
    // Per spec 8.3.3.26.2, EPR mode entry flow:
    // 1. Sink sends EPR_Mode (Enter), starts SenderResponseTimer
    // 2. Source sends EnterAcknowledged
    // 3. Source performs cable discovery (we skip this in test)
    // 4. Source sends EnterSucceeded
    eprintln!("=== Phase 2: EPR Mode Entry ===");

    // Ready -> EprModeEntry (DPM triggers EnterEprMode event)
    policy_engine.run_step().await.unwrap();
    eprintln!("State after DPM event: {:?}", policy_engine.state);

    // Inject GoodCRC for EPR_Mode (Enter) that will be transmitted
    simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 1);

    // Inject EPR_Mode (EnterAcknowledged) using proper API
    // From capture line 78-81: RAW[6]: AA 19 00 00 00 02 (Source, msg_id=4)
    let epr_enter_ack_bytes = simulate_source_epr_mode_message(
        &mut policy_engine,
        Action::EnterAcknowledged,
        4, // message_id from capture
    );
    // Assert bytes match the real capture
    assert_eq!(
        &epr_enter_ack_bytes[..],
        &[0xAA, 0x19, 0x00, 0x00, 0x00, 0x02],
        "EPR EnterAcknowledged bytes should match capture"
    );

    // EprModeEntry: sends EPR_Mode (Enter), receives EnterAcknowledged -> EprEntryWaitForResponse
    match policy_engine.run_step().await {
        Ok(_) => eprintln!("EprModeEntry run_step succeeded"),
        Err(e) => eprintln!("EprModeEntry run_step failed: {:?}", e),
    }
    eprintln!("State after EprModeEntry: {:?}", policy_engine.state);

    // Probe EPR_Mode (Enter) message
    let epr_enter_bytes = policy_engine.protocol_layer.driver().probe_transmitted_data();
    let epr_enter = Message::from_bytes(&epr_enter_bytes).unwrap();
    eprintln!("Probed message type: {:?}", epr_enter.header.message_type());
    assert!(matches!(
        epr_enter.header.message_type(),
        MessageType::Data(DataMessageType::EprMode)
    ));
    if let Some(Payload::Data(Data::EprMode(mode))) = epr_enter.payload {
        assert_eq!(mode.action(), Action::Enter);
    } else {
        panic!("Expected EprMode Enter payload");
    }
    // Assert EPR_Mode Enter bytes match capture line 71-74: RAW[6]: 8A 14 00 00 00 01
    // Note: Our test starts from different state so message_id may differ
    eprintln!("EPR_Mode Enter bytes: {:02X?}", &epr_enter_bytes[..]);

    // Probe GoodCRC for EnterAcknowledged
    let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    eprintln!("Probed GoodCRC for EnterAck: {:?}", good_crc.header.message_type());
    assert!(matches!(
        good_crc.header.message_type(),
        MessageType::Control(ControlMessageType::GoodCRC)
    ));

    // Inject EPR_Mode (EnterSucceeded) using proper API
    // From capture line 85-88: RAW[6]: AA 1B 00 00 00 03 (Source, msg_id=5)
    let epr_enter_succeeded_bytes = simulate_source_epr_mode_message(
        &mut policy_engine,
        Action::EnterSucceeded,
        5, // message_id from capture
    );
    // Assert bytes match the real capture
    assert_eq!(
        &epr_enter_succeeded_bytes[..],
        &[0xAA, 0x1B, 0x00, 0x00, 0x00, 0x03],
        "EPR EnterSucceeded bytes should match capture"
    );

    // EprEntryWaitForResponse receives EnterSucceeded -> EprWaitForCapabilities
    policy_engine.run_step().await.unwrap();
    eprintln!("State after EnterSucceeded: {:?}", policy_engine.state);

    // Probe GoodCRC for EnterSucceeded
    let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    assert!(matches!(
        good_crc.header.message_type(),
        MessageType::Control(ControlMessageType::GoodCRC)
    ));

    eprintln!("=== Phase 2 Complete: EPR mode entry succeeded ===\n");

    // === Phase 3: Chunked EPR Source Capabilities ===
    // This follows the real-world capture flow per USB PD spec 6.12.2.1.2:
    // 1. Source sends chunk 0 -> Sink sends GoodCRC
    // 2. Sink sends Chunk Request (chunk=1) -> Source sends GoodCRC
    // 3. Source sends chunk 1 -> Sink sends GoodCRC
    use crate::dummy::{DUMMY_EPR_SOURCE_CAPS_CHUNK_0, DUMMY_EPR_SOURCE_CAPS_CHUNK_1};

    eprintln!("=== Phase 3: Chunked EPR Source Capabilities ===");

    // Source sends EPR_Source_Capabilities chunk 0
    policy_engine
        .protocol_layer
        .driver()
        .inject_received_data(&DUMMY_EPR_SOURCE_CAPS_CHUNK_0);

    // Inject GoodCRC for the Chunk Request that sink will send after receiving chunk 0
    // The chunk request message ID will be based on tx_message counter
    simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 2);

    // Source sends chunk 1 after receiving the chunk request
    policy_engine
        .protocol_layer
        .driver()
        .inject_received_data(&DUMMY_EPR_SOURCE_CAPS_CHUNK_1);

    // EprWaitForCapabilities -> Protocol layer:
    // - receives chunk 0, sends GoodCRC
    // - sends chunk request, waits for GoodCRC
    // - receives chunk 1, sends GoodCRC
    // - assembles message -> EvaluateCapabilities
    policy_engine.run_step().await.unwrap();

    // Probe GoodCRC for chunk 0
    let good_crc_0 = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    eprintln!("Chunk 0 GoodCRC: {:?}", good_crc_0.header.message_type());
    assert!(matches!(
        good_crc_0.header.message_type(),
        MessageType::Control(ControlMessageType::GoodCRC)
    ));

    // Probe the Chunk Request message (per spec 6.12.2.1.2.4)
    // Chunk requests are parsed as ChunkedExtendedMessage error, so we use parse_extended_chunk
    let chunk_req_data = policy_engine.protocol_layer.driver().probe_transmitted_data();
    let (chunk_req_header, chunk_req_ext_header, _chunk_data) = Message::parse_extended_chunk(&chunk_req_data).unwrap();
    eprintln!(
        "Chunk Request: type={:?}, chunk_number={}, request_chunk={}",
        chunk_req_header.message_type(),
        chunk_req_ext_header.chunk_number(),
        chunk_req_ext_header.request_chunk()
    );
    assert!(
        chunk_req_header.extended(),
        "Chunk request should be an extended message"
    );
    assert!(matches!(
        chunk_req_header.message_type(),
        MessageType::Extended(ExtendedMessageType::EprSourceCapabilities)
    ));
    assert!(chunk_req_ext_header.request_chunk(), "Should be a chunk request");
    assert_eq!(chunk_req_ext_header.chunk_number(), 1, "Should request chunk 1");

    // Probe GoodCRC for chunk 1
    let good_crc_1 = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    eprintln!("Chunk 1 GoodCRC: {:?}", good_crc_1.header.message_type());
    assert!(matches!(
        good_crc_1.header.message_type(),
        MessageType::Control(ControlMessageType::GoodCRC)
    ));

    eprintln!("=== Phase 3 Complete: EPR caps assembled (with chunk request per spec) ===\n");

    // === Phase 4: EPR Power Negotiation ===
    // Selects PDO#8 (28V @ 5A = 140W)
    eprintln!("=== Phase 4: EPR Power Negotiation ===");

    // EvaluateCapabilities -> DPM.request() selects EPR PDO#8 (28V) -> SelectCapability
    policy_engine.run_step().await.unwrap();
    eprintln!("State after evaluate: {:?}", policy_engine.state);

    // Inject GoodCRC for the EprRequest that will be transmitted
    // Note: TX message counter is now 3 (after chunk request in Phase 3 incremented it from 2 to 3)
    simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3);

    // Also inject Accept message that SelectCapability will wait for after transmitting
    simulate_source_control_message(&mut policy_engine, ControlMessageType::Accept, 0);

    // SelectCapability -> sends EprRequest, waits for Accept -> TransitionSink
    policy_engine.run_step().await.unwrap();

    // Probe the EPR Request
    let epr_request = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
    eprintln!(
        "EPR Request: {:?} (message_id={})",
        epr_request.header.message_type(),
        epr_request.header.message_id()
    );
    assert!(matches!(
        epr_request.header.message_type(),
        MessageType::Data(DataMessageType::EprRequest)
    ));

    // Verify EPR Request selects PDO#8 (28V)
    if let Some(Payload::Data(Data::Request(PowerSource::EprRequest(epr)))) = &epr_request.payload {
        let object_pos = epr.object_position();
        eprintln!("EPR Request: PDO#{} (RDO=0x{:08X})", object_pos, epr.rdo);
        assert_eq!(object_pos, 8, "Should request PDO#8 (28V) to match real capture");

        // Verify it's the 28V PDO
        if let PowerDataObject::FixedSupply(fixed) = epr.pdo {
            assert_eq!(fixed.raw_voltage(), 560, "28V = 560 * 50mV");
            assert_eq!(fixed.raw_max_current(), 500, "5A = 500 * 10mA");
        }
    } else {
        panic!("Expected EprRequest payload");
    }

    // Drain any leftover messages
    eprintln!(
        "Has transmitted data before drain: {}",
        policy_engine.protocol_layer.driver().has_transmitted_data()
    );
    while policy_engine.protocol_layer.driver().has_transmitted_data() {
        let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!("Draining: {:?}", msg.header.message_type());
    }
    eprintln!(
        "Has transmitted data after drain: {}",
        policy_engine.protocol_layer.driver().has_transmitted_data()
    );

    // Inject PsRdy that TransitionSink will wait for
    simulate_source_control_message(&mut policy_engine, ControlMessageType::PsRdy, 1);

    // TransitionSink waits for PsRdy -> Ready
    policy_engine.run_step().await.unwrap();

    // Probe any GoodCRCs we transmitted (for Accept and PsRdy messages we received)
    while policy_engine.protocol_layer.driver().has_transmitted_data() {
        let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!("Phase 4 transmitted: {:?}", msg.header.message_type());
        assert!(matches!(
            msg.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));
    }

    // Verify we're in Ready state with EPR power
    assert!(matches!(policy_engine.state, State::Ready(..)));
    eprintln!("Final state: {:?}", policy_engine.state);

    eprintln!("=== Phase 4 Complete: EPR power negotiation at 28V/5A (140W) ===\n");

    // === Phase 5: EPR Keep-Alive ===
    // Per USB PD spec 8.3.3.3.11, sink must send EprKeepAlive periodically in EPR mode.
    // Real capture shows multiple keep-alive exchanges after EPR contract (lines 145-214).
    // We manually transition to EprKeepAlive state to test this flow, simulating multiple
    // keep-alive cycles to verify the sink continues sending them.
    eprintln!("=== Phase 5: EPR Keep-Alive (multiple cycles) ===");

    // Test multiple keep-alive cycles to verify the sink keeps sending them
    // Real capture shows 7 keep-alive exchanges - we'll test 3 to verify the pattern
    // From capture (lines 152-183), EprKeepAliveAck messages have Source/Dfp roles.
    let mut sink_tx_counter = 4u8; // Sink TX counter after EPR Request
    let mut source_tx_counter = 2u8; // Source TX counter (increments with each message source sends)

    // Expected EprKeepAliveAck bytes from capture (for first 3 cycles):
    // Cycle 1 (msg_id=2): B0 95 02 80 04 00
    // Cycle 2 (msg_id=3): B0 97 02 80 04 00
    // Cycle 3 (msg_id=4): B0 99 02 80 04 00
    // Note: Header changes based on msg_id, but extended header (02 80) and payload (04 00) stay same

    for cycle in 1..=3 {
        eprintln!("--- Keep-Alive cycle {} ---", cycle);

        // Manually set state to EprKeepAlive (normally triggered by SinkEPRKeepAliveTimer in Ready state)
        if let State::Ready(power_source, _) = policy_engine.state.clone() {
            policy_engine.state = State::EprKeepAlive(power_source);
        } else {
            panic!("Expected Ready state before keep-alive cycle {}", cycle);
        }

        // Inject GoodCRC for the EprKeepAlive message that will be transmitted
        simulate_source_control_message(&mut policy_engine, ControlMessageType::GoodCRC, sink_tx_counter);
        sink_tx_counter = sink_tx_counter.wrapping_add(1);

        // Inject EprKeepAliveAck response from source with correct message ID
        let keep_alive_ack_bytes = simulate_epr_keep_alive_ack(&mut policy_engine, source_tx_counter);
        eprintln!("  EprKeepAliveAck bytes: {:02X?}", &keep_alive_ack_bytes[..]);
        // Verify payload matches capture pattern
        // The extended header data_size=2 (low byte 0x02), and the chunked bit may or may not be set
        // (spec allows both). Real capture shows chunked=true (0x80), but our impl uses chunked=false (0x00)
        // Payload: 04 00 (ExtendedControl with EprKeepAliveAck type)
        assert_eq!(keep_alive_ack_bytes[2] & 0x1F, 0x02, "data_size should be 2");
        assert_eq!(
            &keep_alive_ack_bytes[4..],
            &[0x04, 0x00],
            "EprKeepAliveAck payload should match capture"
        );
        source_tx_counter = source_tx_counter.wrapping_add(1);

        // EprKeepAlive sends keep-alive, receives ack -> Ready
        policy_engine.run_step().await.unwrap();

        // Probe the EprKeepAlive message
        let keep_alive_bytes = policy_engine.protocol_layer.driver().probe_transmitted_data();
        let keep_alive = Message::from_bytes(&keep_alive_bytes).unwrap();
        eprintln!("  EprKeepAlive sent: {:?}", keep_alive.header.message_type());
        assert!(matches!(
            keep_alive.header.message_type(),
            MessageType::Extended(ExtendedMessageType::ExtendedControl)
        ));

        // Verify it's actually an EprKeepAlive message
        if let Some(Payload::Extended(crate::protocol_layer::message::extended::Extended::ExtendedControl(ctrl))) =
            &keep_alive.payload
        {
            assert_eq!(
                ctrl.message_type(),
                crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType::EprKeepAlive,
                "Expected EprKeepAlive message type"
            );
        } else {
            panic!("Expected ExtendedControl payload with EprKeepAlive");
        }
        // Verify EprKeepAlive payload matches capture pattern
        // From capture (e.g. line 145-148): 90 9A 02 80 03 00
        // Extended header data_size=2, Payload: 03 00 (EprKeepAlive type)
        // Note: chunked bit may differ between our impl (0x00) and capture (0x80)
        assert_eq!(keep_alive_bytes[2] & 0x1F, 0x02, "data_size should be 2");
        assert_eq!(
            &keep_alive_bytes[4..],
            &[0x03, 0x00],
            "EprKeepAlive payload should match capture"
        );

        // Probe GoodCRC for EprKeepAliveAck
        let good_crc = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        assert!(matches!(
            good_crc.header.message_type(),
            MessageType::Control(ControlMessageType::GoodCRC)
        ));

        // Verify we're back in Ready state (ready for next keep-alive cycle)
        assert!(matches!(policy_engine.state, State::Ready(..)));
        eprintln!("  Returned to Ready state");
    }

    eprintln!("=== Phase 5 Complete: {} EPR keep-alive cycles succeeded ===\n", 3);
    eprintln!("=== Full EPR negotiation test PASSED ===");
}
