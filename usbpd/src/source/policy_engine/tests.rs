//! Tests for the policy engine.

use super::Source;
use crate::counters::{Counter, CounterType};
use crate::dummy::{
    DummyDriver, DummyDualRoleDevice, DummySourceDevice, DummyTimer, MAX_DATA_MESSAGE_SIZE,
    get_dummy_source_capabilities,
};
use crate::protocol_layer::message::Message;
use crate::protocol_layer::message::data::Data;
use crate::protocol_layer::message::data::request::{
    CurrentRequest, EprRequestDataObject, FixedVariableSupply, PowerSource, VoltageRequest,
};
use crate::protocol_layer::message::data::source_capabilities::SourceCapabilities;
use crate::protocol_layer::message::header::{ControlMessageType, DataMessageType, Header, MessageType};
use crate::source::device_policy_manager::{CapabilityResponse, SourceDpm};
use crate::source::policy_engine::{DataRoleSwap, FastPowerRoleSwap, PowerRoleSwap, State, SwapState};

fn simulate_sink_control_message<DPM: SourceDpm>(
    policy_engine: &mut Source<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DPM>,
    control_message_type: ControlMessageType,
    message_id: u8,
) {
    let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];

    Message::new(Header::new_control(
        get_sink_header_template(),
        Counter::new_from_value(CounterType::MessageId, message_id),
        control_message_type,
    ))
    .to_bytes(&mut buf);
    policy_engine.protocol_layer.driver().inject_received_data(&buf);
}

/// Get a header template for simulating sink messages (Sink/Ufp roles).
/// This flips the roles from the source's perspective to simulate messages from the sink.
fn get_sink_header_template() -> Header {
    use crate::protocol_layer::message::header::SpecificationRevision;
    use crate::{DataRole, PowerRole};

    // Non-DRP Sink messages have Sink/Ufp roles (opposite of source's Source/Dfp)
    Header::new_template(DataRole::Ufp, PowerRole::Sink, SpecificationRevision::R3_X)
}

fn request_capability_message(message_id: u8, highest_power: bool) -> Message {
    let source_capabilities =
        SourceCapabilities(heapless::Vec::from_slice(get_dummy_source_capabilities().as_slice()).unwrap());
    let header = Header::new_data(
        get_sink_header_template(),
        Counter::new_from_value(CounterType::MessageId, message_id),
        DataMessageType::Request,
        source_capabilities.0.len() as u8,
    );

    let data = match highest_power {
        true => Data::Request(
            PowerSource::new_fixed(CurrentRequest::Highest, VoltageRequest::Highest, &source_capabilities).unwrap(),
        ),
        false => Data::Request(
            PowerSource::new_fixed(CurrentRequest::Highest, VoltageRequest::Safe5V, &source_capabilities).unwrap(),
        ),
    };

    Message::new_with_data(header, data)
}

fn simulate_sink_request<DPM: SourceDpm>(
    policy_engine: &mut Source<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DPM>,
    message_id: u8,
    highest_power: bool,
) {
    let message = request_capability_message(message_id, highest_power);
    let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];
    let len = message.to_bytes(&mut buf);

    policy_engine.protocol_layer.driver().inject_received_data(&buf[..len]);
}

async fn run_test_step<DPM: SourceDpm>(
    policy_engine: &mut Source<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DPM>,
    expected_state: &State,
) {
    policy_engine.run_step().await.unwrap();
    assert!(
        core::mem::discriminant(&policy_engine.state) == core::mem::discriminant(expected_state),
        "state after run_step is {:?}, expected {expected_state:?}",
        policy_engine.state
    );
}

#[tokio::test]
async fn test_negotiation() {
    let driver = DummyDriver::new();
    let device = DummySourceDevice;
    let mut policy_engine = Source::new(driver, device, false);
    {
        // Instantiated in `SendCapabilities` state (not started from role-swap)
        assert!(matches!(policy_engine.state, State::Startup { role_swap: false }));

        // `Startup` -> `SendCapabilities`
        run_test_step(&mut policy_engine, &State::SendCapabilities).await;

        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 0);
        // Request vSafe5V @ highest current
        simulate_sink_request(&mut policy_engine, 1, false);

        // `SendCapabilities` -> Get Request -> `NegotiateCapability`
        policy_engine.run_step().await.unwrap();
        {
            let sent_capabilities =
                Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            assert_eq!(
                sent_capabilities.header.message_type(),
                MessageType::Data(DataMessageType::SourceCapabilities)
            );

            let good_crc =
                Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            assert_eq!(
                good_crc.header.message_type(),
                MessageType::Control(ControlMessageType::GoodCRC)
            );
            assert!(matches!(policy_engine.state, State::NegotiateCapability(_)));
        }

        // `NegotiateCapability` -> DPM: Accept -> `Transition Supply`
        policy_engine.run_step().await.unwrap();
        assert!(matches!(policy_engine.state, State::TransitionSupply(_)));

        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 1);
        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 2);

        // `TransitionSupply` -> Accept -> PsRdy -> `Ready`
        policy_engine.run_step().await.unwrap();
        {
            while policy_engine.protocol_layer.driver().has_transmitted_data() {
                _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
            }
            assert!(matches!(policy_engine.state, State::Ready));
        }
    }
    {
        // Simulate `GoodCrc` to SourceCapabilities
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GetSourceCap, 3);

        // `Ready` -> GetSourceCap Received -> `SendCapabilities`
        run_test_step(&mut policy_engine, &State::SendCapabilities).await;

        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3);
        // Request highest power
        simulate_sink_request(&mut policy_engine, 4, true);

        // `SendCapabilities` -> Get Request -> `NegotiateCapability`
        policy_engine.run_step().await.unwrap();
        assert!(matches!(policy_engine.state, State::NegotiateCapability(_)));

        // `NegotiateCapability` -> Request Can Be Met -> `TransitionSupply`
        policy_engine.run_step().await.unwrap();
        assert!(matches!(policy_engine.state, State::TransitionSupply(_)));

        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 4);
        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 5);

        // `TransitionSupply` -> Accept -> PsRdy -> `Ready`
        run_test_step(&mut policy_engine, &State::Ready).await;

        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
        }
    }
    {
        // Simulate `SoftReset`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::SoftReset, 6);

        // `Ready` -> Soft Reset received -> `SoftReset`
        run_test_step(&mut policy_engine, &State::SoftReset).await;

        // `GoodCrc` for `Accept`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 0);

        // `SoftReset` -> `SendCapabilities`
        run_test_step(&mut policy_engine, &State::SendCapabilities).await;

        // `GoodCrc` for `SourceCapabilities`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 1);

        // Make an invalid request
        let data = Data::Request(PowerSource::FixedVariableSupply(FixedVariableSupply(0xA000_0000)));
        let header = Header::new_data(
            get_sink_header_template(),
            Counter::new_from_value(CounterType::MessageId, 2),
            DataMessageType::Request,
            7,
        );
        let message = Message::new_with_data(header, data);
        let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];
        let len = message.to_bytes(&mut buf);
        policy_engine.protocol_layer.driver().inject_received_data(&buf[..len]);

        policy_engine.run_step().await.unwrap();
        assert!(matches!(policy_engine.state, State::NegotiateCapability(_)));

        // `NegotiateCapability` -> evaluates to reject -> `CapabilityResponse::Reject`
        run_test_step(
            &mut policy_engine,
            &State::CapabilityResponse(CapabilityResponse::Reject),
        )
        .await;

        // `GoodCrc` for `Reject`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 2);

        // `CapabilityResponse::Reject` -> Reject message sent & have a valid contract -> `Ready`
        run_test_step(&mut policy_engine, &State::Ready).await;

        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
        }
    }
}

#[tokio::test]
async fn test_discovery() {
    const MAX_DISCOVERY_ITERS: usize = 52;
    let driver = DummyDriver::new();
    let device = DummySourceDevice;
    let mut policy_engine = Source::new(driver, device, false);

    // HardReset -> Discovery -> Disabled
    {
        assert!(matches!(policy_engine.state, State::Startup { role_swap: false }));

        // `Startup` -> `SendCapabilities`
        run_test_step(&mut policy_engine, &State::SendCapabilities).await;

        // No `GoodCRC` is injected: every caps transmission exhausts `nRetryCount`
        // and the `SenderResponseTimer` never sees a `Request`.
        //
        // `SendCapabilities` -> Capability Send Failure -> `Discovery`
        run_test_step(&mut policy_engine, &State::Discovery).await;

        let mut discovery_iterations = 0;
        loop {
            if !matches!(policy_engine.state, State::Discovery) {
                break;
            }

            // `Discovery` -> `SendCapabilities`
            policy_engine.run_step().await.unwrap();
            discovery_iterations += 1;

            if discovery_iterations > MAX_DISCOVERY_ITERS {
                break;
            }
            if !matches!(policy_engine.state, State::SendCapabilities) {
                break;
            }

            // `SendCapabilities` -> `Discovery`
            policy_engine.run_step().await.unwrap();
        }
        assert!(matches!(policy_engine.state, State::Disabled));
    }
}

/// Take a new policy engine and skip the initial negotation:
/// `Startup -> ... -> Ready`
async fn skip_to_ready<DPM: SourceDpm>(
    policy_engine: &mut Source<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DPM>,
) {
    assert!(matches!(policy_engine.state, State::Startup { role_swap: false }));

    simulate_sink_control_message(policy_engine, ControlMessageType::GoodCRC, 0);
    simulate_sink_request(policy_engine, 1, false);
    simulate_sink_control_message(policy_engine, ControlMessageType::GoodCRC, 1);
    simulate_sink_control_message(policy_engine, ControlMessageType::GoodCRC, 2);

    policy_engine.run_step().await.unwrap(); // `Startup` -> `SendCapabilities`
    policy_engine.run_step().await.unwrap(); // `SendCapabilities` -> Get Request -> `NegotiateCapability`
    policy_engine.run_step().await.unwrap(); // `NegotiateCapability` -> DPM: Accept -> `Transition Supply`
    policy_engine.run_step().await.unwrap(); // `TransitionSupply` -> Accept -> PsRdy -> `Ready`

    // Flush all messages
    while policy_engine.protocol_layer.driver().has_transmitted_data() {
        _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
    }

    assert!(matches!(policy_engine.state, State::Ready));
}

#[tokio::test]
async fn test_role_swapping() {
    {
        let driver = DummyDriver::new();
        let device = DummyDualRoleDevice;
        let mut policy_engine = Source::new_dual_role(driver, device, false);
        skip_to_ready(&mut policy_engine).await;

        simulate_sink_control_message(&mut policy_engine, ControlMessageType::PrSwap, 2);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3); // `GoodCrc` for `Accept`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 4); // `GoodCrc` for `PS_RDY` 3 OR 4??
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::PsRdy, 4);

        // `Ready` -> PR_Swap Message Received -> `DrpSwap(SwapState::Power(PowerRoleSwap::Evaluate))`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::Evaluate)),
        )
        .await;

        // `PowerRoleSwap::Evaluate` -> DRP Evaluates to `true` -> `PowerRoleSwap::Accept`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::Accept)),
        )
        .await;

        // `PowerRoleSwap::Accept` -> `Accept` Sent -> `PowerRoleSwap::TransitionToOff`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::TransitionToOff)),
        )
        .await;

        // `PowerRoleSwap::TransitionToOff` -> DPM turned source off -> `PowerRoleSwap::AssertRd`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::AssertRd)),
        )
        .await;

        // `PowerRoleSwap::AssertRd` -> DPM asserted Rd -> `PowerRoleSwap::WaitSourceOn`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::WaitSourceOn)),
        )
        .await;

        // `PowerRoleSwap::WaitSourceOn` -> `PS_RDY` Sent, `PS_RDY` Received -> `PrSwapToSinkStartup`
        run_test_step(&mut policy_engine, &State::PrSwapToSinkStartup).await;

        // Flush all messages
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
        }
    }
    {
        let driver = DummyDriver::new();
        let device = DummyDualRoleDevice;
        let mut policy_engine = Source::new_dual_role(driver, device, false);
        skip_to_ready(&mut policy_engine).await;

        simulate_sink_control_message(&mut policy_engine, ControlMessageType::FrSwap, 2);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3); // `GoodCrc` for `Accept` 3 OR 4??
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 4); // `GoodCrc` for `PS_RDY` 3 OR 4??
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::PsRdy, 4);

        // `Ready` -> FR_Swap Message Received -> `FastPowerRole::Evaluate`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::Evaluate)),
        )
        .await;

        // `FastPowerRole::Evaluate` -> DPM: Fast Role Swap Signaled -> `FastPowerRole::Accept`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::Accept)),
        )
        .await;

        // `FastPowerRole::Accept` -> `Accept` sent -> `FastPowerRole::TransitionToOff`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::TransitionToOff)),
        )
        .await;

        // `PowerRoleSwap::TransitionToOff` -> DPM turned source off -> `PowerRoleSwap::AssertRd`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::AssertRd)),
        )
        .await;

        // `PowerRoleSwap::AssertRd` -> DPM asserted Rd -> `PowerRoleSwap::WaitSourceOn`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::WaitSourceOn)),
        )
        .await;

        // `PowerRoleSwap::WaitSourceOn` -> `PS_RDY` Sent, `PS_RDY` Received -> `PrSwapToSinkStartup`
        run_test_step(&mut policy_engine, &State::PrSwapToSinkStartup).await;

        // Flush all messages
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
        }
    }
    {
        let driver = DummyDriver::new();
        let device = DummyDualRoleDevice;
        let mut policy_engine = Source::new_dual_role(driver, device, false);
        skip_to_ready(&mut policy_engine).await;

        simulate_sink_control_message(&mut policy_engine, ControlMessageType::DrSwap, 2);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3); // `GoodCrc` for `Accept`

        // `Ready` -> DR_Swap Message Received -> `DataRoleSwap::Evaluate`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Data(DataRoleSwap::Evaluate)),
        )
        .await;

        // `DataRoleSwap::Evaluate` -> DRP: Data Role Swap ok -> `DataRoleSwap::Accept`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Data(DataRoleSwap::Accept)),
        )
        .await;

        // `DataRoleSwap::Accept` -> `Accept` sent -> `DataRoleSwap::Change`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Data(DataRoleSwap::Change)),
        )
        .await;

        // `DataRoleSwap::Change` -> DPM: Port changed data role -> `Ready`
        run_test_step(&mut policy_engine, &State::Ready).await;

        // Flush all messages
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
        }
    }
    {
        // Test rejects at the policy engine layer (i.e. this is not a dual-role device)
        let driver = DummyDriver::new();
        let device = DummySourceDevice;
        let mut policy_engine = Source::new(driver, device, false);
        skip_to_ready(&mut policy_engine).await;

        simulate_sink_control_message(&mut policy_engine, ControlMessageType::PrSwap, 2);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3); // `GoodCrc` for `SendNotSupported`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::FrSwap, 3);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 4); // `GoodCrc` for `SendNotSupported`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::DrSwap, 4);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 5); // `GoodCrc` for `SendNotSupported`

        run_test_step(&mut policy_engine, &State::SendNotSupported).await;
        run_test_step(&mut policy_engine, &State::Ready).await;

        run_test_step(&mut policy_engine, &State::SendNotSupported).await;
        run_test_step(&mut policy_engine, &State::Ready).await;

        run_test_step(&mut policy_engine, &State::SendNotSupported).await;
        run_test_step(&mut policy_engine, &State::Ready).await;

        // Flush all messages
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
        }
    }
    {
        // Test rejects at the DPM layer
        let driver = DummyDriver::new();
        let device = DummySourceDevice;
        let mut policy_engine = Source::new_dual_role(driver, device, false);
        skip_to_ready(&mut policy_engine).await;

        simulate_sink_control_message(&mut policy_engine, ControlMessageType::PrSwap, 2);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3); // `GoodCrc` for `Reject`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::DrSwap, 3);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 4); // `GoodCrc` for `Reject`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::FrSwap, 5);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 5); // `GoodCrc` for `HardReset`

        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::Evaluate)),
        )
        .await;
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::Reject)),
        )
        .await;
        run_test_step(&mut policy_engine, &State::Ready).await;

        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Data(DataRoleSwap::Evaluate)),
        )
        .await;
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Data(DataRoleSwap::Reject)),
        )
        .await;
        run_test_step(&mut policy_engine, &State::Ready).await;

        // Fast Power swaps are not rejected but instead "panicked" out of. Here, we are testing correct
        // state traversal when the DPM does not detect fast role swap on the CC pins:
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::Evaluate)),
        )
        .await;
        run_test_step(&mut policy_engine, &State::HardReset).await;
        run_test_step(&mut policy_engine, &State::TransitionToDefault).await;
        run_test_step(&mut policy_engine, &State::Startup { role_swap: false }).await;

        // Flush all messages
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
        }
    }
}

/// A successful PR_Swap handoff must exit the policy engine with `RunResult::SwapToSink` instead of continuing the loop.
#[tokio::test]
async fn test_pr_swap_to_sink_startup_exits() {
    let driver = DummyDriver::new();
    let device = DummySourceDevice;
    let mut policy_engine =
        Source::<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DummySourceDevice>::new(driver, device, false);

    policy_engine.state = State::PrSwapToSinkStartup;

    let result = policy_engine.run_step().await.unwrap();
    assert!(
        matches!(result, crate::PolicyEngineResult::Exit(crate::RunResult::SwapToSink)),
        "run_step from PrSwapToSinkStartup must yield Exit(SwapToSink), got {:?}",
        result
    );
}

/// Per PD 3.2 Sec. 8.3.3.2.3: the GoodCRC-acked caps send resets HardResetCounter
/// and CapsCounter. 60 cycles of failed + successful renegotiations must never saturate.
#[tokio::test]
async fn test_caps_counter_resets_on_goodcrc() {
    let driver = DummyDriver::new();
    let device = DummySourceDevice;
    let mut policy_engine =
        Source::<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DummySourceDevice>::new(driver, device, false);

    policy_engine.run_step().await.unwrap(); // `Startup` -> `SendCapabilities`

    // Per cycle, the source's tx counter advances by 4: failed caps send (retry-exhaustion
    // counts as one send per 6.12.2.2.1.9), successful caps send, Accept, PS_RDY.
    // GoodCRC ids echo those tx ids. Sink->source rx ids just need to be fresh
    // (a repeated id is treated as a retransmission and dropped), so an independent
    // rx id counter is used.
    let mut rx_id: u8 = 0;
    for cycle in 0..60u8 {
        let caps_tx = 4 * cycle + 1; // tx id of the successful caps send
        let accept_tx = 4 * cycle + 2; // tx id of Accept
        let ps_rdy_tx = 4 * cycle + 3; // tx id of PS_RDY

        // === Phase 1 of the cycle: a FAILED caps transmission ===
        // No GoodCRC is injected: the send exhausts retries -> Discovery.
        // Then Discovery re-arms and we land back in SendCapabilities.
        policy_engine.run_step().await.unwrap(); // SendCapabilities (tx fails) -> Discovery
        assert!(
            matches!(policy_engine.state, State::Discovery),
            "cycle {cycle}: after failed send expected Discovery, got {:?}",
            policy_engine.state
        );
        policy_engine.run_step().await.unwrap(); // Discovery -> SendCapabilities
        assert!(matches!(policy_engine.state, State::SendCapabilities));

        // === Phase 2 of the cycle: a successful, GoodCRC-acked renegotiation ===
        rx_id = rx_id.wrapping_add(1);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, caps_tx % 8);
        simulate_sink_request(&mut policy_engine, rx_id % 8, false);

        // SendCapabilities -> get Request -> NegotiateCapability
        policy_engine.run_step().await.unwrap();
        assert!(
            matches!(policy_engine.state, State::NegotiateCapability(_)),
            "cycle {cycle}: after Request expected NegotiateCapability, got {:?}",
            policy_engine.state
        );

        // GoodCRCs for the coming Accept and PS_RDY.
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, accept_tx % 8);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, ps_rdy_tx % 8);

        // NegotiateCapability -> (DPM Accept) -> TransitionSupply
        policy_engine.run_step().await.unwrap();
        assert!(
            matches!(policy_engine.state, State::TransitionSupply(_)),
            "cycle {cycle}: expected TransitionSupply, got {:?}",
            policy_engine.state
        );

        // TransitionSupply -> Accept -> transition -> PS_RDY -> Ready
        policy_engine.run_step().await.unwrap();
        assert!(
            matches!(policy_engine.state, State::Ready),
            "cycle {cycle}: after PS_RDY expected Ready, got {:?}",
            policy_engine.state
        );

        // Spec 8.3.3.2.3: the GoodCRC-acked caps send must have reset CapsCounter.
        // Force the next caps re-send via Get_Source_Cap and probe the counter.
        rx_id = rx_id.wrapping_add(1);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GetSourceCap, rx_id % 8);
        policy_engine.run_step().await.unwrap();
        assert!(
            matches!(policy_engine.state, State::SendCapabilities),
            "cycle {cycle}: after Get_Source_Cap expected SendCapabilities, got {:?}",
            policy_engine.state
        );
        assert_eq!(
            policy_engine.caps_counter.value(),
            0,
            "cycle {cycle}: CapsCounter must be reset by GoodCRC handling (spec 8.3.3.2.3), got {}",
            policy_engine.caps_counter.value()
        );
        assert!(
            !matches!(policy_engine.state, State::Disabled),
            "CapsCounter saturated after {cycle} cycles (spec: reset on GoodCRC)"
        );

        // Drop pending transmitted messages (our own frames + partner GoodCRC acks).
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            let _ = policy_engine.protocol_layer.driver().probe_transmitted_data();
        }

        // Drop the explicit contract: the next cycle's failed send must go to Discovery
        // (not-PD-connected escalation) rather than the with-contract NoResponse path.
        policy_engine.contract = crate::Contract::default();
    }
}

/// With an explicit contract, repeated tx failures escalate: NoResponseTimer timeout
/// leads to `PE_SRC_Disabled` (PD 3.2 Sec. 8.3.3.2.3), not an endless Discovery cycle.
#[tokio::test]
async fn test_no_response_timer_contract_silent_partner_error_recovers() {
    let driver = DummyDriver::new();
    let device = DummySourceDevice;
    let mut policy_engine =
        Source::<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DummySourceDevice>::new(driver, device, false);

    // Set up an explicit contract (silent partner scenario: charger negotiated, then stopped
    // responding on the wire).
    policy_engine.state = State::SendCapabilities;
    policy_engine.contract = crate::Contract::Explicit(
        crate::protocol_layer::message::data::request::PowerSource::FixedVariableSupply(
            crate::protocol_layer::message::data::request::FixedVariableSupply(0),
        ),
    );

    // The partner never acks: every caps send fails (retries exhausted -> tx error).
    // With an explicit contract, the NoResponseTimer must expire and escalate to
    // ErrorRecovery. The first step enters the state; a later step must exit the run
    // with `ReconnectionRequired`.
    let mut reconnection_required = false;
    for _ in 0..10 {
        match policy_engine.run_step().await {
            Ok(_) => continue,
            Err(super::Error::ReconnectionRequired) => {
                reconnection_required = true;
                break;
            }
            Err(err) => panic!("unexpected error from silent partner escalation: {err:?}"),
        }
    }

    assert!(
        reconnection_required,
        "with-contract silent partner must escalate to ErrorRecovery via NoResponseTimer; \
         last state: {:?}",
        policy_engine.state
    );
}

/// Per PD 3.2 Sec. 6.4.10.3.3 / 6.4.9: an `EPR_Request` not pointing at an EPR PDO
/// of the advertised EPR capabilities must not be negotiated.
#[tokio::test]
async fn test_invalid_epr_request_is_not_negotiated() {
    let driver = DummyDriver::new();
    let device = DummySourceDevice;
    let mut policy_engine =
        Source::<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DummySourceDevice>::new(driver, device, false);

    // Receive an EPR_Request while in SPR mode: per Sec. 6.5.15.1, EPR_Request is
    // only valid in EPR mode -> protocol error (must not be negotiated as power demand).
    policy_engine.state = State::Ready;

    // Invalid: object position 3 (SPR positions 1..=7 are invalid in EPR mode,
    // per Sec. 6.4.10.3.3), with an SPR PDO copy.
    let epr_request = PowerSource::EprRequest(EprRequestDataObject {
        rdo: (3u32 << 28) | 500, // object position 3
        pdo: crate::protocol_layer::message::data::source_capabilities::parse_raw_pdo(
            (5u32 << 20) | (2400 << 10), // FixedSupply 5V PDO copy
        ),
    });

    let header = Header::new_data(
        get_sink_header_template(),
        Counter::new_from_value(CounterType::MessageId, 0),
        DataMessageType::EprRequest,
        2,
    );
    let message = Message::new_with_data(header, Data::Request(epr_request));
    let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];
    let len = message.to_bytes(&mut buf);
    policy_engine.protocol_layer.driver().inject_received_data(&buf[..len]);

    policy_engine.run_step().await.unwrap();

    // The request must NOT reach a negotiation transition.
    assert!(
        !matches!(
            policy_engine.state,
            State::NegotiateCapability(_) | State::TransitionSupply(_)
        ),
        "EPR_Request outside EPR mode must not be negotiated; got {:?}",
        policy_engine.state
    );
}

/// In EPR mode, an `EPR_Request` for a non-existent EPR PDO (PDO copy mismatch) must
/// not be negotiated (PD 3.2 Sec. 6.4.9 / 6.5.15.1).
#[tokio::test]
async fn test_epr_request_pdo_copy_mismatch_not_negotiated() {
    let driver = DummyDriver::new();
    let device = DummySourceDevice;
    let mut policy_engine =
        Source::<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DummySourceDevice>::new(driver, device, false);

    policy_engine.state = State::Ready;
    policy_engine.mode = super::Mode::Epr;

    // The default DPM's EPR caps are vSafe5V-only (no EPR PDOs), so any position 8..=14
    // request references an EPR PDO that does not exist — including with a plausible PDO copy.
    let epr_request = PowerSource::EprRequest(EprRequestDataObject {
        rdo: (8u32 << 28) | 150,
        pdo: crate::protocol_layer::message::data::source_capabilities::parse_raw_pdo(
            (0b01u32 << 30) | (2000 << 17) | (1000 << 8) | 150,
        ),
    });

    let header = Header::new_data(
        get_sink_header_template(),
        Counter::new_from_value(CounterType::MessageId, 0),
        DataMessageType::EprRequest,
        2,
    );
    let message = Message::new_with_data(header, Data::Request(epr_request));
    let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];
    let len = message.to_bytes(&mut buf);
    policy_engine.protocol_layer.driver().inject_received_data(&buf[..len]);

    policy_engine.run_step().await.unwrap();

    // The validation failure path answers Not_Supported (Sec. 6.4.9); the request
    // must land exactly there, never in negotiation.
    assert!(
        matches!(policy_engine.state, State::SendNotSupported),
        "EPR_Request referencing a non-existent EPR PDO must be answered with Not_Supported; got {:?}",
        policy_engine.state
    );
}
