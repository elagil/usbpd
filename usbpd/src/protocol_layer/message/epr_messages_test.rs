//! EPR (Extended Power Range) message parsing tests using real captured data.
//!
//! Test fixtures captured from actual EPR hardware negotiation (KM003C sniffer).
//! Covers: EPR mode entry, chunked source capabilities, EPR requests, keep-alive.

use crate::dummy::{DUMMY_EPR_SOURCE_CAPS_CHUNK_0, DUMMY_EPR_SOURCE_CAPS_CHUNK_1};
use crate::protocol_layer::message::data::Data;
use crate::protocol_layer::message::data::epr_mode::Action;
use crate::protocol_layer::message::data::request::PowerSource;
use crate::protocol_layer::message::extended::Extended;
use crate::protocol_layer::message::extended::chunked::{ChunkResult, ChunkedMessageAssembler};
use crate::protocol_layer::message::header::{DataMessageType, ExtendedMessageType, MessageType};
use crate::protocol_layer::message::{Message, Payload};

// ============================================================================
// Test Fixtures - Real EPR Messages
// ============================================================================

/// EPR Mode: Enter (Sink → Source)
const EPR_MODE_ENTER: &[u8] = &[0x8A, 0x14, 0x00, 0x00, 0x00, 0x01];

/// EPR Mode: EnterAcknowledged (Source → Sink)
const EPR_MODE_ENTER_ACK: &[u8] = &[0xAA, 0x19, 0x00, 0x00, 0x00, 0x02];

/// EPR Mode: EnterSucceeded (Source → Sink)
const EPR_MODE_ENTER_SUCCEEDED: &[u8] = &[0xAA, 0x1B, 0x00, 0x00, 0x00, 0x03];

/// EPR Request for 28V @ 5A (140W) - PDO#8
const EPR_REQUEST_28V: &[u8] = &[0x89, 0x28, 0xF4, 0xD1, 0xC7, 0x80, 0xF4, 0xC1, 0x18, 0x00];

/// EPR Keep-Alive (Sink → Source)
const EPR_KEEP_ALIVE: &[u8] = &[0x90, 0x9A, 0x02, 0x80, 0x03, 0x00];

// ============================================================================
// Core EPR Message Parsing Tests
// ============================================================================

#[test]
fn test_epr_mode_messages() {
    // Test EPR Mode Enter
    let enter = Message::from_bytes(EPR_MODE_ENTER).expect("Failed to parse EPR_MODE_ENTER");
    assert_eq!(enter.header.message_type(), MessageType::Data(DataMessageType::EprMode));
    if let Some(Payload::Data(Data::EprMode(mode))) = enter.payload {
        assert_eq!(mode.action(), Action::Enter);
    } else {
        panic!("Expected EprMode Enter payload");
    }

    // Test EPR Mode EnterAcknowledged
    let ack = Message::from_bytes(EPR_MODE_ENTER_ACK).expect("Failed to parse EPR_MODE_ENTER_ACK");
    if let Some(Payload::Data(Data::EprMode(mode))) = ack.payload {
        assert_eq!(mode.action(), Action::EnterAcknowledged);
    } else {
        panic!("Expected EprMode EnterAcknowledged payload");
    }

    // Test EPR Mode EnterSucceeded
    let success = Message::from_bytes(EPR_MODE_ENTER_SUCCEEDED).expect("Failed to parse EPR_MODE_ENTER_SUCCEEDED");
    if let Some(Payload::Data(Data::EprMode(mode))) = success.payload {
        assert_eq!(mode.action(), Action::EnterSucceeded);
    } else {
        panic!("Expected EprMode EnterSucceeded payload");
    }
}

#[test]
fn test_chunked_epr_source_caps_assembly() {
    let mut assembler = ChunkedMessageAssembler::new();

    // Process chunk 0
    let (header_0, ext_header_0, chunk_data_0) =
        Message::parse_extended_chunk(&DUMMY_EPR_SOURCE_CAPS_CHUNK_0).expect("Failed to parse chunk 0");

    match assembler
        .process_chunk(header_0, ext_header_0, chunk_data_0)
        .expect("Failed to process chunk 0")
    {
        ChunkResult::NeedMoreChunks(next) => {
            assert_eq!(next, 1, "Should request chunk 1 next");
        }
        _ => panic!("Expected NeedMoreChunks after first chunk"),
    }

    // Process chunk 1
    let (header_1, ext_header_1, chunk_data_1) =
        Message::parse_extended_chunk(&DUMMY_EPR_SOURCE_CAPS_CHUNK_1).expect("Failed to parse chunk 1");

    match assembler
        .process_chunk(header_1, ext_header_1, chunk_data_1)
        .expect("Failed to process chunk 1")
    {
        ChunkResult::Complete(assembled_data) => {
            // Parse the assembled EPR Source Capabilities
            let ext = Message::parse_extended_payload(ExtendedMessageType::EprSourceCapabilities, &assembled_data);

            if let Extended::EprSourceCapabilities(pdos) = ext {
                assert_eq!(pdos.len(), 10, "Expected 10 PDOs (6 SPR + 1 separator + 3 EPR)");

                // Verify separator at PDO[6]
                if let crate::protocol_layer::message::data::source_capabilities::PowerDataObject::FixedSupply(pdo) =
                    &pdos[6]
                {
                    assert_eq!(pdo.0, 0, "PDO[6] should be separator (0x00000000)");
                } else {
                    panic!("PDO[6] should be separator");
                }

                // Verify EPR PDO exists at position 7 (28V)
                use uom::si::electric_potential::volt;
                if let crate::protocol_layer::message::data::source_capabilities::PowerDataObject::FixedSupply(pdo) =
                    &pdos[7]
                {
                    assert_eq!(pdo.voltage().get::<volt>() as f64, 28.0);
                } else {
                    panic!("PDO[7] should be 28V EPR FixedSupply");
                }
            } else {
                panic!("Expected EprSourceCapabilities payload");
            }
        }
        _ => panic!("Expected Complete after second chunk"),
    }
}

#[test]
fn test_epr_request_parsing() {
    let msg = Message::from_bytes(EPR_REQUEST_28V).expect("Failed to parse EPR_REQUEST_28V");

    assert_eq!(
        msg.header.message_type(),
        MessageType::Data(DataMessageType::EprRequest)
    );
    assert_eq!(
        msg.header.num_objects(),
        2,
        "EPR Request should have 2 data objects (RDO + PDO)"
    );

    if let Some(Payload::Data(Data::Request(PowerSource::EprRequest(epr)))) = msg.payload {
        // Verify RDO requests PDO#8
        assert_eq!(epr.object_position(), 8, "Should request PDO#8");

        // Verify PDO is 28V
        use uom::si::electric_potential::volt;

        use crate::protocol_layer::message::data::source_capabilities::PowerDataObject;
        if let PowerDataObject::FixedSupply(fixed) = epr.pdo {
            assert_eq!(fixed.voltage().get::<volt>() as f64, 28.0);
        } else {
            panic!("Expected FixedSupply PDO in EprRequest");
        }
    } else {
        panic!("Expected EprRequest payload");
    }
}

#[test]
fn test_epr_keep_alive() {
    let msg = Message::from_bytes(EPR_KEEP_ALIVE).expect("Failed to parse EPR_KEEP_ALIVE");

    assert_eq!(
        msg.header.message_type(),
        MessageType::Extended(ExtendedMessageType::ExtendedControl)
    );

    if let Some(Payload::Extended(Extended::ExtendedControl(ctrl))) = msg.payload {
        use crate::protocol_layer::message::extended::extended_control::ExtendedControlMessageType;
        assert_eq!(ctrl.message_type(), ExtendedControlMessageType::EprKeepAlive);
    } else {
        panic!("Expected ExtendedControl EprKeepAlive payload");
    }
}
