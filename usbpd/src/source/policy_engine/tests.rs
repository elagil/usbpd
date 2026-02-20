//! Tests for the policy engine.

use super::Source;
use crate::counters::{Counter, CounterType};
use crate::dummy::{
    DummyDriver, DummySourceDevice, DummyTimer, MAX_DATA_MESSAGE_SIZE, get_dummy_source_capabilities,
    get_source_capability_request,
};
use crate::protocol_layer::message::Message;
use crate::protocol_layer::message::data::Data;
use crate::protocol_layer::message::data::request::{CurrentRequest, FixedVariableSupply, PowerSource, VoltageRequest};
use crate::protocol_layer::message::data::source_capabilities::SourceCapabilities;
use crate::protocol_layer::message::header::{ControlMessageType, DataMessageType, Header, MessageType};
use crate::source::device_policy_manager::CapabilityResponse;
use crate::source::policy_engine::{DataRoleSwap, FastPowerRoleSwap, PowerRoleSwap, State, SwapState};

fn get_policy_engine() -> Source<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DummySourceDevice> {
    Source::new(DummyDriver::new(), DummySourceDevice {}, false)
}

fn simulate_sink_control_message<DPM: crate::source::device_policy_manager::DevicePolicyManager>(
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

fn simulate_sink_request<DPM: crate::source::device_policy_manager::DevicePolicyManager>(
    policy_engine: &mut Source<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DPM>,
    message_id: u8,
    highest_power: bool,
) {
    let message = request_capability_message(message_id, highest_power);
    let mut buf = [0u8; MAX_DATA_MESSAGE_SIZE];
    let len = message.to_bytes(&mut buf);

    policy_engine.protocol_layer.driver().inject_received_data(&buf[..len]);
}

#[tokio::test]
async fn test_negotiation() {
    let mut policy_engine = get_policy_engine();

    eprintln!("\n<== Starting initial source SPR negotiation test! ==>\n");
    {
        // Instantiated in `SendCapabilities` state (not started from role-swap)
        eprintln!("- Ran Step 0: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::Startup { role_swap: false }));

        // `Startup` -> `SendCapabilities`
        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 1: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::SendCapabilities));

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

            eprintln!("- Ran Step 2: {0:?}", policy_engine.state);
            assert!(matches!(policy_engine.state, State::NegotiateCapability(_)));
        }

        // `NegotiateCapability` -> DPM: Accept -> `Transition Supply`
        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 3: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::TransitionSupply(_)));

        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 1);

        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 2);

        // `TransitionSupply` -> Accept -> PsRdy -> `Ready`
        policy_engine.run_step().await.unwrap();
        {
            while policy_engine.protocol_layer.driver().has_transmitted_data() {
                let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
                eprintln!("- TransitionSupply message: {:?}", msg.header.message_type());
            }

            eprintln!("- Ran Step 4: {0:?}", policy_engine.state);
            assert!(matches!(policy_engine.state, State::Ready));
        }
    }
    eprintln!("\n==> Finished initial source SPR negotiation test! <==\n");

    eprintln!("\n<== Starting source SPR re-negotiation test! ==>\n");
    {
        // Simulate `GoodCrc` to SourceCapabilities
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GetSourceCap, 3);

        // `Ready` -> GetSourceCap Received -> `SendCapabilities`
        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 1: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::SendCapabilities));

        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3);

        // Request highest power
        simulate_sink_request(&mut policy_engine, 4, true);

        // `SendCapabilities` -> Get Request -> `NegotiateCapability`
        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 2: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::NegotiateCapability(_)));

        // `NegotiateCapability` -> Request Can Be Met -> `TransitionSupply`
        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 3: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::TransitionSupply(_)));

        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 4);

        // Simulate `GoodCrc`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 5);

        // `TransitionSupply` -> Accept -> PsRdy -> `Ready`
        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 4: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::Ready));

        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            eprintln!("- Re-negotiation test message: {:?}", msg.header.message_type());
        }
    }
    eprintln!("\n==> Finished source SPR re-negotiation test! <==\n");

    eprintln!("\n==> Starting source soft reset - reject test! <==\n");
    {
        // Simulate `SoftReset`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::SoftReset, 6);

        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 1: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::SoftReset));

        // `GoodCrc` for `Accept`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 0);

        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 2: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::SendCapabilities));

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
        eprintln!("- Ran Step 3: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::NegotiateCapability(_)));

        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 4: {0:?}", policy_engine.state);
        assert!(matches!(
            policy_engine.state,
            State::CapabilityResponse(CapabilityResponse::Reject)
        ));

        // `GoodCrc` for `Reject`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 2);

        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 5: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::Ready));

        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            eprintln!("- SoftReset-Reject message: {:?}", msg.header.message_type());
        }
    }
    eprintln!("\n==> Finished source soft reset - reject test! <==\n");
}

#[tokio::test]
async fn test_discovery() {
    const MAX_DISCOVERY_ITERS: usize = 52;
    let mut policy_engine = get_policy_engine();
    // HardReset -> Discovery -> Disabled
    eprintln!("\n<== Starting source discovery test! ==>\n");
    {
        eprintln!("- Ran Step 0: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::Startup { role_swap: false }));

        // `Startup` -> `SendCapabilities`
        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 1: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::SendCapabilities));

        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 0);

        // `SendCapabilities` -> Capability Send Failure -> `Discovery`
        policy_engine.run_step().await.unwrap();
        eprintln!("- Ran Step 2: {0:?}", policy_engine.state);
        assert!(matches!(policy_engine.state, State::Discovery));

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
        eprintln!(
            "- Ran Step 3: {0:?}, discovery state iterations = {1:?}",
            policy_engine.state, discovery_iterations
        );
        assert!(matches!(policy_engine.state, State::Disabled));
    }
    eprintln!("\n==> Finished source discovery test! <==\n");
}

/// Return a policy engine that has already undergone initial power contract negotiation
async fn get_drp_policy_engine_at_ready() -> Source<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DummySourceDevice> {
    let mut policy_engine = Source::new_dual_role(DummyDriver::new(), DummySourceDevice {}, false);
    assert!(matches!(policy_engine.state, State::Startup { role_swap: false }));

    simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 0);
    simulate_sink_request(&mut policy_engine, 1, false);
    simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 1);
    simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 2);

    policy_engine.run_step().await.unwrap(); // `Startup` -> `SendCapabilities`
    policy_engine.run_step().await.unwrap(); // `SendCapabilities` -> Get Request -> `NegotiateCapability`
    policy_engine.run_step().await.unwrap(); // `NegotiateCapability` -> DPM: Accept -> `Transition Supply`
    policy_engine.run_step().await.unwrap(); // `TransitionSupply` -> Accept -> PsRdy -> `Ready`

    // Flush all messages
    while policy_engine.protocol_layer.driver().has_transmitted_data() {
        let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
        eprintln!("- Initial negotiation: {:?}", msg.header.message_type());
    }

    assert!(matches!(policy_engine.state, State::Ready));

    policy_engine
}

#[tokio::test]
async fn test_role_swapping() {
    eprintln!("\n<== Starting source power role swap test! ==>\n");
    {
        let mut policy_engine = get_drp_policy_engine_at_ready().await;

        simulate_sink_control_message(&mut policy_engine, ControlMessageType::PrSwap, 2);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3); // `GoodCrc` for `Accept`
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 4); // `GoodCrc` for `PS_RDY` 3 OR 4??
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::PsRdy, 4);

        // `Ready` -> PR_Swap Message Received -> `DrpSwap(SwapState::Power(PowerRoleSwap::Evaluate))`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::Evaluate)),
            1,
        )
        .await;

        /// `PowerRoleSwap::Evaluate` -> DRP Evaluates to `true` -> `PowerRoleSwap::Accept`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::Accept)),
            2,
        )
        .await;

        // `PowerRoleSwap::Accept` -> `Accept` Sent -> `PowerRoleSwap::TransitionToOff`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::TransitionToOff)),
            3,
        )
        .await;

        // `PowerRoleSwap::TransitionToOff` -> DPM turned source off -> `PowerRoleSwap::AssertRd`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::AssertRd)),
            4,
        )
        .await;

        // `PowerRoleSwap::AssertRd` -> DPM asserted Rd -> `PowerRoleSwap::WaitSourceOn`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Power(PowerRoleSwap::WaitSourceOn)),
            5,
        )
        .await;

        // `PowerRoleSwap::WaitSourceOn` -> `PS_RDY` Sent, `PS_RDY` Received -> `PrSwapToSinkStartup`
        run_test_step(&mut policy_engine, &State::PrSwapToSinkStartup, 6).await;

        // Flush all messages
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            eprintln!("- Power Role Swap Message: {:?}", msg.header.message_type());
        }
    }
    eprintln!("\n==> Finished source power role swap test! <==\n");

    eprintln!("\n<== Starting source fast power role swap test! ==>\n");
    {
        let mut policy_engine = get_drp_policy_engine_at_ready().await;

        simulate_sink_control_message(&mut policy_engine, ControlMessageType::FrSwap, 2);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3); // `GoodCrc` for `Accept` 3 OR 4??
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 4); // `GoodCrc` for `PS_RDY` 3 OR 4??
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::PsRdy, 4);

        // `Ready` -> FR_Swap Message Received -> `FastPowerRole::Evaluate`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::Evaluate)),
            1,
        )
        .await;

        // `FastPowerRole::Evaluate` -> DPM: Fast Role Swap Signaled -> `FastPowerRole::Accept`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::Accept)),
            2,
        )
        .await;

        // `FastPowerRole::Accept` -> `Accept` sent -> `FastPowerRole::TransitionToOff`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::TransitionToOff)),
            3,
        )
        .await;

        // `PowerRoleSwap::TransitionToOff` -> DPM turned source off -> `PowerRoleSwap::AssertRd`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::AssertRd)),
            4,
        )
        .await;

        // `PowerRoleSwap::AssertRd` -> DPM asserted Rd -> `PowerRoleSwap::WaitSourceOn`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::FastPower(FastPowerRoleSwap::WaitSourceOn)),
            5,
        )
        .await;

        // `PowerRoleSwap::WaitSourceOn` -> `PS_RDY` Sent, `PS_RDY` Received -> `PrSwapToSinkStartup`
        run_test_step(&mut policy_engine, &State::PrSwapToSinkStartup, 6);

        // Flush all messages
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            eprintln!("- Fast Power Role Swap Message: {:?}", msg.header.message_type());
        }
    }

    eprintln!("\n<== Starting source data role swap test! ==>\n");
    {
        let mut policy_engine = get_drp_policy_engine_at_ready().await;

        simulate_sink_control_message(&mut policy_engine, ControlMessageType::DrSwap, 2);
        simulate_sink_control_message(&mut policy_engine, ControlMessageType::GoodCRC, 3); // `GoodCrc` for `Accept`

        // `Ready` -> DR_Swap Message Received -> `DataRoleSwap::Evaluate`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Data(DataRoleSwap::Evaluate)),
            1,
        )
        .await;

        // `DataRoleSwap::Evaluate` -> DRP: Data Role Swap ok -> `DataRoleSwap::Accept`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Data(DataRoleSwap::Accept)),
            2,
        )
        .await;

        // `DataRoleSwap::Accept` -> `Accept` sent -> `DataRoleSwap::Change`
        run_test_step(
            &mut policy_engine,
            &State::DrpSwap(SwapState::Data(DataRoleSwap::Change)),
            3,
        )
        .await;

        // `DataRoleSwap::Change` -> DPM: Port changed data role -> `Ready`
        run_test_step(&mut policy_engine, &State::Ready, 4).await;

        // Flush all messages
        while policy_engine.protocol_layer.driver().has_transmitted_data() {
            let msg = Message::from_bytes(&policy_engine.protocol_layer.driver().probe_transmitted_data()).unwrap();
            eprintln!("- Data Role Swap Message: {:?}", msg.header.message_type());
        }
    }
    eprintln!("\n==> Finished source data role swap test! <==\n");
}

async fn run_test_step<DPM: crate::source::device_policy_manager::DevicePolicyManager>(
    policy_engine: &mut Source<DummyDriver<MAX_DATA_MESSAGE_SIZE>, DummyTimer, DPM>,
    new_state: &State,
    step_number: usize,
) {
    policy_engine.run_step().await.unwrap();
    eprintln!("- Ran Step {step_number}: {0:?}", policy_engine.state);
    if !matches!(&policy_engine.state, new_state) {
        error!(
            "Policy Engine State {0:?} did not match assumed new state {1:?}",
            policy_engine.state, new_state
        );
    }
}
